# CLAUDE.md

Qust is a small Tauri 2 desktop player for HTTP audio streams (Icecast / SHOUTcast v2). The user saves up to six streams on "cartridge" cards and plays one at a time through a single `<audio>` element, with a pixel spectrum analyzer. The UI imitates Windows 11 (Fluent): Mica, system light/dark theme, the user's accent color, and native menus. Targets Windows and macOS. It was an Electron + Bootstrap app until November 2026.

## Commands

```sh
npm install
npm start          # tauri dev: runs the app from src/ (no dev server; edits need a rebuild or window reload)
npm run build      # tauri build: release app + installer, then copied to build/bin/ (git-ignored) by scripts/copy-bin.mjs
cd src-tauri && cargo build   # debug build at src-tauri/target/debug/qust.exe (front end embedded at compile time)
```

You need Rust (stable, via rustup), plus the MSVC build tools on Windows (WebView2 ships with Windows 11) or the Xcode command line tools on macOS. There are no tests, linter or JS bundler. The front end is plain JS, HTML and CSS served as-is from `src/`. A running `qust.exe` locks the file, so close it before `cargo build`.

## Layout

- `src/` (Tauri's `frontendDist`). Each window is its own page. Every page loads `js/bridge.js` first.
  - `index.html` + `js/script.js`: the player window (label `main`).
  - `settings.html` + `js/settings.js`: the Settings window (label `settings`).
  - `info.html` + `js/info.js`: the Stream information window (label `info`).
  - `js/bridge.js`: glue to Rust. It applies `window.__QUST__` (Mica class, accent colors) and defines `saveData` / `loadData` / `deleteData`, `proxied(url)` / `proxiedInfo(url)` and `shortcutKeys()`. It owns right-click: it always blocks the browser menu and shows native edit menus on inputs and on `dd` / `.copyable` values. It also blocks browser keys (reload, find, print).
  - `js/visualizer.js`: the `Spectrum` class (audio graph, analysis, pixel drawing, loading animation), exposed as `window.Spectrum`. Keep this file free of comments.
  - `css/style.css`: all styles, with Fluent tokens as CSS variables (dark by default, light under `prefers-color-scheme: light`).
- `src-tauri/src/`
  - `main.rs`: builds windows through `build_window` (Mica on Windows 11, owner/centering for the panels, the injected `window.__QUST__`), plus the `open_panel` and `set_theme` commands.
  - `store.rs`: settings in `settings.json` in the app config dir (`%APPDATA%\com.pascalmuangkhot.qust`), through `get_data` / `set_data` / `delete_data`.
  - `proxy.rs`: the local stream proxy (see below).
  - `shortcuts.rs`: the customizable global shortcuts.
- `src-tauri/icons/`: generated with `npx tauri icon build/icons/icon.png`. Delete the `android/`, `ios/`, `Square*` and `StoreLogo` outputs afterwards. `build/icons/icon.png` is the source, a rounded square with transparent corners, and is also used by the README.
- `assets/screenshot.png`: README screenshot of the player window.
- `TODO.md`: planned settings and features.

## How it works

**Windows.** The player is 860×267 and not resizable. Settings (520×560) and Stream information (580×590) are separate native windows opened with `invoke('open_panel', { name })`. They are owned by the player, so they stay above it with no taskbar button, and they open centered over it. Esc closes them. All webviews must get identical `additional_browser_args`, because WebView2 shares one browser process. That is why every window goes through `build_window`. Creating windows from a command must be `async` (sync deadlocks on Windows). Capabilities (`capabilities/default.json`) list all three window labels.

**`window.__QUST__`.** Injected by `main.rs`: `proxy` (base URL), `mica`, `accent` (`base` / `light` / `dark` from the Windows `AccentPalette` registry value; the light shade is used in dark mode and vice versa) and `debug`. `app.withGlobalTauri` is on, so pages use `window.__TAURI__.*` directly.

**Theme.** `theme` in the store is `system` / `light` / `dark`. Windows are built with it, and the `set_theme` command applies it to every window. WebView2 then switches `prefers-color-scheme`, so CSS needs no theme class.

**Stream proxy.** The webview enforces CORS, and the visualizer needs CORS on the audio. `proxy.rs` listens on `127.0.0.1:<random port>/<random token>`:
- `?url=<encoded>`: streams the URL back with `Access-Control-Allow-Origin: *`. It requests ICY metadata, strips it from the audio, and records the title.
- `/info?url=<encoded>`: JSON with the stream's details: title, name, genre, description, website, server, content type, format, bitrate, measured rate, sample rate, channels, metadata interval and bytes, bytes received, title changes, reconnects, connected seconds.

Format, sample rate and channels come from the headers (`icy-br`, `ice-audio-info`) or, failing that, from the first MP3/ADTS frame header. A wrong token gets 404, and an upstream error gets 502. Every stream URL and remote fetch in the page must go through `proxied()`. SHOUTcast v1 `ICY 200 OK` responses aren't valid HTTP for `ureq` and fail.

**Persistence.** Store keys:
- `player1-title` / `player1-url` through `player6-*`: the cartridges
- `last-selected-player` / `-title` / `-url`: the last cartridge played
- `volume`, `balance`
- `theme`, `visualizer-colors` (`classic` / `white` / `accent`), `autoplay`
- `shortcut-play`, `shortcut-stop`

The first run imports the Electron version's `<config dir>/qust/config.json`. `loadData` turns Rust's `null` into `undefined`.

**Player state.** `script.js` has `state`: `stopped` / `connecting` / `playing` / `buffering` / `paused`. `setState()` re-renders the buttons, cartridges, title, clock and loader, and broadcasts `player-status`. Stop removes the `src`, which closes the connection. A pause longer than `LIVE_RESUME_SECONDS` reloads the stream on resume, rejoining the live point. While playing, the page polls `/info` every 2 s. The title comes from ICY, then the Icecast `status-json.xsl` fallback, then the stream or cartridge name. Cartridges are rendered from the `cartridges` array (`is-empty` / `is-active` / `is-playing` classes).

**Visualizer.** `Spectrum` routes the element once its 44.1 kHz `AudioContext` runs: source → gain (volume) → stereo panner (balance) → speakers, and source → splitter → two analysers. So the bars ignore the volume. The analysis is an MDCT that builds 19 bars with falling peaks, at a fixed 60 updates/s. The canvas is a 75×16 pixel grid (3 px bars, 1 px gaps, 16 rows with one color per row). It's shown at 1:1, scaled only by whole numbers on high-DPI screens with `image-rendering: pixelated`. `setLoading(true)` (connecting / buffering) shows a row-of-blocks pulse that crossfades with the bars over 0.5 s.

**Events between windows.**
- `settings-changed` `{ key, value }`: Settings → player. Includes `key: "shortcuts"` from Rust.
- `player-status`: player → info, every second and on state changes.
- `player-status-request`: info → player, sent when the info window opens.
- `play-action` / `stop-action`: global shortcuts → player.

**Global shortcuts.** `shortcuts.rs` keeps the bindings as accelerators built from `KeyboardEvent.code` (default `Ctrl+Alt+Insert` / `Ctrl+Alt+KeyH`). While Settings records one, it calls `pause_shortcuts(true)`, because a registered shortcut never reaches the page. `set_shortcut` refuses shortcuts another app owns or that are already used by the other action. Closing the Settings window always resumes them. A shortcut owned by another app at startup is kept but marked unregistered, and Settings shows it crossed out.

## Debugging and testing

Release builds have no DevTools. To drive the app from a script, start it with `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=9222` and use the Chrome DevTools Protocol (`http://127.0.0.1:9222/json`). That variable replaces the browser args from `main.rs`, so add `--autoplay-policy=no-user-gesture-required` to it too.
- Mute tests with `spectrum.setVolume(0)`. It sets only the gain, not the saved volume.
- Capture windows with `PrintWindow` (`PW_RENDERFULLCONTENT`), not screen grabs, and don't send OS keystrokes. The user works on the same desktop.
- Native menus open at the real mouse cursor. To test them, stub `window.__TAURI__.menu.Menu.new` in the page.
- Tests read and write the real `settings.json`. Back it up, but don't restore it over changes the user made meanwhile.

## Gotchas

- An element can be routed into an `AudioContext` only once. `Spectrum` guards this with `wired` / `failed`. If routing fails, audio plays without bars.
- Native menu `accelerator` strings must parse, or `Menu.new` throws. `script.js` retries without them.
- macOS builds are unsigned and not notarized, so the first launch needs right-click → Open.

## CI

`.github/workflows/build.yml` builds Windows (NSIS installer) and macOS (universal `.dmg`) with `tauri-apps/tauri-action` on pushes to `main`, pull requests and manual runs, and keeps the installers as workflow artifacts. Pushing a `v*` tag also creates one draft GitHub release that both platforms upload to.

## License

GPL-3.0-or-later.
