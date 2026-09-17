// Hide the console window on Windows in release builds. Without this, a
// terminal flashes up behind the app every launch — and at every login, once
// the startup entry is on.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    basalt_host_lib::run()
}
