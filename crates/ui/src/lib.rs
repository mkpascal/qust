//! What Qust's windows share: the player's state as typed models, the bridge from `qust-player`
//! to GPUI, theme and accent handling, every window's root, and the capture and test harness.
//!
//! # Starting
//!
//! ```ignore
//! qust_ui::application().run(|cx| {
//!     qust_ui::init(cx);
//!     let bridge = qust_ui::Bridge::start(store, OutputKind::Device, cx);
//!     qust_ui::theme::apply(qust_ui::theme::ThemeChoice::parse(&bridge.models.settings.read(cx).theme), cx);
//!     // open the player window, then: bridge.player.attach();
//! });
//! ```
//!
//! # Building a view
//!
//! A window crate's views hold the [`Models`] entities they show and observe them; they send
//! commands through [`Player`] (`Bridge::global(cx).player`), which never blocks:
//!
//! ```ignore
//! struct Transport { deck: Entity<Deck>, player: Player }
//! impl Transport {
//!     fn new(models: &Models, player: Player, cx: &mut Context<Self>) -> Self {
//!         cx.observe(&models.deck, |_, _, cx| cx.notify()).detach();
//!         Transport { deck: models.deck.clone(), player }
//!     }
//! }
//! impl Render for Transport {
//!     fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
//!         let deck = self.deck.read(cx);
//!         let can_play = (deck.active_slot.is_some() || deck.active_id.is_some()) && !deck.state.is_running();
//!         let player = self.player.clone();
//!         Button::new("play").icon_only(Icon::Play).accent().aria_label("Play").disabled(!can_play)
//!             .on_click(move |_, _, _| player.play())
//!     }
//! }
//! ```
//!
//! What changes often goes in a live layer of the window's [`WindowRoot`] (see [`window`]).

pub mod bridge;
pub mod diag;
#[cfg(feature = "capture")]
pub mod capture;
pub mod fixture;
pub mod model;
pub mod models;
pub mod theme;
pub mod window;

pub use bridge::{Bridge, Event, Player, Reply};
pub use model::{format, Background, Binding, Cartridge, Entry, Playback, PlayerState, Playlist, Preload, SettingChange, Shortcuts, Status, StreamInfo, UpdateStatus};
pub use models::Models;
pub use window::{analyzer_slot, portal_slot, AnalyzerView, WindowRoot};

/// The GPUI application with Qust's assets (icons).
pub fn application() -> gpui::Application {
    gpui_platform::application().with_assets(qust_fluent::Assets)
}

/// Once at startup: GPUI Kit (for `Input`), the fluent elements (Tab keys, focus ring
/// tracking), grayscale text (see `qust_fluent::text`), reduced motion from the system.
pub fn init(cx: &mut gpui::App) {
    gpui_component::init(cx);
    qust_fluent::init(cx);
    cx.set_text_rendering_mode(gpui::TextRenderingMode::Grayscale);
    // Captures only: `QUST_TEXT_RENDERING=subpixel` draws text with ClearType instead, to compare
    // with the runs the real WebView2 window drew with ClearType (reference/renders).
    #[cfg(feature = "capture")]
    if std::env::var("QUST_TEXT_RENDERING").as_deref() == Ok("subpixel") {
        cx.set_text_rendering_mode(gpui::TextRenderingMode::Subpixel);
    }
    let reduced = cx.reduce_motion();
    let mut motion = qust_fluent::motion(cx);
    motion.reduced = reduced;
    cx.set_global(motion);
    // GPUI Kit's theme follows ours from here on.
    let theme = *qust_fluent::theme(cx);
    theme.install(cx);
}
