//! ratiod: a lightweight BitTorrent ratio emulator.

pub mod api;
pub mod bandwidth;
pub mod bencode;
pub mod config;
#[cfg(windows)]
pub mod desktop;
pub mod emulation;
pub mod engine;
pub mod icon;
pub mod instance;
pub mod library;
pub mod server;

#[doc(hidden)]
pub mod testutil;
pub mod torrent;
pub mod tracker;
