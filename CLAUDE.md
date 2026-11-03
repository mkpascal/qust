# CLAUDE.md

Qust is a small Tauri 2 desktop player for HTTP audio streams (Icecast-style). The user saves up to six streams on "cartridge" cards, loads one into a single `<audio>` element, and watches a spectrum visualizer. Targets Windows and macOS. It was an Electron app until November 2026.

## Commands

```sh
npm install
npm start          # tauri dev: runs the app from src/ (no dev server; edits need a window reload)
npm run build      # tauri build: release app + installers in src-tauri/target/release/bundle/
cd src-tauri && cargo build   # just compile the Rust side
```

You need Rust (stable, via rustup), plus the MSVC build tools on Windows (WebView2 ships with Windows 11) or the Xcode command line tools on macOS. There are no tests, linter or JS bundler. The front end is plain JS, HTML and CSS served as-is from `src/`.

## Layout

- `src/`: the front end (Tauri's `frontendDist`).
  - `index.html`: the whole UI (status card, visualizer canvas, transport and volume controls, six player cards, the hidden `#customPrompt` dialog). Its inline `<script>` is the bridge to Rust: `saveData` / `deleteData` / `loadData` (invoke commands), `proxied(url)`, and listeners for the shortcut events.
  - `js/script.js`: renderer logic (playback, card handling, persistence, volume slider, stream metadata).
  - `js/visualizer.js`: the `Spectrum` class (audio routing, spectrum analysis, bar drawing), exposed as `window.Spectrum`. Loaded before `script.js`. Keep this file free of comments.
  - `css/style.css`: app styles. The other files in `css/`, `js/bootstrap*` and `lib/bootstrap-icons-1.11.2/` are vendored Bootstrap 5 and Bootstrap Icons. Don't edit them.
- `src-tauri/`: the Rust side.
  - `src/main.rs`: builds the window in `setup()` (964x283 content, not resizable), registers the global shortcuts, and injects `window.__QUST_PROXY__`.
  - `src/store.rs`: settings in `settings.json` in the app config dir (`%APPDATA%\com.pascalmuangkhot.qust`, `~/Library/Application Support/com.pascalmuangkhot.qust`), through the `get_data` / `set_data` / `delete_data` commands.
  - `src/proxy.rs`: the local stream proxy (see below).
  - `tauri.conf.json`: the version is read from `../package.json`, so bump it there only (and in `Cargo.toml` if you want them to match). `windows` is empty on purpose because the window is created in code.
  - `icons/`: generated with `npx tauri icon build/icons/icon.png`. Regenerate them when the icon changes.
- `build/icons/icon.png`: the source icon. `assets/screenshot.png` is only for the README.

## How it works

**Bridge.** `app.withGlobalTauri` is on, so the page uses `window.__TAURI__.core.invoke` and `window.__TAURI__.event.listen` with no bundler. `loadData` turns Rust's `null` into `undefined`, because `script.js` tests for `typeof data !== 'undefined'`. App commands need no capability entry. `capabilities/default.json` only grants `core:default` (events). The CSP is off (`csp: null`) because the page loads Google Fonts and talks to the local proxy.

**Stream proxy.** The webview enforces CORS, unlike the old Electron window, which ran with `webSecurity: false`. The visualizer needs CORS on the audio, and Icecast status pages rarely send it. So `proxy.rs` listens on `127.0.0.1:<random port>/<random token>?url=<encoded url>`, fetches the URL with `ureq`, and streams it back with `Access-Control-Allow-Origin: *`. Wrong token → 404. Upstream error → 502. Every stream URL and the status fetch go through `proxied()` in the page. A new place that sets `audio.src` or fetches a remote URL must use it too. Being on localhost also keeps http streams from being treated as mixed content. It ignores `Range` and always answers 200 with no length, which is fine for live streams. SHOUTcast v1 servers that answer `ICY 200 OK` aren't valid HTTP for `ureq` and fail with 502.

**Persistence.** Keys:
- `player1-title` / `player1-url` through `player6-*`: saved cartridges
- `last-selected-player`, `last-selected-title`, `last-selected-url`: restored at startup

On the first run (no `settings.json` yet), `Store::load` copies the Electron version's electron-store file (`<config dir>/qust/config.json`), so existing users keep their cartridges.

**Card state** is encoded only in Bootstrap classes on each `.ppp.playerN` card:
- `text-bg-secondary`: empty slot (clicking the body does nothing)
- `text-bg-warning`: loaded
- `text-bg-success`: the active stream

The code finds the slot by searching the card's `className` for the `playerN` token, so keep that class on the card. Clicking a card body plays that slot. Clicking the eject footer opens `#customPrompt` to add or empty it. The number of slots (6) is hard-coded in both `index.html` and `script.js`.

**Playback and visualizer.** `script.js` creates one `Spectrum(canvas, audio)`. `start()` calls `spectrum.start()` and then `audio.load()` / `play()`. `stop()` calls `spectrum.stop()`, which clears the canvas. On its first start, `Spectrum` creates a 44.1 kHz `AudioContext` and routes the element only once the context is running: source → `GainNode` → speakers, and source → channel splitter → one `AnalyserNode` per channel (fftSize 8192, time-domain data only). The analysis runs its own MDCT on 1152 samples and builds 19 bars with falling peaks, updated at a fixed 60 per second whatever the screen's refresh rate. Double-clicking the canvas switches between green-to-red and white bars. The `<audio>` element has `crossorigin="anonymous"`, which the analyser needs. Its hard-coded default `src` is replaced as soon as a stream is restored or selected.

**Volume.** A custom slider (`#volumeSlider`, `role="slider"`) supports dragging, clicking, the mouse wheel and arrow / PageUp / PageDown / Home / End keys. The value lives in the `--volume` CSS variable, which positions the fill and thumb. `spectrum.setVolume()` sets the `GainNode` once the audio is routed (keeping the element at 1) and the element's own volume before that. The bars therefore always show the full signal, whatever the volume. The volume isn't saved between launches and starts at 50%.

**Now-playing title.** Every 15 s while playing, `getMetaDataFromStream()` fetches `/status-json.xsl` on the stream's host (Icecast's JSON status, through the proxy) and matches the source by its `listenurl` path. If the URL ends in `/`, it uses the first source. Servers without that page (including stream.c9.fr) just show "N/A".

**Global shortcuts.** `Ctrl+Alt+Insert` plays and `Ctrl+Alt+H` stops (`tauri-plugin-global-shortcut`, registered in Rust, emitted as `play-action` / `stop-action`). If another program already owns a combination, registration fails, an error is printed to stderr, and the app runs without that shortcut. On Windows the webview gets `--autoplay-policy=no-user-gesture-required`, so a shortcut can start playback before any click in the window.

## Debugging

Release builds have no DevTools. In a debug build (`npm start`), right-click → Inspect works. To drive the app from a script on Windows, start it with `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=9222` and connect over the Chrome DevTools Protocol (`http://127.0.0.1:9222/json`). That variable replaces the browser args set in `main.rs`.

## Gotchas

- An element can be routed into an `AudioContext` only once, and once routed it plays only while that context runs. `Spectrum` guards this with `wired` / `failed`. If routing fails, audio still plays through the element, just without bars.
- Text (stream titles, metadata) is written with `innerHTML`.
- macOS builds are unsigned and not notarized, so Gatekeeper blocks the first launch (right-click → Open, or `xattr -cr Qust.app`).

## CI

`.github/workflows/build.yml` builds on Windows (NSIS installer) and macOS (universal `.dmg`) with `tauri-apps/tauri-action`. It runs on pushes to `main`, pull requests and manual runs, and keeps the installers as workflow artifacts. Pushing a `v*` tag also creates one draft GitHub release that both platforms upload to. Publish it by hand.

## License

GPL-3.0-or-later.
