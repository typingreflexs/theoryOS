//! Audio subsystem — Intel HDA controller and the legacy PC speaker.
//!
//! Two backends:
//! - `pcspeaker` — PIT-driven PC speaker on every IBM-compatible PC ever made.
//!   Square-wave tones, no PCM, but bullet-proof and bare-metal universal.
//! - `hda` — Intel High Definition Audio controller (~90% of modern PCs).
//!   Initializes the HBA, sets up CORB/RIRB, enumerates codecs and output
//!   widgets. PCM playback is groundwork-only (no DMA stream yet).

pub mod hda;
pub mod pcspeaker;

/// Convenience: produce a short beep at 880 Hz for 150 ms using whichever
/// backend is available.
pub fn beep() {
    pcspeaker::tone(880, 150);
}

pub fn init() {
    pcspeaker::init();
    hda::probe();
}
