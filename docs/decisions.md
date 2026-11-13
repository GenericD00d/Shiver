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

## Open items

- **macOS builds.** wry grants every page the camera and microphone there, so a macOS build must not
  ship until Shiver answers those requests itself. See SECURITY.md.
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
