//! Interaction states forced on, the way the reference fixtures force them (`CSS.forcePseudoState`
//! on listed elements: several at once, independent of the pointer). Every interactive element
//! takes a [`Forced`]; in the app it is always `Forced::NONE` and the real pointer and focus
//! decide.

/// `:hover`, `:active`, `:focus`, `:focus-visible` forced on.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Forced {
    pub hover: bool,
    pub active: bool,
    pub focus: bool,
    pub focus_visible: bool,
}

impl Forced {
    pub const NONE: Forced = Forced { hover: false, active: false, focus: false, focus_visible: false };
    pub const HOVER: Forced = Forced { hover: true, ..Forced::NONE };
    pub const ACTIVE: Forced = Forced { active: true, ..Forced::NONE };
    pub const FOCUS: Forced = Forced { focus: true, ..Forced::NONE };
    pub const FOCUS_VISIBLE: Forced = Forced { focus: true, focus_visible: true, ..Forced::NONE };

    /// From a fixture's list (`["hover", "active", "focus", "focus-visible"]`).
    pub fn from_names<'a>(names: impl IntoIterator<Item = &'a str>) -> Forced {
        let mut forced = Forced::NONE;
        for name in names {
            match name {
                "hover" => forced.hover = true,
                "active" => forced.active = true,
                "focus" => forced.focus = true,
                "focus-visible" => {
                    forced.focus = true;
                    forced.focus_visible = true;
                }
                _ => {}
            }
        }
        forced
    }

    pub fn any(&self) -> bool {
        *self != Forced::NONE
    }

    pub fn union(self, other: Forced) -> Forced {
        Forced { hover: self.hover || other.hover, active: self.active || other.active, focus: self.focus || other.focus, focus_visible: self.focus_visible || other.focus_visible }
    }
}
