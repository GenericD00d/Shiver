use std::{fs, path::Path};

fn main() {
    // lib.rs pulls the bridge in with include_str!, which is resolved before `bun run build:bridge`
    // has necessarily run. an empty placeholder keeps a clean checkout compiling.
    let bridge = Path::new("generated/bridge.js");

    if !bridge.exists() {
        fs::create_dir_all("generated").expect("failed to create the generated directory");
        fs::write(
            bridge,
            "/* bridge not built yet, run: bun run build:bridge */\n",
        )
        .expect("failed to write the bridge placeholder");
    }

    println!("cargo:rerun-if-changed=generated/bridge.js");

    tauri_build::build()
}
