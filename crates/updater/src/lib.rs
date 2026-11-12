// Qust's updater: reads the GitHub release's latest.json, downloads the update for this platform,
// verifies its signature and installs it, the way tauri-plugin-updater 2.14 did, so that the
// release format, the signing key and the installers stay the same for both. Copies of Qust that
// still run the Tauri version find their updates in the same latest.json.
//
// Everything here blocks: the caller runs it on a thread. The schedule (30 s after launch, then
// every 8 hours while "Check automatically" is on) and the settings it reads (`auto-update`,
// `update-last-check`) stay with the caller; `Updates` keeps the status the Settings page shows.

mod manifest;
mod signature;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(windows)]
mod windows;
#[cfg(any(target_os = "macos", test))]
mod archive;

pub use manifest::{targets, Platform, Release};
pub use signature::verify;

use std::ffi::OsString;
use std::fmt;
use std::io::Read;
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

// Where installed copies look for updates, Tauri versions included.
pub const ENDPOINT: &str = "https://github.com/mkpascal/qust/releases/latest/download/latest.json";

// The public half of the release signing key (tauri.conf.json > plugins > updater > pubkey):
// base64 of the minisign public key file.
pub const PUBLIC_KEY: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IDE0MDlGREM4NTgxNDFBNEYKUldSUEdoUll5UDBKRkpoWWJHU2JPZHdmOFZHaVhhSlhaTk5JeVkyaytOVzlIZlZHS1FrQ294SmYK";

// The schedule updater.rs had: first check shortly after launch, then whenever the last one is
// this old (looked at every half hour).
pub const FIRST_CHECK_DELAY: Duration = Duration::from_secs(30);
pub const CHECK_EVERY: Duration = Duration::from_secs(8 * 60 * 60);
pub const SCHEDULE_TICK: Duration = Duration::from_secs(30 * 60);

// An installer is a few MB; anything this big is not ours.
const MAX_DOWNLOAD: u64 = 512 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    // The check or the download failed (offline, GitHub down, HTTP error).
    Network(String),
    // latest.json isn't a release both updaters accept.
    Manifest(String),
    // The release has no file for this platform (the keys that were looked for).
    NoTarget(Vec<String>),
    // The file doesn't match its signature, or the signature isn't ours.
    Signature(String),
    Install(String),
    NotInstalled,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Network(message) => write!(f, "Couldn't reach the update server: {message}"),
            Error::Manifest(message) => write!(f, "The update information is invalid: {message}"),
            Error::NoTarget(targets) => write!(f, "No update for this platform ({})", targets.join(", ")),
            Error::Signature(message) => write!(f, "The update's signature is invalid: {message}"),
            Error::Install(message) => write!(f, "Couldn't install the update: {message}"),
            Error::NotInstalled => write!(f, "Updates work in the installed version of Qust"),
        }
    }
}

impl std::error::Error for Error {}

// Only the installed version updates itself: a portable or development copy has no installer to
// update. The NSIS installer puts uninstall.exe next to the app; the portable exe has none. On
// macOS the app must run from its bundle.
pub fn installed() -> bool {
    if cfg!(debug_assertions) {
        return false;
    }
    let Ok(exe) = std::env::current_exe() else {
        return false;
    };
    #[cfg(windows)]
    return exe.parent().map(|dir| dir.join("uninstall.exe").exists()).unwrap_or(false);
    #[cfg(target_os = "macos")]
    return macos::bundle_of(&exe).is_some();
    #[cfg(not(any(windows, target_os = "macos")))]
    return {
        let _ = exe;
        false
    };
}

pub fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |time| time.as_secs())
}

// Whether a scheduled check is due, given `update-last-check` (seconds since 1970).
pub fn check_due(last_check: Option<u64>, now: u64) -> bool {
    now.saturating_sub(last_check.unwrap_or(0)) >= CHECK_EVERY.as_secs()
}

pub struct Updater {
    endpoint: String,
    pubkey: String,
    current: semver::Version,
    agent: ureq::Agent,
}

impl Updater {
    // `current` is this app's version (CARGO_PKG_VERSION), `pubkey` the base64 key (PUBLIC_KEY).
    pub fn new(endpoint: &str, pubkey: &str, current: &str) -> Result<Self, Error> {
        let current = manifest::parse_version(current).map_err(|error| Error::Manifest(format!("current version: {error}")))?;
        // HTTPS_PROXY / ALL_PROXY are honoured, as reqwest did for the Tauri updater; the system
        // proxy settings (Windows registry, macOS network settings) are not.
        let agent = ureq::AgentBuilder::new()
            .try_proxy_from_env(true)
            .timeout_connect(Duration::from_secs(30))
            .timeout_read(Duration::from_secs(60))
            .user_agent(concat!("qust-updater/", env!("CARGO_PKG_VERSION")))
            .build();
        Ok(Updater { endpoint: endpoint.to_string(), pubkey: pubkey.to_string(), current, agent })
    }

    pub fn current_version(&self) -> String {
        self.current.to_string()
    }

    // The newer release for this platform, or None when this version is current. A 204 answer
    // means no update too, as for the Tauri updater.
    pub fn check(&self) -> Result<Option<Update>, Error> {
        let response = self.agent.get(&self.endpoint).set("Accept", "application/json").call().map_err(network)?;
        if response.status() == 204 {
            return Ok(None);
        }
        let text = response.into_string().map_err(|error| Error::Network(error.to_string()))?;
        self.update_from(&text)
    }

    // The part of check() after the request, for the tests.
    fn update_from(&self, text: &str) -> Result<Option<Update>, Error> {
        let release = Release::parse(text)?;
        // A release that is not an update is not an error, even with no file for this platform.
        if release.version <= self.current {
            return Ok(None);
        }
        let platform = release.platform(&targets())?.clone();
        Ok(Some(Update {
            version: release.version.to_string(),
            notes: release.notes.clone(),
            pub_date: release.pub_date.clone(),
            platform,
            pubkey: self.pubkey.clone(),
            agent: self.agent.clone(),
        }))
    }
}

fn network(error: ureq::Error) -> Error {
    match error {
        ureq::Error::Status(code, response) => Error::Network(format!("{} answered {code}", response.get_url())),
        ureq::Error::Transport(transport) => Error::Network(transport.to_string()),
    }
}

#[derive(Clone)]
pub struct Update {
    pub version: String,
    pub notes: Option<String>,
    pub pub_date: Option<String>,
    pub platform: Platform,
    pubkey: String,
    agent: ureq::Agent,
}

impl Update {
    // Downloads the file and verifies it. `progress` gets the bytes received so far and the
    // size when the server says it.
    pub fn download(&self, mut progress: impl FnMut(u64, Option<u64>)) -> Result<Package, Error> {
        let response = self.agent.get(&self.platform.url).set("Accept", "application/octet-stream").call().map_err(network)?;
        let total = response.header("Content-Length").and_then(|value| value.parse::<u64>().ok());
        if total.is_some_and(|total| total > MAX_DOWNLOAD) {
            return Err(Error::Network("the update is too large".into()));
        }
        let mut reader = response.into_reader().take(MAX_DOWNLOAD + 1);
        let mut bytes = Vec::with_capacity(total.unwrap_or(0) as usize);
        let mut buffer = vec![0u8; 64 * 1024];
        loop {
            let read = reader.read(&mut buffer).map_err(|error| Error::Network(error.to_string()))?;
            if read == 0 {
                break;
            }
            bytes.extend_from_slice(&buffer[..read]);
            progress(bytes.len() as u64, total);
        }
        if bytes.len() as u64 > MAX_DOWNLOAD {
            return Err(Error::Network("the update is too large".into()));
        }
        // A download cut short fails here too.
        self.verify(bytes)
    }

    // Wraps already downloaded bytes, once they match the signature.
    pub fn verify(&self, bytes: Vec<u8>) -> Result<Package, Error> {
        verify(&bytes, &self.platform.signature, &self.pubkey, &self.version)?;
        Ok(Package { update: self.clone(), bytes })
    }
}

// A downloaded update whose signature checked out.
pub struct Package {
    pub update: Update,
    bytes: Vec<u8>,
}

impl Package {
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    // Windows: starts the installer (passive, with relaunch) and returns; the caller must then
    // quit at once, and the installer closes whatever is still running, replaces the app and
    // starts it again with `relaunch_args`. macOS: replaces the .app bundle; the caller then
    // starts the app again itself. `relaunch_args` is ignored there.
    pub fn install(&self, relaunch_args: &[OsString]) -> Result<(), Error> {
        // Checked again, as the Tauri updater does, in case the bytes were kept a while.
        verify(&self.bytes, &self.update.platform.signature, &self.update.pubkey, &self.update.version)?;
        #[cfg(windows)]
        return windows::install(&self.bytes, &self.update.version, relaunch_args);
        #[cfg(target_os = "macos")]
        return {
            let _ = relaunch_args;
            macos::install(&self.bytes)
        };
        #[cfg(not(any(windows, target_os = "macos")))]
        return {
            let _ = relaunch_args;
            Err(Error::Install("not supported on this platform".into()))
        };
    }
}

// "idle", "checking", "current", "downloading", "ready", "installing" or "error", as the
// Settings page knows them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Status {
    #[default]
    Idle,
    Checking,
    Current,
    Downloading,
    Ready,
    Installing,
    Error,
}

impl Status {
    pub fn as_str(self) -> &'static str {
        match self {
            Status::Idle => "idle",
            Status::Checking => "checking",
            Status::Current => "current",
            Status::Downloading => "downloading",
            Status::Ready => "ready",
            Status::Installing => "installing",
            Status::Error => "error",
        }
    }
}

// What the Settings page shows. `checked_at` is set when a check reached the server; the caller
// saves it as `update-last-check`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Snapshot {
    pub status: Status,
    pub version: Option<String>,
    pub progress: Option<u32>,
    pub error: Option<String>,
    pub checked_at: Option<u64>,
}

#[derive(Default)]
struct State {
    snapshot: Snapshot,
    ready: Option<Package>,
}

type Listener = Box<dyn Fn(&Snapshot) + Send + Sync>;

// The update flow of updater.rs: a check downloads a newer version right away (status "ready"),
// and it's installed only when the user asks, so playback is never cut without asking.
pub struct Updates {
    updater: Updater,
    state: Mutex<State>,
    listener: Listener,
}

impl Updates {
    // `listener` hears every change (to emit `update-status` and save `update-last-check`).
    pub fn new(updater: Updater, listener: impl Fn(&Snapshot) + Send + Sync + 'static) -> Self {
        Updates { updater, state: Mutex::new(State::default()), listener: Box::new(listener) }
    }

    pub fn updater(&self) -> &Updater {
        &self.updater
    }

    pub fn snapshot(&self) -> Snapshot {
        self.state.lock().unwrap().snapshot.clone()
    }

    fn set(&self, change: impl FnOnce(&mut State)) {
        let snapshot = {
            let mut state = self.state.lock().unwrap();
            change(&mut state);
            state.snapshot.clone()
        };
        (self.listener)(&snapshot);
    }

    // Checks GitHub and, when there is a newer version, downloads it so it's ready to install.
    pub fn check(&self) -> Result<(), String> {
        if !installed() {
            return Err(Error::NotInstalled.to_string());
        }
        self.check_and_download()
    }

    fn check_and_download(&self) -> Result<(), String> {
        {
            let state = self.state.lock().unwrap();
            if matches!(state.snapshot.status, Status::Checking | Status::Downloading | Status::Ready | Status::Installing) {
                return Ok(());
            }
        }
        self.set(|state| {
            state.snapshot.status = Status::Checking;
            state.snapshot.error = None;
        });

        let result = (|| {
            let update = self.updater.check().map_err(|error| error.to_string())?;
            let checked = now();
            let Some(update) = update else {
                self.set(|state| {
                    state.snapshot.status = Status::Current;
                    state.snapshot.checked_at = Some(checked);
                });
                return Ok(());
            };
            self.set(|state| {
                state.snapshot.status = Status::Downloading;
                state.snapshot.version = Some(update.version.clone());
                state.snapshot.progress = Some(0);
                state.snapshot.checked_at = Some(checked);
            });
            let mut shown = 0u32;
            let package = update
                .download(|received, total| {
                    let percent = total.filter(|total| *total > 0).map_or(0, |total| (received * 100 / total).min(100) as u32);
                    if percent > shown {
                        shown = percent;
                        self.set(|state| state.snapshot.progress = Some(percent));
                    }
                })
                .map_err(|error| error.to_string())?;
            self.set(|state| {
                state.snapshot.status = Status::Ready;
                state.snapshot.progress = None;
                state.ready = Some(package);
            });
            Ok::<(), String>(())
        })();

        if let Err(error) = &result {
            self.set(|state| {
                state.snapshot.status = Status::Error;
                state.snapshot.progress = None;
                state.snapshot.error = Some(error.clone());
            });
        }
        result
    }

    // "Restart and update": checks and downloads first when needed, then installs. On success
    // the caller quits (Windows: the installer reopens Qust) or relaunches (macOS).
    pub fn install(&self, relaunch_args: &[OsString]) -> Result<(), String> {
        if self.state.lock().unwrap().ready.is_none() {
            self.check()?;
        }
        let ready = self.state.lock().unwrap().ready.take();
        let Some(package) = ready else {
            return Err("Qust is up to date".into());
        };
        self.set(|state| state.snapshot.status = Status::Installing);
        if let Err(error) = package.install(relaunch_args) {
            let message = error.to_string();
            self.set(|state| {
                state.snapshot.status = Status::Error;
                state.snapshot.error = Some(message.clone());
            });
            return Err(message);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
