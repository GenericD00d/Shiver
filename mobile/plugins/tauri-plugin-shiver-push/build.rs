const COMMANDS: &[&str] = &[];

fn main() {
    // No commands, for the same reason `shiver-secrets` declares none: nothing here is a webview's to
    // call. An endpoint is a capability to wake this phone, and a server's page must not be able to
    // ask for one. The core registers, and hands the endpoint out through the bridge payload.
    tauri_plugin::Builder::new(COMMANDS)
        .android_path("android")
        .build();
}
