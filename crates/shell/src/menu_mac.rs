// Native popup menus on macOS: an NSMenu shown with popUpMenuPositioningItem (synchronous: it runs
// the menu's own tracking loop and returns once it has closed). Untested: written for the macOS
// build, see CLAUDE.md.
//
// As on Windows, the menu is never shown from inside a GPUI callback (its tracking loop would
// re-enter GPUI while it is borrowed): `popup` queues it on the main dispatch queue, which runs it
// between two events, and hands the choice to `done` afterwards.
//
// The chosen item is reported by a small target object: every item's action is `choose:`, which
// records the item's tag (the menu id). Mnemonics ("Cu&t") are Windows-only and are stripped;
// accelerator texts aren't shown (macOS draws key equivalents itself, and these are Windows'
// names).

use super::{Item, Menu, Position};
use objc2::rc::Retained;
use objc2::runtime::NSObject;
use objc2::{define_class, msg_send, sel, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{NSControlStateValueOff, NSControlStateValueOn, NSEvent, NSMenu, NSMenuItem, NSView};
use objc2_foundation::{NSPoint, NSString};
use std::cell::{Cell, RefCell};
use std::ffi::c_void;

type Done = Box<dyn FnOnce(Option<u32>)>;

struct Pending {
    owner: isize,
    menu: Menu,
    position: Position,
    done: Done,
}

thread_local! {
    static PENDING: RefCell<Vec<Pending>> = const { RefCell::new(Vec::new()) };
    static CHOSEN: Cell<Option<u32>> = const { Cell::new(None) };
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "QustMenuTarget"]
    struct Target;

    impl Target {
        #[unsafe(method(choose:))]
        fn choose(&self, sender: &NSMenuItem) {
            CHOSEN.with(|chosen| chosen.set(u32::try_from(sender.tag()).ok()));
        }
    }
);

impl Target {
    fn new(mtm: MainThreadMarker) -> Retained<Target> {
        unsafe { msg_send![Target::alloc(mtm), init] }
    }
}

// libdispatch: run `work(context)` on the main queue, after the current callout.
#[repr(C)]
struct DispatchQueue {
    _private: [u8; 0],
}

unsafe extern "C" {
    static _dispatch_main_q: DispatchQueue;
    fn dispatch_async_f(queue: *const DispatchQueue, context: *mut c_void, work: extern "C" fn(*mut c_void));
}

extern "C" fn run_pending(_: *mut c_void) {
    let Some(mtm) = MainThreadMarker::new() else { return };
    loop {
        let next = PENDING.with(|queue| {
            let mut queue = queue.borrow_mut();
            (!queue.is_empty()).then(|| queue.remove(0))
        });
        let Some(pending) = next else { break };
        let chosen = show(mtm, &pending);
        (pending.done)(chosen);
    }
}

// Shows `menu` for `owner` (the window's NSView, from `gpui_glue::hwnd`) once the current event is
// handled, then calls `done` with the chosen item's id (None when dismissed). Call it on the main
// thread.
pub fn popup(owner: isize, menu: Menu, position: Position, done: impl FnOnce(Option<u32>) + 'static) {
    if MainThreadMarker::new().is_none() {
        done(None);
        return;
    }
    PENDING.with(|queue| queue.borrow_mut().push(Pending { owner, menu, position, done: Box::new(done) }));
    unsafe { dispatch_async_f(&raw const _dispatch_main_q, std::ptr::null_mut(), run_pending) };
}

// "Cu&t" -> "Cut", "&&" -> "&".
fn without_mnemonics(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '&' {
            if chars.peek() == Some(&'&') {
                out.push('&');
                chars.next();
            }
        } else {
            out.push(c);
        }
    }
    out
}

fn build(mtm: MainThreadMarker, menu: &Menu, target: &Target) -> Retained<NSMenu> {
    let ns_menu = NSMenu::new(mtm);
    ns_menu.setAutoenablesItems(false);
    for item in &menu.items {
        match item {
            Item::Separator => ns_menu.addItem(&NSMenuItem::separatorItem(mtm)),
            Item::Action { id, text, enabled, checked, .. } => {
                let title = NSString::from_str(&without_mnemonics(text));
                let empty = NSString::from_str("");
                let ns_item = unsafe { NSMenuItem::initWithTitle_action_keyEquivalent(NSMenuItem::alloc(mtm), &title, Some(sel!(choose:)), &empty) };
                unsafe { ns_item.setTarget(Some(target)) };
                ns_item.setTag(*id as isize);
                ns_item.setEnabled(*enabled);
                ns_item.setState(if *checked { NSControlStateValueOn } else { NSControlStateValueOff });
                ns_menu.addItem(&ns_item);
            }
        }
    }
    ns_menu
}

fn show(mtm: MainThreadMarker, pending: &Pending) -> Option<u32> {
    let target = Target::new(mtm);
    let menu = build(mtm, &pending.menu, &target);
    CHOSEN.with(|chosen| chosen.set(None));
    let view: Option<&NSView> = if pending.owner == 0 { None } else { Some(unsafe { &*(pending.owner as *const NSView) }) };
    match (pending.position, view) {
        (Position::Client { x, y }, Some(view)) => {
            // In the view's coordinates; AppKit's y grows upwards unless the view is flipped.
            let y = if view.isFlipped() { y } else { view.frame().size.height - y };
            menu.popUpMenuPositioningItem_atLocation_inView(None, NSPoint::new(x, y), Some(view));
        }
        _ => {
            // Screen coordinates: at the mouse.
            menu.popUpMenuPositioningItem_atLocation_inView(None, NSEvent::mouseLocation(), None);
        }
    }
    CHOSEN.with(|chosen| chosen.take())
}

#[cfg(test)]
mod tests {
    #[test]
    fn mnemonics() {
        assert_eq!(super::without_mnemonics("Cu&t"), "Cut");
        assert_eq!(super::without_mnemonics("Rock && Roll"), "Rock & Roll");
    }
}
