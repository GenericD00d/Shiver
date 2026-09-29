const COMMANDS: &[&str] = &[];

fn main() {
    // No commands: the rail is drawn by the core, never asked for by a webview. A server's page
    // must not be able to learn what else is on it.
    tauri_plugin::Builder::new(COMMANDS)
        .android_path("android")
        .build();
}
