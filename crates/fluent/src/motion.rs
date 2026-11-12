//! Animation and transition timing, as Chromium runs the page's CSS.
//!
//! GPUI has no CSS transitions or keyframes, so elements keep explicit state (when an animation
//! started, a [`Transition`] for a value) and sample it in `render`. Every sample goes through the
//! [`Motion`] global, which is either live (real time) or frozen for captures, the way the
//! reference renders were frozen:
//! - with no time given: infinite animations at 0, finite ones at their end, transitions at
//!   their end;
//! - with a time `t` (a fixture's `time`): every animation `t` ms after it started (finite ones
//!   clamped to their end), and transitions too when the fixture says `transitions`.
//!
//! An element that animates in live mode asks for the next frame itself
//! (`window.request_animation_frame()`), and only while something moves: frozen captures and
//! holds (the marquee's pauses) ask for nothing, so an idle window draws nothing.
//!
//! Reduced motion (Windows "Animation effects" off) removes the animations the CSS removes under
//! `prefers-reduced-motion: reduce` — marquee, equalizers, the preloading bolt, "dropped", the
//! dialog opening — but not the transitions (slider dot, toggle thumb, chevron).

use gpui::{App, Global};
use std::time::{Duration, Instant};

/// A CSS `cubic-bezier(x1, y1, x2, y2)` timing function, solved like Chromium's
/// `gfx::CubicBezier` (Newton–Raphson, then bisection, to 1e-7).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CubicBezier {
    ax: f64,
    bx: f64,
    cx: f64,
    ay: f64,
    by: f64,
    cy: f64,
}

impl CubicBezier {
    pub const fn new(x1: f64, y1: f64, x2: f64, y2: f64) -> CubicBezier {
        let cx = 3.0 * x1;
        let bx = 3.0 * (x2 - x1) - cx;
        let ax = 1.0 - cx - bx;
        let cy = 3.0 * y1;
        let by = 3.0 * (y2 - y1) - cy;
        let ay = 1.0 - cy - by;
        CubicBezier { ax, bx, cx, ay, by, cy }
    }

    fn sample_x(&self, t: f64) -> f64 {
        ((self.ax * t + self.bx) * t + self.cx) * t
    }

    fn sample_y(&self, t: f64) -> f64 {
        ((self.ay * t + self.by) * t + self.cy) * t
    }

    fn sample_dx(&self, t: f64) -> f64 {
        (3.0 * self.ax * t + 2.0 * self.bx) * t + self.cx
    }

    fn solve_x(&self, x: f64) -> f64 {
        const EPSILON: f64 = 1e-7;
        let mut t = x;
        for _ in 0..8 {
            let error = self.sample_x(t) - x;
            if error.abs() < EPSILON {
                return t;
            }
            let derivative = self.sample_dx(t);
            if derivative.abs() < 1e-6 {
                break;
            }
            t -= error / derivative;
        }
        let (mut low, mut high) = (0.0, 1.0);
        t = x;
        while low < high {
            let value = self.sample_x(t);
            if (value - x).abs() < EPSILON {
                return t;
            }
            if x > value {
                low = t;
            } else {
                high = t;
            }
            t = (high - low) / 2.0 + low;
            if high - low < EPSILON {
                break;
            }
        }
        t
    }

    /// The eased progress for input progress `x` (0–1; outside it, CSS extrapolates, which none
    /// of the page's functions reach).
    pub fn ease(&self, x: f64) -> f64 {
        if x <= 0.0 {
            return 0.0;
        }
        if x >= 1.0 {
            return 1.0;
        }
        self.sample_y(self.solve_x(x))
    }
}

/// `ease`.
pub const EASE: CubicBezier = CubicBezier::new(0.25, 0.1, 0.25, 1.0);
/// `ease-in-out`.
pub const EASE_IN_OUT: CubicBezier = CubicBezier::new(0.42, 0.0, 0.58, 1.0);
/// `cubic-bezier(0.1, 0.9, 0.2, 1)`: Fluent's decelerate curve (dialog, dropped, chevron, toggle).
pub const FLUENT: CubicBezier = CubicBezier::new(0.1, 0.9, 0.2, 1.0);

/// Every timing of the page (style.css; BEHAVIOUR.md section 5).
pub mod timing {
    use super::*;
    pub const MARQUEE_HOLD: f64 = 0.12;
    pub const EQUALIZER: Duration = Duration::from_millis(900);
    /// The second and third bars start this far into their cycle (`animation-delay: -0.3s / -0.6s`).
    pub const EQUALIZER_PHASES: [Duration; 3] = [Duration::ZERO, Duration::from_millis(300), Duration::from_millis(600)];
    pub const PRELOADING: Duration = Duration::from_millis(800);
    pub const DROPPED: Duration = Duration::from_millis(250);
    pub const DIALOG_IN: Duration = Duration::from_millis(250);
    pub const SLIDER_DOT: Duration = Duration::from_millis(100);
    pub const CHEVRON: Duration = Duration::from_millis(200);
    pub const TOGGLE_THUMB: Duration = Duration::from_millis(150);
    pub const TOGGLE_WIDTH: Duration = Duration::from_millis(100);
}

/// An infinite `alternate` animation's directed progress (0–1) after `elapsed`, before easing.
pub fn alternate(elapsed: Duration, period: Duration) -> f64 {
    let cycles = elapsed.as_secs_f64() / period.as_secs_f64().max(1e-9);
    let iteration = cycles.floor();
    let p = cycles - iteration;
    if iteration as u64 % 2 == 1 {
        1.0 - p
    } else {
        p
    }
}

/// The marquee's offset (0 → `distance`) after `elapsed` of `marquee var(--duration) ease-in-out
/// infinite alternate`, keyframes `0%, 12% { 0 }` and `88%, 100% { distance }`: the easing
/// applies to the 12 %–88 % segment only.
pub fn marquee_offset(elapsed: Duration, duration: Duration, distance: f64) -> f64 {
    let p = alternate(elapsed, duration);
    let hold = timing::MARQUEE_HOLD;
    if p <= hold {
        0.0
    } else if p >= 1.0 - hold {
        distance
    } else {
        distance * EASE_IN_OUT.ease((p - hold) / (1.0 - 2.0 * hold))
    }
}

/// How long the marquee stays still from `elapsed` on (inside a hold), so it can wait instead
/// of drawing frames. None while it moves.
pub fn marquee_still_for(elapsed: Duration, duration: Duration) -> Option<Duration> {
    let total = duration.as_secs_f64();
    let cycles = elapsed.as_secs_f64() / total;
    let iteration = cycles.floor();
    let p = cycles - iteration;
    let hold = timing::MARQUEE_HOLD;
    // A hold spans the end of one sweep and the start of the next (alternate): 88 %–100 % then
    // 0 %–12 % of the reversed iteration, 24 % of the duration in all.
    let remaining = if p < hold {
        hold - p
    } else if p >= 1.0 - hold {
        1.0 - p + hold
    } else {
        return None;
    };
    Some(Duration::from_secs_f64(remaining * total))
}

/// One equalizer bar's `scaleY` (0.25 → 1, 0.9 s ease-in-out alternate), bar 0–2.
pub fn equalizer_scale(elapsed: Duration, bar: usize) -> f64 {
    let p = alternate(elapsed + timing::EQUALIZER_PHASES[bar.min(2)], timing::EQUALIZER);
    0.25 + 0.75 * EASE_IN_OUT.ease(p)
}

/// The preloading bolt's opacity (0.15 → 0.6, 0.8 s ease-in-out alternate).
pub fn preloading_opacity(elapsed: Duration) -> f64 {
    0.15 + 0.45 * EASE_IN_OUT.ease(alternate(elapsed, timing::PRELOADING))
}

/// `dropped`: scale 0.94 → 1 over 0.25 s, Fluent easing.
pub fn dropped_scale(elapsed: Duration) -> f64 {
    let p = elapsed.as_secs_f64() / timing::DROPPED.as_secs_f64();
    0.94 + 0.06 * FLUENT.ease(p)
}

/// `dialog-in`: (opacity, scale) after `elapsed`: from (0, 1.05) to (1, 1) over 0.25 s, Fluent.
pub fn dialog_in(elapsed: Duration) -> (f64, f64) {
    let e = FLUENT.ease(elapsed.as_secs_f64() / timing::DIALOG_IN.as_secs_f64());
    (e, 1.05 - 0.05 * e)
}

/// How animations are sampled: live, or frozen for a capture.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Motion {
    pub frozen: Option<Frozen>,
    /// `prefers-reduced-motion: reduce`.
    pub reduced: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Frozen {
    /// The fixture's `time`: every animation this far after it started. None: infinite ones at
    /// 0, finite ones at their end.
    pub time: Option<Duration>,
    /// The fixture's `transitions`: transitions frozen at `time` too (else at their end).
    pub transitions: bool,
}

impl Global for Motion {}

impl Motion {
    pub fn is_live(&self) -> bool {
        self.frozen.is_none()
    }

    /// How far an animation that started at `start` is. `finite`: its duration, or None for an
    /// infinite one.
    pub fn elapsed(&self, start: Instant, finite: Option<Duration>) -> Duration {
        match self.frozen {
            None => start.elapsed(),
            Some(Frozen { time: Some(time), .. }) => finite.map_or(time, |end| time.min(end)),
            Some(Frozen { time: None, .. }) => finite.unwrap_or(Duration::ZERO),
        }
    }

    /// How far a transition of `duration` that started at `start` is.
    pub fn transition_elapsed(&self, start: Instant, duration: Duration) -> Duration {
        match self.frozen {
            None => start.elapsed(),
            Some(Frozen { time: Some(time), transitions: true }) => time.min(duration),
            Some(_) => duration,
        }
    }

    /// Whether a live animation should ask for another frame.
    pub fn wants_frames(&self) -> bool {
        self.frozen.is_none()
    }
}

/// The installed [`Motion`] (live, not reduced, when none was installed).
pub fn motion(cx: &App) -> Motion {
    cx.try_global::<Motion>().copied().unwrap_or_default()
}

/// A CSS transition of one number (`transform: scale(…)`, a width, a rotation), with Chromium's
/// rules for retargeting: a change mid-way starts from the current value with the full
/// duration, except a reversal back to where it came from, which is shortened in proportion
/// (CSS Transitions, "reversing shortening factor").
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Transition {
    from: f64,
    to: f64,
    start: Option<Instant>,
    duration: Duration,
    base: Duration,
    easing: CubicBezier,
    reversing_start: f64,
    shortening: f64,
}

impl Transition {
    /// At rest at `value`.
    pub fn new(value: f64, duration: Duration, easing: CubicBezier) -> Transition {
        Transition { from: value, to: value, start: None, duration, base: duration, easing, reversing_start: value, shortening: 1.0 }
    }

    pub fn target(&self) -> f64 {
        self.to
    }

    /// Eased progress 0–1 at this moment.
    fn progress(&self, motion: &Motion) -> f64 {
        let Some(start) = self.start else { return 1.0 };
        if self.duration.is_zero() {
            return 1.0;
        }
        let elapsed = motion.transition_elapsed(start, self.duration);
        self.easing.ease(elapsed.as_secs_f64() / self.duration.as_secs_f64())
    }

    /// The value now.
    pub fn value(&self, motion: &Motion) -> f64 {
        self.from + (self.to - self.from) * self.progress(motion)
    }

    /// Whether it is still moving (ask for frames while it is).
    pub fn running(&self, motion: &Motion) -> bool {
        self.start.is_some() && self.progress(motion) < 1.0
    }

    /// Sets a new end value (call it from `render` with what the style says now; nothing
    /// happens when it's the current target).
    pub fn set(&mut self, to: f64, motion: &Motion) {
        if to == self.to {
            return;
        }
        let now = Instant::now();
        let current = self.value(motion);
        let running = self.running(motion);
        if running && (to - self.reversing_start).abs() < 1e-9 {
            let progress = self.progress(motion);
            self.shortening = (progress * self.shortening + (1.0 - self.shortening)).abs().clamp(0.0, 1.0);
            self.duration = self.base.mul_f64(self.shortening);
        } else {
            self.shortening = 1.0;
            self.duration = self.base;
        }
        self.reversing_start = if running { self.to } else { current };
        self.from = current;
        self.to = to;
        self.start = Some(now);
    }

    /// Jumps to `value` with no transition.
    pub fn jump(&mut self, value: f64) {
        *self = Transition::new(value, self.base, self.easing);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn curves() {
        // Values Chromium gives (gfx::CubicBezier) at these inputs, to 1e-4.
        assert!((EASE.ease(0.5) - 0.8024).abs() < 1e-3, "{}", EASE.ease(0.5));
        assert!((EASE_IN_OUT.ease(0.5) - 0.5).abs() < 1e-6);
        assert!((EASE_IN_OUT.ease(0.25) - 0.1291).abs() < 1e-3, "{}", EASE_IN_OUT.ease(0.25));
        assert!(FLUENT.ease(0.24) > 0.8 && FLUENT.ease(1.0) == 1.0 && FLUENT.ease(0.0) == 0.0);
    }

    #[test]
    fn marquee_and_loops() {
        let d = Duration::from_secs(10);
        assert_eq!(marquee_offset(Duration::from_millis(500), d, -100.0), 0.0);
        assert_eq!(marquee_offset(Duration::from_millis(9500), d, -100.0), -100.0);
        assert!((marquee_offset(Duration::from_secs(5), d, -100.0) + 50.0).abs() < 1e-6);
        // The second sweep plays backwards: at 15 s it is half way back.
        assert!((marquee_offset(Duration::from_secs(15), d, -100.0) + 50.0).abs() < 1e-6);
        assert_eq!(marquee_offset(Duration::from_millis(19500), d, -100.0), 0.0);
        // At 90 %: the rest of this sweep's hold (10 %) and the next one's first 12 %.
        let still = marquee_still_for(Duration::from_secs(9), d).unwrap();
        assert!((still.as_secs_f64() - 2.2).abs() < 1e-9, "{still:?}");
        assert_eq!(marquee_still_for(Duration::from_secs(5), d), None);
        assert!((equalizer_scale(Duration::ZERO, 0) - 0.25).abs() < 1e-9);
        assert!((equalizer_scale(Duration::from_millis(900), 0) - 1.0).abs() < 1e-9);
        assert!((preloading_opacity(Duration::from_millis(800)) - 0.6).abs() < 1e-9);
        assert_eq!(dropped_scale(Duration::from_millis(250)), 1.0);
        assert_eq!(dialog_in(Duration::ZERO), (0.0, 1.05));
    }

    #[test]
    fn frozen_clock() {
        let start = Instant::now();
        let frozen = |time: Option<u64>, transitions| Motion { frozen: Some(Frozen { time: time.map(Duration::from_millis), transitions }), reduced: false };
        assert_eq!(frozen(None, false).elapsed(start, None), Duration::ZERO);
        assert_eq!(frozen(None, false).elapsed(start, Some(Duration::from_millis(250))), Duration::from_millis(250));
        assert_eq!(frozen(Some(60), false).elapsed(start, Some(Duration::from_millis(250))), Duration::from_millis(60));
        assert_eq!(frozen(Some(600), false).elapsed(start, Some(Duration::from_millis(250))), Duration::from_millis(250));
        assert_eq!(frozen(Some(60), false).transition_elapsed(start, Duration::from_millis(100)), Duration::from_millis(100));
        assert_eq!(frozen(Some(60), true).transition_elapsed(start, Duration::from_millis(100)), Duration::from_millis(60));
    }

    #[test]
    fn transitions_retarget_like_css() {
        let live = Motion::default();
        let mut t = Transition::new(1.0, Duration::from_millis(100), EASE);
        t.set(1.167, &live);
        assert!(t.running(&live));
        assert_eq!(t.target(), 1.167);
        // Reversed straight away: almost no distance covered, so almost nothing to undo.
        t.set(1.0, &live);
        assert!(t.duration <= Duration::from_millis(100));
        t.jump(0.833);
        assert_eq!(t.value(&live), 0.833);
    }
}
