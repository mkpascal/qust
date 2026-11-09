# CLAUDE.md

Qust is a small Tauri 2 desktop player for HTTP audio streams (Icecast / SHOUTcast v2). The user saves up to six streams on "cartridge" cards, plus a playlist of local files and URLs in a drawer under the player, and plays one at a time through `<audio>` elements in the system webview (WebView2 / WKWebView, so the UI is web, not native), with a pixel spectrum analyzer. The UI imitates Windows 11 (Fluent): a solid background (no Mica: it tinted the window with the wallpaper while active), system light/dark theme, the user's accent color, and native menus. Targets Windows and macOS. It was an Electron + Bootstrap app until November 2026.

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
  - `index.html` + `js/script.js`: the player window (label `main`), with the playlist drawer.
  - `settings.html` + `js/settings.js`: the Settings window (label `settings`).
  - `info.html` + `js/info.js`: the Stream information window (label `info`).
  - `js/bridge.js`: glue to Rust. It applies `window.__QUST__` (accent colors) and defines `saveData` / `loadData` / `deleteData`, `proxied(url)`, `fileUrl(path)`, `streamDetails(url)` / `icecastTitle(url)` and `shortcutKeys()`. It owns right-click: it always blocks the browser menu and shows native edit menus on inputs and on `dd` / `.copyable` values. It also blocks browser keys (reload, find, print).
  - `js/visualizer.js`: the `Spectrum` class (audio graph, analysis, pixel drawing, loading animation), exposed as `window.Spectrum`. Keep this file free of comments.
  - `css/style.css`: all styles, with Fluent tokens as CSS variables (dark by default, light under `prefers-color-scheme: light`).
- `src-tauri/src/`
  - `main.rs`: builds windows through `build_window` (owner/centering for the panels, the injected `window.__QUST__`), plus the `open_panel` and `set_theme` commands.
  - `store.rs`: settings in `settings.json` in the app config dir (`%APPDATA%\com.pascalmuangkhot.qust`), through `get_data` / `set_data` / `delete_data`.
  - `proxy.rs`: the local stream proxy, the local file server and the `stream_info` / `icecast_title` commands (see below).
  - `playlist.rs`: the playlist drawer (window height), the Add file picker, and expanding dropped paths.
  - `shortcuts.rs`: the customizable global shortcuts.
  - `updater.rs`: the auto-updater.
  - `background.rs`: the background helper (tray, start with Windows, opening the player on demand).
- `src-tauri/icons/`: generated with `npx tauri icon build/icons/icon.png`. Delete the `android/`, `ios/`, `Square*` and `StoreLogo` outputs afterwards. `build/icons/icon.png` is the source, a rounded square with transparent corners, and is also used by the README.
- `assets/screenshot.png`: README screenshot of the player window.
- `TODO.md`: planned settings and features.

## How it works

**Windows.** The player is 860×267 and not resizable. The playlist drawer adds 320 px (587 in all). Settings (520×560) and Stream information (580×590) are separate native windows opened with `invoke('open_panel', { name })`. They are owned by the player, so they stay above it with no taskbar button, and they open centered over it. Esc closes them. All webviews must get identical `additional_browser_args`, because WebView2 shares one browser process. That is why every window goes through `build_window`. Creating windows from a command must be `async` (sync deadlocks on Windows). Capabilities (`capabilities/default.json`) list all three window labels.

**`window.__QUST__`.** Injected by `main.rs`: `proxy` (base URL), `files` (the file server's base URL), `accent` (`base` / `light` / `dark` from the Windows `AccentPalette` registry value; the light shade is used in dark mode and vice versa) and `debug`. `app.withGlobalTauri` is on, so pages use `window.__TAURI__.*` directly.

**Theme.** `theme` in the store is `system` / `light` / `dark`. Windows are built with it, and the `set_theme` command applies it to every window. WebView2 then switches `prefers-color-scheme`, so CSS needs no theme class. Each webview's background color follows the theme too (`background()` in `main.rs`), so the area the drawer uncovers while the window grows isn't white.

**Stream proxy.** The webview enforces CORS, and the visualizer needs CORS on the audio. `proxy.rs` listens on `127.0.0.1:<random port>/<random token>`:
- `?url=<encoded>`: streams the URL back with `Access-Control-Allow-Origin: *`. It requests ICY metadata, strips it from the audio, and records the title. For plain HTTP files it also passes on `Content-Length`, `Accept-Ranges` and range requests (206), so the webview knows the duration and can seek while it downloads. `Range: bytes=0-` isn't forwarded, as some stream servers don't expect ranges.
- A second listener on its own port (same token), `?path=<encoded>`: serves a local audio file where it is (audio extensions only), with ranges and CORS. It has its own port so files never wait behind six instant-switch streams. It records the file's details for `stream_info`, keyed by the path: the MP3/AAC frame (after any ID3v2 tag), FLAC STREAMINFO or WAV header.
- `stream_info` (command, `streamDetails(url)`): the stream's details: title, name, genre, description, website, server, content type, format, bitrate, measured rate, sample rate, channels, metadata interval and bytes, bytes received, title changes, reconnects, connected seconds. `icecast_title` (`icecastTitle(url)`) reads the title from the server's `status-json.xsl`. Both use IPC, not HTTP: the webview allows 6 connections per host, and with instant switch six streams fill them, so a request to the proxy would wait forever.

Format, sample rate and channels come from the headers (`icy-br`, `ice-audio-info` or `icy-audio-info`) or, failing that, from the first MP3/ADTS frame header. An ADTS AAC-LC header at 24 kHz or less is taken as HE-AAC (implicit SBR): the header has the core rate, and playback is at twice that. A wrong token gets 404, and an upstream error gets 502. Every stream URL in the page must go through `proxied()`, and other remote requests through Rust. SHOUTcast v1 `ICY 200 OK` responses aren't valid HTTP for `ureq` and fail.

**Persistence.** Store keys:
- `player1-title` / `player1-url` through `player6-*`: the cartridges
- `last-selected-player` / `-title` / `-url`: the last cartridge played
- `last-selected-player` is `playlist` when a playlist entry was last selected, and `playlist-index` is its position
- `playlist`: an array of `{ title, path, duration }` (local file) or `{ title, url, duration }`, and `playlist-open` (the drawer)
- `volume`, `balance`
- `theme`, `visualizer-colors` (`classic` / `white` / `accent` / `off`; clicking the analyzer display cycles them), `autoplay`, `background`, `instant-switch`, `reconnect-minutes` (default 5, 0 = never give up)
- `shortcut-play`, `shortcut-stop`, `shortcut-toggle` (empty string = no shortcut)

The first run imports the Electron version's `<config dir>/qust/config.json`. `loadData` turns Rust's `null` into `undefined`.

**Player state.** `script.js` has `state`: `stopped` / `connecting` / `playing` / `buffering` / `paused`. `setState()` re-renders the buttons, cartridges, title, clock and loader, and broadcasts `player-status`. Stop disconnects every stream. A pause longer than `LIVE_RESUME_SECONDS` reloads the stream on resume, rejoining the live point. While playing, the page polls `stream_info` every 2 s. The title comes from ICY, then the Icecast `status-json.xsl` fallback, then the stream or cartridge name. Cartridges are rendered from the `cartridges` array (`is-empty` / `is-active` / `is-playing` classes).

**Streams and instant switch.** What plays is `active()`: the cartridge `cartridges[activeSlot]` or the playlist entry `activeEntry`, never both. There's no fixed `<audio>`: `streams` maps each source to its own `Audio` element by `keyOf()` (the URL, or `file:` and the path), through `streamFor` / `unload`, with `current()` the active one. Keying by URL means reordering cartridges doesn't disturb them. `mediaUrl()` gives `proxied(url)` or `fileUrl(path)`.
- **Instant switch** (the ⚡ button, `instant-switch` in the store): every loaded cartridge streams in the background at level 0, even while stopped, so starting or switching is immediate. Only the 6 cartridges preload, never playlist entries, and at most six streams go through the proxy at once (`PROXY_CONNECTIONS`).
- **`syncStreams()`:** keeps exactly the wanted streams connected (the active one unless stopped, plus every cartridge in instant mode) and sets the levels (active 1, others 0). Call it after anything that changes cartridges, the active source or the state.
- **`play()`:** when the target is already live (`isLive`), it switches immediately with no reconnect. `clockStart` resets the clock for the new station.
- **`pause()`:** pauses only the active stream. Resuming within `LIVE_RESUME_SECONDS` applies only to the same source (`pausedKey`). A file (`finite()`: the element has a finite duration) always resumes where it paused.
- **Files** (local, or online with a length): running out of buffer waits (`buffering`) instead of reconnecting, `ended` plays the next entry, and a reconnect picks up at the same position (`filePosition` covers `restartAudio()`).

**Reconnecting.** `lost()` runs when the active stream drops: `waiting` while playing (buffer ran out), `error` or `ended` after it played, or the watchdog. The first try is immediate, then one every second (counted from the previous try's start) for 15 tries, then every 15 s, until `reconnect-minutes` runs out ("Connection lost"). Each try (`reconnect()`) gives the URL a fresh element, and one that isn't playing after 12 s counts as failed. Playing for 10 s ends the sequence. A stream that never played isn't retried ("Couldn't play this stream"). The clock carries over (`clockBase`), and so does the title until the new connection sends one. Background streams that drop are re-synced after 15 s.

**Playlist.** The Playlist button (`setDrawer`) calls `set_playlist_open`, which animates the window height in Rust and saves `playlist-open`. `build_player` opens the window at the saved height. The page never scrolls: `.app` is fixed at `--player-height`, and the drawer below it is cut off by the window (and `inert` while closed). The loaded cartridges are pinned at the top of the list: they play and edit there, but move and eject only above. Below them, entries come from:
- dropping on the window: Tauri's `onDragDropEvent` gives the paths (HTML drag and drop doesn't reach the page), and `expand_paths` turns files, folders (natural name order, recursive) and `.m3u` / `.pls` lists into entries. Lists inside folders are skipped.
- Add file: `pick_files` (tauri-plugin-dialog, used from Rust only), with the same expansion.
- Add URL and Edit… (right-click): the cartridge dialog, with `dialogSlot` and `dialogEntry` both null (add) or `dialogEntry` set (edit).

`renderPlaylist()` rebuilds the rows, `renderPlaylistState()` sets their classes, and `rowOf` maps items to rows. Selection supports Ctrl and Shift, dragging reorders the selection (pointer events), Delete removes, and Enter or double-click plays. Local files' durations are probed one at a time (`probeDurations`). An entry that fails is marked `missing` and skipped when played. The playlist plays once to the end. For a playlist file with a duration, the volume and balance card shows a seek bar over volume and balance side by side (`renderSeek`, `.sliders.is-seekable`).

**Watchdog.** `watch()` runs every second. It resumes a suspended `AudioContext` (`spectrum.wake()`). If the active stream's `currentTime` hasn't moved for 3 s while `playing`, it calls `lost()`. If the context's clock froze too, it first calls `restartAudio()`: a new context (`spectrum.reset()`) and new elements, since an element feeds only one context. When the context stops rendering (device change, sleep), the element's clock, the bars and the sound all stop with no event, so this is the only way to catch it.

**Visualizer.** Once its 44.1 kHz `AudioContext` runs, `Spectrum` has a shared input: input → gain (volume) → stereo panner (balance) → speakers, and input → splitter → two analysers. So the bars ignore the volume. Each stream joins with `attach(element)`, which routes element → its own gain → input. `setLevel(element, 0 | 1)` fades a stream in or out over ~15 ms, and `detach` removes it. Before the context runs, or if Web Audio fails, levels fall back to `element.volume`. The analysis is an MDCT that builds 19 bars with falling peaks, at a fixed 60 updates/s. The canvas is a 75×16 pixel grid (3 px bars, 1 px gaps, 16 rows with one color per row). It's shown at 1:1, scaled only by whole numbers on high-DPI screens with `image-rendering: pixelated`. `setLoading(true)` (connecting / buffering) shows a row-of-blocks pulse that crossfades with the bars over 0.5 s.

**Events between windows.**
- `settings-changed` `{ key, value }`: Settings → player. Includes `key: "shortcuts"` from Rust.
- `player-status`: player → info, every second and on state changes.
- `player-status-request`: info → player, sent when the info window opens.
- `play-action` / `stop-action` / `toggle-action`: global shortcuts → player.

**Background helper.** With `background` on ("Background helper" in Settings), `tauri-plugin-autostart` adds a Windows startup entry that runs `qust.exe --background`.
- **At sign-in:** that launch creates only the tray icon (`background::create_tray`), with no window.
- **Closing the player:** the window is destroyed, and the `RunEvent::ExitRequested` handler keeps the process alive (`code: None` means "last window closed"). The tray's "Quit Qust" exits with a code, so it gets through.
- **Shortcuts and tray actions:** they go through `background::player_action`. With no player window, Play and Play / Stop / Start open it (`show_player(app, true)` → `PlayRequest`, which the page reads with `take_play_request` once loaded and then plays the last cartridge). Stop does nothing.
- **Single instance:** `tauri-plugin-single-instance` (registered first) sends a second launch to the running process. A plain launch shows the player, and `qust.exe --toggle` acts like Play / Stop / Start. A first launch with `--toggle` opens and plays.
- **Memory:** with the window closed, the idle process holds no webview.

**Updates.** `tauri-plugin-updater` reads `https://github.com/mkpascal/qust/releases/latest/download/latest.json`, and the public key is in `tauri.conf.json` > `plugins.updater`. `updater.rs` checks GitHub, downloads a newer version in the background (status `ready`), and installs it only when the user clicks "Restart and update" in Settings. A dot on the cog shows that an update is waiting. The NSIS installer (passive mode) closes Qust and reopens it. When "Check automatically" (`auto-update`, on by default) is on, it checks 30 s after launch and then whenever `update-last-check` is 8 hours old. Only the installed version updates: `installed()` is false for debug builds and for an exe without `uninstall.exe` next to it (the portable copy). Status reaches the pages as `update-status` events.

**Global shortcuts.** `shortcuts.rs` keeps the bindings as accelerators built from `KeyboardEvent.code` (Play `Ctrl+Alt+Insert`, Stop `Ctrl+Alt+KeyH`, Play / Stop unassigned). Play / Stop plays when stopped or paused and stops otherwise. It exists so a single Stream Deck "Hotkey" button stays in sync, which a "Hotkey Switch" with separate Play and Stop keys does not. Backspace while recording resets a shortcut to its default, which clears Play / Stop. While Settings records one, it calls `pause_shortcuts(true)`, because a registered shortcut never reaches the page. `set_shortcut` refuses shortcuts another app owns or that are already used by another action. Closing the Settings window always resumes them. A shortcut owned by another app at startup is kept but marked unregistered, and Settings shows it crossed out.

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

## Releases and CI

`.github/workflows/build.yml` builds Windows (NSIS installer) and macOS (universal `.dmg`) with `tauri-apps/tauri-action` on pushes to `main`, pull requests and manual runs, and keeps the installers as workflow artifacts.

To release, bump `version` in `package.json` (and in `src-tauri/Cargo.toml` to match) and push to `main`. When no release exists for that version, the `release` job creates a draft "Qust vX.Y.Z". Both builds then add `src-tauri/tauri.release.conf.json` (`createUpdaterArtifacts`), sign the update files and upload them with `latest.json`. The `publish` job makes the release public and latest once both builds succeed, so installed copies only ever see a complete release.

Signing needs the repository secrets `TAURI_SIGNING_PRIVATE_KEY` and `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`. The key pair lives in `C:\Users\p\.tauri\` (`qust.key`, `qust.key.password`, `qust.key.pub`). If the private key is lost, installed copies can't verify new updates, so it must stay backed up. Local `npm run build` doesn't sign and needs no key.

## License

GPL-3.0-or-later.
