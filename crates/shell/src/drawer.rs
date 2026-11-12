// The playlist drawer opens and closes by growing or shrinking the player window: playlist.rs's
// animation. 14 steps 14 ms apart (about 200 ms), eased 1 − (1 − t)³, heights rounded to whole
// logical pixels; opening near the bottom of the work area also moves the window up (same easing)
// so the drawer stays on screen; a newer animation (a quick second click) stops the one running
// and starts from the current height.
//
// It sets the window's size from outside, so GPUI's layout must not depend on the window height:
// the view lays the player out at 267 and the drawer at 320 below it, at fixed sizes, and the
// window only uncovers them (as the page did).

pub const STEPS: u32 = 14;
pub const STEP: std::time::Duration = std::time::Duration::from_millis(14);

pub fn ease(t: f64) -> f64 {
    1.0 - (1.0 - t).powi(3)
}

// One animation, in logical pixels: the client height from → to, and when the window must move
// up, its outer x and top from → to.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Plan {
    pub from: f64,
    pub to: f64,
    pub moving: Option<(f64, f64, f64)>,
}

// `outer` is the window's outer rectangle and `work_area` its monitor's (top, bottom), logical.
pub fn plan(from: f64, to: f64, outer: crate::window::Rect, work_area: Option<(f64, f64)>) -> Plan {
    let mut moving = None;
    if to > from {
        if let Some((top, bottom)) = work_area {
            let overflow = outer.y + outer.height + (to - from) - bottom;
            if overflow > 0.0 {
                moving = Some((outer.x, outer.y, (outer.y - overflow).max(top)));
            }
        }
    }
    Plan { from, to, moving }
}

impl Plan {
    // Step 1…STEPS: the client height and, when moving, the outer top-left.
    pub fn frame(&self, step: u32) -> (f64, Option<(f64, f64)>) {
        let eased = ease(step as f64 / STEPS as f64);
        let height = (self.from + (self.to - self.from) * eased).round();
        let position = self.moving.map(|(x, y0, y1)| (x, y0 + (y1 - y0) * eased));
        (height, position)
    }
}

#[cfg(windows)]
mod imp {
    use super::*;
    use crate::win;
    use std::sync::atomic::{AtomicU64, Ordering};
    use windows::Win32::Foundation::RECT;
    use windows::Win32::UI::WindowsAndMessaging::{GetClientRect, GetWindowRect, SetWindowPos, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOZORDER};

    // Bumped by every animation, so a newer one stops the one running.
    static ANIMATION: AtomicU64 = AtomicU64::new(0);

    // Animates the window's client height to `to` (logical). Returns at once. Call it on the
    // window's thread (GPUI's main thread). The timing runs on a thread of its own (precise 14 ms
    // sleeps); each step is posted back to the window and applied from the message loop, between
    // two events, so GPUI handles the resize as it would a user's. `on_done` runs on the window's
    // thread after the last step (not when a newer animation took over).
    pub fn animate(window: isize, to: f64, on_done: impl FnOnce() + Send + 'static) {
        let id = ANIMATION.fetch_add(1, Ordering::SeqCst) + 1;
        crate::window::ensure_subclass(window);
        std::thread::spawn(move || {
            let scale = crate::window::scale(window);
            let (Some(outer), Some((_, client_height))) = (crate::window::outer_rect(window), crate::window::client_size(window)) else { return };
            let from = client_height as f64 / scale;
            let area = crate::window::work_area(window).map(|area| (area.y, area.y + area.height));
            let plan = plan(from, to, outer, area);
            for step in 1..=STEPS {
                if ANIMATION.load(Ordering::SeqCst) != id {
                    return;
                }
                let (height, position) = plan.frame(step);
                let posted = crate::window::post_to(window, move || {
                    if ANIMATION.load(Ordering::SeqCst) == id {
                        set_client_height(window, height, position, scale);
                    }
                });
                if !posted {
                    return;
                }
                std::thread::sleep(STEP);
            }
            crate::window::post_to(window, move || {
                if ANIMATION.load(Ordering::SeqCst) == id {
                    on_done();
                }
            });
        });
    }

    // The outer size follows from the frame the window has now (whatever GPUI's non-client area
    // is), so the client area gets exactly `height`.
    fn set_client_height(window: isize, height: f64, position: Option<(f64, f64)>, scale: f64) {
        unsafe {
            let handle = win::hwnd(window);
            let (mut outer, mut client) = (RECT::default(), RECT::default());
            if GetWindowRect(handle, &mut outer).is_err() || GetClientRect(handle, &mut client).is_err() {
                return;
            }
            let frame_height = (outer.bottom - outer.top) - (client.bottom - client.top);
            let width = outer.right - outer.left;
            let outer_height = (height * scale).round() as i32 + frame_height;
            match position {
                Some((x, y)) => {
                    let _ = SetWindowPos(handle, None, (x * scale).round() as i32, (y * scale).round() as i32, width, outer_height, SWP_NOZORDER | SWP_NOACTIVATE);
                }
                None => {
                    let _ = SetWindowPos(handle, None, 0, 0, width, outer_height, SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE);
                }
            }
        }
    }
}

#[cfg(windows)]
pub use imp::animate;

// macOS: the app sets the window's frame itself (NSWindow setFrame:display:animate: would do the
// same); nothing here yet.
#[cfg(not(windows))]
pub fn animate(_window: isize, _to: f64, on_done: impl FnOnce() + Send + 'static) {
    on_done();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::window::Rect;

    #[test]
    fn steps_like_playlist_rs() {
        let outer = Rect { x: 100.0, y: 100.0, width: 876.0, height: 306.0 };
        let open = plan(267.0, 587.0, outer, Some((0.0, 1032.0)));
        assert_eq!(open.moving, None);
        let heights: Vec<f64> = (1..=STEPS).map(|step| open.frame(step).0).collect();
        assert_eq!(heights.len(), 14);
        assert_eq!(heights[13], 587.0);
        // Eased out: big steps first.
        assert_eq!(heights[0], (267.0 + 320.0 * (1.0 - (13.0f64 / 14.0).powi(3))).round());
        assert!(heights[0] - 267.0 > heights[13] - heights[12]);
        assert!(heights.windows(2).all(|pair| pair[1] >= pair[0]));
        // Closing never moves.
        let close = plan(587.0, 267.0, Rect { height: 626.0, ..outer }, Some((0.0, 1032.0)));
        assert_eq!(close.moving, None);
        assert_eq!(close.frame(STEPS).0, 267.0);
    }

    #[test]
    fn moves_up_near_the_bottom() {
        // The window's bottom at 1000 of a 1032 work area: 320 more would overflow by 288.
        let outer = Rect { x: 50.0, y: 694.0, width: 876.0, height: 306.0 };
        let open = plan(267.0, 587.0, outer, Some((0.0, 1032.0)));
        assert_eq!(open.moving, Some((50.0, 694.0, 406.0)));
        assert_eq!(open.frame(STEPS).1, Some((50.0, 406.0)));
        // Never above the work area's top.
        let outer = Rect { x: 50.0, y: 100.0, width: 876.0, height: 306.0 };
        let open = plan(267.0, 587.0, outer, Some((40.0, 500.0)));
        assert_eq!(open.moving, Some((50.0, 100.0, 40.0)));
        // A second click mid-way starts from the current height.
        let back = plan(400.0, 267.0, outer, Some((40.0, 500.0)));
        assert_eq!(back.frame(STEPS).0, 267.0);
    }
}
