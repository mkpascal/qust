// Behaviour of the interactive elements, driven through the harness: events dispatched into a
// hidden window (never shown, never focused, no OS input), checked against BEHAVIOUR.md.
//
//   cargo test -p qust-fluent --test behaviour

use gpui::{div, point, prelude::*, px, App, Context, FocusHandle, Point, Window};
use qust_fluent::{focus, Button, Slider, Switch};
use qust_ui::capture::{self, Job, Session};
use qust_ui::fixture::Fixture;
use std::cell::RefCell;
use std::rc::Rc;

#[derive(Default, Debug, Clone)]
struct Log {
    volume_inputs: Vec<i64>,
    volume_commits: Vec<i64>,
    balance_inputs: Vec<i64>,
    clicks: u32,
    switch: Vec<bool>,
}

struct Bench {
    log: Rc<RefCell<Log>>,
    volume: i64,
    balance: i64,
    on: bool,
    volume_focus: FocusHandle,
    button_focus: FocusHandle,
}

impl Render for Bench {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let log = self.log.clone();
        let (l1, l2, l3, l4, l5) = (log.clone(), log.clone(), log.clone(), log.clone(), log.clone());
        let this = cx.entity().downgrade();
        let this2 = this.clone();
        div()
            .size_full()
            // Volume: the slider box at (63, 121), 230 × 24, as in the player.
            .child(
                div().absolute().left(px(63.0)).top(px(121.0)).w(px(230.0)).h(px(24.0)).flex().child(
                    Slider::new("volume", self.volume, 0, 100, "Volume")
                        .focus(&self.volume_focus)
                        .on_input(move |v, _, _| l1.borrow_mut().volume_inputs.push(v))
                        .on_commit(move |v, _, _| l2.borrow_mut().volume_commits.push(v))
                        .flex_1(),
                ),
            )
            // Balance at (63, 153).
            .child(
                div().absolute().left(px(63.0)).top(px(153.0)).w(px(230.0)).h(px(24.0)).flex().child(
                    Slider::new("balance", self.balance, -100, 100, "Balance")
                        .centered(0)
                        .on_input(move |v, _, cx| {
                            l3.borrow_mut().balance_inputs.push(v);
                            let _ = this.update(cx, |b, cx| {
                                b.balance = v;
                                cx.notify()
                            });
                        })
                        .flex_1(),
                ),
            )
            .child(div().absolute().left(px(33.0)).top(px(211.0)).w(px(44.67)).h(px(32.0)).flex().child(
                Button::new("play").icon_only(qust_fluent::Icon::Play).accent().aria_label("Play").focus(&self.button_focus).on_click(move |_, _, _| l4.borrow_mut().clicks += 1).size_full(),
            ))
            .child(div().absolute().left(px(400.0)).top(px(229.0)).child(Switch::new("autoplay", self.on, "Play on launch").on_change(move |on, _, cx| {
                l5.borrow_mut().switch.push(on);
                let _ = this2.update(cx, |b, cx| {
                    b.on = on;
                    cx.notify()
                });
            })))
    }
}

fn p(x: f32, y: f32) -> Point<f32> {
    point(x, y)
}

fn main() {
    let fixture = Fixture::load("stopped").expect("fixture");
    let job = Job { variant: "stopped".into(), time: None, theme: "dark".into(), out: std::env::temp_dir().join("qust-behaviour.png"), fixture };
    let log = Rc::new(RefCell::new(Log::default()));
    let shared = log.clone();
    let failures = Rc::new(RefCell::new(Vec::<String>::new()));
    let failures_out = failures.clone();
    capture::run(move |cx: &mut App| {
        let log = shared.clone();
        let button_focus = focus::tab_stop(cx);
        let button_handle = button_focus.clone();
        let session = Session::open(&job, cx, |_, _, cx| {
            cx.new(|cx| Bench { log: log.clone(), volume: 50, balance: 0, on: false, volume_focus: focus::tab_stop(cx), button_focus }).into()
        })
        .expect("window");
        session.frames(2, cx);
        let mut check = |name: &str, ok: bool, detail: String| {
            println!("{} {name}: {detail}", if ok { "PASS" } else { "FAIL" });
            if !ok {
                failures.borrow_mut().push(name.to_string());
            }
        };
        // Rail of the volume slider: x 73 … 283 (inset 10), y 133.
        session.click(p(73.0 + 210.0 * 0.3, 133.0), cx);
        let l = shared.borrow().clone();
        check("pointer press sets the value from the rail", l.volume_inputs.last() == Some(&30), format!("{:?}", l.volume_inputs));
        check("release commits", l.volume_commits.last() == Some(&30), format!("{:?}", l.volume_commits));
        session.wheel(p(150.0, 133.0), 1.0, cx);
        let l = shared.borrow().clone();
        check("wheel up: +5 % then commit", l.volume_inputs.last() == Some(&35) && l.volume_commits.last() == Some(&35), format!("{:?} {:?}", l.volume_inputs, l.volume_commits));
        // The pointer press focused it: keys now.
        session.key("pageup", cx);
        session.key("left", cx);
        session.key("end", cx);
        let l = shared.borrow().clone();
        check("keys: PageUp +10 %, Left −5 %, End = max", l.volume_inputs.ends_with(&[45, 40, 100]), format!("{:?}", l.volume_inputs));
        // A drag far outside the slider keeps following (clamped).
        session.drag(p(150.0, 133.0), p(2000.0, -300.0), 6, cx);
        let l = shared.borrow().clone();
        check("drag follows the pointer outside, clamped, commits at release", l.volume_commits.last() == Some(&100) && l.volume_inputs.ends_with(&[37, 100]), format!("{:?}", &l.volume_inputs[l.volume_inputs.len().saturating_sub(4)..]));
        // Balance: a press near the centre snaps to it; double-click centres.
        session.click(p(73.0 + 210.0 * 0.52, 165.0), cx);
        session.click(p(73.0 + 210.0 * 0.9, 165.0), cx);
        let l = shared.borrow().clone();
        check("balance snaps within 4 % of the centre", l.balance_inputs.first() == Some(&0) || !l.balance_inputs.contains(&4), format!("{:?}", l.balance_inputs));
        session.double_click(p(73.0 + 210.0 * 0.9, 165.0), cx);
        let l = shared.borrow().clone();
        check("double-click centres the balance", l.balance_inputs.last() == Some(&0), format!("{:?}", l.balance_inputs));
        // Button: click, and keyboard (Tab to it, Enter).
        session.click(p(55.0, 227.0), cx);
        let _ = cx.update_window(session.any(), |_, window, cx| button_handle.focus(window, cx));
        session.key("enter", cx);
        session.key("space", cx);
        let l = shared.borrow().clone();
        check("button: click, then Enter and Space when focused (fired on key-up)", l.clicks == 3, format!("{} clicks", l.clicks));
        // Switch: click toggles.
        session.click(p(440.0, 239.0), cx);
        session.click(p(440.0, 239.0), cx);
        let l = shared.borrow().clone();
        check("switch toggles on click", l.switch == vec![true, false], format!("{:?}", l.switch));
        session.close(cx);
        cx.quit();
    });
    let failures = failures_out.borrow();
    assert!(failures.is_empty(), "failed: {failures:?}");
    println!("behaviour: OK ({:?})", log.borrow().clicks);
}
