//! Automated protections: automod, anti-raid, phishing feed, flag leak watch.
//!
//! Phase 0 ships frozen entry points only; the moderation lane fills the logic.

pub mod automod;
pub mod flagwatch;
pub mod phishing;
pub mod raid;
