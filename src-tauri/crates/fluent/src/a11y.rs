//! Accessibility: every element carries the role and name its HTML counterpart had.
//!
//! GPUI exposes an element to screen readers (AccessKit, UI Automation on Windows) when it has an
//! id and a role: `div().id(..).role(Role::Button).aria_label("Play")`. GPUI has `aria_label`,
//! `aria_selected`, `aria_expanded`, `aria_toggled` (`aria-pressed`, checkboxes, switches),
//! `aria_numeric_value` / `aria_min_numeric_value` / `aria_max_numeric_value` (sliders) but no
//! `aria-disabled`; [`A11yExt::a11y_disabled`] sets it on the AccessKit node directly.
//!
//! The page's roles, element by element: buttons `Button` (`aria-label` for icon buttons,
//! `aria-pressed` on Instant, `aria-expanded` on Playlist); sliders `Slider` with
//! `aria-valuenow/min/max` and their label ("Position", "Volume", "Balance"); the playlist
//! `ListBox` (multi-selectable) with `ListBoxOption` rows (`aria-selected`); the Settings toggles
//! `Switch` (`role="switch"` checkboxes, `aria_toggled`); the dialog `Dialog` labelled by its
//! title; text boxes are GPUI Kit's `Input` (role `TextInput`, labelled by their field label).

use gpui::StatefulInteractiveElement;

pub trait A11yExt: StatefulInteractiveElement + Sized {
    /// `aria-disabled` / `disabled`.
    fn a11y_disabled(self, disabled: bool) -> Self {
        if !disabled {
            return self;
        }
        self.a11y_synthetic_children(|builder| builder.parent_node().set_disabled())
    }
}

impl<T: StatefulInteractiveElement> A11yExt for T {}
