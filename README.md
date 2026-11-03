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
  - Efficient player that leverages a native audio element for low-resource, efficient playback,
  - Features a cartridge-loading concept with platform-persistent loading/saving system,
  - High-refresh rate and reliable audio visualizer for precise sound representation,
  - Supports global shortcuts (Ctrl + Alt + Insert / Ctrl + Alt + H),
  - Capable of handling title metadata for compatible streams,
  - Compatible with all HTTP streaming servers,
  - Supports Windows and MacOS.

## Building

Qust is a [Tauri](https://tauri.app) app. You need [Node.js](https://nodejs.org) and [Rust](https://rustup.rs), plus the Visual Studio C++ build tools on Windows or the Xcode command line tools on macOS.

```sh
npm install
npm start        # run in development
npm run build    # installers in src-tauri/target/release/bundle/
```

Every push to `main` is built on GitHub Actions for Windows and macOS. Pushing a `v*` tag creates a draft release with the installers.
