//! The drawer view: the list island (pinned cartridges, divider, tracks or the empty hint, the
//! scrollbar), the status line, and every interaction of script.js's playlist part.
//!
//! **Rendering.** One view, virtualized by arithmetic: every row is 26 px, so only the rows that
//! intersect the 282 px viewport are built (about 13, whatever the playlist's length), each
//! placed absolutely at its `offsetTop` minus the scroll position inside a clipping viewport.
//! The view re-renders only when what it shows changed: a `player-playlist` (rows rebuilt), a
//! `player-entry` that touches a shown row or the status line, a deck change that touches the
//! pinned rows or which row is active, the selection, the scroll, a drag. Clock ticks and
//! state changes that don't affect it (volume, title, preload bolts) don't reach it.
//!
//! GPUI note, measured in its source (`view.rs`, `prepaint_view`): a cached view that is dirty
//! re-renders its whole subtree, cached child views included (it renders them with
//! `window.refreshing` set), and a notify marks every ancestor view dirty. So nested row views
//! would not let one row re-render alone; a re-render of this view rebuilds the visible rows,
//! which is why it keeps them few.
//!
//! **The playing row's equalizer** animates at the frame rate. Mounted with [`mount`], it is
//! drawn by a `qust_fluent::EqualizerLayer`, a sibling live layer, and this view never
//! re-renders for it.
//! Without the layer it is drawn here, and then this view re-renders every frame while the
//! playing row is on screen. Either way it stands still while the drawer is closed.
//!
//! **Inert while closed.** Still painted (the window uncovers it while growing and hides it
//! while shrinking, as the page's drawer was simply cut off by the window), but with no focus,
//! no mouse or key handlers, no tooltips and no accessibility roles. Only the external file-drag
//! listeners stay, since dragging files over the window opens the drawer.
//!
//! **Pointer handling** follows the page's events: a left press on the list selects an
//! unselected track and arms the reorder drag; the "click" is decided at release (the target is
//! the row if press and release were on the same row, else the list); the drag, the scrollbar
//! drag and the release are window-wide listeners, as the page's `document` listeners were.

use crate::host::{DrawerHost, Panel, PlaylistCommands};
use crate::list::{self, Item, Move, Rect, Rows, Selection, CLIENT_HEIGHT, PADDING, ROW};
use crate::menu::{self, Action};
use gpui::prelude::FluentBuilder;
use gpui::{
    canvas, div, px, AnyElement, App, Context, CursorStyle, DispatchPhase, DragMoveEvent, Entity, ExternalPaths, FileDropEvent, FocusHandle, InteractiveElement,
    IntoElement, KeyDownEvent, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, ParentElement, Pixels, Point, Render, Role, ScrollDelta, ScrollEvent,
    AppContext, SharedString, StatefulInteractiveElement, Styled, WeakEntity, Window,
};
use qust_fluent::equalizer::{EqualizerLayer, EqualizerPlacement};
use qust_fluent::scrollbar::{self, ScrollbarKind};
use qust_fluent::{focus, icon, line, list as row_ui, motion, surface, theme, Forced, Frame, Icon, TextStyles, Theme, TooltipExt};
use qust_ui::models::{Deck, PlaylistModel};
use qust_ui::{portal_slot, Models, WindowRoot};
use std::cell::Cell;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::rc::Rc;
use std::time::{Duration, Instant};

/// The drawer's size (the window's lower part, `--drawer-height`).
pub const WIDTH: f32 = 860.0;
pub const HEIGHT: f32 = 320.0;
/// The island's height: 320 − 14 bottom padding − 16 status line − 6 gap.
const ISLAND_HEIGHT: f32 = 284.0;
/// `::-webkit-scrollbar { width: 10px }`, thumb inside a 3 px transparent border.
const SCROLLBAR: f32 = 10.0;
/// Chromium's wheel: 100 px per notch with the default three lines per notch.
const WHEEL_LINE: f32 = 100.0 / 3.0;
pub const HINT: &str = "Double-click to play \u{b7} drag to reorder \u{b7} Delete to remove";
pub const EMPTY_TITLE: &str = "Drop music files or folders here";
pub const EMPTY_TEXT: &str = "or use Add file and Add URL. Files play from where they are.";

/// A pinned cartridge as its row shows it (`shown_*`: what it draws, `qust_text::shown`).
#[derive(Clone, Debug, PartialEq)]
struct Pinned {
    slot: usize,
    title: SharedString,
    url: SharedString,
    host: SharedString,
    shown_title: SharedString,
    shown_host: SharedString,
}

/// A left press on the list, until its release decides the click.
#[derive(Clone, Copy, Debug)]
struct Press {
    item: Option<Item>,
}

/// The reorder drag (`rowDrag`).
#[derive(Clone, Copy, Debug)]
struct Reorder {
    y: f32,
    active: bool,
    index: Option<usize>,
}

#[derive(Clone, Copy, Debug)]
struct ThumbDrag {
    y: f32,
    scroll: f32,
}

#[derive(Clone, Copy, Debug)]
struct TrackPress {
    /// −1 up, 1 down.
    direction: f32,
    /// The pointer's height on the track (window coordinates).
    y: f32,
    generation: u64,
}

/// Counters for measurements and tests.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Stats {
    /// Renders of the view.
    pub renders: u64,
    /// Row elements built (all renders together).
    pub rows_built: u64,
    /// Menus asked for.
    pub menus: u64,
}

pub struct Drawer {
    playlist: Entity<PlaylistModel>,
    deck: Entity<Deck>,
    commands: Rc<dyn PlaylistCommands>,
    host: Rc<dyn DrawerHost>,
    focus: FocusHandle,
    open: bool,
    rows: Rows,
    pinned: Vec<Pinned>,
    version: Option<u64>,
    active: Option<Item>,
    playing: bool,
    eq_started: Instant,
    total: f64,
    selection: Selection,
    added: Option<Vec<u64>>,
    scroll: f32,
    drop_line: Option<usize>,
    drop_empty: bool,
    external: Option<Vec<PathBuf>>,
    press: Option<Press>,
    suppress_click: bool,
    reorder: Option<Reorder>,
    thumb: Option<ThumbDrag>,
    track: Option<TrackPress>,
    track_generation: u64,
    /// The island's border box in window coordinates, from the last paint.
    geometry: Rc<Cell<Option<Rect>>>,
    /// Entry versions of the rows last rendered (a `player-entry` re-renders only if one moved).
    shown: HashMap<u64, u64>,
    /// What an entry's row draws (title, detail through `qust_text::shown`), by id, with the
    /// entry version it was computed for; emptied by every rebuild.
    display: HashMap<u64, (u64, SharedString, SharedString)>,
    forced_hover: HashSet<Item>,
    /// The playing row's equalizer is drawn by an [`EqualizerLayer`] (see [`mount`]).
    equalizer_layer: bool,
    /// The root, to place the equalizer layer's slot.
    root: Option<WeakEntity<WindowRoot>>,
    pub stats: Stats,
}

impl Drawer {
    /// The drawer over the player's models. `open`: as the window was built (`playlist-open`).
    pub fn new(models: &Models, commands: Rc<dyn PlaylistCommands>, host: Rc<dyn DrawerHost>, open: bool, cx: &mut Context<Self>) -> Drawer {
        cx.observe(&models.playlist, |this, _, cx| {
            if this.sync_playlist(cx) {
                cx.notify();
            }
        })
        .detach();
        cx.observe(&models.deck, |this, _, cx| {
            if this.sync_deck(cx) {
                cx.notify();
            }
        })
        .detach();
        let mut drawer = Drawer {
            playlist: models.playlist.clone(),
            deck: models.deck.clone(),
            commands,
            host,
            focus: focus::tab_stop(cx),
            open,
            rows: Rows::default(),
            pinned: Vec::new(),
            version: None,
            active: None,
            playing: false,
            eq_started: Instant::now(),
            total: 0.0,
            selection: Selection::default(),
            added: None,
            scroll: 0.0,
            drop_line: None,
            drop_empty: false,
            external: None,
            press: None,
            suppress_click: false,
            reorder: None,
            thumb: None,
            track: None,
            track_generation: 0,
            geometry: Rc::new(Cell::new(None)),
            shown: HashMap::new(),
            display: HashMap::new(),
            forced_hover: HashSet::new(),
            equalizer_layer: false,
            root: None,
            stats: Stats::default(),
        };
        drawer.sync_deck(cx);
        drawer.sync_playlist(cx);
        drawer
    }

    // --- The app's side -----------------------------------------------------------------------

    pub fn is_open(&self) -> bool {
        self.open
    }

    /// Opens or closes the drawer (the Playlist button). Closed, it is inert: a focused list
    /// loses focus, as `inert` made it.
    pub fn set_open(&mut self, open: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.open == open {
            return;
        }
        self.open = open;
        if !open && self.focus.is_focused(window) {
            window.blur(cx);
        }
        cx.notify();
    }

    /// The list's focus handle (Tab reaches it after Add URL while the drawer is open).
    pub fn focus_handle(&self) -> &FocusHandle {
        &self.focus
    }

    /// Entries just added (Add URL's answer): selected and revealed, the drawer opened if need
    /// be (`selectAdded`).
    pub fn select_added(&mut self, ids: Vec<u64>, window: &mut Window, cx: &mut Context<Self>) {
        if ids.is_empty() {
            return;
        }
        self.added = Some(ids);
        if !self.open {
            self.open_self(window, cx);
        }
        self.show_added();
        cx.notify();
    }

    /// Paths to add (Add file's picker, or a drop): expanded by the player off the UI thread,
    /// inserted before `index` (None: at the end), then selected and revealed.
    pub fn add_paths(&mut self, paths: Vec<PathBuf>, index: Option<usize>, window: &mut Window, cx: &mut Context<Self>) {
        if paths.is_empty() {
            return;
        }
        let pending = self.commands.add_paths(paths, index);
        cx.spawn_in(window, async move |this, cx| {
            let Some(ids) = pending.await else { return };
            let _ = this.update_in(cx, |drawer, window, cx| drawer.select_added(ids, window, cx));
        })
        .detach();
    }

    // --- State for tests and captures ----------------------------------------------------------

    /// The selected items in row order.
    pub fn selection(&self) -> Vec<Item> {
        self.selection.in_order(&self.rows)
    }

    pub fn anchor(&self) -> Option<Item> {
        self.selection.anchor
    }

    pub fn focused_item(&self) -> Option<Item> {
        self.selection.focus
    }

    pub fn scroll_top(&self) -> f32 {
        self.scroll
    }

    /// Sets the list's `scrollTop` (clamped as Chromium clamps it).
    pub fn set_scroll_top(&mut self, top: f32, cx: &mut Context<Self>) {
        self.scroll = top.clamp(0.0, self.rows.max_scroll());
        cx.notify();
    }

    /// The insertion line's index, if shown.
    pub fn drop_line(&self) -> Option<usize> {
        self.drop_line
    }

    pub fn is_drop_empty(&self) -> bool {
        self.drop_empty
    }

    pub fn is_reordering(&self) -> bool {
        self.reorder.is_some_and(|r| r.active)
    }

    pub fn rows(&self) -> &Rows {
        &self.rows
    }

    /// The status line's left part.
    pub fn status(&self) -> String {
        list::status_text(self.rows.entries(), self.total, self.selection.len())
    }

    /// The page's `select(item, { toggle, extend })` (fixtures call it).
    pub fn select(&mut self, item: Item, toggle: bool, extend: bool, cx: &mut Context<Self>) {
        self.selection.select(&self.rows, item, toggle, extend);
        cx.notify();
    }

    /// The page's `showDropIndex(index)` (fixtures call it).
    pub fn show_drop_index(&mut self, index: Option<usize>, cx: &mut Context<Self>) {
        self.set_drop_index(index);
        cx.notify();
    }

    /// Hover forced on rows (fixtures' `force`).
    pub fn force_hover(&mut self, items: impl IntoIterator<Item = Item>, cx: &mut Context<Self>) {
        self.forced_hover = items.into_iter().collect();
        cx.notify();
    }

    /// A row's border box in window coordinates (from the last paint), if it is in the list.
    pub fn row_bounds(&self, item: Item) -> Option<Rect> {
        let row = self.rows.row_of(item)?;
        let list = self.list_rect();
        Some(Rect { left: list.left + 1.0 + PADDING, top: list.top + 1.0 + self.rows.row_top(row) - self.scroll_px(), width: self.content_width() - 2.0 * PADDING, height: ROW })
    }

    /// The empty hint's border box in window coordinates, when shown.
    pub fn empty_bounds(&self) -> Option<Rect> {
        let top = self.rows.empty_top()?;
        let list = self.list_rect();
        Some(Rect { left: list.left + 1.0 + PADDING + 4.0, top: list.top + 1.0 + top - self.scroll_px(), width: self.content_width() - 2.0 * (PADDING + 4.0), height: list::EMPTY_HEIGHT })
    }

    /// The list's border box in window coordinates.
    pub fn list_rect(&self) -> Rect {
        self.geometry.get().unwrap_or(Rect { left: 22.0, top: qust_fluent::tokens::PLAYER_HEIGHT, width: WIDTH - 44.0, height: ISLAND_HEIGHT })
    }

    // --- Model sync ------------------------------------------------------------------------------

    // The deck's part: the pinned rows (rebuilt when a cartridge's title or URL changed, as
    // `sameCartridges` decided) and the active row. True when something shown changed.
    fn sync_deck(&mut self, cx: &App) -> bool {
        let deck = self.deck.read(cx);
        let pinned: Vec<Pinned> = deck
            .cartridges
            .iter()
            .enumerate()
            .filter_map(|(slot, cartridge)| {
                cartridge.as_ref().map(|c| {
                    let host = list::host(&c.url);
                    Pinned { slot, title: c.title.clone().into(), url: c.url.clone().into(), shown_title: qust_text::shown(&c.title).into(), shown_host: qust_text::shown(&host).into(), host: host.into() }
                })
            })
            .collect();
        let active = match deck.active_slot {
            Some(slot) if deck.cartridges.get(slot).is_some_and(Option::is_some) => Some(Item::Cartridge(slot)),
            _ => deck.active_id.map(Item::Entry),
        };
        let playing = deck.state.is_running();
        let mut changed = false;
        if pinned != self.pinned {
            self.rows.set_pinned(pinned.iter().map(|p| p.slot).collect());
            self.pinned = pinned;
            self.rebuilt();
            changed = true;
        }
        if active != self.active || playing != self.playing {
            // The equalizer's animation starts when the row starts playing.
            if active.is_some() && playing && (active != self.active || !self.playing) {
                self.eq_started = Instant::now();
            }
            self.active = active;
            self.playing = playing;
            changed = true;
        }
        changed
    }

    // `player-playlist` rebuilds the rows; `player-entry` only matters if it changed a row on
    // screen or the total. True when something shown changed.
    fn sync_playlist(&mut self, cx: &App) -> bool {
        let model = self.playlist.read(cx);
        let total: f64 = model.entries.iter().map(|e| e.duration.filter(|d| !d.is_nan()).unwrap_or(0.0)).sum();
        if self.version != Some(model.version) {
            self.version = Some(model.version);
            self.rows.set_ids(model.entries.iter().map(|e| e.id).collect());
            self.total = total;
            self.display.clear();
            self.rebuilt();
            self.show_added();
            return true;
        }
        let mut changed = total != self.total;
        self.total = total;
        for (id, version) in &self.shown {
            if model.entry_versions.get(id).copied().unwrap_or(0) != *version {
                changed = true;
            }
        }
        changed
    }

    // `renderPlaylist`: rows rebuilt, scroll kept (clamped), rows gone leave the selection, the
    // insertion line is gone (its row was replaced), equalizer animations restart (new rows).
    fn rebuilt(&mut self) {
        self.selection.prune(&self.rows);
        self.drop_line = None;
        self.scroll = self.scroll.clamp(0.0, self.rows.max_scroll());
        self.eq_started = Instant::now();
    }

    // `showAdded`: once every added id has its row.
    fn show_added(&mut self) {
        let Some(ids) = &self.added else { return };
        if !ids.iter().all(|id| self.rows.has(Item::Entry(*id))) {
            return;
        }
        let ids = self.added.take().unwrap_or_default();
        self.selection.select_added(&ids);
        if let Some(last) = ids.last() {
            self.reveal(Item::Entry(*last));
        }
        if let Some(first) = ids.first() {
            self.reveal(Item::Entry(*first));
        }
    }

    // --- Helpers -------------------------------------------------------------------------------

    fn reveal(&mut self, item: Item) {
        if let Some(row) = self.rows.row_of(item) {
            self.scroll = list::reveal(&self.rows, row, self.scroll);
        }
    }

    fn set_drop_index(&mut self, index: Option<usize>) {
        self.drop_empty = index.is_some() && self.rows.entries() == 0;
        self.drop_line = index.filter(|_| self.rows.entries() > 0);
    }

    // The scroll position as painted: whole device pixels, as Chromium scrolls.
    fn scroll_px(&self) -> f32 {
        self.scroll.round()
    }

    // The width the rows are laid out in: the padding box, less the scrollbar when it shows.
    fn content_width(&self) -> f32 {
        self.list_rect().width - 2.0 - if self.rows.scrolls() { SCROLLBAR } else { 0.0 }
    }

    // The row under a window position (None: padding, the divider, the hint, outside).
    fn item_at(&self, position: Point<Pixels>) -> Option<Item> {
        let (x, y) = (position.x.as_f32(), position.y.as_f32());
        let list = self.list_rect();
        let left = list.left + 1.0 + PADDING;
        if !list.contains(x, y) || self.over_scrollbar(position) || x < left || x >= left + self.content_width() - 2.0 * PADDING {
            return None;
        }
        let content_y = y - (list.top + 1.0) + self.scroll_px();
        if content_y < 0.0 || y >= list.bottom() - 1.0 {
            return None;
        }
        self.rows.row_at(content_y).and_then(|row| self.rows.item(row))
    }

    fn over_scrollbar(&self, position: Point<Pixels>) -> bool {
        let list = self.list_rect();
        let (x, y) = (position.x.as_f32(), position.y.as_f32());
        self.rows.scrolls() && x >= list.right() - 1.0 - SCROLLBAR && x < list.right() - 1.0 && y >= list.top + 1.0 && y < list.bottom() - 1.0
    }

    // The thumb's (top, length) on the 282 px track, as Chromium's ScrollbarTheme computes them.
    fn thumb_span(&self) -> Option<(f32, f32)> {
        scrollbar::thumb_span(CLIENT_HEIGHT, CLIENT_HEIGHT, self.rows.content_height(), self.scroll, SCROLLBAR)
    }

    fn open_self(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open = true;
        let host = self.host.clone();
        window.defer(cx, move |window, cx| host.open_drawer(window, cx));
        cx.notify();
    }

    fn play(&self, item: Item) {
        match item {
            Item::Cartridge(slot) => self.commands.play_cartridge(slot),
            Item::Entry(id) => self.commands.play_entry(id),
        }
    }

    // `removeEntries(ids)`.
    fn remove(&mut self, ids: Vec<u64>, cx: &mut Context<Self>) {
        if self.selection.plan_removal(&self.rows, &ids) {
            self.commands.remove_entries(ids);
        }
        cx.notify();
    }

    fn defer_host(&self, window: &mut Window, cx: &mut Context<Self>, f: impl FnOnce(&dyn DrawerHost, &mut Window, &mut App) + 'static) {
        let host = self.host.clone();
        window.defer(cx, move |window, cx| f(host.as_ref(), window, cx));
    }

    fn perform(&mut self, action: Action, window: &mut Window, cx: &mut Context<Self>) {
        match action {
            Action::Play(item) => self.play(item),
            Action::EditCartridge(slot) => self.defer_host(window, cx, move |host, window, cx| host.edit_cartridge(slot, window, cx)),
            Action::EditEntry(id) => self.defer_host(window, cx, move |host, window, cx| host.edit_entry(id, window, cx)),
            Action::Remove(ids) => self.remove(ids, cx),
            Action::AddFile => self.defer_host(window, cx, |host, window, cx| host.add_files(window, cx)),
            Action::AddUrl => self.defer_host(window, cx, |host, window, cx| host.add_url(window, cx)),
            Action::ClearPlaylist => {
                let ids = self.rows.ids().to_vec();
                self.remove(ids, cx);
            }
            Action::StreamInformation => self.defer_host(window, cx, |host, window, cx| host.open_panel(Panel::StreamInformation, window, cx)),
            Action::Settings => self.defer_host(window, cx, |host, window, cx| host.open_panel(Panel::Settings, window, cx)),
        }
    }

    // --- Events ----------------------------------------------------------------------------------

    fn any_mouse_down(&mut self, event: &MouseDownEvent, _window: &mut Window, cx: &mut Context<Self>) {
        // `pointerdown`: every press resets the click suppression (GPUI has focused the list).
        self.suppress_click = false;
        if event.button != MouseButton::Left {
            return;
        }
        if self.over_scrollbar(event.position) {
            self.scrollbar_down(event.position, cx);
            return;
        }
        let item = self.item_at(event.position);
        let modifiers = event.modifiers;
        if !(modifiers.control || modifiers.shift || modifiers.platform) {
            if let Some(item @ Item::Entry(_)) = item {
                if !self.selection.contains(item) {
                    self.selection.select(&self.rows, item, false, false);
                }
                self.reorder = Some(Reorder { y: event.position.y.as_f32(), active: false, index: None });
            }
        }
        self.press = Some(Press { item });
        cx.notify();
    }

    fn scrollbar_down(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        let Some((start, length)) = self.thumb_span() else { return };
        let y = position.y.as_f32();
        let on_track = y - (self.list_rect().top + 1.0);
        if on_track >= start && on_track < start + length {
            self.thumb = Some(ThumbDrag { y, scroll: self.scroll });
            return;
        }
        let direction = if on_track < start { -1.0 } else { 1.0 };
        self.track_generation += 1;
        let generation = self.track_generation;
        self.track = Some(TrackPress { direction, y, generation });
        self.page(direction, cx);
        cx.spawn(async move |this, cx| {
            let mut delay = scrollbar::TRACK_REPEAT_FIRST;
            loop {
                cx.background_executor().timer(delay).await;
                delay = scrollbar::TRACK_REPEAT;
                let more = this.update(cx, |drawer, cx| {
                    let Some(track) = drawer.track.filter(|t| t.generation == generation) else { return false };
                    // Repeat while the thumb hasn't reached the pointer.
                    let Some((start, length)) = drawer.thumb_span() else { return false };
                    let on_track = track.y - (drawer.list_rect().top + 1.0);
                    let beyond = if track.direction < 0.0 { on_track < start } else { on_track >= start + length };
                    if beyond {
                        drawer.page(track.direction, cx);
                    }
                    true
                });
                if !matches!(more, Ok(true)) {
                    break;
                }
            }
        })
        .detach();
    }

    fn page(&mut self, direction: f32, cx: &mut Context<Self>) {
        let next = (self.scroll + direction * scrollbar::page_step(CLIENT_HEIGHT)).clamp(0.0, self.rows.max_scroll());
        if next != self.scroll {
            self.scroll = next;
            cx.notify();
        }
    }

    // A move anywhere in the window (the page's `document` pointermove).
    fn window_move(&mut self, event: &MouseMoveEvent, _window: &mut Window, cx: &mut Context<Self>) {
        let y = event.position.y.as_f32();
        if let Some(thumb) = self.thumb {
            let Some((_, length)) = self.thumb_span() else { return };
            let next = scrollbar::dragged_offset(thumb.scroll, y - thumb.y, CLIENT_HEIGHT, length, self.rows.max_scroll());
            if next != self.scroll {
                self.scroll = next;
                cx.notify();
            }
            return;
        }
        if let Some(track) = &mut self.track {
            track.y = y;
            return;
        }
        let Some(mut drag) = self.reorder else { return };
        if event.pressed_button != Some(MouseButton::Left) {
            // The release happened where we couldn't see it: as the page's pointerup.
            self.end_reorder(true, cx);
            return;
        }
        if !drag.active {
            if (y - drag.y).abs() < list::DRAG_THRESHOLD {
                return;
            }
            drag.active = true;
        }
        let list = self.list_rect();
        self.scroll = list::autoscroll(&self.rows, list, self.scroll, y);
        let index = list::drop_index(&self.rows, self.open, list, self.scroll_px(), event.position.x.as_f32(), y);
        drag.index = Some(index);
        self.reorder = Some(drag);
        self.set_drop_index(Some(index));
        cx.notify();
    }

    // A release anywhere in the window: drops (files), ends drags, decides the click.
    fn window_up(&mut self, event: &MouseUpEvent, window: &mut Window, cx: &mut Context<Self>) {
        if self.external.is_some() && cx.has_active_drag() {
            let paths = self.external.take().unwrap_or_default();
            self.external_drop(paths, event.position, window, cx);
            return;
        }
        if event.button != MouseButton::Left {
            return;
        }
        if self.thumb.take().is_some() || self.track.take().is_some() {
            return;
        }
        if self.reorder.is_some() {
            self.end_reorder(true, cx);
        }
        let Some(press) = self.press.take() else { return };
        // `click` reaches the list only if the release is on it too; its target is the row
        // when press and release were on the same row, else the list.
        let list = self.list_rect();
        if !list.contains(event.position.x.as_f32(), event.position.y.as_f32()) || self.over_scrollbar(event.position) {
            return;
        }
        let up = self.item_at(event.position);
        let target = if press.item.is_some() && press.item == up { up } else { None };
        let modifiers = event.modifiers;
        if std::mem::take(&mut self.suppress_click) {
            // The click that ends a drag does nothing.
        } else if let Some(item) = target {
            self.selection.select(&self.rows, item, modifiers.control || modifiers.platform, modifiers.shift);
        } else if !modifiers.control && !modifiers.shift {
            self.selection.clear();
        }
        if event.click_count == 2 {
            if let Some(item) = target {
                self.play(item);
            }
        }
        cx.notify();
    }

    // `endRowDrag(drop)`.
    fn end_reorder(&mut self, drop: bool, cx: &mut Context<Self>) {
        let Some(drag) = self.reorder.take() else { return };
        if !drag.active {
            return;
        }
        self.set_drop_index(None);
        self.suppress_click = true;
        if drop {
            if let Some(index) = drag.index {
                let ids = self.selection.selected_tracks(&self.rows);
                if !ids.is_empty() {
                    self.commands.move_entries(ids, index);
                }
            }
        }
        cx.notify();
    }

    fn wheel(&mut self, event: &ScrollEvent, _window: &mut Window, cx: &mut Context<Self>) {
        cx.stop_propagation();
        let dy = match event.delta {
            ScrollDelta::Pixels(delta) => delta.y.as_f32(),
            ScrollDelta::Lines(delta) => delta.y * WHEEL_LINE,
        };
        let next = (self.scroll - dy).clamp(0.0, self.rows.max_scroll());
        if next != self.scroll {
            self.scroll = next;
            cx.notify();
        }
    }

    fn key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let keystroke = &event.keystroke;
        let modifiers = keystroke.modifiers;
        let key = keystroke.key.as_str();
        if key == "escape" {
            // Esc ends a drag (not prevented, as the page didn't).
            self.end_reorder(false, cx);
            self.reorder = None;
            return;
        }
        if let Some(step) = Move::from_key(key) {
            if let Some(item) = self.selection.move_target(&self.rows, step) {
                self.selection.select(&self.rows, item, false, modifiers.shift);
                self.reveal(item);
            }
        } else if key == "enter" && self.selection.focus.is_some_and(|item| self.rows.has(item)) {
            if let Some(item) = self.selection.focus {
                self.play(item);
            }
        } else if key == "delete" || key == "backspace" {
            let ids = self.selection.selected_tracks(&self.rows);
            self.remove(ids, cx);
        } else if (modifiers.control || modifiers.platform) && key.eq_ignore_ascii_case("a") {
            self.selection.select_all_entries(&self.rows);
        } else if key == "menu" || (key == "f10" && modifiers.shift) {
            // The keyboard's context menu: the list is the target (no row).
            self.context_menu_at(None, window, cx);
        } else {
            return;
        }
        cx.stop_propagation();
        cx.notify();
    }

    fn right_up(&mut self, event: &MouseUpEvent, window: &mut Window, cx: &mut Context<Self>) {
        let item = self.item_at(event.position);
        self.context_menu_at(item, window, cx);
    }

    // The `contextmenu` handler's drawer part.
    fn context_menu_at(&mut self, item: Option<Item>, window: &mut Window, cx: &mut Context<Self>) {
        if self.host.dialog_open(cx) {
            return;
        }
        if let Some(item) = item {
            if !self.selection.contains(item) {
                self.selection.select(&self.rows, item, false, false);
                cx.notify();
            }
        }
        let tracks = self.selection.selected_tracks(&self.rows);
        let (menu, actions) = menu::build(item, &tracks, self.rows.entries());
        self.stats.menus += 1;
        let this = cx.entity().downgrade();
        let handle = window.window_handle();
        let chosen = Box::new(move |id: Option<u32>, cx: &mut App| {
            let Some(action) = id.and_then(|id| actions.into_iter().find(|(other, _)| *other == id)).map(|(_, action)| action) else { return };
            let _ = handle.update(cx, |_, window, cx| {
                let _ = this.update(cx, |drawer, cx| drawer.perform(action, window, cx));
            });
        });
        self.defer_host(window, cx, move |host, window, cx| host.show_menu(menu, chosen, window, cx));
    }

    // Files dragged over the window (Tauri's `enter` / `over`).
    fn external_move(&mut self, event: &DragMoveEvent<ExternalPaths>, window: &mut Window, cx: &mut Context<Self>) {
        if self.host.dialog_open(cx) {
            self.external_leave(cx);
            return;
        }
        let paths = event.drag(cx).paths().to_vec();
        self.external = Some(paths);
        if !self.open {
            self.open_self(window, cx);
        }
        let position = event.event.position;
        let index = list::drop_index(&self.rows, self.open, self.list_rect(), self.scroll_px(), position.x.as_f32(), position.y.as_f32());
        let (line, empty) = (self.drop_line, self.drop_empty);
        self.set_drop_index(Some(index));
        if (line, empty) != (self.drop_line, self.drop_empty) {
            cx.notify();
        }
    }

    // `leave` (or a drag over while the dialog is open).
    fn external_leave(&mut self, cx: &mut Context<Self>) {
        self.external = None;
        if self.drop_line.is_some() || self.drop_empty {
            self.set_drop_index(None);
            cx.notify();
        }
    }

    // `drop`: the paths go to the player with the index under the pointer.
    fn external_drop(&mut self, paths: Vec<PathBuf>, position: Point<Pixels>, window: &mut Window, cx: &mut Context<Self>) {
        if self.host.dialog_open(cx) {
            self.external_leave(cx);
            return;
        }
        let index = list::drop_index(&self.rows, self.open, self.list_rect(), self.scroll_px(), position.x.as_f32(), position.y.as_f32());
        self.set_drop_index(None);
        cx.notify();
        self.add_paths(paths, Some(index), window, cx);
    }

    // --- Rendering -------------------------------------------------------------------------------

    // One row, for the rows on screen. What it draws of the title and detail comes from the
    // pinned cartridge or the `display` cache (`qust_text::shown`, computed when they change).
    fn row_element(&mut self, row: usize, theme: &Theme, model: &PlaylistModel, equalizer: RowEqualizer) -> Option<AnyElement> {
        let item = self.rows.item(row)?;
        let selected = self.selection.contains(item);
        let active = self.active == Some(item);
        let playing = active && self.playing;
        let forced = if self.forced_hover.contains(&item) { Forced::HOVER } else { Forced::NONE };
        let (id, label, title, detail, tooltip, missing): (gpui::ElementId, SharedString, SharedString, SharedString, SharedString, bool) = match item {
            Item::Cartridge(slot) => {
                let pinned = self.pinned.iter().find(|p| p.slot == slot)?;
                (("cartridge", slot).into(), pinned.title.clone(), pinned.shown_title.clone(), pinned.shown_host.clone(), pinned.url.clone(), false)
            }
            Item::Entry(id) => {
                let entry = model.entries.get(self.rows.entry_index(id)?)?;
                let version = model.entry_versions.get(&id).copied().unwrap_or(0);
                self.shown.insert(id, version);
                let (title, detail) = match self.display.get(&id) {
                    Some((v, title, detail)) if *v == version => (title.clone(), detail.clone()),
                    _ => {
                        let title: SharedString = qust_text::shown(&entry.title).into();
                        let detail: SharedString = qust_text::shown(&list::entry_detail(entry.duration, entry.url.as_deref())).into();
                        self.display.insert(id, (version, title.clone(), detail.clone()));
                        (title, detail)
                    }
                };
                let tooltip = entry.path.clone().filter(|p| !p.is_empty()).or_else(|| entry.url.clone()).unwrap_or_default();
                (("entry", id).into(), entry.title.clone().into(), title, detail, tooltip.into(), entry.missing)
            }
        };
        let open = self.open;
        let number = row_ui::row_number(active, !item.is_track(), theme);
        let number = match (playing, equalizer, item) {
            // `.row.is-playing`: the equalizer replaces the number (or the radio icon).
            (true, RowEqualizer::Inline(elapsed), _) => number.child(qust_fluent::equalizer::equalizer(elapsed, theme.accent).mr(px(2.0))),
            // Drawn by the equalizer layer over this box.
            (true, RowEqualizer::Layer, _) => number.child(div().w(px(10.0)).h(px(10.0)).mr(px(2.0))),
            (_, _, Item::Cartridge(slot)) => {
                // The icon is text-2 on the active row too (`row_number`). It sits at the top
                // of its 18 px line box, as the inline SVG did.
                number.child(
                    div()
                        .id(("cartridge-icon", slot))
                        .w(px(14.0))
                        .h(px(18.0))
                        .when(open, |d| d.native_tooltip(format!("Cartridge {}", slot + 1)))
                        .child(icon(Icon::Radio, 14.0).text_color(theme.text_2)),
                )
            }
            (_, _, Item::Entry(_)) => number.child(line(format!("{}", row + 1 - self.rows.pinned().len()))),
        };
        let top = self.rows.row_top(row) - self.scroll_px();
        let reordering = self.is_reordering();
        // A pinned cartridge's radio icon has its own tooltip; in the page the innermost
        // `title` won, but GPUI keeps an outer element's tooltip while a child is hovered, so the
        // row's tooltip covers the row around the icon instead of the whole row.
        let icon_tooltip = match item {
            Item::Cartridge(slot) if !playing => Some(slot),
            _ => None,
        };
        self.stats.rows_built += 1;
        Some(
            row_ui::row(id, selected, forced, theme)
                .absolute()
                .top(px(top))
                .left(px(PADDING))
                .right(px(PADDING))
                .when(open, |r| r.role(Role::ListBoxOption).aria_selected(selected).aria_label(label))
                .when(open && reordering, |r| r.cursor(CursorStyle::ClosedHand))
                .child(number)
                // `.row-title` and `.row-detail` (at most 40 % of the row), cut with Chromium's
                // ellipsis.
                .child(row_ui::row_title(title, active, missing, theme))
                .when(!detail.is_empty(), |r| r.child(row_ui::row_detail(detail, theme)))
                .when(open, |r| match icon_tooltip {
                    Some(slot) => r.children(around_icon(slot, tooltip)),
                    None => r.native_tooltip(tooltip),
                })
                .into_any_element(),
        )
    }

    /// Where the playing row's equalizer goes, when that row is on screen: its 10 × 10 box and
    /// the list's viewport (its clip) in window coordinates, when it started, and whether it
    /// animates (not while the drawer is closed). For the [`EqualizerLayer`].
    pub fn equalizer_placement(&self) -> Option<EqualizerPlacement> {
        let row = self.active.filter(|_| self.playing).and_then(|item| self.rows.row_of(item))?;
        let scroll = self.scroll_px();
        if !self.rows.visible(scroll, CLIENT_HEIGHT).contains(&row) {
            return None;
        }
        let list = self.list_rect();
        let viewport = Rect { left: list.left + 1.0, top: list.top + 1.0, width: self.content_width(), height: CLIENT_HEIGHT };
        // The number column (34 px after the row's 4 px padding), the box 2 px from its end,
        // centred in the 26 px row.
        let left = viewport.left + PADDING + 4.0 + 34.0 - 2.0 - 10.0;
        let top = viewport.top + self.rows.row_top(row) - scroll + 8.0;
        let bounds = |r: Rect| gpui::Bounds::new(gpui::point(px(r.left), px(r.top)), gpui::size(px(r.width), px(r.height)));
        Some(EqualizerPlacement {
            bars: bounds(Rect { left, top, width: 10.0, height: 10.0 }),
            clip: Some(bounds(viewport)),
            started: self.eq_started,
            animate: self.open,
            color: None,
        })
    }

    fn list_contents(&mut self, window: &mut Window, cx: &mut Context<Self>, theme: &Theme) -> Vec<AnyElement> {
        let scroll = self.scroll_px();
        let visible = self.rows.visible(scroll, CLIENT_HEIGHT);
        // The equalizer, when a shown row plays: drawn by the layer when there is one, else
        // here (then this view re-renders at the frame rate).
        let equalizer = if self.equalizer_layer {
            RowEqualizer::Layer
        } else {
            let motion = motion(cx);
            let shown = self.active.filter(|_| self.playing).and_then(|item| self.rows.row_of(item)).is_some_and(|row| visible.contains(&row));
            if motion.reduced {
                RowEqualizer::Inline(None)
            } else {
                if shown && motion.wants_frames() && self.open {
                    window.request_animation_frame();
                }
                RowEqualizer::Inline(Some(motion.elapsed(self.eq_started, None)))
            }
        };
        let playlist = self.playlist.clone();
        let model = playlist.read(cx);
        self.shown.clear();
        let mut children: Vec<AnyElement> = Vec::with_capacity(visible.len() + 3);
        for row in visible {
            if let Some(element) = self.row_element(row, theme, model, equalizer) {
                children.push(element);
            }
        }
        if let Some(top) = self.rows.divider_top() {
            // The divider's box with its 4 px margins.
            children.push(div().absolute().top(px(top - 4.0 - scroll)).left(px(PADDING)).right(px(PADDING)).child(row_ui::divider(theme)).into_any_element());
        }
        if let Some(top) = self.rows.empty_top() {
            children.push(self.empty_hint(top - scroll, theme).into_any_element());
        }
        if let Some(index) = self.drop_line {
            let n = self.rows.entries();
            let row = self.rows.pinned().len() + index.min(n.saturating_sub(1));
            let top = self.rows.row_top(row) - scroll;
            children.push(div().absolute().top(px(top)).left(px(PADDING)).right(px(PADDING)).h(px(ROW)).child(row_ui::drop_line(index >= n, theme)).into_any_element());
        }
        children
    }

    // `.playlist-empty`, accent while files hover an empty list (`.is-drop-empty`).
    fn empty_hint(&self, top: f32, theme: &Theme) -> impl IntoElement {
        let frame = if self.drop_empty {
            Frame::new(6.0).dashed(theme.accent).fill(theme.accent_mix(0.08)).fill_border_box()
        } else {
            Frame::new(6.0).dashed(theme.empty_stroke)
        };
        surface(frame)
            .absolute()
            .top(px(top))
            .left(px(PADDING + 4.0))
            .right(px(PADDING + 4.0))
            .h(px(list::EMPTY_HEIGHT))
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(px(2.0))
            .py(px(18.0))
            .px(px(16.0))
            .type_size(12.0, 16.0)
            .text_color(theme.text_3)
            .text_center()
            .child(div().text_color(theme.text_2).text_size(px(13.0)).semibold().child(EMPTY_TITLE))
            .child(div().child(EMPTY_TEXT))
    }

    fn scrollbar(&self, theme: &Theme) -> impl IntoElement {
        let thumb = self.thumb_span();
        let color = theme.text_3;
        div().relative().w(px(SCROLLBAR)).h_full().flex_none().when_some(thumb, |bar, (start, length)| {
            bar.child(canvas(|_, _, _| {}, move |bounds, _, window, _| scrollbar::paint_thumb(bounds, ScrollbarKind::Playlist, start, length, color, window)).size_full())
        })
    }

    // Window-wide listeners (the page's `document` listeners), and the external drop's leave.
    fn window_listeners(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let this = cx.entity().downgrade();
        let open = self.open;
        canvas(
            |_, _, _| {},
            move |_, _, window, _| {
                if open {
                    let drawer = this.clone();
                    window.on_mouse_event(move |event: &MouseMoveEvent, phase, window, cx| {
                        if phase == DispatchPhase::Capture {
                            let _ = drawer.update(cx, |drawer, cx| drawer.window_move(event, window, cx));
                        }
                    });
                }
                let drawer = this.clone();
                window.on_mouse_event(move |event: &MouseUpEvent, phase, window, cx| {
                    if phase == DispatchPhase::Capture {
                        let _ = drawer.update(cx, |drawer, cx| drawer.window_up(event, window, cx));
                    }
                });
                let drawer = this.clone();
                window.on_mouse_event(move |event: &FileDropEvent, phase, _, cx| {
                    if phase == DispatchPhase::Capture && matches!(event, FileDropEvent::Exited) {
                        let _ = drawer.update(cx, |drawer, cx| drawer.external_leave(cx));
                    }
                });
            },
        )
        .absolute()
        .size_0()
    }
}

// Four transparent boxes carrying a pinned cartridge row's tooltip (its URL) around the radio
// icon's 14 × 18 box (row x 24–38, y 4–22), which carries "Cartridge N".
fn around_icon(slot: usize, tooltip: SharedString) -> Vec<AnyElement> {
    let boxes: [(f32, Option<f32>, Option<f32>, f32, f32); 4] = [
        // (left, width or None = to the right edge, …, top, height)
        (0.0, Some(24.0), None, 0.0, ROW),
        (38.0, None, None, 0.0, ROW),
        (24.0, Some(14.0), None, 0.0, 4.0),
        (24.0, Some(14.0), None, 22.0, 4.0),
    ];
    boxes
        .into_iter()
        .enumerate()
        .map(|(index, (left, width, _, top, height))| {
            div()
                .id(("cartridge-tip", slot * 4 + index))
                .absolute()
                .left(px(left))
                .top(px(top))
                .h(px(height))
                .map(|d| match width {
                    Some(width) => d.w(px(width)),
                    None => d.right_0(),
                })
                .native_tooltip(tooltip.clone())
                .into_any_element()
        })
        .collect()
}

impl Render for Drawer {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.stats.renders += 1;
        let theme = *theme(cx);
        let open = self.open;
        self.scroll = self.scroll.clamp(0.0, self.rows.max_scroll());
        let contents = self.list_contents(window, cx, &theme);
        let scrolls = self.rows.scrolls();
        let focus_visible = open && focus::is_focus_visible(&self.focus, window, cx);
        let geometry = self.geometry.clone();
        let status = self.status();

        let viewport = div().id("playlist-viewport").relative().flex_1().h_full().overflow_hidden().children(contents);
        let island = surface(Frame::island(&theme))
            .id("playlist")
            .h(px(ISLAND_HEIGHT))
            .flex_none()
            .w_full()
            .flex()
            .flex_row()
            .child(viewport)
            .when(scrolls, |island| island.child(self.scrollbar(&theme)))
            .child(
                // Records the island's border box for hit testing and drop indexes.
                canvas(
                    move |bounds, _, _| {
                        geometry.set(Some(Rect {
                            left: bounds.origin.x.as_f32() - 1.0,
                            top: bounds.origin.y.as_f32() - 1.0,
                            width: bounds.size.width.as_f32() + 2.0,
                            height: bounds.size.height.as_f32() + 2.0,
                        }))
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .inset_0(),
            )
            .when(focus_visible, |island| island.child(qust_fluent::focus_ring(8.0, &theme)))
            .when(open, |island| {
                island
                    .track_focus(&self.focus)
                    .role(Role::ListBox)
                    .aria_label("Playlist")
                    .a11y_synthetic_children(|builder| builder.parent_node().set_multiselectable())
                    .on_any_mouse_down(cx.listener(Self::any_mouse_down))
                    .on_scroll(cx.listener(Self::wheel))
                    .on_key_down(cx.listener(Self::key_down))
            });

        // Single lines (Chromium's baseline: 11/16 sits 1 px lower in GPUI's own).
        let status_line = div()
            .h(px(16.0))
            .flex_none()
            .flex()
            .flex_row()
            .justify_between()
            .gap(px(12.0))
            .px(px(4.0))
            .type_size(11.0, 16.0)
            .text_color(theme.text_3)
            .whitespace_nowrap()
            .child(line(status).text_color(theme.text_2).tabular_nums())
            .child(line(HINT));

        div()
            .id("drawer")
            .relative()
            .size_full()
            .font_text()
            .type_size(14.0, 20.0)
            .text_color(theme.text)
            .on_drag_move::<ExternalPaths>(cx.listener(Self::external_move))
            .when(open, |drawer| drawer.on_mouse_up(MouseButton::Right, cx.listener(Self::right_up)))
            .child(div().size_full().flex().flex_col().gap(px(6.0)).px(px(22.0)).pb(px(14.0)).child(island).child(status_line))
            .child(self.window_listeners(cx))
            .when_some(self.root.clone(), |drawer, root| drawer.child(div().absolute().inset_0().child(portal_slot(root, EQUALIZER_SLOT))))
    }
}

/// How the playing row's equalizer is drawn this render (without the layer: here).
#[derive(Clone, Copy, Debug)]
enum RowEqualizer {
    /// Here, after this much of its animation (None: reduced motion, full height).
    Inline(Option<Duration>),
    /// By the [`EqualizerLayer`]: the row keeps the box empty.
    Layer,
}

/// Mounts the drawer the way it is meant to live: as a live layer of the window root (drawn at
/// the 860 × 320 slot `"drawer"`, which the player's content places at y = 267), with the
/// playing row's equalizer as a second live layer above it (its slot is inside the drawer).
/// Returns the layer.
pub fn mount(drawer: &Entity<Drawer>, root: &Entity<WindowRoot>, cx: &mut App) -> Entity<EqualizerLayer> {
    let layer = cx.new(|cx| EqualizerLayer::new(drawer, |drawer: &Drawer, _| drawer.equalizer_placement(), cx));
    let weak = root.downgrade();
    drawer.update(cx, |drawer, cx| {
        drawer.equalizer_layer = true;
        drawer.root = Some(weak);
        cx.notify();
    });
    let (view, eq) = (drawer.clone(), layer.clone());
    root.update(cx, |root, cx| {
        root.set_live("drawer", view, cx);
        root.set_live(EQUALIZER_SLOT, eq, cx);
    });
    layer
}

/// The equalizer layer's slot key (the drawer places the slot over itself).
pub const EQUALIZER_SLOT: &str = "playlist-equalizer";
