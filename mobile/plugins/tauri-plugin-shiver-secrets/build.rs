const COMMANDS: &[&str] = &[];

fn main() {
    // No commands: nothing here is reachable from a webview. Shiver's own pages never need to see a
    // token and a server's page must never see one, so this is a Rust-only api and the ACL has
    // nothing to gate.
    tauri_plugin::Builder::new(COMMANDS)
        .android_path("android")
        .build();
}
