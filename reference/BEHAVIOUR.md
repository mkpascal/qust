# Qust UI behaviour reference

How the webview UI (frozen in `page/`, commit 7018516 of `src/`) responds to the user, taken from
`page/js/*.js` and `page/css/style.css` as they are. The renders and `layout/` files show what each
state looks like; this file covers what a picture can't. Line references are to `page/js/script.js`
unless another file is named. "Rust" means the player, which owns all state: the page only renders
`player-state` and sends commands (`invoke`).

Units: CSS px (logical pixels; multiply by the Windows scale for device pixels). Times in ms unless
marked s. "Accent" means `--accent`: the accent's **light** shade (`--accent-light`) in dark theme
and its **dark** shade (`--accent-dark`) in light theme. `on-accent` is black in dark theme, white
in light theme.

---

## 1. Global rules (all windows)

- **Cursor**: `default` everywhere (`body { cursor: default }`). No element uses a pointer/hand
  cursor, not even buttons. The only other cursor is `grabbing` on playlist rows while a reorder
  drag is active (`.playlist.is-reordering .row`). Text fields show the I-beam (browser default for
  inputs).
- **Text selection**: off everywhere (`user-select: none`), except text fields (`.textbox`) and the
  Stream information values (`.info-card dd`, `user-select: text`).
- **Right-click** (`bridge.js`): the browser menu never shows.
  - On an `input`/`textarea`: the field is focused, then a native menu **Cut / Copy / Paste /
    separator / Select All** (Tauri predefined items, OS-enabled states).
  - On a `dd` or `.copyable` value: if there is no selection inside it, the whole value is selected;
    then a native menu with **Copy**.
  - Anywhere else: the page decides (the player window builds its own menus, section 2.9; Settings
    and Stream information show nothing).
  - Menus open at the mouse cursor (`popup_menu` with no position). Settings combos open their menu
    at a given point (section 3.2).
- **Blocked browser keys** (`bridge.js`): F3, F5, F7, and Ctrl/Cmd + R, P, F, G, U, J are
  swallowed (`preventDefault`). Nothing else is intercepted globally.
- **Focus rings**: `:focus { outline: none }`; `:focus-visible { outline: 2px solid var(--focus);
  outline-offset: 1px }` — i.e. only keyboard focus shows a ring (Chromium's focus-visible
  heuristics: after Tab/arrow keys, or when script moves focus after a keyboard action). `--focus` is
  white (dark) / `rgba(0,0,0,0.896)` (light). The ring follows the element's border-radius.
  Exceptions: cartridge body and eject strip use `outline-offset: -2px` (ring inside), and on the
  active cartridge the ring color is `on-accent`; text fields never show a ring (their focus style is
  the accent underline); the Settings toggles draw the ring on the visible track, not the hidden
  checkbox.
- **Native tooltips**: every `title` attribute (listed per control below) shows as a Windows
  tooltip after the system hover delay. They are not in the renders; their texts are in the layout
  files (`attrs.title`).
- **Theme**: `prefers-color-scheme` follows the window theme set by Rust (`theme` setting:
  system/light/dark, applied live to all windows by `set_theme`). No transition between themes.
  `color-scheme` is set, so form controls and the native scrollbars (none visible here) follow.
- **Reduced motion**: with Windows "Animation effects" off (`prefers-reduced-motion: reduce`), the
  marquee, both equalizers, the preloading bolt pulse, the "dropped" scale-in and the dialog
  opening animation are removed (`animation: none`). Transitions (slider dot, toggle thumb,
  chevron) are **not** removed.
- **Scale**: everything is laid out in CSS px and scaled by the device pixel ratio, except the
  analyzer canvas (section 2.2.4).

---

## 2. Player window (`index.html`, label `main`)

### 2.1 Window

- Client size **860 × 267**; **860 × 587** with the playlist drawer open (+320). Not resizable, not
  maximizable. Built at the saved height (`playlist-open`) so it opens without animating.
- Window background (seen while the window grows, before the page repaints): `#202020` dark,
  `#f3f3f3` light — the same as `--bg`.
- The page never scrolls: `html, body { height: 100%; overflow: hidden }`, and scroll listeners on
  `window` and `body` reset `scrollTop` to 0 (focusing something in the hidden drawer can't scroll
  the page). With the drawer closed, the drawer is laid out below 267 and cut off by the window, and
  is `inert` (no focus, no clicks).
- Layout: `.app` is a grid `330px | 1fr` with a 28 px gap and padding `6 22 22`, height 267 →
  left column 330 wide (x 22–352), right column 458 wide (x 380–838). Left: three "islands"
  stacked with `justify-content: space-between` (display 91 tall, sliders 80, transport 52, gaps
  12). Right: the 3 × 2 cartridge grid (gap 10) over the deck bar (52 tall), gap 12.
- "Island" look (display, sliders, transport, deck bar, playlist, cartridges, Settings rows, info
  cards): 1 px border `--card-stroke`, radius 8, background `--card` clipped to the padding box,
  plus an inset 1 px ring `--card-edge` (a second, lighter edge inside the border). Scrolled
  content (the playlist) paints **over** that inset ring (it is a background-phase shadow).

### 2.2 Display island

#### 2.2.1 Screen (clock + analyzer)

- A black panel (`--display`: `#121212` dark, `#1c1c1c` light — dark in both themes; its text is
  always white), radius 6, padding `10 12 12`, gap 7, inset shadows. Sized by its content: the
  clock (24 px line) over the canvas.
- **Click** anywhere on the screen → `player_cycle_colors` (Rust cycles classic → white → accent →
  off → classic and sends the new `colors` back; the page repaints the last frame with the new
  palette). Tooltip: "Click to change the analyzer colors". No hover or pressed style.
- **Clock** (`renderTransport`): `elapsed` floored to whole seconds (0 while stopped). Format
  `MM:SS` with zero-padded minutes under an hour ("00:08", "12:34"), `H:MM:SS` from one hour
  ("1:02:05", hours not padded). Font: Segoe UI Variable Display semibold 22/24, tabular numbers.
  Stopped: the clock is dim (`rgba(255,255,255,0.35)`) and so is the icon.
- **State icon** (10 × 10, before the time, gap 5): play triangle for playing / buffering /
  connecting, pause bars for paused, stop square for stopped. Its fill is `--accent-light` in
  **both** themes (not `--accent`), or the dim clock color when stopped.

#### 2.2.2 Now playing (station, title, specs)

- Station line: `state.station`, 11/14 `--text-2`, one line, ellipsis.
- Title line: `state.title`; `titleDim` → regular weight `--text-3`, else semibold `--text`.
- **Marquee** (`setTitle`): runs only when the title text or its dim flag changes (not on resize or
  DPI change). It removes `is-scrolling`, measures `overflow = title.offsetWidth −
  titleRow.clientWidth`; if `overflow > 0`:
  - adds `is-scrolling`, which gives the text `padding: 0 6px` and the row a mask
    `linear-gradient(to right, transparent, #000 6px, #000 calc(100% − 12px), transparent)`
    (6 px fade in on the left, 12 px fade out on the right);
  - `--distance = −(overflow + 18) px` (12 px of padding + 6 px), `--duration = max(6, overflow / 20
    + 4) s`;
  - animation `marquee var(--duration) ease-in-out infinite alternate`, keyframes `0%, 12% →
    translateX(0)` and `88%, 100% → translateX(var(--distance))`. So each sweep holds 12 % of the
    duration, moves for 76 % with ease-in-out (the timing function applies to the 12 %→88 % segment),
    holds 12 %, then the next iteration plays backwards. Average speed while moving ≈
    `(overflow + 18) / (0.76 × duration)`: about 20–26 px/s for long titles; short overflows still
    take ≥ 6 s per sweep.
  - Note: the measure happens with the screen at its current width. At 150 % scale the analyzer is
    wider (section 2.2.4), so titles that fit at 100 % / 125 % / 200 % may scroll at 150 % (see
    `renders/player/playing-t0000.*@1.5.png`).
  - A title that fits: no padding, no mask, static.
- Specs (11/16 `--text-2`, numbers semibold `--text` tabular): "`<bitrate>` kbps" (`--` when null
  or 0), "`<kHz>` kHz" where kHz = samplerate/1000 shown as an integer when whole, else with one
  decimal (`toFixed(1)`: 22050 → "22.1", 44100 → "44.1"), `--` when unknown.
- Channel badges MONO and STEREO (9 px, uppercase, letter-spacing 0.04 em, 1 px border, radius 3,
  padding 0 4): MONO lit when `channels === 1`, STEREO when `channels >= 2`; lit = accent text and
  border, unlit = `--text-disabled` with `--control-stroke` border. Right-aligned (margin-left auto).

#### 2.2.3 Analyzer drawing (`visualizer.js`)

- A 75 × 16 pixel grid: 19 bars of 3 px with 1 px gaps (bar *i* at x = 4*i*), 16 rows. Row *r*
  (0 = bottom) is drawn at y = 15 − *r*.
- Frames come from Rust (77 bytes, ~60 Hz, only when changed): byte 0 = loader level (0–255); for
  bar *i*: `1+4i` lit rows (0–15, clamped to 15), `2+4i` peak row (0 = none), `3+4i` loader
  half-height, `4+4i` loader alpha. Each frame clears and repaints the whole canvas. The page never
  clears it by itself (Rust sends a blank frame on stop). A color change repaints the last frame.
- Bars: rows 0 … lit−1 of bar *i*, each in its row color. Peak (if ≥ 1): one row at y = 15 − peak,
  in the peak color (so a peak sits one row above a bar of the same height).
- Palettes: **classic** — fixed per-row colors (bottom to top) `24,132,8` `41,148,0` `49,156,8`
  `57,181,16` `50,190,16` `41,206,16` `148,222,33` `189,222,41` `214,181,33` `222,165,24` `198,123,8`
  `214,115,0` `214,102,0` `214,90,0` `206,41,16` `239,49,16`, peak `rgb(150,150,150)`.
  **white** — row *r* = white at alpha `0.45 + 0.55 × r / 15` (0.45 bottom → 1.0 top), peak solid
  white. **accent** — the same ramp in `--accent-light` (the light shade in **both** themes; if the
  value isn't `#rrggbb`, white), peak solid accent-light. **off** — Rust sends no bars (zeros), the
  loader still shows.
- Loader crossfade: level *L* = byte0 / 255. If *L* < 1 the bars and peaks are drawn at alpha
  1 − *L*; if *L* > 0 the loader is drawn on top: for each bar, `half = min(8, byte)`, alpha
  `L × alpha/255`, rows `8 − half … 7 + half` (vertically centred blocks) in the row colors. Rust
  (`qust_spectrum`) fades the level in over 0.2 s when connecting/buffering starts and out over
  0.5 s; the pulse head sweeps left to right every 1.1 s: `head = ((t / 1.1) mod 1) × 25 − 3`,
  `pulse_i = max(0, 1 − |i − head| / 3)`, half = `1 + round(5 pulse)` (1–6 → 2–12 rows tall),
  alpha = `0.3 + 0.7 pulse`. `harness/frames.mjs` reproduces this.

#### 2.2.4 Canvas scaling rule

`Spectrum.fit()` (on load and on every `resize`, which includes DPI changes): with *r* =
`devicePixelRatio` and *s* = `max(1, round(r))`, the canvas's CSS size is `75·s/r × 16·s/r`, with
`image-rendering: pixelated`. So each grid pixel is exactly *s* × *s* device pixels:

| Windows scale | r | s | CSS size | device px per grid px |
|---|---|---|---|---|
| 100 % | 1 | 1 | 75 × 16 | 1 |
| 125 % | 1.25 | 1 | 60 × 12.8 | 1 |
| 150 % | 1.5 | 2 | 100 × 21.33 | 2 |
| 175 % | 1.75 | 2 | 85.71 × 18.29 | 2 |
| 200 % | 2 | 2 | 75 × 16 | 2 |
| 250 % | 2.5 | 3 | 90 × 19.2 | 3 |

The screen box, and with it the width left for the now-playing text, changes accordingly (screen
99 × 69 CSS px at 100 % and 200 %, 92.95 × 65.8 at 125 % — the clock, not the 60 px canvas, then sets the width — and 124 × 74.33 at 150 %; measured in layout/player/*.json scales).

### 2.3 Sliders island

#### 2.3.1 Layout

- Stream (no duration): volume row over balance row (rows 28 tall, gap 4, padding `9 14`). Each
  row: 16 px icon (`--text-2`), the slider (flex 1, 24 tall), a 34 px right-aligned value (12 px
  `--text-2`, tabular).
- File with a duration (`state.duration` truthy → `.sliders.is-seekable`): a seek row on top
  (slider + time text, min-width 78, right-aligned, 12 px), then volume and balance **side by side**
  (gap 14, inner gap 4, icons 14 px, values 30 px wide at 11 px, thumbs 16 px, rails inset 8 px).

#### 2.3.2 Look

- Rail: 4 px tall, radius 2, `--slider-track`, inset 10 px from the slider's left and right edges
  (8 px in the side-by-side pair; 8 px on the left only for the seek bar). Fill: accent, from
  `min(origin, value)` to `max(origin, value)` where origin is 0 (left) for volume/seek and the
  centre for balance (balance fills from the middle towards the thumb).
- Thumb: 20 px circle (16 in the pair) centred on the value, `--thumb` (`#454545` dark, white
  light), ring `0 0 0 1px --control-stroke` + shadow `0 1px 2px rgba(0,0,0,.25)`; inner dot inset
  4 px in accent. Inner dot scale: 1.167 on slider hover, 0.833 while dragging, transition
  `transform 0.1s ease`.
- Balance has a centre tick: 1 × 12 px at 50 % of the rail, `--slider-track`.
- Disabled seek bar (`aria-disabled="true"`): fill and inner dot `--text-disabled`.
- Values: volume "`N`%"; balance "C" at 0, "L `N`" left, "R `N`" right. Volume icon: muted
  speaker (×) at 0, one wave below 50, two waves from 50.

#### 2.3.3 Interaction (`makeSlider`)

| | Volume | Balance | Seek |
|---|---|---|---|
| range | 0–100 | −100…100, centre 0 | 0–10000 (ten-thousandths of the duration) |
| sends | `player_set_volume` on every change | `player_set_balance` on every change | `player_seek(seconds)` only on commit |

- Values are integers (rounded, clamped).
- **Pointer**: primary button down on the slider (the whole 24 px-tall box, not just the rail),
  unless disabled → pointer capture, `is-dragging`, value from the pointer: `min + (x − rail.left) /
  rail.width × span` (positions beyond the rail clamp). Moves update it continuously; up / cancel /
  lost capture end the drag and **commit**. Snap (pointer only): within 4 % of the span of the
  centre (balance: ±8) the value becomes the centre.
- **Double-click**: balance → centre (C). Volume and seek: nothing.
- **Wheel**: always `preventDefault` (the wheel never scrolls anything from a slider). Up = +5 % of
  the span, down = −5 % (volume ±5, balance ±10, seek ±500 = 5 % of the file), then commit.
  Disabled: nothing.
- **Keys** (focused slider): Right / Up +5 %, Left / Down −5 %, PageUp +10 %, PageDown −10 %,
  Home = min, End = max; then commit. No snap for keys and wheel.
- While the user drags, values from Rust are ignored (no fighting); otherwise Rust's value is shown
  when it differs. Values from Rust never trigger a command.
- Seek bar: value = `min(1, position / duration) × 10000`, or 0 when not seekable or position is
  null. Not seekable (e.g. stopped on a file) → `aria-disabled`, `tabIndex −1`, ignores pointer,
  wheel and keys. Time text: "**pos** / dur" (pos semibold `--text`), both `formatTime`, pos taken
  from the bar's value (so it follows the pointer while dragging).
- `formatTime` (seek text, playlist durations, total): `round(seconds)`, `M:SS` (minutes **not**
  padded: "1:15", "5:01"), `H:MM:SS` from an hour. The clock (2.2.1) differs: floor, `MM:SS`.
- Tooltip: balance "Double-click to center". Accessibility: `role=slider`, `aria-valuenow`.

### 2.4 Transport island

- Six buttons sharing the width equally (`flex: 1`, ≈ 44.67 × 32 at 100 %), gap 8, padding `9 10`,
  16 px icons: Play (accent button), Pause, Stop, Instant switch (toggle), Stream information,
  Settings.
- Enabled rules: **Play** disabled when nothing is active (no active cartridge or entry) or the
  state is playing / buffering / connecting. **Pause** disabled when stopped or paused. **Stop**
  disabled when stopped. Instant, Info and Settings always enabled.
- Actions: Play → `player_play`; Pause → `player_pause`; Stop → `player_stop`; Instant →
  `player_set_instant(!instant)`; Info → closes the dialog if open, then `open_panel('info')`;
  Settings → closes the dialog, then `open_panel('settings')` (an already open panel is focused,
  not reopened).
- Tooltips: Play "Play" or "Play (Ctrl+Alt+Insert)" (the global shortcut's keys when registered);
  Stop "Stop" / "Stop (Ctrl+Alt+H)"; Pause "Pause"; Info "Stream information"; Settings "Settings",
  or "Settings: version X is ready to install" with an update waiting; Instant off: "Instant switch:
  keep every cartridge streaming in the background, even when stopped, for immediate starting and
  switching (uses more data)"; on: "Instant switch is on: every cartridge streams in the background,
  even when stopped, so starting or switching is immediate (uses more data)".
- Update dot: with an update downloaded (`update-status` = ready), a 7 px accent dot at top 5 / right
  7 of the Settings button with a 2 px `--bg` ring.
- Button look (`.button`, also deck bar and dialog): 32 tall, radius 4, 1 px border
  `--control-stroke` with a `--control-stroke-top` top edge, `--control` fill. Hover: `--control-hover`.
  Pressed (`:active`, mouse held): `--control-pressed`, text `--text-2`. Disabled:
  `--control-disabled`, text `--text-disabled`. Accent button: accent fill, `on-accent` text,
  border `rgba(255,255,255,.08)` with bottom `rgba(0,0,0,.14)`; hover 90 % accent, pressed 80 %
  accent with text at 75 %, disabled `--accent-disabled` with no border. The pressed Instant toggle
  (`aria-pressed=true`) looks like the accent button (hover 90 %; no separate pressed style beyond
  the generic one). **No transitions** on any button: state changes are instant.
- Enter / Space on a focused button activates it (native button behaviour).

### 2.5 Cartridges (deck)

#### 2.5.1 Look per state

- Grid 3 × 2, equal cells (≈ 146 × 86.5 at 100 %), gap 10. Each cartridge: a body button (flex 1,
  padding `4 12`, content centred) over a 22 px eject strip (top border `--card-stroke`, 14 px eject
  icon `--text-2`).
- **Empty**: 1 px **dashed** `--empty-stroke` border, no fill, no inset ring, `--text-3`; the body
  is a row: "+" icon (16 px) and "Load" (13/18 regular), gap 6; no host line, no eject strip.
  aria-label "Load cartridge N".
- **Loaded**: name (Segoe UI Variable Display semibold 15/20, one line, ellipsis) over host
  (`new URL(url).host`, with the port when not default, 11/14 `--text-3`, ellipsis). aria-label
  "Play <name>".
- **Active** (`activeSlot`): accent fill, transparent border, no inset ring, `on-accent` text; host
  and eject icon at 70 % `on-accent`; eject divider 15 % `on-accent`.
- **Playing** (active and state not stopped/paused — i.e. also connecting and buffering): an
  equalizer at top 8 / right 9: three bars 2 × 10 px, gap 2, radius 1, current color, each
  `scaleY(0.25 → 1)` from the bottom, 0.9 s ease-in-out infinite alternate, delays 0, −0.3 s,
  −0.6 s.
- **Instant switch**: `preload` "live" → a solid 11 px bolt at top 8 / right 8 in accent; "loading"
  → the same bolt pulsing opacity 0.15 ↔ 0.6, 0.8 s ease-in-out infinite alternate. Rust sends a
  preload for every loaded cartridge except the active one while playing; while **stopped** the
  active cartridge gets one too, and its accent bolt is drawn on its accent fill — invisible
  (faithful quirk, see `instant-stopped`).
- Hover on body or eject strip: `--subtle-hover` fill (on the active cartridge: 8 % `on-accent`);
  pressed: `--subtle-pressed` (active: 4 %). The body and strip highlight separately.

#### 2.5.2 Interaction

- Click on the body: loaded → `cartridge_play(slot)` (switches station; Rust keeps or restarts as
  needed); empty → the Load cartridge dialog. Click on the eject strip → the Edit cartridge dialog
  (tooltip "Edit or eject"). Keyboard: both are buttons (Tab, Enter/Space).
- **Drag to reorder** (pointer events, primary button, loaded cartridges only, not starting on the
  eject strip):
  - Starts once the pointer has moved ≥ 5 px (Euclidean) from the press point. A **ghost** (a
    clone of the cartridge, same size, `position: fixed`, z-index 20, no pointer events, background
    `--dialog` — or accent if it was the active one, shadow `0 16px 32px rgba(0,0,0,.35), 0 2px 8px
    rgba(0,0,0,.25)`) follows the pointer keeping the grab offset, `scale(1.04)` (about its centre).
    The ghost keeps the source's classes, so it shows the equalizer / host / eject like the source.
    The source fades to opacity 0.35.
  - The cartridge under the pointer (any slot, empty ones included, but not the source) is the
    target: 2 px accent outline, offset 2.
  - Release over a target → `cartridge_move(from, to)` (swap; onto an empty slot = move), then both
    slots play `dropped`: `scale(0.94) → 1` over 0.25 s `cubic-bezier(0.1, 0.9, 0.2, 1)`. Release
    elsewhere: nothing moves. Esc or pointer cancel: cancelled. The click that ends a drag never
    plays or opens anything.
- Right-click: section 2.9.

### 2.6 Deck bar

- Three buttons, equal width (flex 1), gap 8, padding `9 10`; text 13 px, padding `0 8`, gap 7,
  icon + label: **Playlist** (list icon, label, 12 px chevron `--text-2`), **Add file** (note icon),
  **Add URL** (link icon).
- Playlist: toggles the drawer (section 2.7); `aria-expanded`; the chevron rotates 180° when open,
  transition `transform 0.2s cubic-bezier(0.1, 0.9, 0.2, 1)`. Tooltip "Show or hide the playlist".
- Add file → `playlist_add_files` (Rust shows the native file picker "Add to playlist", filters
  "Audio files and playlists" / "All files", multi-select); the added entries are then selected and
  revealed, opening the drawer if needed. Tooltip "Add music files or playlists from your
  computer".
- Add URL → the Add URL dialog. Tooltip "Add a stream or an online audio file".

### 2.7 Playlist drawer

#### 2.7.1 Opening and closing

- The page sets `aria-expanded`, `inert` and calls `set_playlist_open(open)` immediately; Rust saves
  `playlist-open` and animates the **window height** 267 ↔ 587 in 14 steps 14 ms apart (≈ 200 ms),
  eased `1 − (1 − t)³`, rounded to whole px. Opening near the bottom of the work area also moves
  the window up (same easing) so the drawer stays on screen. A second click mid-animation starts a
  new animation from the current height. Nothing in the page animates (the drawer is simply
  uncovered by the growing window).
- The drawer opens by itself when files are dragged over the window, and after entries are added.

#### 2.7.2 Content and look

- Drawer: 320 tall, padding `0 22 14`, gap 6: the list island (flex 1 → 284 tall at 100 %) over
  the status line (16 tall).
- List (`role=listbox`, multi-select, focusable): padding 4; vertical scrolling only, overscroll
  contained; scrollbar 10 px wide: thumb `--text-3` with a 3 px transparent border (4 px visible),
  radius 5; no track or arrows; no hover style.
- Rows, in order: the **loaded cartridges** (pinned; radio icon in the number column with tooltip
  "Cartridge N"; detail = host), a **divider** if any cartridge (1 px `--card-stroke` + 1 px
  `--card-edge` below, margin `4 8`), then the **entries** numbered 1…n, then, with no entries, the
  **empty hint**: dashed `--empty-stroke` box (radius 6, margin `6 4 4`, padding `18 16`) with
  "Drop music files or folders here" (13 semibold `--text-2`) and "or use Add file and Add URL.
  Files play from where they are." (12 `--text-3`).
- Row: 26 tall, radius 4, padding `0 10 0 4`, gap 10, 13/18. Number column 34 px, right-aligned,
  12 px `--text-3` tabular (cartridge rows: icon in `--text-2`). Title: flex, ellipsis. Detail
  (12 px `--text-3`, tabular, max 40 % of the row, ellipsis): the duration (`formatTime`), else the
  URL's host for URL entries, else nothing. Row tooltip: the file path or URL (cartridge rows: the
  URL).
- Row states: hover `--subtle-hover`; selected `--row-selected` (hover `--row-selected-hover`) plus
  Fluent's pill: 3 × 12 px accent bar at the row's left edge (top/bottom 7), radius 2; **active**
  (the active cartridge or entry, playing or not): title and number accent, semibold; **playing**
  (active and not stopped/paused): the number is replaced by an accent equalizer (same as the
  cartridge's, 10 px tall, margin-right 2); **missing**: title `--text-disabled`, struck through.
- Status line (11/16): left "`N` tracks" ("1 track") · total duration (if any entry has one) ·
  "`K` selected" (only when more than one row is selected) in `--text-2`; right, in `--text-3`:
  "Double-click to play · drag to reorder · Delete to remove".

#### 2.7.3 Selection and keyboard

- Click a row: select it alone (anchor = it). Ctrl/Cmd+click: toggle it (anchor = it). Shift+click:
  select the range from the anchor to it in row order (cartridge rows included). Click on empty list
  space without Ctrl/Shift: clear the selection. Double-click a row: play it (`cartridge_play` /
  `playlist_play`); the first click of it has already selected it.
- Keyboard, with the list focused (the island gets the focus ring): Up/Down ±1 row, PageUp/PageDown
  ±10, Home/End first/last — from the focused item (or the first row if none), selecting it alone
  (Shift: extending from the anchor) and scrolling it into view. Enter: play the focused item.
  Delete or Backspace: remove the selected **entries** (cartridge rows are never removed); the
  selection moves to the entry after the removed ones (or the new last one). Ctrl/Cmd+A: select all
  entries (not cartridges). There is no visible "focused row" indicator — only the selection.
- Revealing a row scrolls the list (never the page) just enough to show it with a 4 px margin.
- After adding (drop, Add file, Add URL): the new entries become the selection, anchor and focus on
  the first; the list scrolls to show the last, then the first.
- The selection drops rows that no longer exist; rows are rebuilt on every `player-playlist`
  (scroll position kept); a single changed entry (`player-entry`) redraws only its row.

#### 2.7.4 Reordering tracks (drag)

- Primary button, no modifier, on an entry row (not a cartridge row): if the row isn't selected it
  becomes the only selection (a multi-selection is kept when pressing inside it). The drag starts
  after 5 px of **vertical** movement: rows get the grabbing cursor.
- Insertion index = the nearest row boundary under the pointer: `round((y − list.top +
  scrollTop − firstEntry.offsetTop) / rowHeight)`, clamped to 0…n (over the cartridge rows → 0);
  outside the list (or drawer closed) → the end. Shown as a 2 px accent line, radius 1, inset 6 px
  left and right, at the top of the row at that index (−1 px), or at the bottom of the last row for
  the end.
- Autoscroll: on each pointer move within 24 px of the list's top or bottom edge, the list scrolls
  12 px that way (only while the pointer moves).
- Release → `playlist_move(selected ids in list order, index)`; Esc / pointer cancel → cancelled.
  The click ending a drag is ignored.

#### 2.7.5 Dropping files from Explorer (Tauri drag-drop events)

- Ignored while the dialog is open. `enter`/`over`: opens the drawer if closed, sets
  `body.is-dropping` (no style attached), shows the insertion line at the drop index (same rule as
  2.7.4; positions are physical px divided by `devicePixelRatio`); with no entries, the empty hint
  turns accent (border accent, 8 % accent fill). `leave`: clears. `drop`: `playlist_drop(paths,
  index)` → Rust expands folders (natural order, recursive) and `.m3u`/`.pls` lists; the added
  entries are selected and revealed.

### 2.8 Dialog (Load / Edit cartridge, Add URL, Edit track)

- Modal over the whole window: a backdrop `rgba(0,0,0,0.3)` (both themes; appears instantly, no
  fade) centring a 360 px card: 1 px `--dialog-stroke`, radius 8, `--dialog` fill, shadow `0 32px
  64px rgba(0,0,0,.37), 0 2px 21px rgba(0,0,0,.37)`. Opening animation `dialog-in` 0.25 s
  `cubic-bezier(0.1, 0.9, 0.2, 1)` from opacity 0 and scale 1.05. Closing is instant.
- Body (padding `16 20 14`, gap 8): title (Display semibold 18/24), two fields (label 13/18 over a
  32 px text box, gap 4), the error line (12/16 `--error`, hidden unless there is an error). Footer
  (padding `16 20`, top border `--card-stroke`, `--dialog-footer` fill): buttons of equal width,
  gap 8 — Save/Add (accent), Eject (Edit cartridge only), Cancel.
- Card height 247 px — or 271 px with an error line, taller than the 267 px window: it is centred
  and clipped 2 px top and bottom (faithful; see `dialog-error`).
- Variants:

| | title | name label / placeholder | URL label | save | eject | focus |
|---|---|---|---|---|---|---|
| empty cartridge | Load cartridge | Name / "My station" | Stream URL | Save | — | Name |
| loaded cartridge | Edit cartridge | Name / "My station" | Stream URL | Save | Eject | Name |
| Add URL | Add URL | Name (optional) / "Taken from the address" | Stream or audio file URL | Add | — | URL |
| Edit track (file) | Edit track | Name / the file's name without extension | File (read-only path) | Save | — | Name |
| Edit track (URL) | Edit track | Name / the URL's file name or host | Stream or audio file URL | Save | — | Name |

  The URL field's placeholder is always "https://". The read-only path has no fill, no underline
  and `--text-2` text.
- Text box: 32 tall, padding `0 10`, radius 4, 1 px `--control-stroke` with a `--input-stroke-bottom`
  bottom edge, `--control` fill, 14/20 `--text`, placeholder `--text-3`; hover `--control-hover`;
  focused: `--input-focus` fill and an accent underline (bottom border accent + inset `0 -1px 0`
  accent = 2 px), no focus ring. Spellcheck and autocomplete off. Caret: 1 px in the text color,
  system blink rate (not in the renders).
- **Enter** in a field or Save/Add submits: values trimmed; `cartridge_save(slot, title, url)` /
  `playlist_edit(id, title, url | null for files)` / `playlist_add_url(url, title)`. An empty name
  is allowed (Rust names it). Success closes the dialog (Add URL then selects and reveals the new
  entry). Failure: the error text (from Rust, e.g. "Enter an address starting with http:// or
  https://") appears and the URL field is focused. A second submit while one is pending is ignored.
  No client-side validation (`novalidate`).
- **Eject**: closes, then `cartridge_eject(slot)`. **Cancel**: closes. **Esc** (key pressed inside
  the dialog): closes. Clicking the backdrop does nothing.
- **Tab** cycles inside the dialog: the focusable order is Name, URL, Save, Eject (if shown),
  Cancel; Tab on the last goes to the first, Shift+Tab on the first to the last (the read-only path
  is still in the cycle).
- While the dialog is open: the player's right-click menu is suppressed (text boxes still get Cut /
  Copy / Paste), file drops are ignored, and opening Settings or Stream information closes it.

### 2.9 Right-click menus (native, player window)

Built per click; disabled items are greyed; `accelerator` is the global shortcut shown right-aligned
(only when registered; if it can't be parsed the menu is rebuilt without accelerators). Every menu
ends with a separator (if anything precedes) and **Stream information**, **Settings**.

| Where | Items |
|---|---|
| Player area (display, sliders, transport, deck bar, outside cartridges) while playing/connecting/buffering | Pause · Stop (accel. Stop shortcut) |
| … while stopped or paused | Play (enabled when something is active; accel. Play shortcut) · Stop (disabled when stopped) |
| Loaded cartridge | Play · Edit… · Eject |
| Empty cartridge | Load cartridge… |
| Drawer, on an entry row | Play · Edit… · Remove (or "Remove N tracks" when N > 1 entries are selected) · sep · Add file… · Add URL… · Clear playlist |
| Drawer, on a cartridge row | Play · Edit cartridge… · sep · Add file… · Add URL… · Clear playlist |
| Drawer, elsewhere | Add file… · Add URL… · Clear playlist |

Right-clicking a row that isn't selected selects it alone first. "Clear playlist" is disabled with
no entries and removes every entry. Edit… opens the matching dialog; Eject ejects without
confirmation. Recorded item lists: `layout/player/menu-*.json` (`menus`).

### 2.10 Focus order (Tab)

DOM order, skipping disabled and hidden controls: [seek bar, when shown and seekable] → volume →
balance → Play → Pause → Stop → Instant → Info → Settings → for each cartridge: body, eject strip
(loaded only) → Playlist → Add file → Add URL → [the playlist list, when the drawer is open]. The
screen is not focusable. No focus trap outside the dialog. Esc ends a cartridge or row drag.

---

## 3. Settings window (`settings.html`, label `settings`)

### 3.1 Window

- Client size **520 × 850**, owned by the player (stays above it, no taskbar button, not
  minimizable), centred over it (kept inside the work area). The content (931 px tall) scrolls:
  the body scrolls with an 8 px scrollbar (thumb `--text-3`, 2 px transparent border → 4 px
  visible, radius 4). Mouse wheel: native scrolling.
- **Esc** closes the window (unless a shortcut is being recorded). Closing resumes paused global
  shortcuts (Rust).
- Content: padding `16 20 20`; group headings (semibold 14/20, margin `14 0 6`, first 0):
  Appearance, Playback, Global shortcuts, Updates, About. Each setting is a card (min-height 56,
  padding `8 16`, gap 14, radius 6, margin-bottom 3): 18 px icon, title (14/20) over description
  (12/16 `--text-2`), control on the right.

### 3.2 Combos (Theme, Visualizer colors, Reconnect)

- 32 tall, min-width 170, padding `0 10 0 12`, current choice + 12 px chevron (`--text-2`); hover
  `--control-hover`, pressed `--control-pressed`.
- Click → a native menu at the combo's bottom-left (+2 px), one item per choice, the current one
  checked. Choosing: Theme (`Use system setting` / `Light` / `Dark`) → `set_theme` (all windows
  switch at once); Visualizer colors (`Classic` / `White` / `Accent color` / `Off`) → saved and
  broadcast (`settings-changed`) — the combo also follows the analyzer clicks in the player;
  Reconnect (`1 minute`, `5 minutes`, `15 minutes`, `30 minutes`, `1 hour`, `Until stopped`) →
  saved.

### 3.3 Toggles (Play on launch, Background helper, Check automatically)

- A label "On"/"Off" (14/20) then the track (gap 12). Track 40 × 20, 1 px `--text-2` border, radius
  10, `--control` fill; thumb 12 px `--text-2` at (3, 3). Checked: accent track and border, thumb
  `on-accent` moved `translateX(20px)`. Hover (anywhere on the label): thumb 14 px (top 2) and, when
  checked, `translateX(18px)`. Transition: `transform 0.15s cubic-bezier(0.1, 0.9, 0.2, 1)` and
  `width 0.1s ease` (height and top jump). Keyboard: Space toggles the focused (hidden) checkbox;
  the focus ring is drawn on the track.
- Disabled setting (`.setting.is-disabled`): icon, title and toggle at opacity 0.45 (the
  description stays).
- Play on launch → saved (`autoplay`) and broadcast.
- Background helper → `set_background(enabled)`: success shows Rust's answer; failure turns it back
  and shows the error in `--error` in place of the description. Not the installed version: disabled,
  off, description "Works in the installed version of Qust.".
- Check automatically → saved (`auto-update`); disabled when not installed.

### 3.4 Global shortcuts (Play, Stop, Play / Stop / Start)

- Box like a combo (min-width 170, padding `0 4 0 12`, right-aligned content, gap 4, 13/20
  `--text-2`); tooltip "Click, then press a new shortcut". Keys as chips: accent fill, `on-accent`,
  semibold 12/20, padding `1 7`, radius 4, min-width 24, centred. Key names: modifiers as stored
  (Ctrl, Alt, Shift), Super → "Win", KeyX → "X", DigitN → "N", Numpad… → "Num …", Arrow… → Up /
  Down / Left / Right, Backquote → "`", others as their `KeyboardEvent.code` ("Insert", "F9",
  "MediaPlayPause").
- Unassigned: "Not set" in `--text-3`. Unavailable (owned by another app at startup): chips grey
  (`--control-hover` fill, `--text-2`, struck through) and the description in red "Another app is
  using this shortcut. Pick a different one.".
- Click → recording: dashed 1 px accent border, text "Press a shortcut…" in `--text`, description
  "Esc to cancel, Backspace to reset"; Rust pauses all global shortcuts. While recording every
  keydown is swallowed: modifier keys alone are ignored; Esc cancels; Backspace or Delete resets to
  the default (Play Ctrl+Alt+Insert, Stop Ctrl+Alt+H, Play/Stop/Start none); any other key needs at
  least one modifier unless it is F1–F24 or a Media key; the accelerator is `Ctrl+Alt+Shift+Super`
  (those present, in that order) + the key's `code`. Rust may refuse ("Already used for another
  action", "Another app is already using this shortcut", "That isn't a valid shortcut"): the error
  replaces the description in red until the next change. Losing window focus cancels recording.
  Only one shortcut records at a time.

### 3.5 Updates

| status | description | button | button state |
|---|---|---|---|
| not installed | Updates work in the installed version of Qust. | Check for updates | disabled |
| idle | Last checked `ago`. / Not checked yet. | Check for updates | enabled |
| checking | Checking for updates… | Check for updates | disabled |
| downloading | Downloading version X… P% | Check for updates | disabled |
| ready | Version X is ready to install. | **Restart and update** (accent) | enabled |
| installing | Installing… | Check for updates | disabled |
| error | Couldn't update: `error` (red) | Try again | enabled |
| current | You're up to date. Last checked `ago`. | Check for updates | enabled |

`ago`: under 60 s "just now", under an hour "N min ago", under a day "N h ago", else "N days ago"
(also "1 days ago"). Computed when the status is rendered (on open and on each `update-status`
event); it doesn't tick. The button runs `install_update` when ready, else `check_for_updates`.
Title: "Qust `version`".

---

## 4. Stream information window (`info.html`, label `info`)

- Client size **580 × 590**, owned by the player, centred over it. **Esc** closes it. Body scrolls
  if needed (same 8 px scrollbar; the content fits).
- Updates: `player-status` broadcasts (every second and on changes) and a 1 s poll of
  `stream_info(url)` while something plays; values re-render only when their text changes.
- Header: station line (12/16 `--text-2`, "Not playing" when nothing is active; it doesn't
  truncate), title (Display semibold 18/24, ellipsis): while active, the ICY title, else the Icecast
  status title, else the stream name, else the station; stopped: "Start a cartridge to see its
  stream details". State badge (pill: padding `2 10`, radius 11, 1 px `--control-stroke`, 12/16
  `--text-2`): Stopped / Connecting / Buffering / Playing / Paused; **Playing** in accent (text and
  border).
- Three cards (Stream, Network, Stability; equal columns, gap 10) and the ICY metadata card (two
  columns 130 px | rest, gaps 5 × 12). Card headings: 11/16 semibold uppercase, letter-spacing
  0.06 em, accent. Labels 12/16 `--text-2` (13/20 in the ICY card); values semibold 15/20 tabular,
  one line, ellipsis (13/20 regular in the ICY card), margin-bottom 7.
- Empty values show "—". Every value (and the header texts) has its full text as tooltip, so
  truncated values can be read by hovering; values are selectable and right-click offers Copy.
- Formatting: Format = `format` or from the content type (audio/mpeg MP3, audio/aac AAC,
  audio/aacp AAC+, audio/ogg / application/ogg Ogg, audio/flac FLAC, audio/opus Opus); Bitrate "N
  kbps" (0 → —); Sample rate kHz with up to 1 decimal ("44.1 kHz", "48 kHz"); Channels Mono /
  Stereo / "N channels"; Receiving "N kbps"; Buffer "x.x s" (while active); Received = bytes +
  metadata bytes: under 1,000,000 "N KB" (÷1000, rounded), else "x.x MB" (÷10⁶); Connected for
  `M:SS` / `H:MM:SS`; Dropouts (while active), Reconnections, Title changes; Host = the URL's host
  (a local path has none → —); Metadata interval "16,000 bytes" or "None" (while active);
  Metadata received only when there is an interval.

---

## 5. Timing summary

| What | Duration / easing |
|---|---|
| Marquee | `max(6, overflow/20 + 4)` s per sweep, alternate, ease-in-out, holds 12 % at each end |
| Cartridge / row equalizer | 0.9 s ease-in-out alternate infinite, scaleY 0.25↔1, phases 0 / −0.3 / −0.6 s |
| Preloading bolt | 0.8 s ease-in-out alternate infinite, opacity 0.15↔0.6 |
| Cartridge dropped | 0.25 s cubic-bezier(0.1, 0.9, 0.2, 1), scale 0.94→1 |
| Dialog opening | 0.25 s cubic-bezier(0.1, 0.9, 0.2, 1), opacity 0→1, scale 1.05→1; closing instant |
| Slider dot | transform 0.1 s ease (1 → 1.167 hover, 0.833 drag) |
| Playlist chevron | transform 0.2 s cubic-bezier(0.1, 0.9, 0.2, 1) |
| Toggle thumb | transform 0.15 s cubic-bezier(0.1, 0.9, 0.2, 1), width 0.1 s ease |
| Drawer (window height) | 14 × 14 ms ≈ 0.2 s, ease-out cubic (Rust) |
| Loader | fade in 0.2 s, out 0.5 s, sweep 1.1 s (Rust frames) |
| Buttons, rows, cartridges hover/press | instant |
| Drag thresholds | 5 px (cartridges: distance; rows: vertical) |
| Slider steps | key/wheel 5 % (Page 10 %), snap ±4 % around the balance centre |
| Playlist keys | ±1 row, Page ±10 rows; autoscroll 12 px within 24 px of the edges |
