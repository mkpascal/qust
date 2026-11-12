//! The list as the page laid it out, and script.js's selection, keyboard, drop and scrolling
//! rules, without GPUI (unit-tested here; the view only draws and dispatches).
//!
//! **Rows.** The loaded cartridges first, pinned (by slot), then the playlist entries (by id).
//! Every row is 26 px tall, so the layout is arithmetic, not measured: the list's padding 4, the
//! pinned rows, the divider when any cartridge is loaded (margin 4, 1 px line, margin 4), the
//! entries, padding 4. With no entries, the empty hint (72 px, margin 6 4 4) takes their place;
//! its top margin collapses with the divider's bottom one (block layout: max(4, 6) = 6). These
//! are the page's `offsetTop` values: content coordinates from the list's padding box, the way
//! `scrollTop` counts them.
//!
//! **Selection** holds items (a cartridge by slot, an entry by id), so it survives list
//! updates; `anchor` is where Shift ranges start, `focus` where the arrow keys move from (the
//! page's `anchor` and `focusItem`, which may name rows that no longer exist).

use std::collections::{HashMap, HashSet};
use std::ops::Range;

/// `.row`'s height.
pub const ROW: f32 = 26.0;
/// `.playlist`'s padding.
pub const PADDING: f32 = 4.0;
/// `.playlist-divider`: margin 4, 1 px, margin 4.
pub const DIVIDER: f32 = 9.0;
/// `.playlist-empty`'s border box (1 + 18 + 16 + 2 + 16 + 18 + 1) and margins.
pub const EMPTY_HEIGHT: f32 = 72.0;
pub const EMPTY_MARGIN_TOP: f32 = 6.0;
pub const EMPTY_MARGIN_BOTTOM: f32 = 4.0;
/// The list's `clientHeight`: the island's 284 px less its borders.
pub const CLIENT_HEIGHT: f32 = 282.0;
/// Rows start to move after this much vertical travel.
pub const DRAG_THRESHOLD: f32 = 5.0;
/// Autoscroll: within this distance of the list's top or bottom edge, each move scrolls a step.
pub const AUTOSCROLL_EDGE: f32 = 24.0;
pub const AUTOSCROLL_STEP: f32 = 12.0;
/// PageUp / PageDown move this many rows.
pub const PAGE_ROWS: isize = 10;
/// Revealing a row leaves this margin.
pub const REVEAL_MARGIN: f32 = 4.0;

/// What a row shows: a pinned cartridge (its slot, 0–5) or a playlist entry (its id).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Item {
    Cartridge(usize),
    Entry(u64),
}

impl Item {
    pub fn is_track(self) -> bool {
        matches!(self, Item::Entry(_))
    }
}

/// The rows in order: pinned slots, then entry ids.
#[derive(Clone, Debug, Default)]
pub struct Rows {
    pinned: Vec<usize>,
    ids: Vec<u64>,
    index: HashMap<u64, usize>,
}

impl Rows {
    pub fn new(pinned: Vec<usize>, ids: Vec<u64>) -> Rows {
        let mut rows = Rows { pinned, ..Rows::default() };
        rows.set_ids(ids);
        rows
    }

    pub fn set_pinned(&mut self, pinned: Vec<usize>) {
        self.pinned = pinned;
    }

    pub fn set_ids(&mut self, ids: Vec<u64>) {
        self.index = ids.iter().enumerate().map(|(i, id)| (*id, i)).collect();
        self.ids = ids;
    }

    pub fn pinned(&self) -> &[usize] {
        &self.pinned
    }

    pub fn ids(&self) -> &[u64] {
        &self.ids
    }

    pub fn len(&self) -> usize {
        self.pinned.len() + self.ids.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The number of entries (tracks).
    pub fn entries(&self) -> usize {
        self.ids.len()
    }

    pub fn item(&self, row: usize) -> Option<Item> {
        match self.pinned.get(row) {
            Some(slot) => Some(Item::Cartridge(*slot)),
            None => self.ids.get(row - self.pinned.len()).map(|id| Item::Entry(*id)),
        }
    }

    pub fn row_of(&self, item: Item) -> Option<usize> {
        match item {
            Item::Cartridge(slot) => self.pinned.iter().position(|s| *s == slot),
            Item::Entry(id) => self.index.get(&id).map(|i| self.pinned.len() + i),
        }
    }

    /// An entry's position in the playlist.
    pub fn entry_index(&self, id: u64) -> Option<usize> {
        self.index.get(&id).copied()
    }

    pub fn has(&self, item: Item) -> bool {
        self.row_of(item).is_some()
    }

    /// Where the entries start (the first entry's `offsetTop`, even when there is none).
    pub fn entries_top(&self) -> f32 {
        PADDING + ROW * self.pinned.len() as f32 + if self.pinned.is_empty() { 0.0 } else { DIVIDER }
    }

    /// A row's `offsetTop`.
    pub fn row_top(&self, row: usize) -> f32 {
        if row < self.pinned.len() {
            PADDING + ROW * row as f32
        } else {
            self.entries_top() + ROW * (row - self.pinned.len()) as f32
        }
    }

    /// The divider line's top, when there is one.
    pub fn divider_top(&self) -> Option<f32> {
        (!self.pinned.is_empty()).then(|| PADDING + ROW * self.pinned.len() as f32 + 4.0)
    }

    /// The empty hint's top (its border box), when there are no entries.
    pub fn empty_top(&self) -> Option<f32> {
        if !self.ids.is_empty() {
            return None;
        }
        // After the divider, its 4 px bottom margin and the hint's 6 px top margin collapse.
        Some(match self.divider_top() {
            Some(divider) => divider + 1.0 + EMPTY_MARGIN_TOP,
            None => PADDING + EMPTY_MARGIN_TOP,
        })
    }

    /// The list's `scrollHeight` (at least its client height, as Chromium reports it).
    pub fn content_height(&self) -> f32 {
        let content = match self.empty_top() {
            Some(top) => top + EMPTY_HEIGHT + EMPTY_MARGIN_BOTTOM + PADDING,
            None => self.entries_top() + ROW * self.ids.len() as f32 + PADDING,
        };
        content.max(CLIENT_HEIGHT)
    }

    /// Whether the list overflows (and shows its scrollbar).
    pub fn scrolls(&self) -> bool {
        self.content_height() > CLIENT_HEIGHT
    }

    pub fn max_scroll(&self) -> f32 {
        (self.content_height() - CLIENT_HEIGHT).max(0.0)
    }

    /// The row at content height `y`, if any (the divider and padding are no row).
    pub fn row_at(&self, y: f32) -> Option<usize> {
        let p = self.pinned.len();
        if y >= PADDING && y < PADDING + ROW * p as f32 {
            return Some(((y - PADDING) / ROW) as usize);
        }
        let top = self.entries_top();
        if y >= top && y < top + ROW * self.ids.len() as f32 {
            return Some(p + ((y - top) / ROW) as usize);
        }
        None
    }

    /// The rows that intersect content heights `top .. top + height`.
    pub fn visible(&self, top: f32, height: f32) -> Range<usize> {
        let bottom = top + height;
        // Rows are in increasing order of offsetTop: binary searches.
        let first = partition(self.len(), |r| self.row_top(r) + ROW <= top);
        let end = partition(self.len(), |r| self.row_top(r) < bottom);
        first..end.max(first)
    }
}

// The first index in 0..len where `before` turns false (it is true then false).
fn partition(len: usize, before: impl Fn(usize) -> bool) -> usize {
    let (mut low, mut high) = (0, len);
    while low < high {
        let mid = (low + high) / 2;
        if before(mid) {
            low = mid + 1;
        } else {
            high = mid;
        }
    }
    low
}

/// The keys that move the selection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Move {
    Up,
    Down,
    PageUp,
    PageDown,
    Home,
    End,
}

impl Move {
    /// From a GPUI key name.
    pub fn from_key(key: &str) -> Option<Move> {
        Some(match key {
            "up" => Move::Up,
            "down" => Move::Down,
            "pageup" => Move::PageUp,
            "pagedown" => Move::PageDown,
            "home" => Move::Home,
            "end" => Move::End,
            _ => return None,
        })
    }
}

/// The selection: script.js's `selection`, `anchor` and `focusItem`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Selection {
    items: HashSet<Item>,
    pub anchor: Option<Item>,
    pub focus: Option<Item>,
}

impl Selection {
    pub fn contains(&self, item: Item) -> bool {
        self.items.contains(&item)
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// The selected items in row order.
    pub fn in_order(&self, rows: &Rows) -> Vec<Item> {
        let mut items: Vec<(usize, Item)> = self.items.iter().filter_map(|item| rows.row_of(*item).map(|row| (row, *item))).collect();
        items.sort_by_key(|(row, _)| *row);
        items.into_iter().map(|(_, item)| item).collect()
    }

    /// `select(item, { toggle, extend })`: Shift extends from the anchor (when it still has a
    /// row) over every row between, cartridges included; Ctrl toggles; else the item alone.
    pub fn select(&mut self, rows: &Rows, item: Item, toggle: bool, extend: bool) {
        let range = match (extend, self.anchor.and_then(|anchor| rows.row_of(anchor)), rows.row_of(item)) {
            (true, Some(from), Some(to)) => Some(from.min(to)..from.max(to) + 1),
            _ => None,
        };
        if let Some(range) = range {
            self.items = range.filter_map(|row| rows.item(row)).collect();
        } else if toggle {
            if !self.items.remove(&item) {
                self.items.insert(item);
            }
            self.anchor = Some(item);
        } else {
            self.items = HashSet::from([item]);
            self.anchor = Some(item);
        }
        self.focus = Some(item);
    }

    /// A click on empty list space: nothing selected (anchor and focus stay).
    pub fn clear(&mut self) {
        self.items.clear();
    }

    /// Ctrl+A: every entry, no cartridge (anchor and focus stay).
    pub fn select_all_entries(&mut self, rows: &Rows) {
        self.items = rows.ids().iter().map(|id| Item::Entry(*id)).collect();
    }

    /// After a list update: rows that no longer exist leave the selection.
    pub fn prune(&mut self, rows: &Rows) {
        self.items.retain(|item| rows.has(*item));
    }

    /// Entries just added: selected, anchor and focus on the first.
    pub fn select_added(&mut self, ids: &[u64]) {
        self.items = ids.iter().map(|id| Item::Entry(*id)).collect();
        self.anchor = ids.first().map(|id| Item::Entry(*id));
        self.focus = self.anchor;
    }

    /// `selectedTracks()`: the selected entries' ids in playlist order.
    pub fn selected_tracks(&self, rows: &Rows) -> Vec<u64> {
        if self.items.is_empty() {
            return Vec::new();
        }
        rows.ids().iter().copied().filter(|id| self.items.contains(&Item::Entry(*id))).collect()
    }

    /// Where an arrow, Page or Home / End key goes: from the focused row (the first row when
    /// it has none), clamped to the list.
    pub fn move_target(&self, rows: &Rows, key: Move) -> Option<Item> {
        if rows.is_empty() {
            return None;
        }
        let index = self.focus.and_then(|item| rows.row_of(item)).map(|row| row as isize);
        let last = rows.len() as isize - 1;
        let to = match key {
            Move::Up => index.map_or(0, |i| i - 1),
            Move::Down => index.map_or(0, |i| i + 1),
            Move::PageUp => index.map_or(0, |i| i - PAGE_ROWS),
            Move::PageDown => index.map_or(0, |i| i + PAGE_ROWS),
            Move::Home => 0,
            Move::End => last,
        };
        rows.item(to.clamp(0, last) as usize)
    }

    /// `removeEntries(ids)`: the selection moves to the entry after the removed ones (or the
    /// new last one), as the list will be. False when none of `ids` is in the list (then
    /// nothing is sent).
    pub fn plan_removal(&mut self, rows: &Rows, ids: &[u64]) -> bool {
        let removing: HashSet<u64> = ids.iter().copied().collect();
        let Some(first) = rows.ids().iter().position(|id| removing.contains(id)) else { return false };
        let remaining: Vec<u64> = rows.ids().iter().copied().filter(|id| !removing.contains(id)).collect();
        let next = if remaining.is_empty() { None } else { Some(remaining[first.min(remaining.len() - 1)]) };
        self.items = next.map(|id| HashSet::from([Item::Entry(id)])).unwrap_or_default();
        self.anchor = next.map(Item::Entry);
        self.focus = self.anchor;
        true
    }
}

/// The list's border box in window coordinates (`getBoundingClientRect()`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    pub left: f32,
    pub top: f32,
    pub width: f32,
    pub height: f32,
}

impl Rect {
    pub fn right(&self) -> f32 {
        self.left + self.width
    }

    pub fn bottom(&self) -> f32 {
        self.top + self.height
    }

    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.left && x <= self.right() && y >= self.top && y <= self.bottom()
    }
}

/// `dropIndex(x, y)`: where a drop at (x, y) goes in the playlist: the nearest boundary between
/// entries under it (the start when over the cartridges), the end anywhere else or while the
/// drawer is closed. As the page computed it: from the list's border-box top, but with the
/// first entry's `offsetTop`, which counts from the padding box (a 1 px bias, kept).
pub fn drop_index(rows: &Rows, open: bool, list: Rect, scroll: f32, x: f32, y: f32) -> usize {
    let n = rows.entries();
    if !open || n == 0 || !list.contains(x, y) {
        return n;
    }
    let offset = y - list.top + scroll - rows.entries_top();
    // Math.round, then clamped (negative halves end at 0 either way).
    ((offset / ROW).round().max(0.0) as usize).min(n)
}

/// `autoscroll(y)`: one step towards the edge the pointer is near.
pub fn autoscroll(rows: &Rows, list: Rect, scroll: f32, y: f32) -> f32 {
    let next = if y < list.top + AUTOSCROLL_EDGE {
        scroll - AUTOSCROLL_STEP
    } else if y > list.bottom() - AUTOSCROLL_EDGE {
        scroll + AUTOSCROLL_STEP
    } else {
        scroll
    };
    next.clamp(0.0, rows.max_scroll())
}

/// `revealRow(item)`: the scroll position that shows `row` with a 4 px margin, moving as little
/// as possible.
pub fn reveal(rows: &Rows, row: usize, scroll: f32) -> f32 {
    let top = rows.row_top(row);
    let next = if top < scroll {
        top - REVEAL_MARGIN
    } else if top + ROW > scroll + CLIENT_HEIGHT {
        top + ROW - CLIENT_HEIGHT + REVEAL_MARGIN
    } else {
        scroll
    };
    next.clamp(0.0, rows.max_scroll())
}

/// `#playlistCount`: "N tracks" ("1 track") · the total duration (when any) · "K selected"
/// (when more than one row is).
pub fn status_text(entries: usize, total: f64, selected: usize) -> String {
    let mut parts = vec![if entries == 1 { "1 track".to_string() } else { format!("{entries} tracks") }];
    if total != 0.0 && !total.is_nan() {
        parts.push(qust_ui::format::time(total));
    }
    if selected > 1 {
        parts.push(format!("{selected} selected"));
    }
    parts.join(" · ")
}

/// `new URL(url).host`, or the text itself when it isn't a URL (the page's `host()`).
pub fn host(url: &str) -> String {
    qust_ui::format::host(url)
}

/// A row's detail: the duration (`formatTime`), else the URL's host for a URL entry, else
/// nothing. A duration of 0 counts as none (JavaScript truthiness).
pub fn entry_detail(duration: Option<f64>, url: Option<&str>) -> String {
    match (duration.filter(|d| *d != 0.0 && !d.is_nan()), url) {
        (Some(duration), _) => qust_ui::format::time(duration),
        (None, Some(url)) => host(url),
        (None, None) => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // The reference's cartridges (slot 4 empty) and six entries.
    fn rows(entries: usize) -> Rows {
        Rows::new(vec![0, 1, 2, 3, 5], (1..=entries as u64).collect())
    }

    #[test]
    fn layout_matches_the_page() {
        let r = rows(6);
        // drawer-entries: rows at 272, 298 … (list padding box at 268), divider 406, first entry 411.
        assert_eq!(r.row_top(0), 4.0);
        assert_eq!(r.divider_top(), Some(138.0));
        assert_eq!(r.row_top(5), 143.0);
        assert_eq!(r.content_height(), 303.0);
        assert!(r.scrolls());
        // drawer-empty: the hint at 413 (collapsed margins), no scrolling.
        let empty = rows(0);
        assert_eq!(empty.empty_top(), Some(145.0));
        assert!(!empty.scrolls());
        // drawer-empty-no-cartridges: the hint at 278.
        assert_eq!(Rows::new(vec![], vec![]).empty_top(), Some(10.0));
        // drawer-many: scrollHeight 1707.
        assert_eq!(rows(60).content_height(), 1707.0);
        assert_eq!(rows(60).max_scroll(), 1425.0);
    }

    #[test]
    fn hit_testing_and_visible_rows() {
        let r = rows(60);
        assert_eq!(r.row_at(3.0), None);
        assert_eq!(r.row_at(4.0), Some(0));
        assert_eq!(r.row_at(133.9), Some(4));
        assert_eq!(r.row_at(138.0), None);
        assert_eq!(r.row_at(143.0), Some(5));
        assert_eq!(r.item(5), Some(Item::Entry(1)));
        assert_eq!(r.row_of(Item::Cartridge(5)), Some(4));
        assert_eq!(r.row_of(Item::Entry(60)), Some(64));
        // drawer-many-scrolled at 640: entry 20 (row 24) is partly hidden at the top.
        let visible = r.visible(640.0, CLIENT_HEIGHT);
        assert_eq!(visible.start, 24);
        assert_eq!(r.item(visible.end - 1), Some(Item::Entry(30)));
        assert_eq!(r.visible(0.0, CLIENT_HEIGHT), 0..11);
        assert_eq!(Rows::default().visible(0.0, CLIENT_HEIGHT), 0..0);
    }

    #[test]
    fn selection_like_script_js() {
        let r = rows(6);
        let mut s = Selection::default();
        s.select(&r, Item::Entry(2), false, false);
        s.select(&r, Item::Entry(5), false, true);
        assert_eq!(s.len(), 4);
        s.select(&r, Item::Cartridge(1), true, false);
        // drawer-select-multi: "5 selected".
        assert_eq!(s.len(), 5);
        assert_eq!(s.anchor, Some(Item::Cartridge(1)));
        // Shift from the cartridge row covers the cartridges between and the entries.
        s.select(&r, Item::Entry(1), false, true);
        assert_eq!(s.in_order(&r), vec![Item::Cartridge(1), Item::Cartridge(2), Item::Cartridge(3), Item::Cartridge(5), Item::Entry(1)]);
        assert_eq!(s.selected_tracks(&r), vec![1]);
        // Ctrl+A: entries only.
        s.select_all_entries(&r);
        assert_eq!(s.len(), 6);
        assert!(!s.contains(Item::Cartridge(1)));
        // Toggling off keeps the anchor on it.
        s.select(&r, Item::Entry(3), true, false);
        assert!(!s.contains(Item::Entry(3)));
        assert_eq!(s.anchor, Some(Item::Entry(3)));
    }

    #[test]
    fn keys_move_from_the_focus() {
        let r = rows(6);
        let mut s = Selection::default();
        assert_eq!(s.move_target(&r, Move::Down), Some(Item::Cartridge(0)));
        assert_eq!(s.move_target(&r, Move::End), Some(Item::Entry(6)));
        s.select(&r, Item::Cartridge(3), false, false);
        assert_eq!(s.move_target(&r, Move::Down), Some(Item::Cartridge(5)));
        assert_eq!(s.move_target(&r, Move::PageDown), Some(Item::Entry(6)));
        assert_eq!(s.move_target(&r, Move::PageUp), Some(Item::Cartridge(0)));
        // A focus that no longer exists starts from the first row again.
        s.focus = Some(Item::Entry(99));
        assert_eq!(s.move_target(&r, Move::Up), Some(Item::Cartridge(0)));
        assert_eq!(Selection::default().move_target(&Rows::default(), Move::End), None);
    }

    #[test]
    fn removal_selects_the_next_entry() {
        let r = rows(6);
        let mut s = Selection::default();
        assert!(s.plan_removal(&r, &[2, 3]));
        assert_eq!(s.in_order(&r), vec![Item::Entry(4)]);
        assert!(s.plan_removal(&r, &[5, 6]));
        assert_eq!(s.focus, Some(Item::Entry(4)));
        assert!(s.plan_removal(&r, &[1, 2, 3, 4, 5, 6]));
        assert!(s.is_empty());
        assert_eq!(s.anchor, None);
        assert!(!s.plan_removal(&r, &[42]));
    }

    #[test]
    fn drop_index_like_the_page() {
        let r = rows(6);
        let list = Rect { left: 22.0, top: 267.0, width: 816.0, height: 284.0 };
        // reorder-drag: over the fifth entry (top 515), 10 % down → before it (index 4).
        assert_eq!(drop_index(&r, true, list, 0.0, 349.0, 517.6), 4);
        // drop-over-rows: 2 px into the third entry (top 463) → index 2.
        assert_eq!(drop_index(&r, true, list, 0.0, 430.0, 465.0), 2);
        // Over the cartridges: the start. Outside the list or closed: the end.
        assert_eq!(drop_index(&r, true, list, 0.0, 430.0, 300.0), 0);
        assert_eq!(drop_index(&r, true, list, 0.0, 430.0, 100.0), 6);
        assert_eq!(drop_index(&r, false, list, 0.0, 430.0, 465.0), 6);
        assert_eq!(drop_index(&rows(0), true, list, 0.0, 430.0, 420.0), 0);
        // Scrolled: the scroll position counts.
        assert_eq!(drop_index(&rows(60), true, list, 640.0, 430.0, 268.0 + 3.0), 19);
    }

    #[test]
    fn reveal_and_autoscroll() {
        let r = rows(60);
        // Below the view: scrolled so it shows with a 4 px margin at the bottom.
        assert_eq!(reveal(&r, 20, 0.0), r.row_top(20) + ROW - CLIENT_HEIGHT + 4.0);
        // Above: its top 4 px under the top edge.
        assert_eq!(reveal(&r, 5, 640.0), 139.0);
        // Already shown: no move. Clamped at the ends.
        assert_eq!(reveal(&r, 30, 640.0), 640.0);
        assert_eq!(reveal(&r, 0, 640.0), 0.0);
        assert_eq!(reveal(&r, 64, 0.0), r.max_scroll());
        let list = Rect { left: 22.0, top: 267.0, width: 816.0, height: 284.0 };
        assert_eq!(autoscroll(&r, list, 100.0, 280.0), 88.0);
        assert_eq!(autoscroll(&r, list, 100.0, 540.0), 112.0);
        assert_eq!(autoscroll(&r, list, 100.0, 400.0), 100.0);
        assert_eq!(autoscroll(&r, list, 5.0, 270.0), 0.0);
    }

    #[test]
    fn texts() {
        assert_eq!(status_text(0, 0.0, 0), "0 tracks");
        assert_eq!(status_text(1, 0.0, 1), "1 track");
        assert_eq!(status_text(6, 4559.4, 5), "6 tracks · 1:15:59 · 5 selected");
        assert_eq!(entry_detail(Some(320.4), Some("http://a")), "5:20");
        assert_eq!(entry_detail(Some(0.0), Some("http://novazz.ice.infomaniak.ch/novazz-128.mp3")), "novazz.ice.infomaniak.ch");
        assert_eq!(entry_detail(None, None), "");
        assert_eq!(host("not a url"), "not a url");
    }
}
