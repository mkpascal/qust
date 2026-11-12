//! The dialog (`#dialog`, BEHAVIOUR.md 2.8) in its four forms — Load / Edit cartridge, Add URL,
//! Edit track for a URL or a local file — shown as an overlay of the window root while open.
//!
//! Rules kept from script.js: the target is fixed when it opens (`dialogTarget`), and an answer
//! only closes or shows its error on the opening it was for; a second submit while one is
//! pending is ignored; on an error the dialog stays open, shows Rust's message and focuses the
//! URL field; Eject closes then ejects; Esc (inside the dialog) and Cancel close; a click on the
//! backdrop does nothing; Tab cycles Name, URL, Save, Eject (if shown), Cancel.

use crate::view::{forced, Shared};
use gpui::{
    div, prelude::*, AnyWindowHandle, Focusable, App, Context, Entity, FocusHandle, IntoElement, KeyDownEvent, Render, SharedString, Window,
};
use gpui_component::input::InputState;
use qust_fluent::dialog::{backdrop, dialog_body, dialog_error, dialog_footer, dialog_title, field};
use qust_fluent::focus::FocusCycle;
use qust_fluent::{input_state, Button, DialogCard, TextBox, TextStyles};
use qust_shell::menu::{edit_menu, EditCommand};
use std::rc::Rc;
use std::time::Instant;

/// What the dialog edits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DialogTarget {
    /// A cartridge slot (0–5): Load cartridge when empty, Edit cartridge when loaded.
    Cartridge(usize),
    /// A playlist entry: Edit track.
    Entry(u64),
    /// Add URL.
    Url,
}

const AUDIO_EXTENSIONS: [&str; 11] = ["mp3", "m4a", "aac", "mp4", "ogg", "oga", "opus", "flac", "wav", "webm", "weba"];

fn strip_extension(name: &str) -> &str {
    // `.replace(/\.[^.]+$/, '')`
    match name.rfind('.') {
        Some(at) if at + 1 < name.len() => &name[..at],
        _ => name,
    }
}

/// `fileTitle(path)`: the file's name without its extension.
pub fn file_title(path: &str) -> String {
    strip_extension(path.rsplit(['\\', '/']).next().unwrap_or(path)).to_string()
}

fn percent_decode(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = text.get(i + 1..i + 3)?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// `host(url)`: `new URL(url).host`, or the text itself when it isn't a URL.
pub fn host(url: &str) -> String {
    if url.contains("://") {
        qust_ui::format::host(url)
    } else {
        url.to_string()
    }
}

/// `urlTitle(url)`: an online file's name, or a stream's host (what Rust names an entry left
/// without a name).
pub fn url_title(url: &str) -> String {
    let Some((_, rest)) = url.split_once("://") else { return url.to_string() };
    let path = rest.find(['/', '?', '#']).map_or("", |at| &rest[at..]);
    let path = path.split(['?', '#']).next().unwrap_or("");
    let Some(name) = percent_decode(path.rsplit('/').next().unwrap_or("")) else { return url.to_string() };
    let audio = name.rsplit_once('.').is_some_and(|(_, ext)| AUDIO_EXTENSIONS.contains(&ext.to_ascii_lowercase().as_str()));
    if audio {
        strip_extension(&name).to_string()
    } else {
        host(url)
    }
}

/// The texts of one form of the dialog.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Form {
    pub title: SharedString,
    pub name_label: SharedString,
    pub name_placeholder: SharedString,
    pub url_label: SharedString,
    pub save: SharedString,
    pub name: String,
    pub url: String,
    pub ejectable: bool,
    /// A local file's path: shown, not edited.
    pub fixed_url: bool,
}

pub struct DialogView {
    shared: Rc<Shared>,
    target: Option<DialogTarget>,
    /// Which opening is showing: an answer to an earlier one leaves this one alone.
    opening: u64,
    form: Form,
    error: Option<String>,
    saving: bool,
    opened_at: Instant,
    name: Entity<InputState>,
    url: Entity<InputState>,
    save: FocusHandle,
    eject: FocusHandle,
    cancel: FocusHandle,
    pub renders: u64,
}

impl DialogView {
    pub(crate) fn new(shared: Rc<Shared>, window: &mut Window, cx: &mut Context<Self>) -> DialogView {
        let name = input_state("My station", "", window, cx);
        let url = input_state("https://", "", window, cx);
        DialogView {
            shared,
            target: None,
            opening: 0,
            form: Form {
                title: "Load cartridge".into(),
                name_label: "Name".into(),
                name_placeholder: "My station".into(),
                url_label: "Stream URL".into(),
                save: "Save".into(),
                name: String::new(),
                url: String::new(),
                ejectable: false,
                fixed_url: false,
            },
            error: None,
            saving: false,
            opened_at: Instant::now(),
            name,
            url,
            save: qust_fluent::focus::tab_stop(cx),
            eject: qust_fluent::focus::tab_stop(cx),
            cancel: qust_fluent::focus::tab_stop(cx),
            renders: 0,
        }
    }

    pub fn is_open(&self) -> bool {
        self.target.is_some()
    }

    pub fn target(&self) -> Option<DialogTarget> {
        self.target
    }

    pub fn form(&self) -> &Form {
        &self.form
    }

    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    pub fn saving(&self) -> bool {
        self.saving
    }

    pub fn name_input(&self) -> &Entity<InputState> {
        &self.name
    }

    pub fn url_input(&self) -> &Entity<InputState> {
        &self.url
    }

    pub fn button_focus(&self) -> (&FocusHandle, &FocusHandle, &FocusHandle) {
        (&self.save, &self.eject, &self.cancel)
    }

    /// The form for `target` as the window's state is now (None: an entry that isn't there).
    fn form_for(&self, target: DialogTarget, cx: &App) -> Option<Form> {
        let models = &self.shared.models;
        Some(match target {
            DialogTarget::Cartridge(slot) => {
                let cartridge = models.deck.read(cx).cartridges.get(slot).cloned().flatten();
                Form {
                    title: if cartridge.is_some() { "Edit cartridge" } else { "Load cartridge" }.into(),
                    name_label: "Name".into(),
                    name_placeholder: "My station".into(),
                    url_label: "Stream URL".into(),
                    save: "Save".into(),
                    name: cartridge.as_ref().map(|c| c.title.clone()).unwrap_or_default(),
                    url: cartridge.as_ref().map(|c| c.url.clone()).unwrap_or_default(),
                    ejectable: cartridge.is_some(),
                    fixed_url: false,
                }
            }
            DialogTarget::Entry(id) => {
                let entry = models.playlist.read(cx).entry(id)?.clone();
                let file = entry.path.is_some();
                Form {
                    title: "Edit track".into(),
                    name_label: "Name".into(),
                    name_placeholder: match (&entry.path, &entry.url) {
                        (Some(path), _) => file_title(path),
                        (None, Some(url)) => url_title(url),
                        (None, None) => String::new(),
                    }
                    .into(),
                    url_label: if file { "File" } else { "Stream or audio file URL" }.into(),
                    save: "Save".into(),
                    name: entry.title.clone(),
                    url: entry.path.clone().or(entry.url.clone()).unwrap_or_default(),
                    ejectable: false,
                    fixed_url: file,
                }
            }
            DialogTarget::Url => Form {
                title: "Add URL".into(),
                name_label: "Name (optional)".into(),
                name_placeholder: "Taken from the address".into(),
                url_label: "Stream or audio file URL".into(),
                save: "Add".into(),
                name: String::new(),
                url: String::new(),
                ejectable: false,
                fixed_url: false,
            },
        })
    }

    /// `openDialog(slot)` / `openEntryDialog(id)` / `openUrlDialog()`.
    pub fn open(&mut self, target: DialogTarget, window: &mut Window, cx: &mut Context<Self>) {
        let Some(form) = self.form_for(target, cx) else { return };
        let was_open = self.target.is_some();
        self.target = Some(target);
        self.opening += 1;
        self.error = None;
        self.name.update(cx, |state, cx| {
            state.set_value(form.name.clone(), window, cx);
            state.set_placeholder(form.name_placeholder.clone(), window, cx);
        });
        self.url.update(cx, |state, cx| {
            state.set_value(form.url.clone(), window, cx);
            state.set_placeholder("https://", window, cx);
        });
        self.form = form;
        // The opening animation runs when the backdrop goes from hidden to shown.
        if !was_open {
            self.opened_at = Instant::now();
            let view = cx.entity();
            if let Some(root) = self.shared.root.upgrade() {
                root.update(cx, |root, cx| root.set_overlay("dialog", view, cx));
            }
        }
        let field = if target == DialogTarget::Url { &self.url } else { &self.name };
        field.read(cx).focus_handle(cx).focus(window, cx);
        cx.notify();
    }

    /// `closeDialog()`: instant.
    pub fn close(&mut self, cx: &mut Context<Self>) {
        if self.target.take().is_none() {
            return;
        }
        if let Some(root) = self.shared.root.upgrade() {
            root.update(cx, |root, cx| root.remove_overlay("dialog", cx));
        }
        cx.notify();
    }

    /// The URL field's text (for tests and captures that type into it).
    pub fn set_url_text(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.url.update(cx, |state, cx| state.set_value(text.to_string(), window, cx));
    }

    pub fn set_name_text(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.name.update(cx, |state, cx| state.set_value(text.to_string(), window, cx));
    }

    /// The form's submit: Enter in a field, or Save / Add.
    pub fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(target) = self.target else { return };
        if self.saving {
            return;
        }
        self.saving = true;
        let title = self.name.read(cx).value().trim().to_string();
        let url = self.url.read(cx).value().trim().to_string();
        let commands = self.shared.commands.clone();
        let fixed = self.form.fixed_url;
        // Every answer as Ok(ids added) or Err(message).
        let answer: std::pin::Pin<Box<dyn std::future::Future<Output = Option<Result<Vec<u64>, String>>>>> = match target {
            DialogTarget::Cartridge(slot) => {
                let reply = commands.save_cartridge(slot, title, url);
                Box::pin(async move { reply.await.map(|r| r.map(|_| Vec::new())) })
            }
            DialogTarget::Entry(id) => {
                let reply = commands.edit_entry(id, title, if fixed { None } else { Some(url) });
                Box::pin(async move { reply.await.map(|r| r.map(|_| Vec::new())) })
            }
            DialogTarget::Url => commands.add_url(url, title),
        };
        let opening = self.opening;
        cx.spawn_in(window, async move |this, cx| {
            let result = answer.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.saving = false;
                let Some(result) = result else { return };
                let current = this.opening == opening && this.target.is_some();
                match result {
                    Ok(ids) => {
                        if current {
                            this.close(cx);
                        }
                        if target == DialogTarget::Url && !ids.is_empty() {
                            this.shared.host.added(ids, window, cx);
                        }
                    }
                    Err(error) => {
                        if !current {
                            return;
                        }
                        this.error = Some(error);
                        this.url.read(cx).focus_handle(cx).focus(window, cx);
                        cx.notify();
                    }
                }
            });
        })
        .detach();
    }

    /// Eject: closes, then ejects.
    pub fn eject(&mut self, cx: &mut Context<Self>) {
        let Some(DialogTarget::Cartridge(slot)) = self.target else { return };
        self.close(cx);
        self.shared.commands.eject(slot);
    }

    fn edit_menu(&self, input: Entity<InputState>, window: &mut Window, cx: &mut App) {
        let handle: AnyWindowHandle = window.window_handle();
        self.shared.host.show_menu(
            edit_menu(),
            window,
            cx,
            Box::new(move |chosen, cx: &mut App| {
                let Some(command) = chosen.and_then(EditCommand::from_id) else { return };
                let _ = handle.update(cx, |_, window, cx| {
                    input.read(cx).focus_handle(cx).focus(window, cx);
                    let action: Box<dyn gpui::Action> = match command {
                        EditCommand::Cut => Box::new(gpui_component::input::Cut),
                        EditCommand::Copy => Box::new(gpui_component::input::Copy),
                        EditCommand::Paste => Box::new(gpui_component::input::Paste),
                        EditCommand::SelectAll => Box::new(gpui_component::input::SelectAll),
                    };
                    window.dispatch_action(action, cx);
                });
            }),
        );
    }
}

impl Render for DialogView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.renders += 1;
        if self.target.is_none() {
            return div().into_any_element();
        }
        let this = cx.entity().downgrade();
        let form = self.form.clone();
        let name_focus = self.name.read(cx).focus_handle(cx);
        let url_focus = self.url.read(cx).focus_handle(cx);
        let mut cycle = vec![name_focus.clone(), url_focus.clone(), self.save.clone()];
        if form.ejectable {
            cycle.push(self.eject.clone());
        }
        cycle.push(self.cancel.clone());
        let cycle = FocusCycle::new(cycle);

        let menu_this = this.clone();
        let name_input = self.name.clone();
        let name_box = TextBox::new("streamTitle", &self.name).aria_label(form.name_label.clone()).forced(forced(cx, "#streamTitle")).on_context_menu(move |_, window, cx| {
            let input = name_input.clone();
            let _ = menu_this.update(cx, |dialog, cx| dialog.edit_menu(input, window, cx));
        });
        let menu_this = this.clone();
        let url_input = self.url.clone();
        let url_box = TextBox::new("streamURL", &self.url).aria_label(form.url_label.clone()).readonly(form.fixed_url).forced(forced(cx, "#streamURL")).on_context_menu(move |_, window, cx| {
            let input = url_input.clone();
            let _ = menu_this.update(cx, |dialog, cx| dialog.edit_menu(input, window, cx));
        });

        let body = dialog_body()
            .child(dialog_title(form.title.clone(), cx))
            .child(field(form.name_label.clone(), name_box, cx))
            .child(field(form.url_label.clone(), url_box, cx))
            .when_some(self.error.clone(), |body, error| body.child(dialog_error(crate::text::shown(&error), cx)));

        let submit = this.clone();
        let eject = this.clone();
        let cancel = this.clone();
        let footer = dialog_footer(cx)
            .child(
                Button::new("saveButton")
                    .label(form.save.clone())
                    .accent()
                    .focus(&self.save)
                    .forced(forced(cx, "#saveButton"))
                    .on_click(move |_, window, cx| {
                        let _ = submit.update(cx, |dialog, cx| dialog.submit(window, cx));
                    })
                    .flex_1(),
            )
            .when(form.ejectable, |footer| {
                footer.child(
                    Button::new("ejectButton")
                        .label("Eject")
                        .focus(&self.eject)
                        .forced(forced(cx, "#ejectButton"))
                        .on_click(move |_, _, cx| {
                            let _ = eject.update(cx, |dialog, cx| dialog.eject(cx));
                        })
                        .flex_1(),
                )
            })
            .child(
                Button::new("cancelButton")
                    .label("Cancel")
                    .focus(&self.cancel)
                    .forced(forced(cx, "#cancelButton"))
                    .on_click(move |_, _, cx| {
                        let _ = cancel.update(cx, |dialog, cx| dialog.close(cx));
                    })
                    .flex_1(),
            );

        let keys = this.clone();
        let card = div()
            .id("dialogForm")
            .capture_key_down(move |event: &KeyDownEvent, window, cx| {
                let key = event.keystroke.key.as_str();
                let plain = !event.keystroke.modifiers.modified();
                if key == "escape" {
                    let _ = keys.update(cx, |dialog, cx| dialog.close(cx));
                    cx.stop_propagation();
                } else if key == "enter" && plain && (name_focus.is_focused(window) || url_focus.is_focused(window)) {
                    // Implicit submission from a text field.
                    let _ = keys.update(cx, |dialog, cx| dialog.submit(window, cx));
                    cx.stop_propagation();
                }
            })
            .child(DialogCard::new("dialog", self.opened_at, form.title.clone()).child(body).child(footer));
        let _ = window;
        backdrop(cx)
            .font_text()
            // Right-clicks inside the dialog show no player menu (text boxes have their own).
            .on_mouse_up(gpui::MouseButton::Right, |_, _, cx| cx.stop_propagation())
            .child(cycle.trap(card))
            .into_any_element()
    }
}
