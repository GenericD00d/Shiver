"""Fails if the eight places that carry Shiver's version stop agreeing.

Nothing derives this from anything else, so a release means editing eight files by hand — and the
two documents that told you which eight disagreed with each other and with the tree. `RELEASING.md`
said three places and listed four files; `SHIVER.md` said six. There are eight, and the two that
neither document mentioned are the shared crates.

**A missed one is not a build failure.** It is an installer that reports the wrong version, or —
on Android, where `versionCode` is derived from this — an APK that cannot upgrade the copy already
on the phone, which is discovered by a user rather than by a build.

The plugin is deliberately not in this list. It installs separately and is bumped when it changes,
which is why `plugin/manifest.json` sitting at 0.1.0 against an app at 0.1.3 is correct rather
than a drift.
"""

import json
import os
import re
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))

# Each entry is (path, how to read the version out of it). The Cargo manifests are read with a
# regex anchored to the `[package]` table, so a dependency's own `version = ` is never mistaken
# for the crate's.
JSON_FILES = (
    os.path.join("desktop", "package.json"),
    os.path.join("desktop", "src-tauri", "tauri.conf.json"),
    os.path.join("mobile", "package.json"),
    os.path.join("mobile", "src-tauri", "tauri.conf.json"),
)

CARGO_FILES = (
    os.path.join("desktop", "src-tauri", "Cargo.toml"),
    os.path.join("mobile", "src-tauri", "Cargo.toml"),
    os.path.join("shared", "shiver-core", "Cargo.toml"),
    os.path.join("shared", "sharkord-client", "Cargo.toml"),
)


def from_json(path):
    with open(os.path.join(ROOT, path), encoding="utf8") as handle:
        return json.load(handle).get("version")


def from_cargo(path):
    with open(os.path.join(ROOT, path), encoding="utf8") as handle:
        source = handle.read()

    package = re.search(r"^\[package\]\s*$(.*?)(?=^\[|\Z)", source, re.M | re.S)

    if not package:
        sys.exit("%s: no [package] table" % path)

    found = re.search(r'^version\s*=\s*"([^"]+)"', package.group(1), re.M)

    return found.group(1) if found else None


def main():
    found = [(path, from_json(path)) for path in JSON_FILES]
    found += [(path, from_cargo(path)) for path in CARGO_FILES]

    missing = [path for path, version in found if not version]

    if missing:
        for path in missing:
            print("%s: no version found" % path, file=sys.stderr)

        return 1

    versions = sorted({version for _, version in found})

    if len(versions) > 1:
        print("The version is not the same in all %d places:\n" % len(found), file=sys.stderr)

        for path, version in sorted(found, key=lambda pair: pair[1]):
            print("    %-44s %s" % (path.replace("\\", "/"), version), file=sys.stderr)

        print(
            "\nBump every one of them. Android derives versionCode from its own, so a copy left"
            "\nbehind ships an APK that cannot upgrade an installed Shiver. See RELEASING.md.",
            file=sys.stderr,
        )

        return 1

    print("all %d version fields agree (%s)" % (len(found), versions[0]))

    return 0


if __name__ == "__main__":
    sys.exit(main())
