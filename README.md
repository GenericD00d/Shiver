<p align="center"><img src="icon.png" alt="Shiver logo" width="160"></p>

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

- **No independent security audit has been done.** It handles your session tokens and, by default,
  your password. Both go in the OS keychain — or `EncryptedSharedPreferences` on Android — never in
  a file, but read the code before you trust it with either. Its internal reviews, and what each one
  found, are listed in [SECURITY.md](SECURITY.md#review-history); they are not an outside audit.
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
  `EncryptedSharedPreferences` on Android — never stored unencrypted. Keeping the password
  is the default, because it is what lets Shiver sign itself in again before a session expires
  (Sharkord's sessions last seven days and cannot be refreshed), in the background, so opening a
  server never waits on it. The session is handed to a server's
  page from memory and never written to the webview's storage. **Forget my password** in a server's menu
  removes it.
- **HTTPS only.** Shiver refuses to add a server, sign in to one, or open a socket over `http://`,
  with no exemption for localhost or a private address.
- **Unread badges per server**, cleared by opening it, with per-channel mutes excluded
- **A direct message inbox across servers**, ordered by its latest message, sent or received, and labelled
  with the server it is on. It lives on Shiver's own screen rather than inside a server's page,
  which is what keeps one server from being handed the name of everyone you talk to on the others
- **Per-channel mute that follows you** — Sharkord's own "Mute Channel" (0.0.27 and newer), kept
  in step across your devices and Shiver's background connections: no notification, no sound, no
  unread badge
- **A notification level per server** — every message, only mentions and direct messages, or
  only direct messages. A mention of a role you hold counts. Unread badges still count everything
- **Your colours** applied to Shiver and to each server's client. On the defaults Shiver restyles
  nothing, so servers look exactly as they do in a browser
- **A sound volume slider that goes to 250%**, for a notification tone that has to carry over a call
- **Sharkord's "open in the desktop app" links** (`sharkord://`) open Shiver's add screen with the
  server filled in, or the server itself if it is already in the rail. Shiver takes the scheme over
  from Sharkord's own desktop app, so install one or the other
- **Every server connects at launch**, so the inbox is complete the moment Shiver opens — and the
  servers you are not looking at cost a socket rather than a browser, because the core speaks
  Sharkord's protocol itself, read-only, to the ones it is not displaying

**Desktop**

- **One unified notification inbox** behind the bell, across every server, and a system
  notification for what arrives while Shiver is not in front (a direct message also flashes the
  taskbar)
- **Global voice controls** in the rail, reachable whichever server is on screen. One call at a
  time across all of them
- **A system-wide shortcut** for muting your microphone, and **Ctrl+1 to 9** and
  **Ctrl+Alt+Up/Down** to move between servers while Shiver is in front
- **Close to the tray** (its icon marked while anything is unread) and **start at login**, both
  off until you turn them on
- **A log file** of what Shiver and each server's page ran into, kept on your computer, for
  attaching to a bug report (Settings → About)
- **Speaks Sharkord's desktop api** (Sharkord 0.0.29 and newer), push-to-talk system-wide
  included: its notifications lead to the exact message (or thread reply) they are for, and its own
  keyboard shortcuts work system-wide for the server on screen. A server on a different version
  of it is marked in the rail, saying whether the server or Shiver needs updating

**Android** (8.0 or newer: older versions no longer get WebView security updates)

- **Two-level swipe** — Sharkord's channel drawer, then on past it to Shiver's rail (direct
  messages, your servers and their menus, settings), drawn over the server so that going back to it
  is instant: the page never unloads, so it never reconnects, and a call carries on
- **Edits in the text box** — editing a message opens it over the text box at the bottom, saved
  with Send
- **Push notifications while Shiver is closed**, over UnifiedPush (ntfy or another distributor), per
  server and off by default. Needs the companion plugin on that server

**The companion plugin** ([`plugin/`](plugin/README.md), optional)

Copied into a server's `plugins/` directory. It stores per-user settings on the server so they
follow you between devices: muted channels, custom statuses, and the push endpoints that let a
server wake your phone. It also draws usernames in their role colour (an admin can turn that off).
Everyone on the server gets statuses and colours, browser users included.

## Install

From the [latest release](https://github.com/GenericD00d/Shiver/releases/latest):

- **Windows**: `Shiver_<version>_x64-setup.exe`, or the `.msi`. The installers are unsigned, so
  SmartScreen warns the first time; updates after that are signed and checked (see
  [RELEASING.md](RELEASING.md)).
- **Android 8.0 or newer**: `Shiver_<version>_arm64.apk`, installed by hand.
- **Linux**: build it from source, as below.
- **macOS** isn't supported yet; [SECURITY.md](SECURITY.md) says what is missing.

The companion plugin is optional and goes on a server; [plugin/README.md](plugin/README.md) covers
installing it.

## Usage

- **Add a server** with **+** at the bottom of the rail. Type its address (https only), or follow
  Sharkord's "open in the desktop app" link. Shiver checks the server, shows its name and logo, and
  signs you in.
- **Move between servers** with the rail. On desktop, Ctrl+1 to 9 and Ctrl+Alt+Up/Down do the same.
  Drag a server to reorder it, or onto another to make a folder.
- **A server's menu** (right-click on desktop, hold on Android) has its notification level, signing
  in and out, and **Forget my password**.
- **Notifications** from every server collect behind the bell on desktop. Direct messages from every
  server are in the list at the top of the rail.
- **On Android**, swipe past Sharkord's channel drawer to reach the rail.

## Settings

Settings is the gear at the bottom of the rail:

- **Appearance**: colour presets, and the five colours Shiver applies to itself and to each server's
  client. On the defaults it restyles nothing.
- **Notifications**: the sound volume, up to 250%, and the platform's own switches.
- **Desktop only**: the microphone mute shortcut, closing to the tray, starting at login, camera and
  microphone answers (Windows), and the log folder under About.
- **Android only**: push notifications per server, over UnifiedPush.

## Known limitations

- **The DM inbox on Android, for a server Shiver holds no session for.** Android lists each
  server's conversations from Shiver's own connection to it, or from its page while it is on screen.
  A server you sign in to on its own page without "log in automatically" leaves Shiver nothing to
  connect with after a restart, so its conversations join the list once you have opened it.

## What it deliberately does not do

Each server's client runs pinned to its own origin with no way to call into Shiver: even Sharkord's
desktop api, which Shiver offers each page, only queues what the page says for the core to check. On desktop the
rail is Shiver's own webview; on Android it, and the screens it opens, are drawn over the server's
page in a WebView of its own, which loads nothing but Shiver's page and which the server's page cannot
read. Either way a server's page
is told nothing about your other servers.

## Building

Needs [Rust](https://rustup.rs), [Bun](https://bun.sh), and a platform toolchain — Visual Studio
Build Tools plus the WebView2 runtime on Windows, Xcode command line tools on macOS,
`webkit2gtk-4.1`, `libayatana-appindicator3` (the tray) and friends on Linux (the CI's
"Desktop (Linux)" job lists the packages). Android additionally needs JDK 17, the SDK and the NDK.

```bash
cd desktop && bun install && bun run app        # run it
cd desktop && bun run app:build                 # an installer

cd mobile && bun install
./node_modules/.bin/tauri android build --apk   # call the binary directly, not through bun
```

A release APK needs the signing key, and the build refuses to produce an unsigned one rather than
handing you an artifact that installs on your own phone and can never update anybody else's. Use
`--debug` to build without it. [`RELEASING.md`](RELEASING.md) covers both signing keys, what each
protects, and what is and is not recoverable if one is lost.

Working on the code: [AGENTS.md](AGENTS.md) has the checks to run and the rules not to break, and
[ARCHITECTURE.md](ARCHITECTURE.md) maps every module.

## The Shiver family

- **Shiver**, this repo: the desktop and Android client, and the companion plugin
- [shiver-server-manager](https://github.com/GenericD00d/shiver-server-manager): sets up and runs
  Sharkord servers, with the Shiver plugins, from a terminal menu or a web page
- [shiver-media-player](https://github.com/GenericD00d/shiver-media-player): a plugin for watching
  videos together in a voice channel
- [shiver-temp-share](https://github.com/GenericD00d/shiver-temp-share): a plugin for sharing large
  files with links that expire

## Licence

MIT. See [LICENSE](LICENSE).

