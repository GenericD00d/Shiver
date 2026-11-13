# Decisions

Why Shiver is the way it is, oldest first. Security decisions are explained in full in
[SECURITY.md](../SECURITY.md#design-decisions-worth-knowing); entries here link to it rather than
repeat it.

## 2026-09-13: Show each server's own client, and reimplement nothing

**Why:** Sharkord's own client is good at one server, and a reimplementation would always lag
behind it. Shiver adds only what a single-server client cannot have: the rail, one inbox, and voice
controls that stay put.

**Consequences:** the bridges depend on details of Sharkord's page: test ids, storage keys and
classes. `scripts/check-sharkord.py` checks them at the pinned Sharkord commit in CI, and against
Sharkord's latest every week (`.github/workflows/sharkord.yml`).

## 2026-09-13: https only, with no exemption for localhost or private addresses

**Why:** every boundary in Shiver is keyed on an origin, and an http address would put the password
and session on the wire in the clear.

**Consequences:** a test server needs a certificate. `normalize_origin` refuses everything else.

## 2026-09-19: Release keys stay on one machine, and an unsigned release APK can't be built

**Why:** the Android SDK, JDK 17 and two signing keys are needed, and keeping the keys off CI means
fewer moving parts and nothing for a workflow to leak.

**Consequences:** the keys are a single point of failure. RELEASING.md covers backing them up, and
what happens if one is lost or leaked.

## 2026-09-23: A codebase map for agents

**Why:** most of the code is written by agents, who search better with a map of every module,
command and hook.

**Consequences:** ARCHITECTURE.md is updated in the same commit as anything it lists.

## 2026-09-24: Servers are checked against Mozilla's roots only

**Why:** see [SECURITY.md](../SECURITY.md#design-decisions-worth-knowing).

**Consequences:** a server behind a private CA does not connect, even if the machine trusts it. The
desktop updater, which has its own client, uses the OS trust store.

## 2026-09-25: The companion plugin's server half is one file

**Why:** a server that reloads only a plugin's entry file could otherwise run it against stale
copies of the others.

**Consequences:** `plugin/server/index.js` is long and divided into sections. A test fails if it
imports anything local.

## 2026-11-13: Follow the Shiver standard

**Why:** four repos with one maintainer, and agents doing most of the writing, need the same files,
checks and security baseline everywhere.

**Consequences:** [docs/standards.md](standards.md) is copied into all four repos.
`scripts/check-docs.py` checks the structure in CI. This file replaces decisions that were only in
commit messages.

## 2026-11-13: Standard version 2: dependency audits run weekly too

Security baseline rule 4 now reads "on every push and weekly". GO-2026-6859 and RUSTSEC-2026-0538 both surfaced only because something happened to be pushed, and the manager's v0.3.1 shipped with the Go one. Each repo runs its audit in `.github/workflows/audit.yml`, on pushes, pull requests, a Monday schedule and by hand; a failed scheduled run emails the owner. Copied to all four repos the same day, with the new `STANDARDS_SHA256`.

## 2026-11-13: Disconnect on a server's page keeps Shiver signed out of it

**Why:** choosing Disconnect in Sharkord's server menu is the user asking to leave, so Shiver must
not undo it. Forgetting the password as well would make signing in again harder for no gain, since
the user said nothing about the password.

**Consequences:** `ServerEntry.disconnected` stops every automatic sign-in for that entry, across
restarts, until the user signs in again with Shiver's form or on the page. The session is dropped
and the page rebuilt without it. The bridge can only tell a Disconnect from a refused session by
the menu item being chosen shortly before, so a refusal within two minutes of choosing it and then
cancelling is taken as a Disconnect too: the user then signs in by hand. A page can claim a
Disconnect at any time, which only stops Shiver signing that one entry in.

## 2026-11-13: Fixed CI runners and NDK, and a check on the generated Android project

`ubuntu-latest`, `windows-latest` and the image's newest NDK let a build change while the repo
didn't, and `tauri android init` in CI moved the Android project to AGP 9.3.1 unseen, which broke
the Android build from 7 November. Runners now have fixed labels, the NDK is installed at a pinned
version with `sdkmanager`, and a step fails when the generated AGP, Kotlin plugin or Gradle versions
differ from the ones written in the workflow. RELEASING.md says what to do when they move.

## 2026-11-13: The Android seed key is an HMAC

The key a `#shiver-seed` fragment must carry was SHA-256 of the per-launch secret followed by the
origin. That wasn't exploitable, since an origin can't carry the padding a length extension needs,
but HMAC-SHA256 is the textbook construction. `seed_key_for` and `seedKeyFor`
(`shared/web/seed-key.ts`) now compute HMAC-SHA256 of the origin keyed with the secret, with
one shared test vector, and `document_start` serialises the key with `serde_json`, as every value
evaluated into a page is.

## 2026-11-13: Disconnect on a server's page keeps Android signed out of it too

As on the desktop: `ServerEntry.disconnected` is saved with the registry, the bridge reports the
user's own Disconnect (`shared/web/bridge/disconnect.ts`, now shared) as `userDisconnected` and
signing in as `signedIn`, read with the mutes. `inbox::disconnected_by_user` sets the flag and drops
the stored session, keeping the password. While it is set, `session_for` and `renew` give no
session, so the page gets no seed and `sync` no connection, and the page's own reconnect
(`reconnect.ts`) leaves the connect form alone. Signing in with Shiver's form, or on the page,
clears it. Not yet tried on a device.

## Open items

- **macOS builds.** wry grants every page the camera and microphone there, so a macOS build must not
  ship until Shiver answers those requests itself. See SECURITY.md.
- **Disconnect on Android.** The desktop app keeps a server signed out after Disconnect on its page
  (decision of 2026-11-13). The Android app still signs back in. The same flag and bridge report
  are needed in `mobile/`, tested on a device.
- **Android SDK 37 and Tauri v3.** The Tauri CLI's new template sets `compileSdk` and `targetSdk`
  to 37 and turns off AGP 9's built-in Kotlin and new DSL in `gradle.properties` "until Tauri v3".
  Move to SDK 37 when Google Play requires it, testing on a device, since a new target SDK changes
  runtime behaviour; drop those flags when Tauri v3 does. The plugins' build files already use
  `kotlin { compilerOptions }`.
- **The Android seed key** is SHA-256 of the per-launch secret followed by the origin. HMAC-SHA256
  is the textbook construction. The current one isn't exploitable, because an origin cannot contain
  the padding a length-extension attack would need. Changing it needs a test on a device.

## Bug log

### A background renewal could sign a logged-out server back in (fixed)

Found by the 2026-10-10 audit. A renewal now stores its session under `CREDENTIALS`, the lock that a
log-out or removal holds, and only while the entry is still signed in (`cbc672b`).

### Unread floors only ever rose (fixed)

A channel read after its floor was taken counted nothing new until its old backlog piled up again.
A read below the floor now lowers it (`settle_floor`, `cbc672b`).

### Release APKs carried every Android architecture (fixed)

Symlinks a full build left in `jniLibs` reached later arm64 builds. `abiFilters` pins release APKs
to `arm64-v8a` (`4221156`).

### An Android build could quietly claim to be version 1.0 (documented)

With no `version` in `mobile/src-tauri/tauri.conf.json`, Gradle falls back to 1.0.
`scripts/check-version.py` checks the version is there, and RELEASING.md says how to verify an APK.

### Disconnect on a server's page was undone at once (fixed)

Sharkord's Disconnect removes the session from the page, which looks just like a refused session,
so recovery signed straight back in with the stored password. The desktop bridge now reports the
user's own Disconnect, and the core keeps that entry signed out until the user signs in again
(`session::disconnected_by_user`). Android does the same since (`inbox::disconnected_by_user`).

### The Android build failed after the Tauri CLI moved to AGP 9 (fixed)

From 0.2.7 the CLI's template generates the Android Gradle plugin 9.3.1, which compiles Java for 11
unless told otherwise, while `app/build.gradle.kts`, kept from the 0.1.0 template, set only
Kotlin's target, 1.8. Kotlin's plugin refuses the mismatch, so CI's Android build, which generates
the project afresh, failed from 7 November. The app now sets both to 1.8, as the new template does.

### The cap on sign-in attempts could reset at the connect form (fixed)

`apply_page_state` forgot a server's recovery attempts whenever a page was ready and not signed
out. Sharkord's connect form is ready too, and isn't reported signed out when the page's own
storage has `sharkord-auto-login` set, so the cap in `session.rs` could reset without anyone
signing in. Only a page reporting `signedIn` confirms a recovery now (`recovery_confirmed`,
tested in `drain.rs`).
