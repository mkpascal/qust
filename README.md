# Qust

<div align="center"><img src="build/icons/icon.png" width="200"></div>

&nbsp;
&nbsp;
&nbsp;

### <div align="center">Efficient, cartridge-loaded, streaming software player</div>

&nbsp;
&nbsp;
&nbsp;
&nbsp;

<div align="center"><img src="assets/screenshot.png"></div>

&nbsp;



**Qust** is an open-source, simple, **qu**ick **st**reaming player.
  - Native Rust core: the whole audio and stream engine is compiled native code, so it starts instantly, decodes efficiently and uses very little CPU and memory,
  - Built-in stream engine: its own engine reads song titles and stream details directly,
  - Native interface: drawn by the GPU with GPUI, all in Rust, with no web engine or browser underneath,
  - Small and self-contained: a 6 MB installer, with nothing else to install,
  - Features a cartridge-loading concept: six slots, saved with your volume, balance and settings,
  - Playlist drawer: drop music files, folders or .m3u / .pls lists, or add any stream or online audio file. Files play from where they are, with seeking,
  - Reconnects by itself when a stream drops or stalls,
  - Classic pixel spectrum analyzer that follows the sound precisely, with volume and balance controls,
  - Shows the current song (ICY metadata), bitrate, sample rate and mono/stereo,
  - Stream information window with format, network, stability and ICY details,
  - Customizable global shortcuts (Ctrl + Alt + Insert / Ctrl + Alt + H by default),
  - Background helper: starts with Windows in the tray, so shortcuts work even while Qust is closed,
  - Updates itself from the GitHub releases (checks every 8 hours, installs when you restart),
  - Windows 11 look: light and dark themes, your accent color, system menus,
  - Compatible with all HTTP streaming servers (Icecast, SHOUTcast v2),
  - Supports Windows and macOS.

## Building

Qust is written in [Rust](https://rustup.rs), with its windows drawn by [GPUI](https://gpui.rs). You need Rust, plus the Visual Studio C++ build tools and the Windows SDK on Windows, or Xcode on macOS. The installer is built with [NSIS](https://nsis.sourceforge.io) and [Node.js](https://nodejs.org).

```sh
cargo run -p qust-app                   # run in development
cargo build --release -p qust-app       # target/release/qust.exe
node packaging/windows/build-installer.mjs --exe target/release/qust.exe --version 1.2.0   # installer into build/bin/
```

Every push to `main` is built on GitHub Actions for Windows and macOS. Bumping the version in `Cargo.toml` publishes a [release](https://github.com/mkpascal/qust/releases), which installed copies of Qust update to.
