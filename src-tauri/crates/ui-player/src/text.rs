//! What a title, a station name, a host or a file name shows: `qust_text::shown` of it (right-to-
//! left text in the order GPUI needs it; anything else as it is), for every such text of this
//! crate: the station line, the marquee, the cartridges' names and hosts (the drag ghost's too)
//! and the dialog's error. Tooltips, accessibility labels and text boxes keep the logical text.
//!
//! The views re-render on a model change or a pointer move (the drag ghost), not per frame; the
//! few strings are remembered so that a text is converted once when it appears, not at each of
//! those renders. Everything else about text (kerning, Chromium's baseline and ellipsis,
//! letter-spacing) is `qust_fluent::text`.

use gpui::SharedString;
use std::cell::RefCell;
use std::collections::HashMap;

/// How many texts are remembered (six cartridges' names and hosts, the station, the title, the
/// dialog's error, with room for the ones they replace).
const REMEMBERED: usize = 64;

thread_local! {
    static SHOWN: RefCell<HashMap<String, SharedString>> = RefCell::new(HashMap::new());
}

/// `qust_text::shown(text)`, remembered.
pub fn shown(text: &str) -> SharedString {
    SHOWN.with(|memo| {
        let mut memo = memo.borrow_mut();
        if let Some(found) = memo.get(text) {
            return found.clone();
        }
        let shown: SharedString = qust_text::shown(&text.to_string()).into();
        if memo.len() >= REMEMBERED {
            memo.clear();
        }
        memo.insert(text.to_string(), shown.clone());
        shown
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn left_to_right_text_is_unchanged() {
        assert_eq!(shown("KEXP 90.3"), "KEXP 90.3");
        assert_eq!(shown("KEXP 90.3"), "KEXP 90.3");
        // Right-to-left text comes back changed (its order or a mark for GPUI), remembered.
        let arabic = shown("فيروز - كيفك إنت");
        assert_ne!(arabic.as_ref(), "فيروز - كيفك إنت");
        assert_eq!(shown("فيروز - كيفك إنت"), arabic);
    }
}
