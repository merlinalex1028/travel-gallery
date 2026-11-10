#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
fn main() {
    #[cfg(feature = "performance-probe")]
    travel_gallery::performance_probe::mark_process_start();
    travel_gallery::desktop::run();
}
