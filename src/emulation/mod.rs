//! Emulation of real BitTorrent clients from Joal `.client` profiles.

pub mod algorithm;
pub mod generator;
pub mod urlencode;

pub use generator::Event;
