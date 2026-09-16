//! Basalt desktop shell.
//!
//! Deliberately thin. The window is frameless because the app draws its own
//! title bar (see `TitleBar.tsx`), and transparency is off: the design is solid
//! graphite, and a transparent window would cost compositing work for an effect
//! the palette never uses.

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .run(tauri::generate_context!())
        .expect("error while running Basalt");
}
