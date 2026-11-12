# Qust UI reference

Ground truth for rebuilding Qust's interface natively: the webview UI as it was (frozen), rendered
in every state, measured, and described. The rebuild is measured against this.

| | |
|---|---|
| `page/` | The pages exactly as they were in `src/` (commit 7018516, byte for byte): `index.html` (player), `settings.html`, `info.html`, `css/style.css`, `js/*.js`. All icons are inline SVG symbols in the HTML; there are no other assets. |
| `states/*.json` | State fixtures: what Rust sends plus the user actions that lead to each state (list below). |
| `renders/<window>/<state>.<theme>@<scale>.png` | Reference renders at the window's exact client size. |
| `layout/<window>/<state>.json` | Measurements of every element in that state (both themes). |
| `layout/tokens.json` | Every CSS variable, declared and resolved in dark and light; keyframes; the fonts actually used. |
| `BEHAVIOUR.md` | Everything a picture can't show: interactions, keys, menus, timings, formats, scaling. |
| `compare.mjs` | Compares a capture of the rebuilt UI with a reference render. |
| `harness/` | The renderer (`render.mjs`), the Tauri stand-in (`stub.js`), the measurer (`collect.js`), analyzer frames (`frames.mjs`), the fixture generator (`make-states.mjs`). |

Setup once: `npm install` in this folder (one dependency, `pngjs`). Needs Node 22+ (built-in
WebSocket) and Microsoft Edge (`msedge.exe`; set `QUST_REF_EDGE` if it lives elsewhere).

## How the renders are made

Since the 1.1.5 rewrite the pages are pure views: they render the state Rust sends. So
`harness/render.mjs` loads the frozen pages in **headless Microsoft Edge** (`--headless=new`, its own
temporary profile, nothing on the desktop, no sound) — the same Chromium engine WebView2 uses (Edge
157 = WebView2 157 on this PC) — with `window.__TAURI__` and `window.__QUST__` replaced by
`harness/stub.js`. The unmodified page scripts run: `player_attach` returns the fixture's state and
playlist, `get_data` its settings, `get_shortcuts` / `update_status` / `get_background` /
`player_status` / `stream_info` their values; analyzer frames go through the same `Channel`, drops
through the same `onDragDropEvent` handler, and user actions are real DevTools input events
(mouse, keys) or calls of the page's own functions. Every command the page sends is recorded
(`calls` in the layout file), and so is every native menu it builds (`menus`).

Fixed for determinism: the window theme (`prefers-color-scheme`), the device scale factor, the
accent (`window.__QUST__.accent`), `Date.now` (fixture `now`), animations (every animation frozen
`time` ms after it started: infinite ones at 0 unless the fixture says otherwise, finite ones at
their end; transitions at their end), and the text caret (hidden: its blink can't be frozen).

The default accent is the Windows default blue as `accent()` (crates/player/src/accent.rs) reads it
from the `AccentPalette` registry value: base = entry 3 `#0078d4`, light = entry 1 `#4cc2ff`,
dark = entry 4 `#0067c0` (this PC has that palette). Note that the CSS fallback in style.css
(`#60cdff` / `#005fb8`) is only used when no accent is injected (macOS). The `accent-orange-*` and
`settings-accent-orange` states use an illustrative second accent (`#da3b01` / `#ff9f6e` /
`#a92e00`) to show what follows the accent.

Sizes: player 860 × 267 (860 × 587 with the drawer), Settings 520 × 850, Stream information
580 × 590, as `main.rs` / `playlist.rs` build them. At a fractional scale the PNG is rounded the way
the real window's client area is (860 × 267 at 125 % → 1075 × 334).

## Regenerating

The renders (`renders/`, `renders-gray/`) aren't in the repository (39 MB of PNGs): make them
with the first and last commands below before running a capture comparison. The states, layouts
and tokens are committed, and the cargo tests read them.

```sh
node harness/make-states.mjs                 # rewrite states/*.json from the lists in it
node harness/render.mjs                      # every state: scale 1 in both themes, plus the states marked for all scales
node harness/render.mjs "drawer-*" stopped   # some states (names or globs; time variants match their base name)
node harness/render.mjs --all-scales         # the full matrix: every state at 1, 1.25, 1.5 and 2
node harness/render.mjs paused --scales 1.75 --themes light
node harness/render.mjs --gray               # grayscale text into renders-gray/ (what the window drew for most text)
```

Options: `--scales`, `--themes`, `--all-scales`, `--jobs N` (tabs in parallel, default 4),
`--no-layout`, `--no-tokens`. A full default run takes under a minute. `layout/tokens.json` is
rewritten on full runs (no state names given).

As delivered: 136 states (121 fixtures, some with several frozen times), 406 PNGs (every state at
scale 1 in dark and light — the `menu-*` states in dark only — and 24 states also at 1.25, 1.5 and
2), about 19 MB of renders and 12 MB of layout files. The full matrix (`--all-scales`) is 1,048 PNGs.

## Adding a state

Add it to `harness/make-states.mjs` (and run it), or write a JSON file in `states/` by hand —
`make-states.mjs` only rewrites the files it lists. A hand-written fixture can start from another
with `"extends": "<state>"` (deep merge). Then `node harness/render.mjs <name>`. Fixture fields:

| field | |
|---|---|
| `window` | `player`, `settings` or `info` |
| `description` | what the state shows |
| `drawer` | player: open the playlist drawer (window 587 tall) |
| `state`, `playlist` | player: the `player-state` snapshot and the playlist entries (`{ id, title, path | url, duration, missing }`) |
| `frame` | player: the analyzer frame: `{ shape: "music" | "full" | "quiet" | "empty" | { bars, peaks }, loader: 0-1, loaderTime: s }` or 77 raw bytes (`harness/frames.mjs`) |
| `settings` | `get_data` answers (`theme`, `visualizer-colors`, `autoplay`, `reconnect-minutes`, …) |
| `shortcuts`, `update`, `background`, `version`, `now` | answers of `get_shortcuts`, `update_status`, `get_background`, `app.getVersion`, and `Date.now()` in seconds |
| `status`, `info` | Stream information: `player_status` and `stream_info` answers |
| `errors` | `{ command: message }`: these commands reject with that message |
| `actions` | in order: `{ eval: js }` (page global scope: `select(3)`, `openDialog(0)`, `showDropIndex(2)` …), `{ emit, payload }`, `{ frame }`, `{ dragDrop: { type, position (CSS px), paths } }`, `{ force: { selector: ["hover" | "active" | "focus" | "focus-visible"] } }`, `{ press | dragTo | move | click | contextmenu: selector or { selector, fx, fy, dx, dy }, steps, modifiers }`, `{ release: true }`, `{ key: { key, code, modifiers } }`, `{ scroll: selector, top }`, `{ wait: ms }` |
| `time` / `times` | freeze animations at this time (ms, or `"50%"` of the longest infinite animation's iteration); `times` makes one state per time (`<name>-t0450`, `<name>-p50`) |
| `transitions` | also freeze transitions at `time` (else they are finished) |
| `scales`, `themes`, `accent`, `size` | what to render, the injected accent `{ base, light, dark }`, a window size override |

## Layout files

`layout/<window>/<state>.json`, one element per line:

- `elements`: every rendered element in document order: `name` (stable across states: the nearest
  id, then `>.firstClass[index]` steps, e.g. `#cartridges>.cartridge[2]>.cartridge-body>.cartridge-name`,
  `#playlist>.row[6]`), `tag`, `box` `[x, y, width, height]` in CSS px (getBoundingClientRect, so
  transforms included), `class` (state classes included), `attrs` (title = tooltip, aria state,
  disabled, values), `icon` (the SVG symbol), `text`, `truncated` (ellipsis active), `visible` (the
  part not clipped by a scroller or the window), `focused`, and `style`: the computed values that
  define the look — layout (display, flex/grid, gaps, padding, margin, position), font (family,
  size, weight, line-height, letter-spacing, numeric variant, transform, decoration) on elements
  with text, color, background (color, gradients, clip), borders (width / style / color per side,
  radii), box shadows (parsed: `{ inset, x, y, blur, spread, color }`), opacity, outline,
  transform, mask, fill, cursor, transitions and animations (durations in ms).
- `pseudo`: `::before` / `::after` boxes (the slider thumb dot, the balance tick, the selection
  pill, the drop line, the update dot, the toggle thumb) with their styles.
- `text`: every text run with its line rects, clip, color and font — what `compare.mjs` masks and
  compares.
- `animations`: every running animation and transition: target, keyframes, duration, delay,
  easing, iterations, direction, and the frozen `currentTime`.
- `hidden` (elements present but `display: none`), `scroll` (scroll positions), `menus` (native
  menus built), `calls` (commands the page sent).
- `scales`: for states also rendered at other scales, the boxes and text rects measured there
  (they differ: the analyzer's CSS size depends on the scale, see BEHAVIOUR.md 2.2.4).

Conventions: numbers are numbers (px without unit, ms for times, weights as numbers); colors are
`[r, g, b, a]` (r g b 0-255, possibly fractional for `color-mix()` results; a 0-1); four equal
sides collapse to one value, else `[top, right, bottom, left]` (radii: `[tl, tr, br, bl]`); a value
that differs between the themes is `{ "dark": …, "light": … }` (`null` = initial value in that
theme). A property is omitted when it has its initial value (`tokens.json` → `defaults`), and
inherited cursor / user-select are omitted when equal to the parent's. Computed alphas are
quantized by Chromium to 1/255 (`0.786` → `0.784`); `tokens.json` keeps the declared values too.

## Comparing a capture of the rebuilt UI

Capture the rebuilt window's client area at the same size and scale (e.g. 860 × 267 at 100 %), in
the same state, then:

```sh
node compare.mjs renders/player/stopped.dark@1.png capture.png
node compare.mjs renders/player/stopped.dark@1.5.png capture.png --json report.json
```

It reads the scale, theme and layout file from the reference's path (or `--scale`, `--theme`,
`--layout`). It reports:

- **pixels**: non-text pixels whose channels differ by more than `--tolerance` (default 16),
  counting a pixel only when no pixel in the 3 × 3 neighbourhood of the other image matches it
  (anti-aliasing and sub-pixel edge differences are forgiven; `--strict` turns that off, and then a
  1 px shift shows up);
- **regions**: the same per element (ids, islands, cartridges, buttons, rows, sliders, settings
  cards, dialog, …), worst first, with the mean difference;
- **text**: each text run is excluded from the pixel comparison (glyph rasterization never matches
  between toolkits) and compared by its ink instead: the ink's bounding box (within
  max(2, 1.5 × scale) device px on each side), coverage (70–140 % of the reference's ink pixels)
  and color (strongest ink pixels within 40 per channel), plus the background around it;
- a picture `capture.diff.png` next to the capture: reference | capture | difference (red =
  different, yellow = anti-aliasing-only, text boxes green when they match and red when not).

Exit code 0 when ≤ 0.1 % of the non-text pixels differ and every text run matches, 1 otherwise,
2 on errors (e.g. different sizes).

## Fidelity: headless Edge vs the WebView2 window

- Same engine and version (Edge and the WebView2 runtime are both 157.0.4341.46 on this PC), same browser flags (`--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection`),
  sRGB output (`--force-color-profile=srgb`).
- Fonts: the real system fonts. `tokens.json` → `fonts` shows what Chromium picked:
  Segoe UI Variable Display Semibold (clock, cartridge names), Segoe UI Variable Text (regular and
  semibold).
- Validated against a real screenshot of the running app (player, dark, drawer open, stopped,
  100 % scale; the user's own image, not stored here): the same state rendered by the harness and
  lined up with the screenshot's client area (the capture is at device scale 1: the window's title
  bar is 31 px and the client area is cropped 1 px left, 11 px right and 5 px bottom) gives
  **identical geometry**: every island, border, cartridge, button, slider, row and divider on the
  same pixel, every non-text pixel within 1/255 (3,632 pixels off by 1 level in the Instant and
  Info button fills, 0 more), the same accent (`#4cc2ff` sampled from the active cartridge), and all
  46 text runs at the same position and size. Text drawn with ClearType is **bit-identical**.
- Text antialiasing: headless Edge draws all text with ClearType (sub-pixel, colored fringes). The
  real WebView2 window drew **some** text with ClearType (clock, station, title, cartridge names
  and hosts, Playlist and Add file labels) and **some in grayscale** (the kbps / kHz line, MONO /
  STEREO, the volume and balance values, Add URL, every playlist row and the status line) — same
  glyph positions and weights, only the antialiasing mode differs. That split is decided by
  Chromium's compositor (layer/tile raster state), not by the CSS, so it is not a property of the
  design to reproduce; `compare.mjs` compares text by ink box, coverage and color, which both modes
  pass. A user with ClearType off sees grayscale everywhere.
- Scrollbars: the page styles its scrollbars (`::-webkit-scrollbar`), so they render the same;
  headless does not hide them.
- DPI: emulated with the device scale factor. Real windows at fractional scales have integer
  device sizes (334 px tall at 125 %, so the viewport is 267.2 CSS px); the emulation keeps the CSS
  viewport at 267 (≤ 1 device px difference at the bottom edge).
- Hover, pressed and focus states are forced through DevTools (`CSS.forcePseudoState`) on the
  listed elements, so several show at once; the real UI shows one hover at a time. Drags use real
  DevTools mouse events.
- Not rendered (native, outside the page): the window frame and title bar, native menus (their
  items are recorded), tooltips (texts recorded), the file picker, the window-height animation of
  the drawer, the caret.

## States

See the list in `states/` (each file has a `description`). Not covered, and why:

- Native context menus, the Settings combo menus, tooltips, the title bar: drawn by Windows, not
  the page (items and texts are in the layout files and BEHAVIOUR.md).
- The drawer opening as an animation: it is the window resizing (Rust); the page has two states,
  both rendered.
- The text caret and text selection highlight inside fields / Stream information values: the caret
  blinks with the system timer; selection colors are Chromium defaults (not styled by the page).
- `prefers-reduced-motion: reduce`: no visual state of its own (animations simply don't run:
  equivalent to the `-t0000`/`-p0` frames with the dialog at its end).
- Scrollbar hover/pressed: not styled by the page (same look).
- The Stream information window scrolled: its content fits 580 × 590.
