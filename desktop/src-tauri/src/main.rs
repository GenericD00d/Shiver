// the windows release build is a gui app, so keep it from spawning a console window
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    shiver_lib::run()
}
