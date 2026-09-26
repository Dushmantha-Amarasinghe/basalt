/// Every command the Kotlin side answers. The ones the interface calls are
/// allowed in `permissions/default.toml`; the rest are Rust's to call.
const COMMANDS: &[&str] = &[
    "pick_files",
    "pick_folder",
    "list_folder",
    "open_fd",
    "create_download",
    "finish_download",
    "open_with",
    "open_download",
    "share_download",
    "take_shared",
    "keep_alive",
    "let_go",
    "set_immersive",
    "set_orientation",
    "minimize",
    "install_apk",
    "can_install_apks",
    "open_install_settings",
    "haptic",
    "request_notifications",
    "insets",
];

fn main() {
    tauri_plugin::Builder::new(COMMANDS)
        .android_path("android")
        .build();
}
