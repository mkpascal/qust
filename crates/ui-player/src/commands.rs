//! What the player window asks of the player: the commands `script.js` sent with `invoke`, as a
//! trait so the view runs the same against the real player ([`qust_ui::Player`] implements it)
//! and against a recording stand-in in captures and tests.
//!
//! Commands with an answer return an [`Answer`]: a future resolved on the UI thread. With the
//! real player it resolves after the events the command caused have reached the models (the
//! bridge's ordering), which the dialog and the "dropped" animation rely on, as the page did.

use std::future::Future;
use std::pin::Pin;

/// An answer to come back on the UI thread; `None` if the player went away.
pub type Answer<T> = Pin<Box<dyn Future<Output = Option<T>>>>;

/// An answer that is already there (stand-ins).
pub fn ready<T: 'static>(value: T) -> Answer<T> {
    Box::pin(std::future::ready(Some(value)))
}

pub trait Commands: 'static {
    /// `player_play`
    fn play(&self);
    /// `player_pause`
    fn pause(&self);
    /// `player_stop`
    fn stop(&self);
    /// `player_set_instant`
    fn set_instant(&self, on: bool);
    /// `player_cycle_colors` (a click on the screen)
    fn cycle_colors(&self);
    /// `player_set_volume`, 0–100, on every change the user makes.
    fn set_volume(&self, value: i64);
    /// `player_set_balance`, −100–100, on every change the user makes.
    fn set_balance(&self, value: i64);
    /// `player_seek`, when the user settles on a position.
    fn seek(&self, seconds: f64);
    /// `cartridge_play`
    fn select_slot(&self, slot: usize);
    /// `cartridge_eject`
    fn eject(&self, slot: usize);
    /// `cartridge_save`: Rust checks the address (`Err` is the message the dialog shows).
    fn save_cartridge(&self, slot: usize, title: String, url: String) -> Answer<Result<(), String>>;
    /// `cartridge_move`: a swap, or a move onto an empty slot.
    fn move_cartridge(&self, from: usize, to: usize) -> Answer<()>;
    /// `playlist_edit`: `url` is None for a local file.
    fn edit_entry(&self, id: u64, title: String, url: Option<String>) -> Answer<Result<(), String>>;
    /// `playlist_add_url`: the ids added.
    fn add_url(&self, url: String, title: String) -> Answer<Result<Vec<u64>, String>>;
}

impl Commands for qust_ui::Player {
    fn play(&self) {
        qust_ui::Player::play(self)
    }
    fn pause(&self) {
        qust_ui::Player::pause(self)
    }
    fn stop(&self) {
        qust_ui::Player::stop(self)
    }
    fn set_instant(&self, on: bool) {
        qust_ui::Player::set_instant(self, on)
    }
    fn cycle_colors(&self) {
        qust_ui::Player::cycle_colors(self)
    }
    fn set_volume(&self, value: i64) {
        qust_ui::Player::set_volume(self, value)
    }
    fn set_balance(&self, value: i64) {
        qust_ui::Player::set_balance(self, value)
    }
    fn seek(&self, seconds: f64) {
        qust_ui::Player::seek(self, seconds)
    }
    fn select_slot(&self, slot: usize) {
        qust_ui::Player::select_slot(self, slot)
    }
    fn eject(&self, slot: usize) {
        qust_ui::Player::eject(self, slot)
    }
    fn save_cartridge(&self, slot: usize, title: String, url: String) -> Answer<Result<(), String>> {
        Box::pin(qust_ui::Player::save_cartridge(self, slot, title, url).get())
    }
    fn move_cartridge(&self, from: usize, to: usize) -> Answer<()> {
        Box::pin(qust_ui::Player::move_cartridge(self, from, to).get())
    }
    fn edit_entry(&self, id: u64, title: String, url: Option<String>) -> Answer<Result<(), String>> {
        Box::pin(qust_ui::Player::edit_entry(self, id, title, url).get())
    }
    fn add_url(&self, url: String, title: String) -> Answer<Result<Vec<u64>, String>> {
        Box::pin(qust_ui::Player::add_url(self, url, title).get())
    }
}
