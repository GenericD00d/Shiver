use std::{fs, path::Path};

/// `include_str!` needs the bundled scripts to exist before `bun run build:bridge` has necessarily
/// run, so a clean checkout gets placeholders.
fn main() {
    for name in ["bridge.js", "document-start.js"] {
        let path = Path::new("generated").join(name);

        if !path.exists() {
            fs::create_dir_all("generated").expect("failed to create the generated directory");
            fs::write(&path, "/* not built yet, run: bun run build:bridge */\n")
                .expect("failed to write a placeholder");
        }

        println!("cargo:rerun-if-changed=generated/{name}");
    }

    tauri_build::build()
}
