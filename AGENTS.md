# Shiver: agent guide

## What this repo is

Shiver is a multi-server client for [Sharkord](https://github.com/sharkord/sharkord): a Tauri 2
desktop app and a Tauri 2 Android app that share Rust crates and TypeScript, plus an optional
companion plugin that runs on Sharkord servers.

Each server's own web client runs in a webview pinned to that server's origin, with no IPC. A
bridge script injected into the page talks to the Rust core through `window.__SHIVER_*__` hooks
that the core polls or calls. The core also holds a read-only socket to each server that isn't on
screen.

Shiver handles session tokens and passwords, and injects code into every Sharkord page it shows, so
most changes cross a security boundary. Read SECURITY.md before changing anything in
[Rules that must not break](#rules-that-must-not-break).

## The Shiver family

The four Shiver repos, and the contracts between them, are in
[docs/standards.md](docs/standards.md#the-shiver-family). This repo's side of them:

- It publishes the companion plugin as `shiver-plugin-<version>.zip` with `SHA256SUMS.txt`. The
  manager installs that release.
- It owns the plugin id `shiver` and the plugin's actions (`onLoad` in `plugin/server/index.js`),
  which the bridges call through `shared/web/bridge/plugin.ts`.
- `SHARKORD_REF` in `scripts/check-sharkord.py` names the Sharkord release the family is tested
  against.

## Commands

```bash
scripts/check.sh --quick    # docs, versions, web and plugin unit tests: no build, seconds
scripts/check.sh            # everything CI runs except the Android build; run before pushing
```

The pieces, for running one at a time:

```bash
python3 scripts/check-docs.py                # the docs follow docs/standards.md
python3 scripts/check-version.py             # versions agree, and the changelog has the version
bun test shared/web desktop/bridge           # TypeScript unit tests
(cd plugin && node --test test/*.test.js)    # companion plugin tests
(cd desktop && bun install --frozen-lockfile && bun run build:bridge)   # before any cargo command
(cd mobile && bun install --frozen-lockfile && bun run build:bridge && bun run build:rail)
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
python3 scripts/check-sharkord.py            # Sharkord still has what the bridges match
```

The bridges are compiled into the Rust crates with `include_str!`, so build them before any cargo
command or the crates compile against placeholders. Run the desktop app with
`cd desktop && bun run app`. Building the Android app is in [README.md](README.md#building), and
releases are in [RELEASING.md](RELEASING.md).

## Layout

```
shared/shiver-core/      Rust: origin rules, registry, rail, http, login, sessions, links
shared/sharkord-client/  Rust: read-only Sharkord client (tRPC over WebSocket) and its watch loop
shared/web/              TypeScript and React that both apps use; bridge/ is shared page code
desktop/                 Desktop app: src-tauri/ is the Rust core, src/ the UI, bridge/ the page script
mobile/                  Android app: the same split, plus rail/ and plugins/ (Kotlin)
plugin/                  The companion Sharkord plugin, plain JavaScript with no build step
scripts/                 Checks: docs, versions, Sharkord drift
```

[ARCHITECTURE.md](ARCHITECTURE.md) maps every module, command, hook and exported item.

## Rules that must not break

SECURITY.md explains why each one matters.

| Rule | Enforced in | Tested by |
|---|---|---|
| Every webview boundary compares origins with `is_same_origin`, and every address goes through `normalize_origin` | `shared/shiver-core/src/origin.rs` | Its tests |
| https only, with no exemption for localhost or private addresses | `normalize_origin`, `sign_in` | `origin.rs` and `login.rs` tests |
| Shiver's own http calls never follow a redirect, and read bounded bodies | `shared/shiver-core/src/http.rs` | Review |
| Tokens and passwords live only in the OS keychain or the Android Keystore store, keyed by rail-entry id; never in `servers.json`, a log or an error message | `desktop/src-tauri/src/secrets.rs`, `mobile/plugins/tauri-plugin-shiver-secrets/` | Review |
| A session never reaches a webview's storage: the shim serves it from memory | `shared/web/session.ts` | `shared/web/session.test.ts` |
| The desktop bridge, with the session in its config, runs only in the top frame on its entry's own origin | `bridge_script` in `desktop/src-tauri/src/webviews/mod.rs` | `the_bridge_runs_only_in_the_entrys_top_frame` |
| Commands are taken only from Shiver's own webviews. Server pages get no capability | `only_shiver_chrome` in `desktop/src-tauri/src/lib.rs`, `only_home` in `mobile/src-tauri/src/lib.rs` | `shivers_own_webviews_stay_on_its_exact_origin` |
| What a page reports is a claim: size-capped, parsed, and never trusted beyond its own entry | `desktop/src-tauri/src/drain.rs`, `mobile/src-tauri/src/webview.rs` | Review |
| Values evaluated into a page are serialised with `serde_json`, never formatted by hand | Every `eval` in `desktop/src-tauri/src/` and `mobile/src-tauri/src/` | Review |
| A link a page hands over opens only after a native dialog, or a site trusted from that same server | `shared/shiver-core/src/links.rs` | Its tests |
| The Android seed is accepted only with the per-launch, per-origin key, and never leaves in a URL | `mobile/src-tauri/src/webview.rs`, `mobile/bridge/document-start.ts` | `seed_keys_are_per_origin`, `a_seed_never_leaves_in_a_url` |
| The plugin sends push wake-ups only to vetted public addresses, pinned, on port 443 | `isPrivateAddress`, `vetEndpoint` and `deliver` in `plugin/server/index.js` | `plugin/test/plugin.test.js` |
| The plugin's client half never turns stored text into markup | `plugin/client/index.js` | Review |

## Conventions

- **Errors** in both apps (`error.rs`) are user-facing strings: the shared `shiver_core::Error`,
  re-exported as `Core`, plus the platform's own. Never put a token, password or path in one.
- **The registry** (`servers.json`) holds no secrets and no logos. Android keeps logos in `icons/`.
  Secrets are keyed by rail-entry id, never by origin.
- **Registry edits** go through `store.update(|registry| ...)` (`RegistryStore`). It edits a copy,
  which replaces the registry only once it is written.
- **Rail and folder logic** lives in `shiver_core::rail`. The clients call `registry.rail().<op>()`.
- **Shared constants** stay in step between `shared/web/settings.ts` and `shiver_core::model`.
- **Android's generated project** (`mobile/src-tauri/gen/android/`) is mostly generated, but
  `MainActivity.kt`, `AndroidManifest.xml`, `res/xml/` and `res/values*/` are hand-written. Merge
  the output of `tauri android init`; never accept it whole.
- **Compatibility code** to delete once upgrading from those versions is no longer supported:
  - `push::migrate_tokens` and `ensure_push_tokens`, for Android 0.1.4 and older;
  - `remove_legacy_cache` in `shiver_core::store`, for 0.1.0 to 0.1.2;
  - the version-1 path in `bridge/plugin.ts`, for companion plugin 0.1.0;
  - `adoptOldStore` in the plugin, for Sharkord 0.0.24.
- **Comments** say why. Docs use British spelling.
- **Commits** have a plain-English subject saying what changed and a body saying why.

## When you change something

- **Adding, removing, renaming or moving** a module, command, hook or exported item: update
  ARCHITECTURE.md in the same commit.
- **Changing a trust boundary**, meaning anything in the table above or a new place where untrusted
  data comes in: update SECURITY.md's Trust boundaries table, and add a test.
- **Matching something new in Sharkord's page**, such as a test id, storage key or class: add it to
  `scripts/check-sharkord.py`.
- **Changing the companion plugin**: bump `plugin/manifest.json` and `plugin/package.json`
  together. The plugin's version is its own, not the apps'.
- **Changing what either app does**: add a line under `## Unreleased` in CHANGELOG.md. Version
  bumps are in RELEASING.md.
- **docs/standards.md** changes only as its last section describes.

## Further reading

- [ARCHITECTURE.md](ARCHITECTURE.md): every module, flow and contract
- [SECURITY.md](SECURITY.md): trust boundaries, scope and review history
- [RELEASING.md](RELEASING.md): versions, keys, building and verifying
- [plugin/README.md](plugin/README.md): installing and running the companion plugin
- [docs/decisions.md](docs/decisions.md): why things are the way they are
- [docs/standards.md](docs/standards.md): the standard all four Shiver repos follow
