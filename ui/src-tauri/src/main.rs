// Verio — Phase 0.
// Thin commands/events bridge onto verio-app. Audio NEVER crosses this boundary.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    verio_lib::run();
}
