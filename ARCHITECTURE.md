# Shiver — codebase map (for agents)

> Read this before searching the tree, and update it in the same commit whenever you add, remove,
> rename or move a module, command, hook or exported item.

Multi-server client for Sharkord. Two apps (Tauri 2 desktop, Tauri 2 Android), shared Rust crates,
shared TS, and a Sharkord server plugin. Server pages run in webviews **with no IPC**: bridges talk to
the core through `window.__SHIVER_*__` hooks that the core polls or calls.

## Layout

```
icon.png                   1024px master icon, the input to `tauri icon` for both apps
Cargo.toml                 workspace (rust 1.77.2): shared/*, desktop/src-tauri, mobile/src-tauri, mobile/plugins/*
shared/shiver-core/        Rust: origin rules, registry store, rail, http, login, probe, rate limit
shared/sharkord-client/    Rust: read-only Sharkord tRPC-over-WebSocket client, watch loop, add-server check
shared/web/                TS for both frontends (relative imports; no .tsx: react won't resolve here)
shared/web/bridge/         TS for both bridges
desktop/src-tauri/         desktop core (crate `shiver`); capabilities/: event listen only, Shiver's own webviews
desktop/src/               desktop UI (React): shell, bell, popup — one bundle, picked by webview label
desktop/bridge/            desktop bridge (IIFE, include_str!'d as an init script)
mobile/src-tauri/          Android core (crate `shiver-mobile`); capabilities/: event listen only, own page
mobile/src/                Android UI (React)
mobile/bridge/             mobile bridge (inside server pages; knows only that server); document-start.ts runs first
mobile/plugins/            tauri-plugin-shiver-push (UnifiedPush), tauri-plugin-shiver-secrets (Keystore)
plugin/                    Sharkord companion plugin (server/ + client/, plain JS, node tests)
scripts/check-version.py   checks the workspace and tauri.conf.json versions agree
.github/workflows/checks.yml  CI
```

## Checks (run before pushing)

`cargo fmt --all --check` · `cargo clippy --workspace --all-targets -- -D warnings` ·
`cargo test --workspace` · `bunx tsc --noEmit` in `desktop` and `mobile` · `bun test shared/web` ·
`node --test test/*.test.js` in `plugin` · `python3 scripts/check-version.py` ·
`bun run build:bridge` in each app (bridges must build before the Rust crates compile).

## Conventions

- Both apps' error types (`error.rs`) are user-facing strings, serialised as a string: the shared
  `shiver_core::Error` (re-exported as `Core`) plus the platform's own. Never put tokens, passwords
  or paths in messages.
- The registry (`servers.json`) holds no secrets, and no logos (Android keeps those in `icons/`). Secrets: keychain (desktop `secrets.rs`),
  `tauri-plugin-shiver-secrets` (mobile). Keys are rail-entry ids, never origins.
- Registry edits go through `store.update(|registry| ...)` (`RegistryStore`): runs on a copy, which
  replaces the registry only once it is written.
- Rail/folder logic lives in `shiver_core::rail`; the clients call `registry.rail().<op>()`.
- Keep shared constants in step: `shared/web/settings.ts` ↔ `shiver_core::model`.
- `mobile/src-tauri/gen/android/` is mostly generated, but `MainActivity.kt` (insets, back handling,
  `MediaGate`: camera and microphone only for the server on screen, asked once per server; a still of the server page
  as a swipe right or back leaves it, kept in memory for `takeStill`),
  `AndroidManifest.xml` and `res/xml/` + `res/values*/` are hand-written: re-running
  `tauri android init` must be merged, not accepted.
- Compatibility code, to delete once upgrading from those versions is no longer supported:
  `push::migrate_tokens`/`ensure_push_tokens` (Android ≤0.1.4 registered push under entry ids),
  `remove_legacy_cache` in `shiver_core::store` (`messages.json`, written by 0.1.0–0.1.2), the
  version-1 path in `bridge/plugin.ts` (companion plugin 0.1.0, shipped with Shiver ≤0.1.4) and
  `adoptOldStore` in the plugin (its settings file from Sharkord 0.0.24).
- Human docs, not covered here: `README.md` (features, building), `RELEASING.md` (signing keys,
  releases), `SECURITY.md` (reporting), `plugin/README.md` (installing the plugin).

## shared/shiver-core (`shiver_core`)

| module | items |
|---|---|
| `error` | `Error` {InvalidOrigin, Storage, Unreachable, NotSharkord, Refused, InvalidInput, UnknownServer, UnknownFolder}, `Result` |
| `origin` | `normalize_origin`, `is_same_origin` (the one webview-boundary comparison) |
| `store` | `Store<R>` (`load`, `registry`→`ReadGuard`, `edit`); `LockExt::locked` (poison-tolerant lock). An edit applies only once written, and one that changes nothing is not written; readers never wait on the disk; atomic writes, owner-only on Unix |
| `model` | `Folder`, `MutedChannel`, `DEFAULT_THEME_COLOR`, `DEFAULT_ACCENT_COLOR`, `MAX_SOUND_VOLUME`, `default_true`, `default_sound_volume`, `sanitised_color`, `sanitised_optional_color`, `rgb`, `theme_payload`, `muted_for`, `normalized_mutes`, `set_muted_for` |
| `rail` | `RailServer` trait + `rail_server!(Type)` macro; `registry!(Registry, Server)` (that plus `server`, `server_mut`, `muted_for`, `set_muted_for`, `next_position`, `rail`); `RailRef {kind,id}`; `next_position`; `Rail {servers, folders}`: `create_folder`, `rename_folder`, `set_folder_expanded`, `delete_folder`, `set_server_folder`, `place`, `reorder`, `reorder_servers`, `prune_folders` |
| `http` | `client()` (pooled, no redirects), `bytes_within_limit`, `MAX_BODY` |
| `login` | `sign_in` (`POST /login` → token) |
| `text` | `presentable` (server words made safe to show), `clamp` (bounded, invisible marks dropped) |
| `probe` | `ServerInfo`, `fetch_info` (`GET /info`, name and description cleaned and bounded), `fetch_icon` (data URI) |
| `limit` | `Openings`: `take(key, n)` (5/s, 10/10s per key), `grant`, `forget` |
| `links` | links a page asks to open: `site` (http(s) only, none for a link carrying credentials), `question` (names the site first), `trust` (the bounded list of sites that open without asking) |
| `hash` | `java_string` (Java `String.hashCode`) |

## shared/sharkord-client (`sharkord_client`)

- Types: `Joined` (join payload: read states, user names, `plugin_version`…), `DirectMessage`,
  `NewMessage` (`is_own`, `author`, `body`), `Event`, `Error` (`TooLarge`, `Refused` matter; converts
  into `shiver_core::Error`), `readable_size`.
- Watch loop: `watch(key, impl Watcher)`; `Watcher` trait: `target()→Option<Target{origin,token,accept_any_size}>`,
  `joined`, `event`, `refused(n)→retry now?`, `too_large`, `disconnected`; reconnects with backoff.
- Unread math: `set_unread`, `apply_delta`, `unread_total`.
- `check_server(origin, identity, password, &CheckedSessions) -> ServerCheck {info, plugin}` (plugin: absent =
  not asked); `CheckedSessions::take` hands `add_server` the check's session for the same credentials.

## desktop/src-tauri (`shiver`)

| module | role / key items |
|---|---|
| `lib` | `run()`: plugins (`RustOnly`: a plugin without its page script, for the dialog plugin, which would replace `alert`/`confirm` in server pages), state, command registration (`only_shiver_chrome`: commands from Shiver's own webviews only), startup |
| `commands` | all `#[tauri::command]`s for shell/bell/popup: registry & servers (`list_registry`, `check_server`, `add_server`, `remove_server`, `log_out_server`, `sign_in_server`, `forget_password`, `refresh_server_info`, `set_accept_any_size`, `reset_media_permissions`), rail (`reorder_servers`, `reorder_rail`, `create_folder_with`, `rename_folder`, `delete_folder`, `set_server_folder`, `set_folder_expanded`, `show_folder_menu`, `show_server_menu`), navigation (`select_server`, `prepare_server`, `show_shell`, `exit_dm_split`, `open_dm`, `open_message`, `select_channel`), voice (`voice_status`, `voice_control`), settings (`app_version`, `get_settings`, `update_settings`, `forget_trusted_links`), feed (`list_notifications`, `list_dms`, `feed_summary`, `unread_counts`, `mark_server_read`, `mark_notifications_read`, `clear_notifications`, `set_channel_muted`), popup (`toggle_popup`, `close_popup`, `dismiss_popup`) |
| `model` | `ServerEntry` (`label`), `Settings` (`sanitised`, `pages_kept`), `Registry` (`registry!` helpers), `*_PAGES_KEPT` |
| `store` | `Store` alias, `RegistryStore::update`, `load` |
| `webviews` | window/webview layout and page lifecycle: labels (`MAIN_WINDOW`, `SHELL_WEBVIEW`, `OVERLAY_WEBVIEW`, `POPUP_WEBVIEW`, `webview_label`, `is_shiver_chrome`), `ActiveServer` (`is_on_screen`), `create_main_window`, `main_window`, `chrome_webview` (Shiver's own webviews, pinned to its pages), `show_shell_only`, `set_popup_open`, `set_page_fullscreen`, `relayout`, `show_server`, `preload_server`, `trim_pages`, `close_server`, conversations in the server's own page beside the DM list (`show_conversation`, `end_conversation`), profiles (`discard_profiles`, `prune_profiles`; a directory each, a data store on macOS 14+), pushes into pages (`push_visibility`, `push_theme`, `push_voice_lock`, `push_muted`, `run_voice_action`, `mark_all_read`), `ask_to_open` (a native dialog before a page's link opens), `Openings` |
| `drain` | polls pages' `__SHIVER_DRAIN__` (on screen, connecting or in a call every 0.75 s, the rest every ~2 s; an unchanged page answers null); events `shiver://feed|dm-failed|server-ready|voice|signed-out|status|open-message` (`VOICE_EVENT`, `OPEN_MESSAGE_EVENT` also sent by commands); `Readiness`, `ServerStatus`, `Broadcast`, `spawn`, `notify_feed_changed` |
| `watch` | core sockets to servers without an open page (`sharkord_client::watch`): `sync`, `restart`, `forget`, `Missed`, `ReadStates`, `Plugins`, `Reported`, `Watcher`, `channel_read`, `mark_read`, `publish_floor` |
| `feed` | `Feed` (notifications + DMs): `push`, `push_update`, `set_dms`, `mark_*`, `clear`, `forget_entry`, `unread_*`, `summary`; `FeedSummary` (the feed event's payload), `Notification`, `DmEntry`, `DmChannel`, `DrainResult`, `RawNotification`, `QueuedMute` |
| `session` | `Recovery` + `recover`: re-sign-in when a page's seeded session is refused |
| `secrets` | `Secret` (Session/Password) in the OS keychain, read as `SecretString` (zeroised); `*_off_thread` helpers |
| `jwt` | `needs_refresh`, `is_live` (reads `exp` only) |
| `voice` | `VoiceState` (one call across servers, started only by the page on screen), `VoiceStatus`, `VoiceSnapshot` |
| `hotkey` | global mic-mute shortcut `apply` |
| `badge` / `badges` | taskbar badge `refresh`; `channel_viewed`, `is_on_screen` |
| `permissions` | camera/mic and downloads for pages: `gate` (WebView2: on-screen page only; camera/mic after a per-server yes in a native dialog, nothing saved in the profile), `forget_consents` |
| `update` | signed self-update: `start` (announces `shiver://update`), `check_for_update`, `install_update`, `available_update`, `skip_update`, `open_repository`, `Available` |

Page hooks (desktop bridge ↔ core): `__SHIVER_DRAIN__` (page→core queue) and core→page
`__SHIVER_SET_THEME__`, `_SET_MUTED__`, `_SET_HIDDEN__`, `_CONVERSATION__`,
`_SELECT_CHANNEL__`, `_MARK_ALL_READ__`, `_VOICE__`, `_SET_VOICE_LOCK__`, `_SET_SOUND_VOLUME__`,
`_SET_ATTACHMENT_CARDS__`, `_SET_READ_FLOOR__`.

## mobile/src-tauri (`shiver-mobile`)

| module | role / key items |
|---|---|
| `lib` | `run()`, `only_home` (commands refused unless Shiver's own page is on screen), page loads Shiver did not open sent home, `ask_to_open` (a native dialog before a page's link opens); `pub use sharkord_client as sharkord` |
| `commands` | `list_registry`, `probe_server`, `check_server`, `add_server`, `remove_server`, `refresh_server_info`, `server_icons`, `server_still`, `set_accept_any_size`, `log_out_server`, `sign_in_server`, `forget_password`, `forget_sessions`, `session_states`/`SessionStates` (signed out, password kept, problems, plugins), push (`push_status`, `set_push_server`, `set_push_distributor`; `PushStatus`, `PushServer`), `unread_counts`, `list_dms`, `select_server`, `app_version`, `update_settings`, `forget_trusted_links`, rail (`reorder_rail`, `create_folder_with`, `set_server_folder`, `rename_folder`, `delete_folder`, `set_folder_expanded`) |
| `model` | `ServerEntry` (+`push_token`, `retired_push_endpoints`), `Settings`, `Registry` (`registry!` helpers, `entry_for_push_token`, `ensure_push_tokens`) |
| `store` | as desktop |
| `icons` | logos as files (`icons/<entry id>`, a `data:` uri each): `save` (none deletes), `load`, `restore` (prune the gone, fetch the missing) |
| `inbox` | core sockets for servers not on screen + secret storage: `Inbox` (tokens, problems, plugins, dms, unread, signed_out, baselines), `sync`, `restart`, `restore`, `remember_session`, `remember_password`, `forget_password`, `forget_everywhere`, `harvest_token`, `replace_mutes`, `watch_mutes`, `collect_dms`, `DmEntry`, `INBOX_EVENT` |
| `webview` | the single webview (`main_window`): `Showing` (home, current server and whether it loaded, a stray page from history, pending DM user; `at_home`), `show_server` (drops any still), `take_still` (the still `MainActivity` took of the page left, once), `show_failed` (back to Shiver's page with `#failed=<id>`), `go_home`, `emit_home` (events only while Shiver's page is up), `without_seed`, `install_bridge`/`PageContext` (that entry's own config only), `read_mutes` (the page's mutes and outside links), navigation guard (`is_allowed`, `navigation_allowed`; allows only, since Android also asks it for frames and cancelled navigations), arrival on load (`is_home`, `landed_home`), `background_color`, `document_start` (bundle wrapped with the per-launch seed secret; `seed_key_for` derives each origin's key), `Openings` |
| `push` | UnifiedPush per chosen server: `Push`, `start`, `register_wanted`, `set_wanted` (turning off retires the endpoint; the page clears it with the plugin), `unregister`, `migrate_tokens`, `PUSH_EVENT` |
| `update` | notify-only: `start` (announces `shiver://update`), `check_for_update`, `update_available`, `skip_update`, `open_releases`, `open_repository` |

Page hooks (mobile): `__SHIVER__` (config), `__SHIVER_MUTED__`, `__SHIVER_OPEN__`, `__SHIVER_BACK__`,
`__SHIVER_MOBILE_INSTALLED__`, `__SHIVER_SESSION_SHIM__`.

Android plugins: `PushExt` (`distributors`, `set_distributor`, `register`, `unregister`, `on_event`,
`PushEvent`); `SecretsExt` (`set`, `get`, `remove`, `keys`, `wipe_origin`: that origin's webview storage and camera/mic consent). Kotlin in `android/src/main/java`.

## shared/web (TS)

| file | exports |
|---|---|
| `types.ts` | `Folder`, `MutedChannel`, `ServerInfo`, `ServerCheck` (re-exported by each app's `types.ts`) |
| `settings.ts` | `DEFAULT_THEME_COLOR`, `DEFAULT_ACCENT_COLOR`, `DEFAULT_RAIL_COLOR`, `MAX_SOUND_VOLUME` |
| `rail.ts` | `initials`, `byPosition`, `membersOf` |
| `colors.ts` | `automaticTextColor`, `lift` |
| `theme.ts` | `applyTheme` (`--shiver-*` vars on Shiver's own pages) |
| `session.ts` | `installSessionShim`, `takeSeedFromLocation`, `AUTO_LOGIN*` (session kept off disk) |
| `bridge/dom.ts` | `ensureStyle`, `defineHook`, `onDomSettled` (hands callbacks what changed), `touched`, `isTopFrame`, `whenDocumentReady`, `openMenuOnScreen`, `addedMenu`, `installExternalLinks` |
| `bridge/sharkord.ts` | Sharkord store types, test-id selectors (`SIDEBAR`, `CHANNEL_ITEM`, `DM_ITEM`…), `sharkordStore`, `watchStore`, `rowName`, `channelOfRow`, `markAllChannelsRead`, `installMuteStyles`, `paintMuted`, `addMuteItem`, `addMenuItem`, `closeDialog` (Sharkord's topmost open dialog, as Escape), `SHIVER_PLUGIN_ID` |
| `bridge/plugin.ts` | `callPlugin`, `waitForPlugin`, `syncMutesWithPlugin`, `pushMutesToPlugin`, `storeReadFloor` |
| `bridge/features.ts` | `installSoundVolume`, `installAttachmentCards`, `installVoiceColors`, `installRoleColors`, `installStatusButton` |
| `bridge/theme.ts` | `ShiverTheme`, `applyPageTheme` |

## Frontends

- **desktop/src**: `main.tsx` (label → `App` | `Bell` | `NotificationsPopup`), `api.ts` (`api.*` invoke
  wrappers, `errorMessage`), `events.ts` (`EVENTS`, `useCoreEvent`), `types.ts` (`*_PAGES_KEPT`), `sounds.ts` (`playNotificationSound`), `components/`:
  `ServerRail`, `AddServerPanel`, `SignInPanel`, `SettingsPanel`, `HotkeyField`, `DirectMessagesPanel`,
  `NotificationList` (`relativeTime`), `RenameFolderPanel`, `RemoveServerPanel`, `ConnectingPanel`, `WelcomePanel`,
  `VoiceTile`, `UpdateNotice`, `icons`.
- **desktop/bridge/index.ts**: one file: session seeding, DM reading, conversation mode (`showConversation`; an open dialog such as
  Sharkord's settings is closed first, as for a clicked notification's channel), voice read/control/lock,
  channel menu mute, notification capture, drain queue.
- **mobile/src**: `App.tsx` (screens; `boot` opens last server, or waits on the rail after `#home`, over a still of the page left; `__SHIVER_BACK__` reopens it), `api.ts`, `types.ts`, `components/`:
  `Boot` (confirms rail menu actions, the way back after `#home`), `Rail` (`RailRef`), `ServerList`, `AddServer`, `SignInServer`,
  `SettingsScreen`, `BackgroundNotifications`, `Sessions` (+`TrustedLinks`), `DirectMessages`, `UpdateNotice`, `icons`.
- **mobile/bridge**: `index.ts` (install, `seedSession`), `home.ts` (`setHome`, `goHome`, `installHomeSwipe`: back and a
  swipe past the drawer leave for Shiver's page), `touch.ts` (`installTouchStyles`, `installReturnMakesALine`, `installReactionNames`,
  channel menu with mark all read: `installChannelMenu`, `closeChannelMenu`; `drawerIsOpen`, `openConversation`), `reconnect.ts` (`installAutoReconnect`), `document-start.ts` (seed,
  `__SHIVER_OPEN__`, theme), `types.ts` (`ShiverConfig`).

## plugin (Sharkord companion)

- `server/index.js`: registers actions `setStatus`, `getStatuses`, `getOwnStatus`, `getMutedChannels`,
  `setMutedChannels`, `setReadFloor`, `setPushEndpoint`, `clearPushEndpoint` (writes rate limited per
  user); `adoptOldStore`, `primeFromUserRows`.
- `server/rows.js` (`createRows`, `createLimiter`: serialised per-user rows), `settings.js` (mutes,
  unread floor: `createSettings`, `mutedFrom`, `floorFrom`), `status.js` (custom statuses:
  `createStatuses`, `statusFrom`), `push.js` (UnifiedPush delivery: `createPush`, `endpointsFrom`,
  `normaliseEndpoint`, `deliver`; SSRF vetting: `isPrivateAddress`, `vetEndpoint`, `REFUSED`),
  `files.js` (`installFileNaming`, `uniqueName`: a random suffix, separators and control characters
  replaced; `splitName`).
- `client/index.js`: client half: announces itself as `__SHIVER_PLUGIN__` (`{version}`), relays the bridge's calls to
  server actions (`callPlugin`), plus custom-status UI. Tests: `plugin/test/plugin.test.js`.
