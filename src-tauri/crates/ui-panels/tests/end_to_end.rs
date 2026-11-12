// The panels with the real player: `qust_player::Player` on the null output (no sound) with a
// temporary store, wired through `qust_ui::Bridge` as the app does, the services implemented on
// the bridge (menus answered here instead of shown). A change made in the Settings view must
// reach the player and the store; a generated file played must fill in Stream information.
//
//   cargo test -p qust-ui-panels --features capture --test end_to_end
//
// Opens two hidden windows (never shown, never focused) and draws them by hand.

use gpui::{point, px, size, App, AppContext, AsyncApp, Point, Window};
use qust_player::{OutputKind, Store};
use qust_shell::menu::{Menu, Position};
use qust_shell::recorder::Recorded;
use qust_ui::capture::{window_options, Session};
use qust_ui::{Background, Bridge, Playback};
use qust_ui_panels::{info_window, settings_window, InfoRequest, InfoView, MenuDone, PanelServices, SettingsView};
use serde_json::{json, Value};
use std::any::Any;
use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

/// The services on the bridge, as the app implements them (minus the shell's OS parts).
struct BridgeServices {
    bridge: Bridge,
    menu: RefCell<Option<MenuDone>>,
    log: RefCell<Vec<String>>,
}

impl PanelServices for BridgeServices {
    fn now(&self) -> u64 {
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
    }
    fn version(&self) -> String {
        "1.1.5".into()
    }
    fn save_setting(&self, key: &str, value: Value, _: &mut App) {
        self.bridge.player.write_setting(key, Some(value));
    }
    fn broadcast_setting(&self, key: &str, value: Value, cx: &mut App) {
        self.bridge.models.apply_setting(key, &value, cx);
    }
    fn set_theme(&self, theme: &str, cx: &mut App) {
        self.bridge.player.write_setting("theme", Some(json!(theme)));
        qust_ui::theme::apply(qust_ui::theme::ThemeChoice::parse(theme), cx);
    }
    fn pause_shortcuts(&self, paused: bool, _: &mut App) {
        self.log.borrow_mut().push(format!("pause_shortcuts {paused}"));
    }
    fn set_shortcut(&self, action: &str, accelerator: &str, _: &mut App) -> Result<(), String> {
        self.log.borrow_mut().push(format!("set_shortcut {action} {accelerator}"));
        Ok(())
    }
    fn capture_keys(&self, _: &Window, _: Box<dyn FnMut(Recorded)>) -> Option<Box<dyn Any>> {
        None
    }
    fn set_background(&self, _: bool, _: &mut App) -> Result<Background, String> {
        Err("The background helper works in the installed version of Qust".into())
    }
    fn check_for_updates(&self, _: &mut App) {}
    fn install_update(&self, _: &mut App) {}
    fn popup_menu(&self, _: &Window, _: Menu, _: Position, done: MenuDone, _: &mut App) {
        *self.menu.borrow_mut() = Some(done);
    }
    fn copy_text(&self, _: String, _: &mut App) {}
    fn stream_info(&self, url: String, _: &mut App) -> InfoRequest {
        let reply = self.bridge.player.stream_info(url);
        Box::pin(async move { reply.get().await })
    }
    fn close_window(&self, window: &mut Window, _: &mut App) {
        window.remove_window();
    }
}

#[derive(Default, Debug)]
struct Outcome {
    colors_in_player: Option<String>,
    colors_in_store: Option<Value>,
    autoplay_in_store: Option<Value>,
    reconnect_in_store: Option<Value>,
    theme_dark_after: Option<bool>,
    theme_in_store: Option<Value>,
    status: Option<Playback>,
    texts: Vec<(String, String)>,
    title: String,
    state: String,
}

async fn wait_until(cx: &mut AsyncApp, timeout: Duration, draw: &dyn Fn(&mut AsyncApp), mut done: impl FnMut(&mut AsyncApp) -> bool) -> bool {
    let start = Instant::now();
    while start.elapsed() < timeout {
        draw(cx);
        if done(cx) {
            return true;
        }
        cx.background_executor().timer(Duration::from_millis(20)).await;
    }
    false
}

fn main() {
    let dir = std::env::temp_dir().join(format!("qust-panels-e2e-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let wav = dir.join("tone.wav");
    qust_ui::diag::write_tone_wav(&wav, 20.0).expect("wav");
    let settings = dir.join("settings.json");
    std::fs::write(&settings, r#"{ "visualizer-colors": "classic", "volume": 0, "theme": "dark" }"#).expect("settings");
    let outcome = Rc::new(RefCell::new(Outcome::default()));
    let result = outcome.clone();

    qust_ui::application().run(move |cx: &mut App| {
        qust_ui::capture::init(cx);
        let store = Store::open(settings.clone());
        let bridge = Bridge::start(store.clone(), OutputKind::Null, cx);
        qust_ui::theme::apply(qust_ui::theme::ThemeChoice::Dark, cx);
        // What main.rs did: the player follows the settings the Settings window writes.
        let player = bridge.player.clone();
        // (QUST_NO_WATCH shows what happens without it: the player never hears.)
        let watch = std::env::var_os("QUST_NO_WATCH").is_none();
        store.watch(move |key, value| {
            if !watch {
                return;
            }
            let (key, value) = (key.to_string(), value.cloned());
            let _ = player.ask(move |p| p.setting_changed(&key, value));
        });
        bridge.player.attach();
        let services = Rc::new(BridgeServices { bridge: bridge.clone(), menu: RefCell::new(None), log: RefCell::new(Vec::new()) });
        let models = bridge.models.clone();
        let dyn_services: Rc<dyn PanelServices> = services.clone();
        let settings_handle = cx
            .open_window(window_options(520.0, 850.0), {
                let models = models.clone();
                let services = dyn_services.clone();
                move |window, cx| settings_window(&models, services, window, cx).0
            })
            .expect("settings window");
        let _ = cx.update_window(settings_handle.into(), |_, window, _| window.resize(size(px(520.0), px(850.0))));
        let settings = Session { handle: settings_handle, models: models.clone() };
        cx.spawn(async move |cx| {
            let draw = |cx: &mut AsyncApp| cx.update(|cx| settings.frame(cx));
            // Laid out at its size (the resize of a hidden window lands a moment later).
            let view = cx.update(|cx| settings_handle.read(cx).ok().and_then(|root| root.content().clone().downcast::<SettingsView>().ok())).expect("view");
            wait_until(cx, Duration::from_secs(2), &draw, |cx| cx.update(|cx| view.read(cx).scroll_handle().bounds().size.height == px(850.0))).await;
            let click = |at: Point<f32>, cx: &mut AsyncApp| cx.update(|cx| settings.click(at, cx));
            let choose = |id: u32, cx: &mut AsyncApp| {
                let done = services.menu.borrow_mut().take();
                if let Some(done) = done {
                    cx.update(|cx| done(Some(id), cx));
                }
            };
            // Visualizer colors → White: saved, the player follows (its state's colors).
            click(point(390.0, 136.0), cx);
            choose(2, cx);
            wait_until(cx, Duration::from_secs(3), &draw, |cx| cx.update(|cx| models.snapshot.read(cx).state.colors == "white")).await;
            // Play on launch on; Reconnect → Until stopped; Theme → Light.
            click(point(455.0, 239.0), cx);
            click(point(390.0, 305.0), cx);
            choose(1, cx);
            click(point(390.0, 70.0), cx);
            choose(2, cx);
            // The store is written on the player's command thread, in order: the last write
            // landing means the others did.
            wait_until(cx, Duration::from_secs(3), &draw, |_| store.get("theme") == Some(json!("light"))).await;
            cx.update(|cx| {
                let mut out = result.borrow_mut();
                out.colors_in_player = Some(models.snapshot.read(cx).state.colors.clone());
                out.colors_in_store = store.get("visualizer-colors");
                out.autoplay_in_store = store.get("autoplay");
                out.reconnect_in_store = store.get("reconnect-minutes");
                out.theme_dark_after = Some(qust_fluent::theme(cx).dark);
                out.theme_in_store = store.get("theme");
            });

            // Stream information while a generated file plays.
            let info_handle = cx
                .update(|cx| {
                    cx.open_window(window_options(580.0, 590.0), {
                        let models = models.clone();
                        let services = dyn_services.clone();
                        move |window, cx| info_window(&models, services, window, cx).0
                    })
                })
                .expect("info window");
            let _ = cx.update(|cx| cx.update_window(info_handle.into(), |_, window, _| window.resize(size(px(580.0), px(590.0)))));
            let info = Session { handle: info_handle, models: models.clone() };
            let draw_info = |cx: &mut AsyncApp| cx.update(|cx| info.frame(cx));
            let ids = bridge.player.add_paths(vec![wav.clone()], None).get().await.unwrap_or_default();
            if let Some(id) = ids.first() {
                bridge.player.select_entry(*id);
            }
            let view = cx.update(|cx| info_handle.read(cx).ok().and_then(|root| root.content().clone().downcast::<InfoView>().ok())).expect("info view");
            wait_until(cx, Duration::from_secs(6), &draw_info, |cx| {
                cx.update(|cx| {
                    let texts = view.read(cx).texts();
                    texts.state == "Playing" && texts.value("connected") != "—" && texts.value("samplerate") != "—"
                })
            })
            .await;
            // A couple more polls.
            cx.background_executor().timer(Duration::from_millis(1500)).await;
            draw_info(cx);
            cx.update(|cx| {
                let mut out = result.borrow_mut();
                out.status = Some(models.status.read(cx).status.state);
                let texts = view.read(cx).texts().clone();
                out.texts = qust_ui_panels::info::VALUES.iter().map(|id| (id.to_string(), texts.value(id).to_string())).collect();
                out.title = texts.title.clone();
                out.state = texts.state.clone();
            });
            cx.update(|cx| {
                info.close(cx);
                settings.close(cx);
                bridge.player.detach_and_wait(Duration::from_secs(3));
                cx.quit();
            });
        })
        .detach();
    });

    let out = outcome.borrow();
    println!("{out:#?}");
    let _ = std::fs::remove_dir_all(&dir);
    let value = |id: &str| out.texts.iter().find(|(k, _)| k == id).map(|(_, v)| v.as_str()).unwrap_or("");
    let mut failed = Vec::new();
    let mut check = |name: &str, ok: bool| {
        println!("{} {name}", if ok { "PASS" } else { "FAIL" });
        if !ok {
            failed.push(name.to_string());
        }
    };
    check("colors: saved, and the player's state follows", out.colors_in_store == Some(json!("white")) && out.colors_in_player.as_deref() == Some("white"));
    check("play on launch saved", out.autoplay_in_store == Some(json!(true)));
    check("reconnect: until stopped saved as 0", out.reconnect_in_store == Some(json!(0)));
    check("theme: light applied to the windows and saved", out.theme_dark_after == Some(false) && out.theme_in_store == Some(json!("light")));
    check("the player plays the file", out.status == Some(Playback::Playing));
    check("Stream information: state and title", out.state == "Playing" && out.title == "tone");
    check("Stream information: the file's details", value("samplerate") == "44.1 kHz" && value("channels") == "Stereo" && value("connected") != "—" && (value("received").ends_with("KB") || value("received").ends_with("MB")));
    check("Stream information: a local file has no host, no ICY interval", value("host") == "—" && value("icyMetaint") == "None");
    assert!(failed.is_empty(), "failed: {failed:?}");
    println!("end to end: OK");
}
