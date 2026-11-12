//! The deck bar (`.deck-bar`, BEHAVIOUR.md 2.6): Playlist (with its chevron, turned 180° while
//! the drawer is open, `transform 0.2s cubic-bezier(0.1, 0.9, 0.2, 1)`), Add file, Add URL.

use crate::dialog::DialogTarget;
use crate::view::{forced, Shared};
use gpui::{prelude::*, px, Context, FocusHandle, IntoElement, Render, Window};
use qust_fluent::motion::{motion, timing, Transition, FLUENT};
use qust_fluent::{rotated_icon, surface, theme, Button, Frame, Icon};
use std::f32::consts::PI;
use std::rc::Rc;

pub struct DeckView {
    shared: Rc<Shared>,
    pub focus: [FocusHandle; 3],
    chevron: Transition,
    pub renders: u64,
}

impl DeckView {
    pub(crate) fn new(shared: Rc<Shared>, cx: &mut Context<Self>) -> DeckView {
        let open = shared.drawer_open.get();
        let focus = std::array::from_fn(|_| qust_fluent::focus::tab_stop(cx));
        DeckView { shared, focus, chevron: Transition::new(if open { 1.0 } else { 0.0 }, timing::CHEVRON, FLUENT), renders: 0 }
    }
}

impl Render for DeckView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.renders += 1;
        let theme = *theme(cx);
        let motion = motion(cx);
        let open = self.shared.drawer_open.get();
        self.chevron.set(if open { 1.0 } else { 0.0 }, &motion);
        let turn = self.chevron.value(&motion) as f32;
        if self.chevron.running(&motion) && motion.wants_frames() {
            window.request_animation_frame();
        }
        let chevron = rotated_icon(Icon::Chevron, 12.0, turn * PI).ml(px(1.0)).text_color(theme.text_2);

        let s = self.shared.clone();
        let this = cx.entity().downgrade();
        let playlist = Button::new("playlistButton")
            .icon(Icon::List)
            .label("Playlist")
            .trailing(chevron)
            .gap(7.0)
            .expanded(open)
            .tooltip("Show or hide the playlist")
            .focus(&self.focus[0])
            .forced(forced(cx, "#playlistButton"))
            .on_click(move |_, window, cx| {
                // setDrawer(!drawerOpen): the button follows at once, the window grows after.
                let open = !s.drawer_open.get();
                s.drawer_open.set(open);
                let _ = this.update(cx, |_, cx| cx.notify());
                s.host.set_drawer(open, window, cx);
            })
            .flex_1()
            .px(px(8.0))
            .text_size(px(13.0));
        let s = self.shared.clone();
        let add_file = Button::new("addFileButton")
            .icon(Icon::Note)
            .label("Add file")
            .gap(7.0)
            .tooltip("Add music files or playlists from your computer")
            .focus(&self.focus[1])
            .forced(forced(cx, "#addFileButton"))
            .on_click(move |_, window, cx| s.host.add_files(window, cx))
            .flex_1()
            .px(px(8.0))
            .text_size(px(13.0));
        let s = self.shared.clone();
        let add_url = Button::new("addUrlButton")
            .icon(Icon::Link)
            .label("Add URL")
            .gap(7.0)
            .tooltip("Add a stream or an online audio file")
            .focus(&self.focus[2])
            .forced(forced(cx, "#addUrlButton"))
            .on_click(move |_, window, cx| s.open_dialog(DialogTarget::Url, window, cx))
            .flex_1()
            .px(px(8.0))
            .text_size(px(13.0));

        surface(Frame::island(&theme)).size_full().flex().gap(px(8.0)).py(px(9.0)).px(px(10.0)).child(playlist).child(add_file).child(add_url)
    }
}
