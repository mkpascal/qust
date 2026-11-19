// The plugin thread: a thread of its own with a hidden window and a message loop (plugins' windows
// and their timers need one), in a single-threaded apartment with OLE (their drag and drop). Every
// plugin is made, set up, edited and dropped here; only `process` runs elsewhere, in the chain.
//
// Commands arrive through a channel, with a message to the hidden window to wake the loop. The
// host's state is a thread-local borrowed only while handling a command or a tick; anything a
// plugin may call during that (its handler, its frame, its window) keeps to its own objects, and
// a message that arrives meanwhile (a plugin running a modal loop) waits for the next turn.

mod chain;
mod editor;
mod instance;
mod module;
mod objects;

use crate::bundle;
use crate::saved::{self, Saved};
use crate::{PluginRow, PluginState, Snapshot};
use chain::Chain;
use instance::Instance;
use module::Module;
use objects::HostApp;
use qust_audio::EffectSlot;
use qust_player::Store;
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::mpsc;
use std::sync::Arc;
use std::time::{Duration, Instant};
use vst3::ComWrapper;
use windows::core::PCWSTR;
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Ole::{OleInitialize, OleUninitialize};
use windows::Win32::UI::WindowsAndMessaging::*;

pub const CRASHED: &str = "Qust closed while starting this plugin, so it was turned off. Turn it on to try again.";

const CLASS: &str = "QustPluginHost";
const WM_COMMAND_READY: u32 = WM_APP + 1;
const WM_STOP: u32 = WM_APP + 3;
const TICK: Duration = Duration::from_millis(40);
// Settings changed in an editor are saved this long after the last change.
const SAVE_DELAY: Duration = Duration::from_secs(2);

pub enum Command {
    Add(Vec<PathBuf>),
    Remove(u64),
    Move(u64, usize),
    Enable(u64, bool),
    Edit(u64, Option<isize>),
    Dark(bool),
    Shutdown(mpsc::Sender<()>),
}

#[derive(Clone)]
pub struct Handle {
    tx: mpsc::Sender<Command>,
    window: isize,
}

impl Handle {
    pub fn start(store: Store, slot: Arc<EffectSlot>, on_change: Box<dyn Fn(Snapshot) + Send>) -> Option<Handle> {
        let (tx, rx) = mpsc::channel();
        let (ready_tx, ready_rx) = mpsc::channel::<Option<isize>>();
        let spawned = std::thread::Builder::new().name("qust-plugins".into()).spawn(move || run(store, slot.clone(), on_change, rx, ready_tx));
        if spawned.is_err() {
            return None;
        }
        let window = ready_rx.recv().ok().flatten()?;
        let handle = Handle { tx, window };
        Some(handle)
    }

    pub fn send(&self, command: Command) {
        if self.tx.send(command).is_ok() {
            unsafe {
                let _ = PostMessageW(Some(HWND(self.window as *mut _)), WM_COMMAND_READY, WPARAM(0), LPARAM(0));
            }
        }
    }

    pub fn shutdown(&self) {
        let (tx, rx) = mpsc::channel();
        self.send(Command::Shutdown(tx));
        let _ = rx.recv_timeout(Duration::from_secs(5));
    }
}

struct Plugin {
    id: u64,
    saved: Saved,
    state: PluginState,
    instance: Option<Instance>,
    save_at: Option<Instant>,
}

struct Host {
    store: Store,
    dir: PathBuf,
    slot: Arc<EffectSlot>,
    on_change: Box<dyn Fn(Snapshot) + Send>,
    rx: mpsc::Receiver<Command>,
    window: isize,
    app: ComWrapper<HostApp>,
    plugins: Vec<Plugin>,
    next_id: u64,
    rate: Option<u32>,
    dark: bool,
    notice: Option<String>,
    // Loaded binaries, by path: kept until Qust exits.
    modules: HashMap<PathBuf, Rc<Module>>,
    last: Option<Snapshot>,
    stopping: bool,
    // The tick runs only while plugins do: an idle Qust in the tray isn't woken for nothing.
    ticking: bool,
}

thread_local! {
    static HOST: RefCell<Option<Host>> = const { RefCell::new(None) };
}

fn instance() -> HINSTANCE {
    unsafe { GetModuleHandleW(None).map(|module| HINSTANCE(module.0)).unwrap_or_default() }
}

fn run(store: Store, slot: Arc<EffectSlot>, on_change: Box<dyn Fn(Snapshot) + Send>, rx: mpsc::Receiver<Command>, ready: mpsc::Sender<Option<isize>>) {
    unsafe {
        let ole = OleInitialize(None).is_ok();
        let class: Vec<u16> = CLASS.encode_utf16().chain(Some(0)).collect();
        let wnd_class = WNDCLASSW { lpfnWndProc: Some(proc), hInstance: instance(), lpszClassName: PCWSTR(class.as_ptr()), ..Default::default() };
        RegisterClassW(&wnd_class);
        let Ok(window) = CreateWindowExW(WINDOW_EX_STYLE::default(), PCWSTR(class.as_ptr()), PCWSTR::null(), WS_OVERLAPPED, 0, 0, 0, 0, Some(HWND_MESSAGE), None, Some(instance()), None) else {
            let _ = ready.send(None);
            return;
        };
        let raw = window.0 as isize;
        let mut host = Host::new(store, slot.clone(), on_change, rx, raw);
        host.recover();
        host.emit();
        HOST.with(|cell| *cell.borrow_mut() = Some(host));
        // Engines started and stopped from now on (and the one running, at once).
        let tx_window = raw;
        let (engine_tx, engine_rx) = mpsc::channel::<Option<u32>>();
        slot.on_engine(move |rate| {
            if engine_tx.send(rate).is_ok() {
                let _ = PostMessageW(Some(HWND(tx_window as *mut _)), WM_COMMAND_READY, WPARAM(0), LPARAM(0));
            }
        });
        ENGINE_RX.with(|cell| *cell.borrow_mut() = Some(engine_rx));
        let _ = ready.send(Some(raw));
        let mut message = MSG::default();
        while GetMessageW(&mut message, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }
        let _ = DestroyWindow(window);
        // The binaries stay mapped: their code may still be referenced as the process ends.
        HOST.with(|cell| {
            if let Some(host) = cell.borrow_mut().take() {
                std::mem::forget(host.modules);
            }
        });
        if ole {
            OleUninitialize();
        }
    }
}

thread_local! {
    static ENGINE_RX: RefCell<Option<mpsc::Receiver<Option<u32>>>> = const { RefCell::new(None) };
}

// Runs `f` on the host, unless it is busy (a plugin pumping messages inside a call): then the
// work waits for the call to finish, which drains the channels again.
fn with_host(f: impl FnOnce(&mut Host)) {
    HOST.with(|cell| {
        if let Ok(mut host) = cell.try_borrow_mut() {
            if let Some(host) = host.as_mut() {
                f(host);
            }
        }
    });
}

unsafe extern "system" fn proc(window: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match message {
        WM_COMMAND_READY => {
            with_host(|host| host.drain());
            LRESULT(0)
        }
        WM_TIMER => {
            with_host(|host| {
                host.drain();
                host.tick();
            });
            LRESULT(0)
        }
        editor::WM_EDITOR_CLOSED => {
            let plugin = wparam.0 as u64;
            with_host(|host| host.save_now(plugin));
            LRESULT(0)
        }
        WM_STOP => {
            PostQuitMessage(0);
            LRESULT(0)
        }
        _ => DefWindowProcW(window, message, wparam, lparam),
    }
}

impl Host {
    fn new(store: Store, slot: Arc<EffectSlot>, on_change: Box<dyn Fn(Snapshot) + Send>, rx: mpsc::Receiver<Command>, window: isize) -> Host {
        let dir = saved::state_dir(&store);
        let plugins = saved::load(&store)
            .into_iter()
            .enumerate()
            .map(|(index, saved)| {
                let state = if saved.crashed { PluginState::Failed(CRASHED.into()) } else { PluginState::Off };
                Plugin { id: index as u64 + 1, saved, state, instance: None, save_at: None }
            })
            .collect::<Vec<_>>();
        Host {
            next_id: plugins.len() as u64 + 1,
            store,
            dir,
            slot,
            on_change,
            rx,
            window,
            app: ComWrapper::new(HostApp),
            plugins,
            rate: None,
            dark: false,
            notice: None,
            modules: HashMap::new(),
            last: None,
            stopping: false,
            ticking: false,
        }
    }

    // A plugin that was starting when Qust last closed made it close: off until turned on again.
    fn recover(&mut self) {
        let Some(key) = self.store.get(saved::LOADING).and_then(|value| value.as_str().map(str::to_string)) else { return };
        self.store.remove(saved::LOADING);
        if let Some(plugin) = self.plugins.iter_mut().find(|plugin| plugin.saved.key == key) {
            plugin.saved.enabled = false;
            plugin.saved.crashed = true;
            plugin.state = PluginState::Failed(CRASHED.into());
            self.persist();
        }
    }

    fn persist(&self) {
        let list: Vec<Saved> = self.plugins.iter().map(|plugin| plugin.saved.clone()).collect();
        saved::save(&self.store, &list);
    }

    fn snapshot(&self) -> Snapshot {
        Snapshot {
            plugins: self
                .plugins
                .iter()
                .map(|plugin| PluginRow {
                    id: plugin.id,
                    name: plugin.saved.name.clone(),
                    vendor: plugin.saved.vendor.clone(),
                    path: plugin.saved.path.clone(),
                    enabled: plugin.saved.enabled,
                    state: plugin.state.clone(),
                    editor_open: editor::is_open(plugin.id),
                })
                .collect(),
            notice: self.notice.clone(),
        }
    }

    fn emit(&mut self) {
        let running = self.plugins.iter().any(|plugin| plugin.instance.is_some());
        if running != self.ticking {
            self.ticking = running;
            let window = Some(HWND(self.window as *mut _));
            unsafe {
                if running {
                    SetTimer(window, 1, TICK.as_millis() as u32, None);
                } else {
                    let _ = KillTimer(window, 1);
                }
            }
        }
        let snapshot = self.snapshot();
        if self.last.as_ref() != Some(&snapshot) {
            (self.on_change)(snapshot.clone());
            self.last = Some(snapshot);
        }
    }

    fn drain(&mut self) {
        loop {
            let engine = ENGINE_RX.with(|cell| cell.borrow().as_ref().and_then(|rx| rx.try_recv().ok()));
            if let Some(rate) = engine {
                self.engine(rate);
                continue;
            }
            match self.rx.try_recv() {
                Ok(command) => self.handle(command),
                Err(_) => break,
            }
        }
        self.emit();
    }

    fn handle(&mut self, command: Command) {
        if !matches!(command, Command::Dark(_) | Command::Shutdown(_)) {
            self.notice = None;
        }
        match command {
            Command::Add(paths) => self.add(paths),
            Command::Remove(id) => self.remove(id),
            Command::Move(id, index) => self.move_to(id, index),
            Command::Enable(id, on) => self.enable(id, on),
            Command::Edit(id, near) => self.edit(id, near),
            Command::Dark(dark) => {
                self.dark = dark;
                editor::set_dark(dark);
            }
            Command::Shutdown(reply) => {
                self.stopping = true;
                self.engine(None);
                let _ = reply.send(());
                unsafe {
                    let _ = PostMessageW(Some(HWND(self.window as *mut _)), WM_STOP, WPARAM(0), LPARAM(0));
                }
            }
        }
        self.emit();
    }

    /* The engine */

    fn engine(&mut self, rate: Option<u32>) {
        if rate.is_some() && self.stopping {
            return;
        }
        if self.rate.is_some() && rate != self.rate {
            self.unload_all();
        }
        self.rate = rate;
        if rate.is_some() {
            let ids: Vec<u64> = self.plugins.iter().filter(|plugin| plugin.saved.enabled && plugin.instance.is_none()).map(|plugin| plugin.id).collect();
            for id in ids {
                self.load(id);
            }
            self.rebuild();
        }
        self.emit();
    }

    // Saves and lets go of every plugin: the engine is gone (the player window closed).
    fn unload_all(&mut self) {
        editor::close_all();
        // Out of the chain first: then nothing runs them.
        drop(self.slot.set(None));
        let dir = self.dir.clone();
        for plugin in &mut self.plugins {
            if let Some(instance) = plugin.instance.take() {
                if let Some((component, controller)) = instance.save() {
                    saved::write_state(&dir, &plugin.saved.key, &component, &controller);
                }
            }
            plugin.save_at = None;
            // A plugin that failed to start tries again with the next engine.
            plugin.state = if plugin.saved.crashed { PluginState::Failed(CRASHED.into()) } else { PluginState::Off };
        }
    }

    fn module(&mut self, binary: &Path) -> Result<Rc<Module>, String> {
        if let Some(module) = self.modules.get(binary) {
            return Ok(module.clone());
        }
        let module = Rc::new(Module::load(binary, &self.app)?);
        self.modules.insert(binary.to_path_buf(), module.clone());
        Ok(module)
    }

    // Starts a plugin for the running engine. Its state comes from its file.
    fn load(&mut self, id: u64) {
        let Some(rate) = self.rate else { return };
        let Some(index) = self.plugins.iter().position(|plugin| plugin.id == id) else { return };
        if self.plugins[index].instance.is_some() {
            return;
        }
        self.plugins[index].state = PluginState::Loading;
        self.emit();
        let saved = self.plugins[index].saved.clone();
        // Marked while it starts: if this takes Qust down, the next launch turns it off.
        self.store.set(saved::LOADING, serde_json::json!(saved.key));
        let made = bundle::locate(&saved.path).and_then(|located| self.module(&located.binary)).and_then(|module| {
            let name = module.class.name.clone();
            let vendor = module.class.vendor.clone();
            let state = saved::read_state(&self.dir, &saved.key);
            Instance::create(module, &self.app, rate, state).map(|instance| (instance, name, vendor))
        });
        self.store.remove(saved::LOADING);
        let plugin = &mut self.plugins[index];
        match made {
            Ok((instance, name, vendor)) => {
                plugin.instance = Some(instance);
                plugin.state = PluginState::Ready;
                if (!name.is_empty() && name != plugin.saved.name) || vendor != plugin.saved.vendor {
                    if !name.is_empty() {
                        plugin.saved.name = name;
                    }
                    plugin.saved.vendor = vendor;
                    self.persist();
                }
            }
            Err(error) => plugin.state = PluginState::Failed(error),
        }
    }

    // The enabled, running plugins, in order, as the engine's chain.
    fn rebuild(&mut self) {
        if self.rate.is_none() {
            drop(self.slot.set(None));
            return;
        }
        let stages: Vec<_> = self.plugins.iter().filter(|plugin| plugin.saved.enabled).filter_map(|plugin| plugin.instance.as_ref()).map(Instance::stage).collect();
        let chain: Option<Box<dyn qust_audio::Effect>> = if stages.is_empty() { None } else { Some(Box::new(Chain { stages })) };
        // The old chain comes back here, to be dropped on this thread.
        drop(self.slot.set(chain));
    }

    /* The list */

    fn add(&mut self, paths: Vec<PathBuf>) {
        let mut failures: Vec<(String, String)> = Vec::new();
        let mut added = Vec::new();
        for path in paths {
            let found = bundle::locate(&path).and_then(|located| self.module(&located.binary).map(|module| (located, module)));
            match found {
                Ok((located, module)) => {
                    let name = if module.class.name.is_empty() { bundle::display_name(&located.bundle) } else { module.class.name.clone() };
                    let id = self.next_id;
                    self.next_id += 1;
                    self.plugins.push(Plugin { id, saved: Saved::new(located.bundle, name, module.class.vendor.clone()), state: PluginState::Off, instance: None, save_at: None });
                    added.push(id);
                }
                Err(error) => failures.push((bundle::display_name(&path), error)),
            }
        }
        if !added.is_empty() {
            self.persist();
            for id in added {
                self.load(id);
            }
            self.rebuild();
        }
        self.notice = match failures.as_slice() {
            [] => None,
            [(name, error)] => Some(format!("Couldn't add \u{201c}{name}\u{201d}. {error}")),
            [(name, error), rest @ ..] => Some(format!("Couldn't add \u{201c}{name}\u{201d} and {} more. {error}", rest.len())),
        };
    }

    fn remove(&mut self, id: u64) {
        let Some(index) = self.plugins.iter().position(|plugin| plugin.id == id) else { return };
        editor::close(id);
        let plugin = self.plugins.remove(index);
        self.rebuild();
        drop(plugin.instance);
        saved::remove_state(&self.dir, &plugin.saved.key);
        self.persist();
    }

    fn move_to(&mut self, id: u64, index: usize) {
        let Some(from) = self.plugins.iter().position(|plugin| plugin.id == id) else { return };
        let plugin = self.plugins.remove(from);
        let to = index.min(self.plugins.len());
        self.plugins.insert(to, plugin);
        if from != to {
            self.persist();
            self.rebuild();
        }
    }

    fn enable(&mut self, id: u64, on: bool) {
        let Some(plugin) = self.plugins.iter_mut().find(|plugin| plugin.id == id) else { return };
        if plugin.saved.enabled == on && !(on && plugin.saved.crashed) {
            return;
        }
        plugin.saved.enabled = on;
        if on && plugin.saved.crashed {
            plugin.saved.crashed = false;
            plugin.state = PluginState::Off;
        }
        // Turned on again after failing to start: one more try.
        if on && matches!(plugin.state, PluginState::Failed(_)) && plugin.instance.is_none() {
            plugin.state = PluginState::Off;
        }
        self.persist();
        if on {
            self.load(id);
        }
        self.rebuild();
    }

    fn edit(&mut self, id: u64, near: Option<isize>) {
        let Some(index) = self.plugins.iter().position(|plugin| plugin.id == id) else { return };
        if self.plugins[index].instance.is_none() {
            // Off plugins start when their window is asked for (they stay out of the chain).
            if self.rate.is_none() {
                return;
            }
            if self.plugins[index].saved.crashed {
                self.plugins[index].saved.crashed = false;
                self.persist();
            }
            self.load(id);
        }
        let plugin = &self.plugins[index];
        let Some(instance) = &plugin.instance else { return };
        let result = match &instance.controller {
            Some(controller) => editor::open(id, &plugin.saved.name, controller, near, self.dark, self.window),
            None => Err(editor::NO_WINDOW.into()),
        };
        if let Err(error) = result {
            self.notice = Some(format!("\u{201c}{}\u{201d}: {error}", plugin.saved.name));
        }
    }

    /* Every tick */

    fn tick(&mut self) {
        let now = Instant::now();
        let mut restart = Vec::new();
        for plugin in &mut self.plugins {
            let Some(instance) = &plugin.instance else { continue };
            instance.deliver_output();
            if instance.take_changed() {
                plugin.save_at = Some(now + SAVE_DELAY);
            }
            if instance.wants_restart() {
                restart.push(plugin.id);
            }
        }
        if !restart.is_empty() {
            // Out of the chain while they restart, back in after.
            let restarting: Vec<u64> = restart.clone();
            let stages: Vec<_> = self
                .plugins
                .iter()
                .filter(|plugin| plugin.saved.enabled && !restarting.contains(&plugin.id))
                .filter_map(|plugin| plugin.instance.as_ref())
                .map(Instance::stage)
                .collect();
            drop(self.slot.set(if stages.is_empty() { None } else { Some(Box::new(Chain { stages })) }));
            for plugin in self.plugins.iter_mut().filter(|plugin| restarting.contains(&plugin.id)) {
                if let Some(instance) = plugin.instance.as_mut() {
                    if let Err(error) = instance.restart() {
                        plugin.instance = None;
                        plugin.state = PluginState::Failed(error);
                    }
                }
            }
            self.rebuild();
        }
        let due: Vec<u64> = self.plugins.iter().filter(|plugin| plugin.save_at.is_some_and(|at| at <= now)).map(|plugin| plugin.id).collect();
        for id in due {
            self.save_now(id);
        }
        self.emit();
    }

    fn save_now(&mut self, id: u64) {
        let Some(plugin) = self.plugins.iter_mut().find(|plugin| plugin.id == id) else { return };
        plugin.save_at = None;
        if let Some((component, controller)) = plugin.instance.as_ref().and_then(Instance::save) {
            saved::write_state(&self.dir, &plugin.saved.key, &component, &controller);
        }
        self.emit();
    }
}
