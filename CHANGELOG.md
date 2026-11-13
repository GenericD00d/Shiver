# Changelog

Shiver's apps share one version. The companion plugin's own version is noted where it changed.
Entries up to 0.2.8 were reconstructed from the commit history on 2026-11-13.

## Unreleased

### Changed

- CI runs on fixed runner images (`ubuntu-24.04`, `windows-2025`) with a pinned Android NDK
  (30.0.16248370), and fails if a Tauri CLI update changes the generated Android Gradle plugin,
  Kotlin or Gradle versions, so the build can't change while the repo doesn't.
- The docs follow the Shiver standard ([docs/standards.md](docs/standards.md)): `AGENTS.md` is the
  entry point for agents, and SECURITY.md has a trust-boundaries table and a review history.
  `scripts/check-docs.py` checks the structure in CI, and `scripts/check.sh` runs every local check.
- `scripts/check-version.py` also requires a changelog entry for the version.

### Fixed

- On desktop, choosing Disconnect on a server's own page now keeps Shiver signed out of that server,
  after a restart too, until you sign in again there or with Shiver's form. The saved password is
  kept. Shiver used to sign straight back in. Android still does, for now.

## 0.2.8 (2026-11-12)

### Changed

- Server names and logos refresh by themselves. Android applies a theme change at once.

## 0.2.7 (2026-11-07)

### Added

- Sharkord 0.0.30 support, push-to-talk system-wide, and five things borrowed from Sharkord's
  desktop app.
- Settings in the shape of Sharkord's own: a bar with Back, sections with icons, cards, switches
  and a save bar.

### Changed

- Refactoring across both apps; the companion plugin's two per-user throttles became one helper
  (plugin 0.3.2).

## 0.2.6 (2026-10-31)

### Added

- Sharkord 0.0.29's desktop api.

### Changed

- Cord-like's accent text is its text white.

## 0.2.5 (2026-10-31)

### Added

- Sharkord's desktop api; `sharkord://` links open the add screen; translated notification titles
  are read.
- Colours: an accent text colour, presets, accent icons and rail rings, and a sidebar colour for the
  channel list, member list and top bars.
- Long pasted lines wrap in the composer.

### Security

- rustls, rustls-webpki, quinn-proto and the webpki roots taken past their advisories; Vite 7.3.7.

## 0.2.4 (2026-10-17)

### Changed

- Follows Sharkord 0.0.28: direct messages from the join, Sharkord's own channel mute, and role
  mentions.

## 0.2.3 (2026-10-16)

### Added

- Android: menus, settings and direct messages drawn over the server page; edits open over the
  composer.

## 0.2.2 (2026-10-11)

### Fixed

- Unread floors that only ever rose, and other findings of the 2026-10-06 and 2026-10-10 audits.

### Security

- A background session renewal finishing during a log-out could sign the server back in.

### Changed

- Large modules split by concern on both platforms.

## 0.2.1 (2026-10-04)

### Fixed

- Desktop retries a server that cannot be reached instead of giving up on it.
- Android's inbox lists the conversations of the server on screen from its own page.

## 0.2.0 (2026-10-01)

### Added

- Direct messages ordered by their latest message, sent or received, on both platforms.
- One React rail, menu model, DM list and settings sections shared by both clients.
- Android's quick rail in a WebView of its own, with direct messages, adding a server and settings.
- Custom statuses and role colours moved into the companion plugin.

### Security

- A whole-repo review added a bridge origin guard.

## 0.1.8 (2026-09-29)

### Added

- A notification level per server: everything, mentions and direct messages, or direct messages
  only.
- Desktop: rail shortcuts, system notifications while Shiver isn't in front, closing to the tray,
  starting at login, and a single instance.
- Sessions renew in the background.

### Security

- The gaps the 2026-09-28 security review found are closed. Links are trusted per site and per
  server. Android wipes secrets after use, and the plugin rate-limits its reads (plugin 0.2.2).

## 0.1.7 (2026-09-28)

### Added

- Android: pictures can be pinched, panned and double-tapped.

### Fixed

- A rate limit is a reason to wait, not a refused session.
- Release APKs are pinned to arm64.

## 0.1.6 (2026-09-26)

### Security

- The fixes from the in-depth security analysis:
  - a native dialog before a page's link opens, naming the site first;
  - camera, microphone and downloads only for the server on screen;
  - Android seeds accepted only with a per-launch key;
  - the desktop bridge only in its entry's top frame;
  - plugin push delivery pinned to port 443, with a cap on deliveries in flight;
  - invisible characters dropped from server text.

### Changed

- The companion plugin's server half is one file.
- Android shows Sharkord's reconnect as a spinner above the chat box.

## 0.1.5 (2026-09-24)

### Added

- CI for the workspace, including an Android build.
- `ARCHITECTURE.md`, a codebase map for agents.

### Security

- Plugin: push delivery pinned, SSRF gaps closed, row writes serialised.

## 0.1.4 (2026-09-22)

### Security

- The 46 findings of the first source review, and the second pass's, fixed.

### Added

- Each rail entry has its own browser profile.
- A release APK can no longer be built unsigned.

## 0.1.3 (2026-09-19)

### Added

- Settings sections and an update prompt.
- Tests, type checks and the rail guard run on every push.

## 0.1.2 (2026-09-16)

### Added

- Servers that are not on screen are watched over sockets.

### Fixed

- Unread badges.

## 0.1.0 (2026-09-13)

- The first release. 0.1.1 is the same commit.
