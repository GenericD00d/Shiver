// Prevents an extra console window on Windows when this is run on a desktop for testing.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    shiver_mobile_lib::run()
}
