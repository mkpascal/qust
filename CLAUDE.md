# CLAUDE.md

Qust is a small Tauri 2 desktop player for HTTP audio streams (Icecast / SHOUTcast v2). The user saves up to six streams on "cartridge" cards, plus a playlist of local files and URLs in a drawer under the player, and plays one at a time, with a pixel spectrum analyzer. Rust does all the audio and all the player logic: its own engine connects, decodes (Symphonia), mixes and plays to the output device, and computes the analyzer. The pages in the system webview (WebView2 / WKWebView, so the UI is web, not native) are only a view: they render what Rust sends and send back what the user does. The UI imitates Windows 11 (Fluent): a solid background (no Mica: it tinted the window with the wallpaper while active), system light/dark theme, the user's accent color, and native menus. Targets Windows and macOS. It was an Electron + Bootstrap app until November 2026, and the webview played the audio through `<audio>` elements until it moved to Rust.

## Commands

```sh
npm install
npm start          # tauri dev: runs the app from src/ (no dev server; edits need a rebuild or window reload)
npm run build      # tauri build: release app + installer, then copied to build/bin/ (git-ignored) by scripts/copy-bin.mjs
cd src-tauri && cargo build             # debug build at src-tauri/target/debug/qust.exe (front end embedded at compile time)
cd src-tauri && cargo test --workspace  # all tests: headless, on a silent output, no network
```

You need Rust (stable, via rustup), plus the MSVC build tools on Windows (WebView2 ships with Windows 11) or the Xcode command line tools on macOS, and Node.js (the Tauri CLI comes from npm). There is no linter, JS bundler or JS test. The front end is plain JS, HTML and CSS served as-is from `src/`. A running `qust.exe` locks the file, so close it before `cargo build`.

**Tests.** `cargo test --workspace` runs the crates' unit and integration tests and the player's end-to-end tests (`src/player/tests.rs`). They need no sound device and no network: the engine runs on `OutputKind::Null` (a thread that consumes the mix in real time), files are WAVs written to the temp dir, streams come from a small HTTP server inside the test on 127.0.0.1 (live WAV with ICY titles, gaps, drops, refused connections, ranged files, a `status-json.xsl`), and the store is a temporary file. They play in real time, so they take a while. On Windows the Media Foundation tests in `qust-codecs` need the system AAC decoder. One timing test is ignored by default: `cargo test -p qust-spectrum --release --test cost -- --ignored --nocapture`. CI doesn't run the tests.

**Examples**, for checking real streams and files without the app (with a URL they use the network):

```sh
cargo run -p qust-audio --example probe -- [--device] [--quiet] SECONDS ACTION...
cargo run -p qust-codecs --example decode -- <file or URL> [seconds] [--repeat N] [--raw out.f32]
```

- `probe` drives the engine: on the Null output, or with `--device` on the real device at volume 0. It prints events, every loaded key's status each second, the tap, and `info()` at the end. Each action is `TIME=VERB,KEY[,ARG]`: `0=load,a,<URL or path>` (`URL@START` for a start position), `0=level,a,1`, `3=pause,a`, `5=resume,a`, `6=seek,a,30`, `9=unload,a`, `9=unloadall,-`, `2=volume,-,0.5`.
- `decode` decodes through `qust_codecs::registry()` into memory (never to a device) and reports the codec, output rate and channels, frames decoded against what the packets account for, peak and RMS, and the share of energy above 11 kHz (near zero for HE-AAC decoded as plain LC). `--repeat N` decodes N times with fresh decoders, to check memory stays flat. `--raw` writes interleaved f32 to compare against a reference decoder.

## Layout

- `src/` (Tauri's `frontendDist`). Each window is its own page. Every page loads `js/bridge.js` first.
  - `index.html` + `js/script.js`: the player window (label `main`), with the playlist drawer. A view: see "Commands" and "Events" below.
  - `settings.html` + `js/settings.js`: the Settings window (label `settings`).
  - `info.html` + `js/info.js`: the Stream information window (label `info`).
  - `js/bridge.js`: glue to Rust. It applies `window.__QUST__` (accent colors) and defines `saveData` / `loadData` / `deleteData`, `streamDetails(url)`, `shortcutKeys()` and `popupMenu(menu, x, y)`. It owns right-click: it always blocks the browser menu and shows native edit menus on inputs and on `dd` / `.copyable` values. It also blocks browser keys (reload, find, print).
  - `js/visualizer.js`: the `Spectrum` painter (draws the frames Rust sends on the canvas, with the chosen palette), exposed as `window.Spectrum`. Keep this file free of comments.
  - `css/style.css`: all styles, with Fluent tokens as CSS variables (dark by default, light under `prefers-color-scheme: light`).
- `src-tauri/`: a Cargo workspace. The app crate `qust` is at the root (`Cargo.toml`, `src/`). The three crates under `crates/` have no Tauri dependency.
  - `src/main.rs`: registers the plugins and commands, starts the player, and builds windows through `build_window` (owner/centering for the panels, the injected `window.__QUST__`), plus the `open_panel` and `set_theme` commands.
  - `src/store.rs`: settings in `settings.json` in the app config dir (`%APPDATA%\com.pascalmuangkhot.qust`), through `get_data` / `set_data` / `delete_data`. One `Store` is shared by the commands and the player thread (clones are the same store).
  - `src/player/`: the player. `mod.rs` (the `Player` handle, its thread and the `Host` trait), `machine.rs` (`Core`: the state machine), `model.rs` (cartridges, entries, their default names, `URL_ERROR`), `commands.rs` (the Tauri commands and `TauriHost`), `tests.rs` (end-to-end tests).
  - `src/visualizer.rs`: the 60 Hz analyzer thread.
  - `src/playlist.rs`: the playlist drawer (window height), the Add file picker (`pick`), expanding dropped paths (`expand`), and `AUDIO_EXTENSIONS`.
  - `src/menu.rs`: the `popup_menu` command, which shows the native menus the pages build (see Gotchas).
  - `src/shortcuts.rs`: the customizable global shortcuts.
  - `src/updater.rs`: the auto-updater.
  - `src/background.rs`: the background helper (tray, start with Windows, opening the player on demand).
  - `crates/audio` (`qust-audio`): the engine. `source.rs` (HTTP via `ureq` 2, local files), `decode.rs` (Symphonia demuxing and decoding, conversion to stereo, resampling with `rubato`), `ring.rs`, `stream.rs`, `mixer.rs` (the mixer and the analyzer `Tap`), `output.rs` (`cpal`), `info.rs` (`info()`, `icecast_title()`). `probe_duration()` is in `lib.rs`.
  - `crates/codecs` (`qust-codecs`): `registry()`, Symphonia's decoders plus Opus (`opus.rs`) and, on Windows, AAC through Media Foundation (`mf.rs`).
  - `crates/spectrum` (`qust-spectrum`): the analyzer (`Spectrum`), ported from the old page's JS. `tests/reference/` holds a frozen copy of that JS and the frames it drew.
- `src-tauri/icons/`: generated with `npx tauri icon build/icons/icon.png`. Delete the `android/`, `ios/`, `Square*` and `StoreLogo` outputs afterwards. `build/icons/icon.png` is the source, a rounded square with transparent corners, and is also used by the README.
- `assets/screenshot.png`: README screenshot of the player window.
- `TODO.md`: planned settings and features.

## How it works

**Windows.** The player is 860×267 and not resizable. The playlist drawer adds 320 px (587 in all). Settings (520×850) and Stream information (580×590) are separate native windows opened with `invoke('open_panel', { name })`. They are owned by the player, so they stay above it with no taskbar button, and they open centered over it. Esc closes them. All webviews must get identical `additional_browser_args`, because WebView2 shares one browser process. That is why every window goes through `build_window`. Creating windows from a command must be `async` (sync deadlocks on Windows). Capabilities (`capabilities/default.json`) list all three window labels.

**`window.__QUST__`.** Injected by `main.rs`: `accent` (`base` / `light` / `dark` from the Windows `AccentPalette` registry value; the light shade is used in dark mode and vice versa) and `debug`. `app.withGlobalTauri` is on, so pages use `window.__TAURI__.*` directly.

**Theme.** `theme` in the store is `system` / `light` / `dark`. Windows are built with it, and the `set_theme` command applies it to every window. WebView2 then switches `prefers-color-scheme`, so CSS needs no theme class. Each webview's background color follows the theme too (`background()` in `main.rs`), so the area the drawer uncovers while the window grows isn't white.

**The engine (`qust-audio`).** `Engine::new(Config { codecs, output, user_agent }, on_event)`, then `load(key, Source::Url | Source::File, start_at)`, `unload`, `unload_all`, `keys`, `status`, `set_level` (0 or 1, reached with a ~15 ms fade), `set_paused`, `seek` (files only), `set_volume`, `set_balance` (Web Audio's `StereoPannerNode` law) and `tap()`. Keys are opaque to the engine. `load` replaces a stream already under the key with a fresh connection, and every load gets a new `StreamId`. A new stream has level 0 and is consumed in real time as soon as it has audio.
- **Per stream** (`decode.rs`, a thread of its own): the source, a Symphonia probe (hinted by the Content-Type and the extension), a decoder from `config.codecs`, stereo f32 (mono on both sides, surround with Web Audio's downmix), resampled to the mixing rate (rubato FFT), then a lock-free ring (`ring.rs`) that the mixer drains. A full ring blocks the thread, so the network backs up as it would in a browser. The ring holds 8 s for URLs (`STREAM_SECONDS`: the burst a server sends on connect is what rides out later hiccups) and 3 s for local files (`FILE_SECONDS`). A live stream skips bad packets, and gives up on an I/O error, an unsupported format, or 100 failed packets in a row (`MAX_ERRORS`). A chained Ogg stream (a new logical stream per track, common on Icecast) gets a new decoder at each change.
- **HTTP source** (`source.rs`): what the old proxy did, minus the HTTP server. It sends `Icy-MetaData: 1`, strips the ICY metadata from the audio and records the title, with 10 s connect and 30 s read timeouts. A response with `Content-Length` and no `icy-metaint` is a file: it has a duration, and with `Accept-Ranges: bytes` it can seek, with a new `Range` request (forward jumps up to 256 KB read through the current connection, and the first 256 KB are kept for the demuxer's probing). Local files seek exactly, HTTP files by the byte estimate. A file cut short is a failure (so the player reconnects), while a live server closing is the stream's end. Ogg streams carry titles as Vorbis comments, which count when there are no ICY titles. SHOUTcast v1 `ICY 200 OK` responses aren't valid HTTP for `ureq` and fail.
- **Mixer** (`mixer.rs`, run in the output callback: no waiting locks, no allocation): sums every stream that has audio and isn't paused, at its level, writes the sum to the `Tap` (before volume, overs kept), then applies volume and balance (smoothed over 5 ms) and clamps. Streams at level 0 are consumed all the same, so they stay at the live point. That is instant switch. A stream starts once 150 ms is buffered. After running out (`Waiting`) it resumes at 0.5 s buffered, twice that after each further stall up to 2 s, and back to 0.5 s after 30 s without one, so a stream arriving barely in real time settles instead of flapping. Pausing fades out over 15 ms, then stops consuming. An unloaded stream fades out for 40 ms before the mixer lets go of it.
- **Output** (`output.rs`): the default device through cpal, in its own format. Stereo goes to the first two channels, and mono devices get the average. The mixing rate is the default device's rate when the engine starts (48 kHz without one, and on `OutputKind::Null`). A device at another rate gets the mix resampled in the callback. The output opens when the first stream loads and closes 3 s after the last one goes (`IDLE_CLOSE`), so it doesn't hold the device or keep the PC awake. While open, it checks every second whether the default device moved, and reopens on the current default when it did or the stream errored, keeping the streams, retrying every second while there is no device.
- **Events**, from one thread (`qust-events`, every 20 ms), with no engine lock held, so the handler may call back in. Per stream: `Duration` first (`None` for a live stream), then `Playing` / `Waiting` alternating, then one `Ended` or `Failed`. Only the stream currently under the key reports. `Failed` before any audio comes at once, and after audio once the buffered audio has played. A seek clears the end, and a file stays open after `Ended`, so seeking back plays it again.
- **`Status`:** `started`, `live` (being consumed now: not paused, waiting, refilling after a seek, or done), `paused`, `ended`, `failed`, `position` (a file's play head, showing a pending seek target at once; for a stream, the same as `played`), `played` (seconds consumed since the load, muted time included, paused time not), `duration` (`Some` only for a file of known length) and `buffered` (seconds ahead of the play head).
- **`info(key)`** (`info.rs`): the details by key, kept across reconnects of the key. The JSON fields are `name`, `title`, `genre`, `description`, `website`, `server`, `contentType`, `format`, `bitrate`, `rate` (measured, from a baseline 5 s in), `samplerate`, `channels`, `metaint`, `size`, `bytes`, `metaBytes`, `titleChanges`, `reconnects` (loads after the first) and `connectedSeconds`. Format, sample rate and channels come from the headers (`icy-br`, `icy-sr`, `ice-audio-info` or `icy-audio-info`), then from the first MP3/ADTS frame header (after any ID3v2 tag), then from the decoder, which fills only what is still unknown (its codec name replaces a format guessed from the content type). An ADTS AAC-LC header at 24 kHz or less is taken as HE-AAC (implicit SBR): the header has the core rate, and playback is at twice that. A local file is described from its MP3/AAC frame, FLAC STREAMINFO or WAV header. A file with no bitrate gets its average from size and duration.
- `icecast_title(url)` reads the title from the server's `status-json.xsl` (blocking, 10 s). `probe_duration(path)` gives a local file's duration without decoding it (`Err` when it can't be opened or isn't audio).

**Formats.** Containers are Symphonia's (MP3 / ADTS, MP4 / M4A, Ogg, Matroska / WebM, FLAC, WAV). Decoders come from `qust_codecs::registry()`:
- MP1, MP2, MP3, FLAC, Vorbis, PCM, ADPCM, ALAC: Symphonia.
- Opus (Ogg, WebM, MP4): `OpusDecoder` (`opus.rs`), a wrapper around the pure-Rust `opus1` crate. Mono and stereo only (surround is refused), always 48 kHz.
- AAC on Windows: `MfAacDecoder` (`mf.rs`, short name `aac-mf`), the system's Media Foundation decoder, for AAC-LC, HE-AAC v1 (SBR) and v2 (PS), from ADTS and MP4. It is registered at `Tier::Preferred`, above Symphonia's, and falls back to Symphonia's AAC-LC by itself when the system has no AAC decoder (Windows N without the Media Feature Pack) or it won't take the stream.
- AAC elsewhere (macOS): Symphonia's AAC-LC.

Files the app accepts have one of the extensions `mp3 m4a aac mp4 ogg oga opus flac wav webm weba`. Streams play whatever their URL.

**Persistence.** Store keys:
- `player1-title` / `player1-url` through `player6-*`: the cartridges
- `last-selected-player` / `-title` / `-url`: the last cartridge played
- `last-selected-player` is `playlist` when a playlist entry was last selected, and `playlist-index` is its position
- `playlist`: an array of `{ title, path, duration }` (local file) or `{ title, url, duration }`, and `playlist-open` (the drawer)
- `volume`, `balance`
- `theme`, `visualizer-colors` (`classic` / `white` / `accent` / `off`; clicking the analyzer display cycles them), `autoplay`, `background`, `instant-switch`, `reconnect-minutes` (default 5, 0 = never give up)
- `shortcut-play`, `shortcut-stop`, `shortcut-toggle` (empty string = no shortcut)

The player writes its own keys from Rust. Volume and balance are saved 300 ms after the last change, the playlist 400 ms, and what is pending is written when the player window closes. The Settings window writes its keys with `saveData`, and the store tells the player at once when a page writes `visualizer-colors`, `reconnect-minutes` or `autoplay` (`Store::watch`; the player's own writes don't trigger it). The first run imports the Electron version's `<config dir>/qust/config.json`. `loadData` turns Rust's `null` into `undefined`.

**The player.** One thread (`qust-player`) owns everything: `Core` in `machine.rs`, a port of the old page's player with the same function names where they still exist (`active`, `current`, `sync_streams`, `play`, `pause`, `stop`, `lost`, `reconnect`, `watch`), the same constants and the same store keys. It takes messages: commands (`Player::call` runs a closure on the core and answers once the events it caused have gone out, `Player::cast` doesn't wait), engine events, and its own timers (`next_deadline` / `timers`). After every message, `refresh()` emits what changed. Only `commands.rs` knows Tauri: the rest sees a `Host` (events to the windows), the `Store` and the frame sink. The engine starts when the page attaches (`player_attach`) and is dropped when the player window is destroyed (`Player::detach`).

**Player state.** `State` is `stopped` / `connecting` / `playing` / `buffering` / `paused`. `set_state()` sets it and tells the visualizer. `refresh()` sends `player-state` when anything but the clock changed, and every 250 ms while not stopped. It sends `player-status` on changes and every second. Stop disconnects every stream except the instant switch ones. A pause longer than `LIVE_RESUME` (10 s) reloads the stream on resume, rejoining the live point. While playing, the player reads `qust_audio::info()` every 2 s. The title comes from the stream (ICY or Ogg comments), then the Icecast `status-json.xsl` fallback (at most every 15 s, on a thread of its own, dropped if the station changed meanwhile), then, for a cartridge, the stream's name, then the cartridge or entry name. The page derives what is only presentation: which buttons are enabled, time formatting, a URL's host, row classes (`is-empty` / `is-active` / `is-playing` on cartridges).

**Streams and instant switch.** What plays is `active()`: the cartridge `cartridges[active_slot]` or the playlist entry `active_entry`, never both. Each source is a stream in the engine under its key (`Entry::key()`: the URL, or `file:` and the path), with `current()` the active one and its `Status`. Keying by URL means reordering cartridges doesn't disturb them.
- **Instant switch** (the ⚡ button, `instant-switch` in the store): every loaded cartridge streams in the background at level 0, even while stopped, so starting or switching is immediate. Only the 6 cartridges preload, never playlist entries, and at most six streams play at once (`MAX_STREAMS`; a playing URL entry takes one). `preload` in each cartridge's state drives the bolt (`loading`, then `live`).
- **`sync_streams()`:** keeps exactly the wanted streams loaded (the active one unless stopped, plus every cartridge in instant mode), reloads background ones that failed or ended, and sets the levels (active 1, others 0). Call it after anything that changes cartridges, the active source or the state.
- **`play()`:** when the target is a stream already being consumed (`status.live`), it switches immediately with no reconnect, and `clock_start` starts the clock from where that stream is. Otherwise it loads it (`connecting`).
- **`pause()`:** pauses only the active stream: its ring fills and the connection backs up behind it. Resuming within `LIVE_RESUME` applies only to the same source (`paused_key`), and only if it hasn't failed or ended. A file (`status.duration` is `Some`) always resumes where it paused.
- **Files** (local, or online with a length): running out of buffer waits (`buffering`) instead of reconnecting, `Ended` plays the next entry, and a reconnect picks up at the same position (`load` with `start_at`). `player_seek` seeks only a file.

**Reconnecting.** `lost()` runs when the active stream drops: a live stream still out of audio `STALL_TIMEOUT` (3 s) after `Waiting`, `Failed` or `Ended` after it played, or the watchdog. The first try is immediate, then one every second (counted from the previous try's start) for 15 tries, then every 15 s, until `reconnect-minutes` runs out ("Connection lost"). Each try (`reconnect()`) loads the source again under its key, which gives a fresh connection and decoder. A try that isn't playing after 12 s (`ATTEMPT_TIMEOUT`) counts as failed. Playing for 10 s (`RECOVERED_TIMEOUT`) ends the sequence. A source that never played isn't retried: a cartridge stops with "Couldn't play this stream", and a playlist entry is marked `missing` and the next one plays, until every entry has been tried ("Couldn't play this file" or "… stream"). The clock carries over (`clock_base`), and so does the title until the new connection sends one. Background streams that drop are re-synced after 15 s.

**Watchdog.** `watch()` runs every second. If the active stream's time (a file's `position`, a stream's `played`) hasn't moved for 3 s while `playing`, it calls `lost()`, unless no output is running (`Tap::read` gives `None`). Device changes and sleep are the engine's job: it reopens the output by itself, and a stream that doesn't move meanwhile isn't at fault.

**Playlist.** The Playlist button (`setDrawer`) calls `set_playlist_open`, which animates the window height in Rust and saves `playlist-open`. `build_player` opens the window at the saved height. The page never scrolls: `.app` is fixed at `--player-height`, and the drawer below it is cut off by the window (and `inert` while closed). The loaded cartridges are pinned at the top of the list: they play and edit there, but move and eject only above. Below them, entries come from:
- dropping on the window: Tauri's `onDragDropEvent` gives the paths (HTML drag and drop doesn't reach the page), and `playlist_drop` adds them at the drop position, expanded by `playlist::expand`: files, folders (natural name order, recursive) and `.m3u` / `.pls` lists. Lists inside folders are skipped. One drop adds at most 10,000 entries (`MAX_ENTRIES`).
- Add file: `playlist_add_files` opens the picker (`playlist::pick`, tauri-plugin-dialog, used from Rust only), with the same expansion.
- Add URL and Edit… (right-click): the cartridge dialog, with `dialogTarget` set to `{ kind: 'url' }` (`playlist_add_url`) or `{ kind: 'entry', id, file }` (`playlist_edit`; a local file's path is shown read-only and sent as `url: null`).

The entries live in Rust (`Entry` in `model.rs`). Each has an `id` that lasts as long as the app runs and isn't saved. The page keeps a copy (`entries`) from `player-playlist` and `player-entry`. `renderPlaylist()` rebuilds the rows, `renderPlaylistState()` sets their classes, and `rowOf` maps items (an entry id, or `slot0` to `slot5` for a pinned cartridge) to rows. Selection supports Ctrl and Shift, dragging reorders the selection (pointer events), Delete removes, and Enter or double-click plays. Added entries are selected and revealed (`selectAdded`). Local files' durations are probed one at a time on another thread (`probe_durations`). A file that can't be opened or isn't audio is marked `missing`. Removing the active entry stops it, and the next one becomes what Play plays. The playlist plays once to the end. For a playlist file with a duration, the volume and balance card shows a seek bar over volume and balance side by side (`renderSeek`, `.sliders.is-seekable`). The state's `duration` is set even while stopped, from the entry's saved duration, so the layout doesn't jump.

**Visualizer.** `visualizer.rs` runs a 60 Hz thread while a page is attached and the player isn't stopped (while paused, only until the bars have fallen: `Spectrum::idle()`). Each tick reads the engine's `Tap` (the mix before volume, so the bars ignore the volume): `WAVE_LEN` (8192) frames ending `LEAD_SECONDS` ahead of what is heard, output latency included. It runs `Spectrum::tick` with the real time since the last tick and sends the 77-byte frame to the page through the `Channel` given to `player_attach`, only when it differs from the last one sent. `set_loading` follows `connecting` / `buffering`, and `set_off` the `off` colors. On stop it resets the spectrum and sends one all-zero frame, because the page doesn't clear the canvas by itself. The tap gives silence while nothing plays, so the bars fall. With no output running there is no analysis, and the bars hold still.

The analysis (`qust-spectrum`) is the old page's, ported number for number: the audio is resampled to 44.1 kHz, and a 576-line MDCT (an MP3 granule) folds into 75 columns and 19 bars with falling peaks, at a fixed 60 updates/s. While loading, a row-of-blocks pulse crossfades with the bars (in over 0.2 s, out over 0.5 s). The frame is byte 0, the loader level (0–255), then 4 bytes per bar: `rows` (lit rows from the bottom, 0–15), `peak` (0 = none, else its row), and the loader block's `half` height and `glow` (alpha). With `off`, rows and peaks are 0. `visualizer.js` paints bars at alpha `1 - level` and loader blocks at `level * glow / 255`, in its own palette (colors come from CSS). The canvas is a 75×16 pixel grid (3 px bars, 1 px gaps, 16 rows with one color per row). It's shown at 1:1, scaled only by whole numbers on high-DPI screens with `image-rendering: pixelated`.

**Commands (page → Rust).** Argument names are camelCase in JS. The player commands answer once Rust has handled them and emitted what they changed (the cartridge drop animation starts when `cartridge_move` resolves). Errors are strings, shown as they are (a bad address gets `URL_ERROR`: "Enter an address starting with http:// or https://"). An empty title means the default name: a cartridge's URL host, or an entry's file name without extension (an online file's name, else the host).
- `player_attach { frames }` → `{ state, playlist: { entries } }`: once per page load (a reload attaches again and the new channel replaces the old). Rust emits nothing to the page before it, and the page queues the events that arrive before the answer (`early` in `script.js`). Attaching starts the engine, syncs the streams, starts the duration probes, and plays the last cartridge if asked to (the `autoplay` setting, or the helper opened the window to play).
- `player_play`, `player_pause`, `player_stop`, `player_seek { seconds }`, `player_set_volume { value }` (0–100), `player_set_balance { value }` (−100–100), `player_set_instant { on }`, `player_cycle_colors` (saves, and emits `settings-changed { key: 'colors' }`), `player_status` (the `player-status` object, for the info window as it opens).
- `cartridge_play { slot }`, `cartridge_save { slot, title, url }`, `cartridge_eject { slot }`, `cartridge_move { from, to }` (swap, or move to an empty slot).
- `playlist_play { id }`, `playlist_add_url { url, title }`, `playlist_add_files`, `playlist_drop { paths, index }` (the three return the ids added), `playlist_remove { ids }`, `playlist_move { ids, index }` (before `index` in the list as it is), `playlist_edit { id, title, url }` (a changed URL reconnects if it was playing, and resets the duration and `missing`).
- `stream_info { url }` (`streamDetails`): `info()` for the bare URL or path (Rust adds `file:`). `set_playlist_open { open }`.

**Events (Rust → pages).**
- `player-state` (the state below) and `player-playlist` `{ entries }` (when the list changes: add, remove, move; never for a state change, since a playlist can be long). `player-entry` (one entry, when it alone changes: duration found, marked missing, edited).
- `player-status` `{ state, station, url, buffer, stalls, statusTitle }`: for the Stream information window, which also asks `player_status` when it opens.
- `settings-changed` `{ key, value }`: Settings → everyone (`colors`, `autoplay`, `reconnectMinutes`). Rust sends `colors` (the analyzer click) and `shortcuts`. The player doesn't listen to it: Rust watches the store.
- `update-status`: the updater's status.

```js
// player-state
{
  state,                          // 'stopped' | 'connecting' | 'playing' | 'buffering' | 'paused'
  cartridges,                     // 6 × null | { title, url, preload: null | 'loading' | 'live' }
  activeSlot, activeId,           // the active cartridge (0–5) or playlist entry id; null otherwise
  instant,
  station, title, titleDim,       // the display's two lines: "Playlist · 3 of 12", "Reconnecting…", …
  bitrate, samplerate, channels,  // kbps, Hz, count; null when unknown or stopped
  volume, balance, colors,
  elapsed,                        // the clock, in seconds
  duration, position, seekable,   // the seek bar (a playlist file of known length)
}
// an entry (player-playlist, player-entry)
{ id, title, path | url, duration: number | null, missing }
```

**Background helper.** With `background` on ("Background helper" in Settings), `tauri-plugin-autostart` adds a Windows startup entry that runs `qust.exe --background`.
- **At sign-in:** that launch creates only the tray icon (`background::create_tray`), with no window.
- **Closing the player:** the window is destroyed, which detaches the player (`Player::detach`: pending saves are written, playback stops and the engine goes). The `RunEvent::ExitRequested` handler keeps the process alive (`code: None` means "last window closed"). The tray's "Quit Qust" exits with a code, so it gets through.
- **Shortcuts and tray actions:** they go through `background::player_action`, which calls `Player::action` (`play` / `stop` / `toggle`). With no player window, Play and Play / Stop / Start open it (`show_player(app, true)` → `Player::request_play`), and the player plays the last cartridge once the page has attached. Stop does nothing.
- **Single instance:** `tauri-plugin-single-instance` (registered first) sends a second launch to the running process. A plain launch shows the player, and `qust.exe --toggle` acts like Play / Stop / Start. A first launch with `--toggle` opens and plays.
- **Memory:** with the window closed, the idle process holds no webview and no audio engine. Only the player thread is left, waiting for messages.

**Updates.** `tauri-plugin-updater` reads `https://github.com/mkpascal/qust/releases/latest/download/latest.json`, and the public key is in `tauri.conf.json` > `plugins.updater`. `updater.rs` checks GitHub, downloads a newer version in the background (status `ready`), and installs it only when the user clicks "Restart and update" in Settings. A dot on the cog shows that an update is waiting. The NSIS installer (passive mode) closes Qust and reopens it. When "Check automatically" (`auto-update`, on by default) is on, it checks 30 s after launch and then whenever `update-last-check` is 8 hours old. Only the installed version updates: `installed()` is false for debug builds and for an exe without `uninstall.exe` next to it (the portable copy). Status reaches the pages as `update-status` events.

**Global shortcuts.** `shortcuts.rs` keeps the bindings as accelerators built from `KeyboardEvent.code` (Play `Ctrl+Alt+Insert`, Stop `Ctrl+Alt+KeyH`, Play / Stop unassigned). Play / Stop plays when stopped or paused and stops otherwise. It exists so a single Stream Deck "Hotkey" button stays in sync, which a "Hotkey Switch" with separate Play and Stop keys does not. Backspace while recording resets a shortcut to its default, which clears Play / Stop. While Settings records one, it calls `pause_shortcuts(true)`, because a registered shortcut never reaches the page. `set_shortcut` refuses shortcuts another app owns or that are already used by another action. Closing the Settings window always resumes them. A shortcut owned by another app at startup is kept but marked unregistered, and Settings shows it crossed out.

## Debugging and testing

Prefer the headless tests and the examples for anything about audio or the player: they need no window, no sound and no network.

The bars must stay identical to the old page's. `tests/reference.rs` in `qust-spectrum` rebuilds synthetic signals sample for sample and compares every frame byte for byte with `tests/reference/frames.txt`, which `tests/reference.mjs` generated from `tests/reference/visualizer.js`, a frozen copy of the page's analyzer from before the port. Never edit that copy. The port mirrors the JS operation by operation in f64, with V8's `sin` / `cos` (`fdlibm.rs`), `Math.round` (`js_round`) and `Math.pow` (`GROWTH`), so don't simplify an expression: rerun the test. The fixture only needs regenerating when the signals change (in both `reference.mjs` and `reference.rs`): `node tests/reference.mjs` in `src-tauri/crates/spectrum`. `ticks_never_allocate` guards the 60 Hz path.

Release builds have no DevTools. To drive the UI from a script, start it with `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=9222` and use the Chrome DevTools Protocol (`http://127.0.0.1:9222/json`). That variable replaces the browser args from `main.rs`.
- The page plays no audio, so there is no page-side mute. `player_set_volume` changes the saved volume too, so put it back afterwards.
- Capture windows with `PrintWindow` (`PW_RENDERFULLCONTENT`), not screen grabs, and don't send OS keystrokes. The user works on the same desktop.
- Native menus open at the real mouse cursor (or at the position given to `popupMenu`). To test them, stub `popupMenu` (or `window.__TAURI__.menu.Menu.new`) in the page, so nothing reaches `invoke('popup_menu')`.
- Scripted runs of the app read and write the real `settings.json` (the cargo tests use a temporary one). Back it up, but don't restore it over changes the user made meanwhile.

## Gotchas

- `MfAacDecoder` holds COM's multithreaded apartment (`CoIncrementMTAUsage`) instead of initializing COM, so it can be created, used, moved and dropped on any thread that isn't a single-threaded apartment. The decode threads are plain `std::thread`s, which are in the implicit MTA. Never create one on a thread that called `CoInitialize` for an STA. For the same reason, `output::device_rate()` asks cpal on a thread of its own, so the caller's COM setup isn't touched.
- The rate and channels of decoded audio come from each buffer, never from the codec parameters: HE-AAC switches to twice the announced rate (and to stereo for v2) after the first packets, and `Converter` rebuilds its resampler when the rate changes. After a seek, time is counted in decoded frames from the first packet's timestamp, because the Media Foundation decoder hands out each packet's audio one call late (the first call gives none, and a file's last 23 to 46 ms stays inside).
- On macOS, and on Windows without a system AAC decoder, HE-AAC plays only its LC core (half the rate, no high band). The info still shows the doubled rate from the ADTS header rule.
- `seek()` is ignored until `Duration(Some)` has arrived. To start a file at a position, pass `start_at` to `load()`.
- The engine suppresses events from replaced or unloaded streams, but the player still drops any event whose `StreamId` isn't the current `Status`'s.
- The output closes 3 s after the last stream goes and reopens on the next load. Meanwhile `Tap::read` returns `None`, and nothing is consumed. The mixing rate is fixed when the engine starts.
- A live stream whose server sends no burst can give a `Waiting` right after it starts. That is why a live stream gets `STALL_TIMEOUT` to come back before it reconnects.
- The player thread must never block: `icecast_title` and the duration probes run on threads of their own and report back with `cast`, and the commands are async so `Player::call` doesn't wait on the main thread. `Player::detach` runs from the main thread's window event and waits for the player, so the player must never wait on the main thread.
- Debug builds optimise every dependency and the three crates at `opt-level = 2` (`Cargo.toml`). Unoptimised, the decoders and resamplers can't keep up with real time. `package."*"` doesn't cover workspace members, so each crate has its own entry, and a new crate needs one too.
- Debug builds skip the single-instance plugin: they share the installed app's identifier, so they would hand over to an installed Qust in the tray. They also share its `settings.json`.
- Decoders can go over full scale (Opus is unbounded). The tap keeps the overs, as the browser's analyser saw them, and the output clamps.
- The audio extensions are listed three times: `AUDIO_EXTENSIONS` (`playlist.rs`), `AUDIO_FILE` (`script.js`) and `audio_type` (`info.rs`). Keep them in sync.
- macOS is untested. CI builds it, so the code compiles, but cpal's CoreAudio output, device changes and the tests have never run there.
- Never show a native menu with `menu.popup()`. Use `popupMenu()` (`invoke('popup_menu', { rid, x, y })`, `x` / `y` in logical pixels, at the cursor without them). A menu blocks the main thread until it closes, and Tauri shows it from inside an event-loop callback, during which the event loop (tao) buffers every other event, so the clock and the analyzer frames Rust pushes to the pages froze behind an open menu. On Windows, `menu.rs` shows the menu from a thread timer callback (`SetTimer`), outside any event-loop callback, so the loop keeps running, and the command returns before the menu closes. Elsewhere it is shown the usual way. Anything else that blocks the main thread inside an event-loop callback freezes the pages the same way.
- Native menu `accelerator` strings must parse, or `Menu.new` throws. `script.js` retries without them.
- macOS builds are unsigned and not notarized, so the first launch needs right-click → Open.

## Releases and CI

`.github/workflows/build.yml` builds Windows (NSIS installer) and macOS (universal `.dmg`) with `tauri-apps/tauri-action` on pushes to `main`, pull requests and manual runs, and keeps the installers as workflow artifacts. It doesn't run the tests.

To release, bump `version` in `package.json` (and in `src-tauri/Cargo.toml` to match) and push to `main`. When no release exists for that version, the `release` job creates a draft "Qust vX.Y.Z". Both builds then add `src-tauri/tauri.release.conf.json` (`createUpdaterArtifacts`), sign the update files and upload them with `latest.json`. The `publish` job makes the release public and latest once both builds succeed, so installed copies only ever see a complete release.

Signing needs the repository secrets `TAURI_SIGNING_PRIVATE_KEY` and `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`. The key pair lives in `C:\Users\p\.tauri\` (`qust.key`, `qust.key.password`, `qust.key.pub`). If the private key is lost, installed copies can't verify new updates, so it must stay backed up. Local `npm run build` doesn't sign and needs no key.

## License

GPL-3.0-or-later.
