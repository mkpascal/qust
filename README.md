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
  - Small, efficient native app (a 4.5 MB installer) that plays streams through a native audio element,
  - Features a cartridge-loading concept: six slots, saved with your volume, balance and settings,
  - Classic pixel spectrum analyzer that follows the sound precisely, with volume and balance controls,
  - Shows the current song (ICY metadata), bitrate, sample rate and mono/stereo,
  - Stream information window with format, network, stability and ICY details,
  - Customizable global shortcuts (Ctrl + Alt + Insert / Ctrl + Alt + H by default),
  - Windows 11 look: Mica, light and dark themes, your accent color, native menus,
  - Compatible with all HTTP streaming servers (Icecast, SHOUTcast v2),
  - Supports Windows and macOS.

## Building

Qust is a [Tauri](https://tauri.app) app. You need [Node.js](https://nodejs.org) and [Rust](https://rustup.rs), plus the Visual Studio C++ build tools on Windows or the Xcode command line tools on macOS.

```sh
npm install
npm start        # run in development
npm run build    # app and installer, copied to build/bin/
```

Every push to `main` is built on GitHub Actions for Windows and macOS. Pushing a `v*` tag creates a draft release with the installers.
