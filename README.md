# Shiver

A multi-server desktop and Android client for [Sharkord](https://github.com/sharkord/sharkord).

Sharkord's own client is excellent at one server. Shiver is for people in several. It does not
reimplement anything: it opens the server you are looking at in that server's own web client, speaks
Sharkord's protocol directly to the ones you are not, and adds only what a single-server client
cannot have — a rail of your servers, one notification inbox across all of them, and voice controls
that stay put when you switch. Nothing here requires changes to a server.

## A disclaimer worth reading

**This project is vibecoded.** Close to all of it — the Rust, the TypeScript, the Kotlin, the
commit-by-commit reasoning — was written by an LLM working from a person's direction, review and
bug reports, rather than typed by a developer.

What that means in practice:

- **No security audit has been done**, by anyone. It handles your session tokens and, by default,
  your password. Both go in the OS keychain, never in a file — but read the code before you trust
  it with either.
- **The installers are unsigned**, so Windows SmartScreen will warn on first run. Desktop updates
  are checked against a signing key built into the binary and will not install without it, so
  losing control of the GitHub account is not enough to ship code to anyone; Windows still has no
  reason to trust the installer you downloaded first.
- It is early software. Expect rough edges, and expect the shape of things to change.

It is offered as-is. If that trade is not one you want to make, a browser and Sharkord's own client
will serve you perfectly well.

## What it does

**Both clients**

- **A server rail** — add, remove, reorder by dragging, group into folders, per-server menu
- **Seamless sign-in.** Shiver signs in for you and the server opens straight into the app rather
  than onto a login page. Sessions and passwords live in the OS keychain, or
  `EncryptedSharedPreferences` on Android — never in a file, never in plaintext. Keeping the password
  is the default, because it is what lets Shiver sign itself back in when a session expires;
  Sharkord's last seven days and cannot be refreshed. **Forget my password** in a server's menu
  removes it
- **HTTPS only.** Shiver refuses to add a server, sign in to one, or open a socket over `http://`,
  with no exemption for localhost or a private address
- **Unread badges per server**, cleared by opening it, with per-channel mutes excluded
- **Per-channel mute** — dimmed in the channel list, no notification, no sound
- **Your colours** applied to Shiver and to each server's client. On the defaults Shiver restyles
  nothing, so servers look exactly as they do in a browser
- **A sound volume slider that goes to 250%**, for a notification tone that has to carry over a call
- **Every server connects at launch**, so the inbox is complete the moment Shiver opens — and the
  servers you are not looking at cost a socket rather than a browser, because the core speaks
  Sharkord's protocol itself, read-only, to the ones it is not displaying

**Desktop**

- **One unified notification inbox** behind the bell, across every server
- **Global voice controls** in the rail, reachable whichever server is on screen. One call at a
  time across all of them
- **A system-wide shortcut** for muting your microphone
- A **direct message inbox** across servers, labelled by account

**Android**

- **Two-level swipe** — Sharkord's channel drawer, then Shiver's rail over the top of it
- **Push notifications while Shiver is closed**, over UnifiedPush (ntfy or another distributor), per
  server and off by default. Needs the companion plugin on that server

**The companion plugin** ([`plugin/`](plugin/README.md), optional)

Copied into a server's `plugins/` directory. It stores per-user settings on the server so they
follow you between devices: muted channels, custom statuses, and the push endpoints that let a
server wake your phone. Everyone on the server sees statuses, browser users included.

## What it deliberately does not do

Each server's client runs pinned to its own origin with no way to call into Shiver. The only things
about your *other* servers that ever reach one are what a rail cannot be drawn without: a name, a
logo as image data, and an opaque id — **never an address**, and never who you talk to elsewhere.

## Building

Needs [Rust](https://rustup.rs), [Bun](https://bun.sh), and a platform toolchain — Visual Studio
Build Tools plus the WebView2 runtime on Windows, Xcode command line tools on macOS,
`webkit2gtk-4.1` and friends on Linux. Android additionally needs JDK 17, the SDK and the NDK.

```bash
cd desktop && bun install && bun run app        # run it
cd desktop && bun run app:build                 # an installer

cd mobile && bun install
./node_modules/.bin/tauri android build --apk   # call the binary directly, not through bun
```

## Layout

| Path | What it is |
| --- | --- |
| `desktop/` | The desktop client. Self-contained: its own dependencies, build and Rust crate |
| `mobile/` | The Android client, separate because it cannot share the desktop's shape — Android gives a window one webview |
| `plugin/` | The optional Sharkord companion plugin. Neither client's, used by both |
| `shared/` | Rust both clients depend on. Currently the Sharkord protocol itself, so there is one transcription of it rather than two that drift |

Inside each client: `src/` is Shiver's own UI, `bridge/` is the script injected into every Sharkord
page, and `src-tauri/src/` is the Rust core.
