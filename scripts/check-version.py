"""Fails unless every place that carries Shiver's version agrees with the workspace's."""

import json
import pathlib
import sys
import tomllib

root = pathlib.Path(__file__).resolve().parent.parent
version = tomllib.loads((root / "Cargo.toml").read_text())["workspace"]["package"]["version"]
found = {
    # Android reads its version from here and silently builds 1.0 without one
    "mobile/src-tauri/tauri.conf.json": json.loads((root / "mobile/src-tauri/tauri.conf.json").read_text()).get("version"),
    "desktop/src-tauri/tauri.conf.json": json.loads((root / "desktop/src-tauri/tauri.conf.json").read_text()).get("version", version),
}
wrong = {path: value for path, value in found.items() if value != version}

if wrong:
    sys.exit("\n".join(f"{path} has {value}, Cargo.toml has {version}" for path, value in wrong.items()))

print(f"every version is {version}")
