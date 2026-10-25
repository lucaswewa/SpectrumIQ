//! `spectrumiq-desktop`: the SpectrumIQ desktop shell (ADR-0040).

// No console window in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    spectrumiq_desktop::run();
}
