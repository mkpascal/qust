// The player window's behaviour (reference/BEHAVIOUR.md, sections 2.2–2.6, 2.8–2.10), driven with
// dispatched input in hidden harness windows against a recording stand-in for the player and the
// outside world. Each case asserts the commands sent (with their arguments), the calls out, and
// the view state where it matters.
//
//   cargo test -p qust-ui-player --features capture --test behaviour [-- <case name filter>]
//
// Windows are hidden and never activated; menus are recorded, never shown.

use futures::future::LocalBoxFuture;
use gpui::{point, App, AppContext, AsyncApp, Entity, FocusHandle, Modifiers, MouseButton, Point};
use qust_shell::menu::Menu;
use qust_ui::capture::{Job, Session};
use qust_ui::fixture::{element_box, layout, Fixture};
use qust_ui_player::cartridges::GHOST_SCALE;
use qust_ui_player::dialog::DialogTarget;
use qust_ui_player::menus::{ids, rows};
use qust_ui_player::testing::{self, Command, HostCall, Recorder};
use qust_ui_player::{Panel, PlayerView};
use serde_json::Value;
use std::time::Duration;

/// A view in a harness window, opened on a fixture.
struct Rig {
    session: Session,
    view: Entity<PlayerView>,
    recorder: Recorder,
    layout: Value,
    cx: AsyncApp,
}

impl Rig {
    async fn open(fixture: &str, cx: &mut AsyncApp) -> Result<Rig, String> {
        let fixture = Fixture::load(fixture)?;
        let variant = fixture.variants()[0].0.clone();
        let job = Job { fixture: fixture.clone(), variant: variant.clone(), time: None, theme: "dark".into(), out: std::env::temp_dir().join("qust-ui-player-behaviour.png") };
        let recorder = Recorder::new();
        let slot: std::rc::Rc<std::cell::RefCell<Option<Entity<PlayerView>>>> = Default::default();
        let (session, view) = cx.update(|cx| {
            cx.set_global(qust_ui_player::ForcedStates::default());
            let rec = recorder.clone();
            let out = slot.clone();
            let session = Session::open(&job, cx, move |setup, window, cx| {
                let (view, player) = testing::build(setup, &rec, window, cx);
                *out.borrow_mut() = Some(player);
                view
            })?;
            session.frames(3, cx);
            let view = slot.borrow().clone().ok_or("no view")?;
            Ok::<_, String>((session, view))
        })?;
        testing::wait_for_size(&session, job.fixture.window_size(), cx).await;
        let rig = Rig { session, view, recorder, layout: layout("player", &variant).unwrap_or(Value::Null), cx: cx.clone() };
        rig.wait(30).await;
        Ok(rig)
    }

    /// Lets the executor run (answers resolve, icons load) and draws.
    async fn wait(&self, ms: u64) {
        self.cx.background_executor().timer(Duration::from_millis(ms)).await;
        self.cx.update(|cx| self.session.frames(2, cx));
    }

    fn at(&self, name: &str) -> Point<f32> {
        self.at_frac(name, 0.5, 0.5)
    }

    fn at_frac(&self, name: &str, fx: f32, fy: f32) -> Point<f32> {
        let b = element_box(&self.layout, name, 1.0).unwrap_or_else(|| panic!("no element {name} in the layout"));
        point(b[0] + fx * b[2], b[1] + fy * b[3])
    }

    fn click(&self, p: Point<f32>) {
        self.cx.update(|cx| self.session.click(p, cx));
    }

    fn right_click(&self, p: Point<f32>) {
        self.cx.update(|cx| {
            self.session.frame(cx);
            self.session.mouse_move(p, None, Modifiers::default(), cx);
            self.session.mouse_down(p, MouseButton::Right, 1, Modifiers::default(), cx);
            self.session.mouse_up(p, MouseButton::Right, 1, Modifiers::default(), cx);
            self.session.frame(cx);
        });
    }

    fn press(&self, p: Point<f32>) {
        self.cx.update(|cx| {
            self.session.frame(cx);
            self.session.mouse_move(p, None, Modifiers::default(), cx);
            self.session.mouse_down(p, MouseButton::Left, 1, Modifiers::default(), cx);
            self.session.frame(cx);
        });
    }

    fn move_held(&self, p: Point<f32>) {
        self.cx.update(|cx| {
            self.session.mouse_move(p, Some(MouseButton::Left), Modifiers::default(), cx);
            self.session.frame(cx);
        });
    }

    fn release(&self, p: Point<f32>) {
        self.cx.update(|cx| {
            self.session.mouse_up(p, MouseButton::Left, 1, Modifiers::default(), cx);
            self.session.frame(cx);
        });
    }

    fn key(&self, key: &'static str) {
        self.cx.update(|cx| self.session.key(key, cx));
    }

    fn type_text(&self, text: &str) {
        self.cx.update(|cx| self.session.type_text(text, cx));
    }

    fn read<R>(&self, f: impl FnOnce(&PlayerView, &App) -> R) -> R {
        self.cx.update(|cx| f(self.view.read(cx), cx))
    }

    fn focused(&self, handle: &FocusHandle) -> bool {
        self.cx.update(|cx| cx.update_window(self.session.any(), |_, window, _| handle.is_focused(window)).unwrap_or(false))
    }

    fn focus_visible(&self, handle: &FocusHandle) -> bool {
        self.cx.update(|cx| cx.update_window(self.session.any(), |_, window, cx| qust_fluent::focus::is_focus_visible(handle, window, cx)).unwrap_or(false))
    }

    fn with_window<R>(&self, f: impl FnOnce(&Entity<PlayerView>, &mut gpui::Window, &mut App) -> R) -> R {
        let view = self.view.clone();
        self.cx.update(|cx| cx.update_window(self.session.any(), |_, window, cx| f(&view, window, cx)).expect("window"))
    }

    fn commands(&self) -> Vec<Command> {
        self.recorder.commands()
    }

    fn calls(&self) -> Vec<HostCall> {
        self.recorder.calls()
    }

    fn dialog(&self) -> Option<(DialogTarget, String, Option<String>)> {
        self.read(|view, cx| {
            let dialog = view.dialog()?;
            let dialog = dialog.read(cx);
            Some((dialog.target()?, dialog.form().title.to_string(), dialog.error().map(str::to_string)))
        })
    }

    fn dialog_values(&self) -> (String, String) {
        self.read(|view, cx| {
            let dialog = view.dialog().expect("dialog");
            let dialog = dialog.read(cx);
            (dialog.name_input().read(cx).value().to_string(), dialog.url_input().read(cx).value().to_string())
        })
    }

    fn input_focus(&self) -> (FocusHandle, FocusHandle) {
        self.read(|view, cx| {
            use gpui::Focusable;
            let dialog = view.dialog().expect("dialog");
            let dialog = dialog.read(cx);
            (dialog.name_input().read(cx).focus_handle(cx), dialog.url_input().read(cx).focus_handle(cx))
        })
    }

    fn apply_state(&self, f: impl FnOnce(&mut serde_json::Value)) {
        let fixture = Fixture::load("stopped").expect("fixture");
        let _ = fixture;
        self.cx.update(|cx| {
            let snapshot = self.session.models.snapshot.read(cx).state.clone();
            let mut value = serde_json::to_value(&snapshot).expect("state");
            f(&mut value);
            let state = serde_json::from_value(value).expect("state");
            self.session.models.apply_state(state, cx);
            self.session.frames(2, cx);
        });
    }

    fn close(self) {
        self.cx.update(|cx| self.session.close(cx));
    }
}

macro_rules! check {
    ($cond:expr, $($msg:tt)+) => {
        if !$cond {
            return Err(format!("{} (line {})", format!($($msg)+), line!()));
        }
    };
}

fn menu_texts(menu: &Menu) -> Vec<String> {
    rows(menu).into_iter().map(|r| r.map_or("—".to_string(), |(t, e, a)| format!("{t}{}{}", if e { "" } else { " (off)" }, a.map(|a| format!(" [{a}]")).unwrap_or_default()))).collect()
}

type Case = fn(&mut AsyncApp) -> LocalBoxFuture<'_, Result<(), String>>;

fn transport_buttons(cx: &mut AsyncApp) -> LocalBoxFuture<'_, Result<(), String>> {
    Box::pin(async move {
        // Stopped with an active cartridge: Play enabled, Pause and Stop disabled.
        let rig = Rig::open("stopped", cx).await?;
        rig.click(rig.at("#pauseButton"));
        rig.click(rig.at("#stopButton"));
        check!(rig.commands().is_empty(), "disabled Pause and Stop sent {:?}", rig.commands());
        rig.click(rig.at("#playButton"));
        rig.click(rig.at("#instantButton"));
        check!(rig.commands() == vec![Command::Play, Command::SetInstant(true)], "play, instant: {:?}", rig.commands());
        rig.click(rig.at("#infoButton"));
        rig.click(rig.at("#settingsButton"));
        check!(rig.calls() == vec![HostCall::OpenPanel(Panel::Info), HostCall::OpenPanel(Panel::Settings)], "panels: {:?}", rig.calls());
        // Values from the player never come back as commands.
        rig.recorder.clear();
        rig.apply_state(|s| {
            s["volume"] = 30.into();
            s["balance"] = (-40).into();
        });
        check!(rig.commands().is_empty(), "player values echoed: {:?}", rig.commands());
        let (volume, balance) = rig.read(|v, cx| {
            let s = v.sliders().read(cx);
            (s.volume.shown, s.balance.shown)
        });
        check!(volume == 30 && balance == -40, "labels follow the player: {volume} {balance}");
        rig.close();

        // Playing: Play disabled, Pause and Stop enabled; Instant on sends false.
        let rig = Rig::open("instant-playing", cx).await?;
        rig.click(rig.at("#playButton"));
        rig.click(rig.at("#pauseButton"));
        rig.click(rig.at("#stopButton"));
        rig.click(rig.at("#instantButton"));
        check!(rig.commands() == vec![Command::Pause, Command::Stop, Command::SetInstant(false)], "playing: {:?}", rig.commands());
        rig.close();

        // Nothing active: Play disabled.
        let rig = Rig::open("stopped-none-active", cx).await?;
        rig.click(rig.at("#playButton"));
        check!(rig.commands().is_empty(), "play with nothing active: {:?}", rig.commands());
        rig.close();

        // Paused: Play and Stop enabled, Pause disabled.
        let rig = Rig::open("paused", cx).await?;
        rig.click(rig.at("#pauseButton"));
        rig.click(rig.at("#playButton"));
        rig.click(rig.at("#stopButton"));
        check!(rig.commands() == vec![Command::Play, Command::Stop], "paused: {:?}", rig.commands());
        rig.close();
        Ok(())
    })
}

fn tooltips(cx: &mut AsyncApp) -> LocalBoxFuture<'_, Result<(), String>> {
    Box::pin(async move {
        let rig = Rig::open("update-dot", cx).await?;
        let mut seen = Vec::new();
        for (name, expected) in [
            ("#playButton", "Play (Ctrl+Alt+Insert)"),
            ("#stopButton", "Stop (Ctrl+Alt+H)"),
            ("#pauseButton", "Pause"),
            ("#instantButton", qust_ui_player::transport::INSTANT_OFF),
            ("#infoButton", "Stream information"),
            ("#screen", "Click to change the analyzer colors"),
            ("#balanceSlider", "Double-click to center"),
            ("#playlistButton", "Show or hide the playlist"),
            ("#addFileButton", "Add music files or playlists from your computer"),
            ("#addUrlButton", "Add a stream or an online audio file"),
            ("#cartridges>.cartridge[0]>.cartridge-eject", "Edit or eject"),
        ] {
            let p = rig.at(name);
            rig.cx.update(|cx| {
                rig.session.mouse_move(point(p.x + 1.0, p.y), None, Modifiers::default(), cx);
                rig.session.mouse_move(p, None, Modifiers::default(), cx);
            });
            rig.wait(650).await;
            let shown = rig.cx.update(|cx| qust_fluent::tooltip::last_shown(cx)).map(|s| s.to_string());
            seen.push((name, shown.clone()));
            check!(shown.as_deref() == Some(expected), "{name}: tooltip {shown:?}, expected {expected:?}");
            rig.cx.update(|cx| rig.session.mouse_move(point(5.0, 260.0), None, Modifiers::default(), cx));
            rig.wait(20).await;
        }
        // The update dot's tooltip on Settings.
        let p = rig.at("#settingsButton");
        rig.cx.update(|cx| rig.session.mouse_move(p, None, Modifiers::default(), cx));
        rig.wait(650).await;
        let shown = rig.cx.update(|cx| qust_fluent::tooltip::last_shown(cx)).map(|s| s.to_string());
        check!(shown.as_deref().is_some_and(|s| s.starts_with("Settings: version ") && s.ends_with(" is ready to install")), "settings tooltip {shown:?}");
        rig.close();
        // Instant on, and an unregistered Play shortcut.
        let rig = Rig::open("menu-shortcut-unavailable", cx).await?;
        rig.apply_state(|s| s["instant"] = true.into());
        for (name, expected) in [("#playButton", "Play"), ("#instantButton", qust_ui_player::transport::INSTANT_ON)] {
            let p = rig.at(name);
            rig.cx.update(|cx| rig.session.mouse_move(p, None, Modifiers::default(), cx));
            rig.wait(650).await;
            let shown = rig.cx.update(|cx| qust_fluent::tooltip::last_shown(cx)).map(|s| s.to_string());
            check!(shown.as_deref() == Some(expected), "{name}: {shown:?}");
            rig.cx.update(|cx| rig.session.mouse_move(point(5.0, 260.0), None, Modifiers::default(), cx));
            rig.wait(20).await;
        }
        rig.close();
        Ok(())
    })
}

fn screen_cycles_colors(cx: &mut AsyncApp) -> LocalBoxFuture<'_, Result<(), String>> {
    Box::pin(async move {
        let rig = Rig::open("playing", cx).await?;
        // On the analyzer itself (a live layer above the screen): the click reaches the screen.
        rig.click(rig.at("#canvas"));
        rig.click(rig.at("#clock"));
        check!(rig.commands() == vec![Command::CycleColors, Command::CycleColors], "{:?}", rig.commands());
        rig.close();
        Ok(())
    })
}

fn sliders(cx: &mut AsyncApp) -> LocalBoxFuture<'_, Result<(), String>> {
    Box::pin(async move {
        let rig = Rig::open("stopped", cx).await?;
        let rail = |rig: &Rig, name: &str, f: f32| rig.at_frac(&format!("{name}>.slider-rail"), f, 0.5);
        // A drag sends every value; the player's own values don't fight it.
        rig.press(rail(&rig, "#volumeSlider", 0.2));
        rig.move_held(rail(&rig, "#volumeSlider", 0.4));
        rig.apply_state(|s| s["volume"] = 90.into());
        let shown = rig.read(|v, cx| v.sliders().read(cx).volume.shown);
        check!(shown == 40, "a value from the player during the drag showed: {shown}");
        rig.move_held(rail(&rig, "#volumeSlider", 0.1));
        rig.release(rail(&rig, "#volumeSlider", 0.1));
        check!(rig.commands() == vec![Command::SetVolume(20), Command::SetVolume(40), Command::SetVolume(10)], "volume drag: {:?}", rig.commands());
        let shown = rig.read(|v, cx| v.sliders().read(cx).volume.shown);
        check!(shown == 10, "label after the drag {shown}");
        // After the drag the player's value shows again (not echoed).
        rig.recorder.clear();
        rig.apply_state(|s| s["volume"] = 55.into());
        let shown = rig.read(|v, cx| v.sliders().read(cx).volume.shown);
        check!(shown == 55 && rig.commands().is_empty(), "player value after the drag: {shown} {:?}", rig.commands());
        // Beyond the rail: clamped.
        rig.press(point(64.0, 133.0));
        rig.move_held(point(400.0, 133.0));
        rig.release(point(400.0, 133.0));
        check!(rig.commands().first() == Some(&Command::SetVolume(0)) && rig.commands().last() == Some(&Command::SetVolume(100)), "clamped: {:?}", rig.commands());

        // Wheel: ±5 % of the span.
        rig.recorder.clear();
        rig.apply_state(|s| {
            s["volume"] = 50.into();
            s["balance"] = 0.into();
        });
        rig.cx.update(|cx| rig.session.wheel(rig.at("#volumeSlider"), 1.0, cx));
        rig.cx.update(|cx| rig.session.wheel(rig.at("#balanceSlider"), -1.0, cx));
        check!(rig.commands() == vec![Command::SetVolume(55), Command::SetBalance(-10)], "wheel: {:?}", rig.commands());

        // Balance: snap within ±8 of the centre (pointer only), double-click to centre.
        rig.recorder.clear();
        rig.apply_state(|s| s["balance"] = 30.into());
        rig.press(rail(&rig, "#balanceSlider", 0.515));
        rig.release(rail(&rig, "#balanceSlider", 0.515));
        check!(rig.commands() == vec![Command::SetBalance(0)], "snap: {:?}", rig.commands());
        rig.recorder.clear();
        rig.apply_state(|s| s["balance"] = 60.into());
        let p = rail(&rig, "#balanceSlider", 0.8);
        rig.cx.update(|cx| rig.session.double_click(p, cx));
        let commands = rig.commands();
        check!(commands.last() == Some(&Command::SetBalance(0)), "double-click centres: {commands:?}");
        let label = rig.read(|v, cx| qust_ui::format::balance(v.sliders().read(cx).balance.shown));
        check!(label == "C", "balance label {label}");

        // Keys on the focused slider: ±5 %, Page ±10 %, Home, End; no snap.
        rig.recorder.clear();
        // (The slider takes the player's value when the value changes: set another one first.)
        rig.apply_state(|s| {
            s["volume"] = 49.into();
            s["balance"] = 1.into();
        });
        rig.apply_state(|s| {
            s["volume"] = 50.into();
            s["balance"] = 0.into();
        });
        let volume_focus = rig.read(|v, cx| v.sliders().read(cx).volume_focus().clone());
        rig.with_window(|_, window, cx| volume_focus.focus(window, cx));
        for key in ["right", "up", "left", "pageup", "pagedown", "down", "end", "home"] {
            rig.key(key);
        }
        check!(
            rig.commands() == vec![Command::SetVolume(55), Command::SetVolume(60), Command::SetVolume(55), Command::SetVolume(65), Command::SetVolume(55), Command::SetVolume(50), Command::SetVolume(100), Command::SetVolume(0)],
            "keys: {:?}",
            rig.commands()
        );
        let balance_focus = rig.read(|v, cx| v.sliders().read(cx).balance_focus().clone());
        rig.recorder.clear();
        rig.with_window(|_, window, cx| balance_focus.focus(window, cx));
        rig.key("right");
        check!(rig.commands() == vec![Command::SetBalance(10)], "balance key, no snap: {:?}", rig.commands());
        rig.close();
        Ok(())
    })
}

fn seek_bar(cx: &mut AsyncApp) -> LocalBoxFuture<'_, Result<(), String>> {
    Box::pin(async move {
        let rig = Rig::open("seek-middle", cx).await?;
        let rail = |f: f32| rig.at_frac("#seekSlider>.slider-rail", f, 0.5);
        // Seeks only on release, to the value under the pointer then.
        rig.press(rail(0.25));
        rig.move_held(rail(0.5));
        check!(rig.commands().is_empty(), "seek sent during the drag: {:?}", rig.commands());
        // Positions from the player don't move it while dragging; the time follows the pointer.
        rig.apply_state(|s| s["position"] = 290.0.into());
        let (shown, seek) = rig.read(|v, cx| (v.live().map(|l| l.seek.read(cx).mirror.shown), ()));
        let _ = seek;
        check!(shown == Some(5000), "seek shows {shown:?} while dragging");
        rig.move_held(rail(0.7));
        rig.release(rail(0.7));
        let commands = rig.commands();
        let seconds = match commands.as_slice() {
            [Command::Seek(s)] => *s,
            other => return Err(format!("seek on release: {other:?}")),
        };
        check!((seconds - 0.7 * 301.2).abs() < 0.2, "seek to {seconds}");
        // Keys: one seek per step.
        rig.recorder.clear();
        let focus = rig.read(|v, cx| v.live().map(|l| l.seek.read(cx).focus_handle().clone())).ok_or("no seek")?;
        rig.with_window(|_, window, cx| focus.focus(window, cx));
        rig.key("right");
        check!(matches!(rig.commands().as_slice(), [Command::Seek(_)]), "seek key: {:?}", rig.commands());
        rig.close();

        // Not seekable: the bar ignores the pointer.
        let rig = Rig::open("seek-disabled", cx).await?;
        let p = rig.at_frac("#seekSlider>.slider-rail", 0.5, 0.5);
        rig.press(p);
        rig.release(p);
        rig.cx.update(|cx| rig.session.wheel(p, 1.0, cx));
        check!(rig.commands().is_empty(), "disabled seek: {:?}", rig.commands());
        rig.close();
        Ok(())
    })
}

fn cartridge_clicks(cx: &mut AsyncApp) -> LocalBoxFuture<'_, Result<(), String>> {
    Box::pin(async move {
        let rig = Rig::open("stopped", cx).await?;
        rig.click(rig.at("#cartridges>.cartridge[1]>.cartridge-body"));
        check!(rig.commands() == vec![Command::SelectSlot(1)], "play cartridge: {:?}", rig.commands());
        rig.click(rig.at("#cartridges>.cartridge[4]>.cartridge-body"));
        check!(rig.dialog().map(|d| (d.0, d.1)) == Some((DialogTarget::Cartridge(4), "Load cartridge".into())), "empty slot: {:?}", rig.dialog());
        rig.key("escape");
        check!(rig.dialog().is_none(), "Esc closes");
        rig.click(rig.at("#cartridges>.cartridge[2]>.cartridge-eject"));
        check!(rig.dialog().map(|d| (d.0, d.1)) == Some((DialogTarget::Cartridge(2), "Edit cartridge".into())), "eject strip: {:?}", rig.dialog());
        check!(rig.dialog_values() == ("Radio Paradise".to_string(), "http://stream.radioparadise.com/aac-320".to_string()), "values {:?}", rig.dialog_values());
        check!(rig.commands() == vec![Command::SelectSlot(1)], "nothing else sent: {:?}", rig.commands());
        rig.close();
        Ok(())
    })
}

fn cartridge_drag(cx: &mut AsyncApp) -> LocalBoxFuture<'_, Result<(), String>> {
    Box::pin(async move {
        let rig = Rig::open("stopped", cx).await?;
        let from = rig.at("#cartridges>.cartridge[0]");
        // Under the threshold: still a click.
        rig.press(from);
        rig.move_held(point(from.x + 3.0, from.y + 2.0));
        check!(!rig.read(|v, cx| v.cartridges().read(cx).dragging()), "a 3.6 px move started a drag");
        rig.release(point(from.x + 3.0, from.y + 2.0));
        check!(rig.commands() == vec![Command::SelectSlot(0)], "short move is a click: {:?}", rig.commands());

        // A drag onto the third cartridge: ghost at the pointer minus the grab offset, scaled
        // 1.04 about its centre; the target outlined; release swaps; no click.
        rig.recorder.clear();
        let to = point(rig.at("#cartridges>.cartridge[2]").x - 20.0, rig.at("#cartridges>.cartridge[2]").y + 6.0);
        rig.press(from);
        for step in 1..=6 {
            let t = step as f32 / 6.0;
            rig.move_held(point(from.x + (to.x - from.x) * t, from.y + (to.y - from.y) * t));
        }
        let (dragging, target, ghost, overlay) = rig.read(|v, cx| {
            let c = v.cartridges().read(cx);
            (c.dragging(), c.drag().and_then(|d| d.target), c.ghost().read(cx).bounds(), ())
        });
        let _ = overlay;
        check!(dragging && target == Some(2), "drag {dragging} target {target:?}");
        let reference = rig.cx.update(|_| layout("player", "cartridge-drag")).and_then(|l| element_box(&l, "body>.cartridge", 1.0)).ok_or("no ghost box")?;
        let ghost = [ghost.origin.x.as_f32(), ghost.origin.y.as_f32(), ghost.size.width.as_f32(), ghost.size.height.as_f32()];
        check!(ghost.iter().zip(reference.iter()).all(|(a, b)| (a - b).abs() < 0.05), "ghost {ghost:?} vs the page's {reference:?}");
        check!((ghost[2] / GHOST_SCALE - 146.0).abs() < 0.01, "ghost scale");
        let has_overlay = rig.cx.update(|cx| rig.session.handle.read(cx).map(|root| root.has_overlay("cartridge-ghost")).unwrap_or(false));
        check!(has_overlay, "the ghost is an overlay");
        rig.release(to);
        rig.wait(20).await;
        check!(rig.commands() == vec![Command::MoveCartridge(0, 2)], "drop: {:?}", rig.commands());
        let (dropped0, dropped2, dragging) = rig.read(|v, cx| {
            let c = v.cartridges().read(cx);
            (c.dropped_since(0).is_some(), c.dropped_since(2).is_some(), c.dragging())
        });
        check!(dropped0 && dropped2 && !dragging, "dropped animation on both slots after the move answer");
        let has_overlay = rig.cx.update(|cx| rig.session.handle.read(cx).map(|root| root.has_overlay("cartridge-ghost")).unwrap_or(true));
        check!(!has_overlay, "the ghost is gone");

        // Released over the source itself: nothing moves and the click is swallowed.
        rig.recorder.clear();
        rig.press(from);
        rig.move_held(point(from.x + 30.0, from.y));
        rig.move_held(point(from.x + 2.0, from.y));
        rig.release(point(from.x + 2.0, from.y));
        rig.wait(20).await;
        check!(rig.commands().is_empty(), "drop on itself: {:?}", rig.commands());

        // Released elsewhere: nothing.
        rig.press(from);
        rig.move_held(point(from.x, from.y + 150.0));
        rig.release(point(from.x, from.y + 150.0));
        rig.wait(20).await;
        check!(rig.commands().is_empty(), "drop outside: {:?}", rig.commands());

        // Esc cancels.
        rig.press(from);
        rig.move_held(point(to.x, to.y));
        rig.key("escape");
        check!(!rig.read(|v, cx| v.cartridges().read(cx).dragging()), "Esc ends the drag");
        rig.release(to);
        rig.wait(20).await;
        check!(rig.commands().is_empty(), "after Esc: {:?}", rig.commands());

        // Onto an empty slot (a move), from the second cartridge.
        let from = rig.at("#cartridges>.cartridge[1]");
        let empty = rig.at("#cartridges>.cartridge[4]");
        rig.press(from);
        rig.move_held(point(from.x, from.y + 30.0));
        rig.move_held(empty);
        let target = rig.read(|v, cx| v.cartridges().read(cx).drag().and_then(|d| d.target));
        check!(target == Some(4), "empty target {target:?}");
        rig.release(empty);
        rig.wait(20).await;
        check!(rig.commands() == vec![Command::MoveCartridge(1, 4)], "move to empty: {:?}", rig.commands());

        // No drag from the eject strip or from an empty slot.
        rig.recorder.clear();
        for start in [rig.at("#cartridges>.cartridge[3]>.cartridge-eject"), empty] {
            rig.press(start);
            rig.move_held(point(start.x + 40.0, start.y));
            check!(!rig.read(|v, cx| v.cartridges().read(cx).dragging()), "drag started from {start:?}");
            rig.release(point(start.x + 40.0, start.y));
        }
        rig.wait(20).await;
        check!(rig.commands().iter().all(|c| !matches!(c, Command::MoveCartridge(..))), "{:?}", rig.commands());
        rig.close();
        Ok(())
    })
}

fn menus(cx: &mut AsyncApp) -> LocalBoxFuture<'_, Result<(), String>> {
    Box::pin(async move {
        let rig = Rig::open("stopped", cx).await?;
        rig.right_click(rig.at("#screen"));
        let menu = rig.recorder.last_menu().ok_or("no menu")?;
        check!(menu_texts(&menu) == ["Play [Ctrl+Alt+Ins]", "Stop (off) [Ctrl+Alt+H]", "—", "Stream information", "Settings"], "stopped menu {:?}", menu_texts(&menu));
        rig.cx.update(|cx| rig.recorder.choose(Some(ids::PLAY), cx));
        check!(rig.commands() == vec![Command::Play], "Play from the menu: {:?}", rig.commands());
        rig.right_click(rig.at("#sliders"));
        rig.cx.update(|cx| rig.recorder.choose(Some(ids::SETTINGS), cx));
        check!(rig.calls().last() == Some(&HostCall::OpenPanel(Panel::Settings)), "Settings from the menu");
        // A loaded cartridge.
        rig.recorder.clear();
        rig.right_click(rig.at("#cartridges>.cartridge[1]"));
        let menu = rig.recorder.last_menu().ok_or("no menu")?;
        check!(menu_texts(&menu) == ["Play", "Edit…", "Eject", "—", "Stream information", "Settings"], "cartridge menu {:?}", menu_texts(&menu));
        check!(rig.calls().len() == 1, "one menu only (not the player's too): {:?}", rig.calls().len());
        rig.cx.update(|cx| rig.recorder.choose(Some(menu_id(&menu, "Eject")), cx));
        check!(rig.commands() == vec![Command::Eject(1)], "eject: {:?}", rig.commands());
        rig.right_click(rig.at("#cartridges>.cartridge[1]"));
        let menu = rig.recorder.last_menu().ok_or("no menu")?;
        rig.cx.update(|cx| rig.recorder.choose(Some(menu_id(&menu, "Edit…")), cx));
        rig.wait(10).await;
        check!(rig.dialog().map(|d| d.0) == Some(DialogTarget::Cartridge(1)), "Edit… opens the dialog");
        // While the dialog is open: no player menu.
        rig.recorder.clear();
        rig.right_click(point(40.0, 250.0));
        check!(rig.recorder.last_menu().is_none(), "a menu while the dialog is open");
        rig.key("escape");
        // An empty slot.
        rig.right_click(rig.at("#cartridges>.cartridge[4]"));
        let menu = rig.recorder.last_menu().ok_or("no menu")?;
        check!(menu_texts(&menu) == ["Load cartridge…", "—", "Stream information", "Settings"], "empty menu {:?}", menu_texts(&menu));
        rig.cx.update(|cx| rig.recorder.choose(Some(menu_id(&menu, "Load cartridge…")), cx));
        check!(rig.dialog().map(|d| (d.0, d.1)) == Some((DialogTarget::Cartridge(4), "Load cartridge".into())), "Load cartridge…");
        rig.close();

        // Playing: Pause / Stop.
        let rig = Rig::open("menu-player-playing", cx).await?;
        rig.right_click(rig.at("body>.app>.player>.transport"));
        let menu = rig.recorder.last_menu().ok_or("no menu")?;
        check!(menu_texts(&menu) == ["Pause", "Stop [Ctrl+Alt+H]", "—", "Stream information", "Settings"], "playing menu {:?}", menu_texts(&menu));
        rig.cx.update(|cx| rig.recorder.choose(Some(ids::PAUSE), cx));
        check!(rig.commands() == vec![Command::Pause], "{:?}", rig.commands());
        rig.close();

        // The Play shortcut owned by another app: no accelerator on Play.
        let rig = Rig::open("menu-shortcut-unavailable", cx).await?;
        rig.right_click(rig.at("#screen"));
        let menu = rig.recorder.last_menu().ok_or("no menu")?;
        check!(menu_texts(&menu) == ["Play", "Stop (off) [Ctrl+Alt+H]", "—", "Stream information", "Settings"], "{:?}", menu_texts(&menu));
        rig.close();

        // Nothing loaded.
        let rig = Rig::open("menu-player-nothing", cx).await?;
        rig.right_click(rig.at("#screen"));
        let menu = rig.recorder.last_menu().ok_or("no menu")?;
        check!(menu_texts(&menu) == ["Play (off) [Ctrl+Alt+Ins]", "Stop (off) [Ctrl+Alt+H]", "—", "Stream information", "Settings"], "{:?}", menu_texts(&menu));
        rig.close();
        Ok(())
    })
}

fn menu_id(menu: &Menu, text: &str) -> u32 {
    menu.items
        .iter()
        .find_map(|item| match item {
            qust_shell::menu::Item::Action { id, text: t, .. } if t == text => Some(*id),
            _ => None,
        })
        .unwrap_or(0)
}

fn deck_bar(cx: &mut AsyncApp) -> LocalBoxFuture<'_, Result<(), String>> {
    Box::pin(async move {
        let rig = Rig::open("stopped", cx).await?;
        rig.click(rig.at("#playlistButton"));
        check!(rig.read(|v, _| v.drawer_open()), "the Playlist button flips at once");
        rig.click(rig.at("#playlistButton"));
        rig.click(rig.at("#addFileButton"));
        check!(rig.calls() == vec![HostCall::SetDrawer(true), HostCall::SetDrawer(false), HostCall::AddFiles], "{:?}", rig.calls());
        // The drawer opening by itself.
        rig.with_window(|view, _, cx| view.update(cx, |v, cx| v.set_drawer_open(true, cx)));
        check!(rig.read(|v, _| v.drawer_open()), "set_drawer_open");
        rig.click(rig.at("#addUrlButton"));
        check!(rig.dialog().map(|d| (d.0, d.1)) == Some((DialogTarget::Url, "Add URL".into())), "Add URL: {:?}", rig.dialog());
        let (name, url) = rig.input_focus();
        check!(rig.focused(&url) && !rig.focused(&name), "Add URL focuses the URL field");
        rig.close();
        Ok(())
    })
}

fn dialog_submit(cx: &mut AsyncApp) -> LocalBoxFuture<'_, Result<(), String>> {
    Box::pin(async move {
        // Load cartridge: Name focused; typing, Enter in a field submits, values trimmed.
        let rig = Rig::open("stopped", cx).await?;
        rig.click(rig.at("#cartridges>.cartridge[4]>.cartridge-body"));
        let (name, url) = rig.input_focus();
        check!(rig.focused(&name), "Name focused");
        rig.type_text("  My Radio ");
        rig.key("tab");
        check!(rig.focused(&url), "Tab to URL");
        rig.type_text(" http://radio.example/live  ");
        rig.key("enter");
        rig.wait(20).await;
        check!(rig.commands() == vec![Command::SaveCartridge(4, "My Radio".into(), "http://radio.example/live".into())], "{:?}", rig.commands());
        check!(rig.dialog().is_none(), "closes on success");

        // Edit cartridge: Eject closes then ejects; Cancel closes; the backdrop does nothing.
        rig.recorder.clear();
        rig.click(rig.at("#cartridges>.cartridge[0]>.cartridge-eject"));
        rig.click(point(30.0, 30.0));
        check!(rig.dialog().is_some() && rig.commands().is_empty(), "a click on the backdrop");
        rig.click(point(430.0 + 0.0, 133.0 + 89.0));
        rig.wait(10).await;
        rig.with_window(|view, window, cx| {
            if let Some(dialog) = view.read(cx).dialog() {
                dialog.update(cx, |d, cx| {
                    let _ = window;
                    d.eject(cx)
                });
            }
        });
        check!(rig.dialog().is_none() && rig.commands() == vec![Command::Eject(0)], "eject: {:?} {:?}", rig.dialog(), rig.commands());
        rig.close();

        // Rust refuses: the error shows, the dialog stays, the URL field is focused; a second
        // submit while one is pending is ignored; the success after closes and selects the entry.
        let rig = Rig::open("stopped", cx).await?;
        rig.recorder.fail("playlist_add_url", "Enter an address starting with http:// or https://");
        rig.click(rig.at("#addUrlButton"));
        rig.type_text("stream.example.com/live");
        rig.key("enter");
        rig.wait(20).await;
        let (_, url) = rig.input_focus();
        check!(rig.dialog().and_then(|d| d.2) == Some("Enter an address starting with http:// or https://".into()), "error {:?}", rig.dialog());
        check!(rig.focused(&url), "URL focused after the error");
        rig.recorder.succeed("playlist_add_url");
        rig.recorder.hold(true);
        rig.with_window(|view, window, cx| {
            if let Some(dialog) = view.read(cx).dialog() {
                dialog.update(cx, |d, cx| {
                    d.set_url_text("https://example.com/a.mp3", window, cx);
                    d.submit(window, cx);
                    d.submit(window, cx);
                });
            }
        });
        rig.key("enter");
        rig.wait(20).await;
        let adds = rig.commands().iter().filter(|c| matches!(c, Command::AddUrl(..))).count();
        check!(adds == 2, "pending submits ignored: {:?}", rig.commands());
        check!(rig.dialog().is_some(), "still open while pending");
        rig.recorder.release();
        rig.wait(20).await;
        check!(rig.dialog().is_none(), "closed on success");
        check!(rig.calls().last() == Some(&HostCall::Added(vec![100])), "the added entry is selected: {:?}", rig.calls());
        rig.close();
        Ok(())
    })
}

fn dialog_entries(cx: &mut AsyncApp) -> LocalBoxFuture<'_, Result<(), String>> {
    Box::pin(async move {
        let rig = Rig::open("drawer-entries", cx).await?;
        // A local file: the path is shown, not sent back.
        rig.with_window(|view, window, cx| view.update(cx, |v, cx| v.open_entry_dialog(1, window, cx)));
        let form = rig.read(|v, cx| v.dialog().map(|d| d.read(cx).form().clone())).ok_or("no dialog")?;
        check!(form.title == "Edit track" && form.url_label == "File" && form.fixed_url && form.name_placeholder == "01 One More Time", "file form {form:?}");
        rig.key("enter");
        rig.wait(20).await;
        check!(rig.commands() == vec![Command::EditEntry(1, "One More Time".into(), None)], "{:?}", rig.commands());
        // A stream entry: its URL is editable; its placeholder is the host.
        rig.recorder.clear();
        rig.with_window(|view, window, cx| view.update(cx, |v, cx| v.open_entry_dialog(4, window, cx)));
        let form = rig.read(|v, cx| v.dialog().map(|d| d.read(cx).form().clone())).ok_or("no dialog")?;
        check!(form.url_label == "Stream or audio file URL" && !form.fixed_url && form.name_placeholder == "novazz-128", "url form {form:?}");
        rig.key("enter");
        rig.wait(20).await;
        check!(rig.commands() == vec![Command::EditEntry(4, "Radio Nova".into(), Some("http://novazz.ice.infomaniak.ch/novazz-128.mp3".into()))], "{:?}", rig.commands());
        // An online file: its name.
        rig.with_window(|view, window, cx| view.update(cx, |v, cx| v.open_entry_dialog(6, window, cx)));
        let form = rig.read(|v, cx| v.dialog().map(|d| d.read(cx).form().clone())).ok_or("no dialog")?;
        check!(form.name_placeholder == "ep-142", "online file placeholder {:?}", form.name_placeholder);
        // An id the playlist doesn't have: nothing opens (the open one stays).
        rig.key("escape");
        rig.with_window(|view, window, cx| view.update(cx, |v, cx| v.open_entry_dialog(999, window, cx)));
        check!(rig.dialog().is_none(), "unknown entry opened a dialog");
        rig.close();
        Ok(())
    })
}

fn dialog_focus(cx: &mut AsyncApp) -> LocalBoxFuture<'_, Result<(), String>> {
    Box::pin(async move {
        let rig = Rig::open("stopped", cx).await?;
        rig.click(rig.at("#cartridges>.cartridge[0]>.cartridge-eject"));
        let (name, url) = rig.input_focus();
        let (save, eject, cancel) = rig.read(|v, cx| {
            let d = v.dialog().expect("dialog");
            let d = d.read(cx);
            let (a, b, c) = d.button_focus();
            (a.clone(), b.clone(), c.clone())
        });
        let order = [&name, &url, &save, &eject, &cancel, &name];
        for (step, handle) in order.iter().enumerate().skip(1) {
            rig.key("tab");
            check!(rig.focused(handle), "Tab step {step}");
        }
        rig.key("shift-tab");
        check!(rig.focused(&cancel), "Shift+Tab from Name goes to Cancel");
        // The focus ring on a button reached with the keyboard.
        check!(rig.focus_visible(&cancel), "keyboard focus shows the ring");
        rig.key("enter");
        check!(rig.dialog().is_none(), "Enter on Cancel closes");
        check!(rig.commands().is_empty(), "Cancel sent {:?}", rig.commands());

        // Opening Settings closes the dialog.
        rig.click(rig.at("#addUrlButton"));
        rig.with_window(|view, window, cx| view.update(cx, |v, cx| {
            v.run_menu_item(ids::SETTINGS, window, cx);
        }));
        check!(rig.dialog().is_none(), "Settings closes the dialog");

        // The text fields' edit menu.
        rig.click(rig.at("#addUrlButton"));
        rig.wait(10).await;
        rig.recorder.clear();
        rig.right_click(point(430.0, 150.0));
        let menu = rig.recorder.last_menu().ok_or("no edit menu")?;
        check!(menu_texts(&menu) == ["Cu&t [Ctrl+X]", "&Copy [Ctrl+C]", "&Paste [Ctrl+V]", "—", "Select &All [Ctrl+A]"], "edit menu {:?}", menu_texts(&menu));
        rig.close();
        Ok(())
    })
}

fn tab_order(cx: &mut AsyncApp) -> LocalBoxFuture<'_, Result<(), String>> {
    Box::pin(async move {
        // A file playing: seek bar first; Play is disabled (playing), so it is skipped.
        let rig = Rig::open("seek-middle", cx).await?;
        let seek = rig.read(|v, cx| v.live().map(|l| l.seek.read(cx).focus_handle().clone())).ok_or("seek")?;
        let (volume, balance) = rig.read(|v, cx| {
            let s = v.sliders().read(cx);
            (s.volume_focus().clone(), s.balance_focus().clone())
        });
        let transport = rig.read(|v, cx| v.transport().read(cx).focus.clone());
        let (bodies, ejects) = rig.read(|v, cx| {
            let c = v.cartridges().read(cx);
            (c.body_focus.clone(), c.eject_focus.clone())
        });
        let deck = rig.read(|v, cx| v.deck().read(cx).focus.clone());
        let mut expected: Vec<(&str, FocusHandle)> = vec![("seek", seek), ("volume", volume), ("balance", balance)];
        for (i, name) in ["play", "pause", "stop", "instant", "info", "settings"].into_iter().enumerate() {
            if i != 0 {
                expected.push((name, transport[i].clone()));
            }
        }
        for slot in 0..6 {
            expected.push(("body", bodies[slot].clone()));
            if slot != 4 {
                expected.push(("eject", ejects[slot].clone()));
            }
        }
        for (i, name) in ["playlist", "add file", "add url"].into_iter().enumerate() {
            expected.push((name, deck[i].clone()));
        }
        // With nothing focused, Tab goes to the first control (as from the page's body): the window
        // root holds the focus then and starts Tab from the beginning.
        rig.key("tab");
        if !rig.focused(&expected[0].1) {
            let actual: Vec<&str> = expected.iter().filter(|(_, h)| rig.focused(h)).map(|(n, _)| *n).collect();
            return Err(format!("the first Tab on a fresh window: expected the seek bar, focused {actual:?}"));
        }
        // Shift+Tab from the first control wraps to the last one, and Tab comes back.
        rig.key("shift-tab");
        rig.key("tab");
        check!(rig.focused(&expected[0].1), "Shift+Tab then Tab comes back to the seek bar");
        check!(rig.focus_visible(&expected[0].1), "the seek bar has no ring");
        for (step, (name, handle)) in expected.iter().enumerate().skip(1) {
            rig.key("tab");
            if !rig.focused(handle) {
                let actual: Vec<&str> = expected.iter().filter(|(_, h)| rig.focused(h)).map(|(n, _)| *n).collect();
                return Err(format!("Tab {}: expected {name}, focused {actual:?}", step + 1));
            }
            check!(rig.focus_visible(handle), "Tab {}: {name} has no ring", step + 1);
        }
        // A mouse click focuses without the ring.
        let pause = transport[1].clone();
        rig.click(rig.at("#pauseButton"));
        check!(rig.focused(&pause) && !rig.focus_visible(&pause), "a click shows a ring");
        rig.close();
        Ok(())
    })
}

fn main() {
    let filter: Option<String> = std::env::args().skip(1).find(|a| !a.starts_with('-'));
    let cases: Vec<(&str, Case)> = vec![
        ("transport_buttons", transport_buttons),
        ("tooltips", tooltips),
        ("screen_cycles_colors", screen_cycles_colors),
        ("sliders", sliders),
        ("seek_bar", seek_bar),
        ("cartridge_clicks", cartridge_clicks),
        ("cartridge_drag", cartridge_drag),
        ("menus", menus),
        ("deck_bar", deck_bar),
        ("dialog_submit", dialog_submit),
        ("dialog_entries", dialog_entries),
        ("dialog_focus", dialog_focus),
        ("tab_order", tab_order),
    ];
    let cases: Vec<_> = cases.into_iter().filter(|(name, _)| filter.as_deref().is_none_or(|f| name.contains(f))).collect();
    let results = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let out = results.clone();
    qust_ui::capture::run(move |cx: &mut App| {
        cx.spawn(async move |cx| {
            for (name, case) in cases {
                let result = case(cx).await;
                out.borrow_mut().push((name, result));
            }
            cx.update(|cx| cx.quit());
        })
        .detach();
    });
    let mut failed = 0;
    for (name, result) in results.borrow().iter() {
        match result {
            Ok(()) => println!("ok    {name}"),
            Err(error) => {
                failed += 1;
                println!("FAIL  {name}: {error}");
            }
        }
    }
    println!("{} passed, {failed} failed", results.borrow().len() - failed);
    if failed > 0 {
        std::process::exit(1);
    }
}
