"""Fails unless every place that carries Shiver's version agrees with the workspace's."""

import json
import pathlib
import re
import sys

try:
    import tomllib
except ModuleNotFoundError:  # tomllib is 3.11+; releases are cut on whatever python that machine has
    tomllib = None

root = pathlib.Path(__file__).resolve().parent.parent
manifest = (root / "Cargo.toml").read_text()

if tomllib:
    version = tomllib.loads(manifest)["workspace"]["package"]["version"]
else:
    section = re.search(r"^\[workspace\.package\]$(.*?)(?=^\[|\Z)", manifest, re.M | re.S)
    found_version = re.search(r'^version = "([^"]+)"', section.group(1) if section else "", re.M)

    if not found_version:
        sys.exit("Cargo.toml has no version under [workspace.package]")

    version = found_version.group(1)

found = {
    # Android reads its version from here and silently builds 1.0 without one
    "mobile/src-tauri/tauri.conf.json": json.loads((root / "mobile/src-tauri/tauri.conf.json").read_text()).get("version"),
    "desktop/src-tauri/tauri.conf.json": json.loads((root / "desktop/src-tauri/tauri.conf.json").read_text()).get("version", version),
}
wrong = {path: value for path, value in found.items() if value != version}

if wrong:
    sys.exit("\n".join(f"{path} has {value}, Cargo.toml has {version}" for path, value in wrong.items()))

print(f"every version is {version}")
