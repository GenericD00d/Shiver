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
shared/shiver-core/        Rust: origin rules, registry store, rail, http, login, session freshness, probe, rate limit
shared/sharkord-client/    Rust: read-only Sharkord tRPC-over-WebSocket client, watch loop, add-server check
shared/web/                TS for both frontends (relative imports); components/: React both draw, on each app's own
                           React (`resolve.dedupe` in its vite config, `paths` in its tsconfig)
shared/web/bridge/         TS for both bridges
desktop/src-tauri/         desktop core (crate `shiver`); capabilities/: event listen only, Shiver's own webviews
desktop/src/               desktop UI (React): shell, bell, popup — one bundle, picked by webview label
desktop/bridge/            desktop bridge (IIFE, include_str!'d as an init script)
mobile/src-tauri/          Android core (crate `shiver-mobile`); capabilities/: event listen only, own page
mobile/src/                Android UI (React)
mobile/bridge/             mobile bridge (inside server pages; knows only that server); document-start.ts runs first
mobile/plugins/            tauri-plugin-shiver-push (UnifiedPush), tauri-plugin-shiver-secrets (Keystore),
                           tauri-plugin-shiver-rail (the quick rail: direct messages, servers, add, settings;
                           native views over the server page)
plugin/                    Sharkord companion plugin (server/ + client/, plain JS, node tests)
scripts/check-version.py   checks the workspace and tauri.conf.json versions agree
scripts/check-sharkord.py  checks Sharkord still has the test ids, storage keys, classes and conventions the bridges
                           and the plugin's client half match (`SHARKORD_REF`, the commit last checked against;
                           `--latest` for its default branch)
.github/workflows/checks.yml  CI; sharkord.yml: the Sharkord check against its latest, weekly
```

## Checks (run before pushing)

`cargo fmt --all --check` · `cargo clippy --workspace --all-targets -- -D warnings` ·
`cargo test --workspace` · `bunx tsc --noEmit` and `bun audit` in `desktop` and `mobile` · `bun test shared/web` ·
`node --test test/*.test.js` in `plugin` · `python3 scripts/check-version.py` · `python3 scripts/check-sharkord.py` ·
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
  `MediaGate`: camera and microphone only for the server on screen, asked once per server),
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
| `model` | `Palette`/`palette` (Shiver's page colours as `theme.ts` derives them, for native views), `DEFAULT_RAIL_COLOR`, `NotifyLevel` {All, Mentions, Dms} (`allows(is_dm, mentions_me)`; an unreadable value reads as All), `Folder`, `MutedChannel`, `DEFAULT_THEME_COLOR`, `DEFAULT_ACCENT_COLOR`, `MAX_SOUND_VOLUME`, `default_true`, `default_sound_volume`, `sanitised_color`, `sanitised_optional_color`, `rgb`, `theme_payload`, `muted_for`, `normalized_mutes`, `set_muted_for` |
| `rail` | `initials`, `rows`→`RailRow` {Server, Folder(folder, servers)} (the rail as drawn; the quick rail's order); `RailServer` trait + `rail_server!(Type)` macro; `registry!(Registry, Server)` (that plus `server`, `server_mut`, `muted_for`, `set_muted_for`, `next_position`, `rail`); `RailRef {kind: RailKind (Server/Folder), id}`; `next_position`; `Rail {servers, folders}`: `create_folder`, `rename_folder`, `set_folder_expanded`, `delete_folder`, `set_server_folder`, `place`, `reorder` (items left out follow in their old order), `reorder_servers`, `prune_folders` |
| `http` | `client()` (pooled, no redirects), `bytes_within_limit`, `MAX_BODY` |
| `login` | `sign_in` (`POST /login` → token) |
| `jwt` | reads `exp` only: `freshness`→`Freshness` {Fresh, Due (use it, renew in the background), Expired}, `outlasts`; `Renewals` (`begin`/`succeeded`: one background renewal per entry per 5 min) |
| `text` | `presentable` (server words made safe to show), `clamp` (bounded, invisible marks dropped), `notice_line` (`author in #channel: text`, both apps' system notifications) |
| `probe` | `ServerInfo`, `fetch_info` (`GET /info`, name and description cleaned and bounded), `fetch_icon` (data URI), `public_file_url` (a file under `/public/`, one segment, with a signed link's token) |
| `limit` | `Openings`: `take(key, n)` (5/s, 10/10s per key), `grant`, `forget`; `Joins`/`JoinPlace`: `take` (the next place for a join of an entry's server, 4 per 62 s, under Sharkord's 5 a minute per user; places queue in order), `give_back`, `forget` |
| `links` | links a page asks to open: `decide`→`Decision` {Refuse, Open, Ask{site, question, always}}, `Answer::from_choice` (the dialog's buttons: `OPEN`, `always`, `CANCEL`), `site` (http(s) only, none for a link carrying credentials), `question` (names the site first); `TrustedLink` {entry_id, site}: `is_trusted`, `trust` (bounded, per server), `forget_entry` |
| `hash` | `java_string` (Java `String.hashCode`) |

## shared/sharkord-client (`sharkord_client`)

- Types: `Joined` (join payload: read states, user names and `user_avatars`, `plugin_version`…), `Avatar` (`url`), `DirectMessage` (with the partner's `avatar_url`),
  `NewMessage` (`is_own`, `author`, `body`, `mentioned`/`mentions_me`: read from the html as Sharkord's `hasMention` does), `Event`, `Error` (`TooLarge`, `Refused` and `Busy` (a rate limit, retried later, never a refused session) matter; converts
  into `shiver_core::Error`), `readable_size`.
- Watch loop: `watch(key, impl Watcher)`; `Watcher` trait: `target()→Option<Target{origin,token,accept_any_size}>`,
  `joined`, `event`, `refused(n)→retry now?`, `too_large`, `disconnected`; reconnects with backoff.
- Unread math: `set_unread`, `apply_delta`, `unread_total`, `choose_floor` (shared floor, else this device's, else the join; both apps).
- `check_server(origin, identity, password, &CheckedSessions) -> ServerCheck {info, plugin}` (plugin: absent =
  not asked); `CheckedSessions::take` hands `add_server` the check's session for the same credentials.

## desktop/src-tauri (`shiver`)

| module | role / key items |
|---|---|
| `lib` | `run()`: plugins (single instance first: a second launch shows the first's window; autostart with `tray::AT_LOGIN_ARG`; `RustOnly`: a plugin without its page script, for the dialog and notification plugins, which would replace `alert`/`confirm` and `Notification` in server pages), state, command registration (`only_shiver_chrome`: commands from Shiver's own webviews only), startup |
| `commands` | all `#[tauri::command]`s for shell/bell/popup: registry & servers (`list_registry`, `check_server`, `add_server`, `remove_server`, `log_out_server`, `sign_in_server`, `forget_password`, `refresh_server_info`, `set_accept_any_size`, `set_notify_level` (closes the page if out of sight and not in the call, so it reloads with Sharkord's switches to match), `reset_media_permissions`), rail (`reorder_servers`, `reorder_rail`, `create_folder_with`, `rename_folder`, `delete_folder`, `set_server_folder`, `set_folder_expanded`, `show_folder_menu`, `show_server_menu`), navigation (`select_server`, `prepare_server`, `show_shell`, `exit_dm_split`, `open_dm`, `open_message`, `select_channel`), voice (`voice_status`, `voice_control`), settings (`app_version`, `get_settings`, `update_settings`, `forget_trusted_links`), feed (`list_notifications`, `list_dms`, `feed_summary`, `unread_counts`, `mark_server_read`, `mark_notifications_read`, `clear_notifications`, `set_channel_muted`), popup (`toggle_popup`, `close_popup`, `dismiss_popup`) |
| `model` | `ServerEntry` (`label`), `Settings` (`sanitised`, `pages_kept`), `Registry` (`registry!` helpers, `view`→`RegistryView`: what the shell gets, without the floors), `*_PAGES_KEPT` |
| `store` | `Store` alias, `RegistryStore::update`, `load` |
| `webviews` | window/webview layout and page lifecycle: labels (`MAIN_WINDOW`, `SHELL_WEBVIEW`, `OVERLAY_WEBVIEW`, `POPUP_WEBVIEW`, `webview_label`, `is_shiver_chrome`), `ActiveServer` (`is_on_screen`, `is_being_read`: on screen and the window in sight, `set_window_hidden`, `conversation_with`: who Shiver asked a page to open a DM with), `create_main_window` (a close goes to the tray when its icon is up), `set_window_hidden` (minimised or in the tray; tells the pages), `is_in_front` (on Windows the foreground window, since focus in a webview reads as unfocused), `main_window`, `chrome_webview` (Shiver's own webviews, pinned to Tauri's exact origin by `is_shiver_page`), `show_shell_only`, `set_popup_open`, `set_page_fullscreen`, `relayout`, `show_server`, `preload_server`, `trim_pages`, `close_server`, conversations in the server's own page beside the DM list (`show_conversation`, `end_conversation`), profiles (`discard_profiles`, `prune_profiles`; a directory each, a data store on macOS 14+), pushes into pages (`push_visibility`, `push_theme`, `push_voice_lock`, `push_muted`, `push_notify_level`, `run_voice_action`, `mark_all_read`), `ask_to_open` (a native dialog before a page's link opens), `Openings` |
| `drain` | polls pages' `__SHIVER_DRAIN__` (on screen, connecting or in a call every 0.75 s, the rest every ~2 s; an unchanged page answers null); events `shiver://feed|dm-failed|server-ready|voice|signed-out|status|open-message` (`VOICE_EVENT`, `OPEN_MESSAGE_EVENT` also sent by commands); `Readiness`, `ServerStatus`, `Broadcast`, `spawn`, `notify_feed_changed` |
| `watch` | core sockets to servers without an open page (`sharkord_client::watch`): `sync`, `restart`, `forget`, `Missed`, `ReadStates`, `Plugins`, `Reported`, `Watcher`, `channel_read`, `mutes_changed`, `mark_read`, `publish_floor` |
| `feed` | `Feed` (notifications + DMs): `push` (returns what it added; one message by both `Route`s, page and socket, is one entry), `push_update`, `set_dms`, `mark_*`, `clear`, `forget_entry`, `unread_*`, `summary`; `FeedSummary` (the feed event's payload), `Notification`, `DmEntry`, `DmChannel`, `DrainResult`, `RawNotification`, `QueuedMute` |
| `notify` | system notifications for what reaches the feed while the window is not in front (`announce`; a DM also asks for attention), `Posted` (channel messages spaced per server) |
| `session` | `token` (the session for a page or socket: a due one renewed in the background, only a missing or expired one waited on), `token_for_new_page` (none when the page exists: rail clicks never touch the keychain); `Recovery` + `recover`: re-sign-in when a page's seeded session is refused |
| `secrets` | `Secret` (Session/Password) in the OS keychain, read as `SecretString` (zeroised); `*_off_thread` helpers |
| `voice` | `VoiceState` (one call across servers, started only by the page on screen), `VoiceStatus`, `VoiceSnapshot` |
| `hotkey` | global shortcuts: the mic mute (`apply`, which replaces only its own), and the rail's (Ctrl+1…9, Ctrl+Alt+Up/Down; `shiver://shortcut` to the shell) registered only while Shiver is in front (`watch_front`) |
| `badge` / `badges` | taskbar badge `refresh`; `channel_viewed`, `is_on_screen`, `is_being_read` (only then does the channel shown count as read) |
| `permissions` | camera/mic and downloads for pages: `gate` (WebView2: on-screen page only; camera/mic after a per-server yes in a native dialog, nothing saved in the profile), `gate_downloads` (elsewhere: Tauri's download hook, on-screen page only), `forget_consents` |
| `tray` | tray icon while "close to the tray" is on (`apply`, `show`, `hide_instead_of_closing`, `show_unread`: the badge's total as its tooltip), start at login (`set_start_at_login`, `AT_LOGIN_ARG`: started minimised or in the tray), `start` |
| `update` | signed self-update: `start` (announces `shiver://update`), `check_for_update`, `install_update`, `available_update`, `skip_update`, `open_repository`, `Available` |

Page hooks (desktop bridge ↔ core): `__SHIVER_DRAIN__` (page→core queue) and core→page
`__SHIVER_SET_THEME__`, `_SET_MUTED__`, `_SET_HIDDEN__`, `_CONVERSATION__`,
`_SELECT_CHANNEL__`, `_MARK_ALL_READ__`, `_VOICE__`, `_SET_VOICE_LOCK__`, `_SET_SOUND_VOLUME__`,
`_SET_ATTACHMENT_CARDS__`, `_SET_READ_FLOOR__`, `_SET_NOTIFY__`.

## mobile/src-tauri (`shiver-mobile`)

| module | role / key items |
|---|---|
| `lib` | `run()`, `only_home` (commands refused unless Shiver's own page is on screen), navigation guard (a page's `#home` becomes `rail::open`; a page starting to load closes the rail), `on_window_event` (`onPause`/`onResume` → `inbox::set_background`), page loads Shiver did not open sent home, `ask_to_open` (a native dialog before a page's link opens); `pub use sharkord_client as sharkord` |
| `commands` | `list_registry`, `probe_server`, `check_server`, `add_server`, `remove_server`, `refresh_server_info`, `server_icons`, `set_accept_any_size`, `set_notify_level`, `log_out_server`, `sign_in_server`, `forget_password`, `forget_sessions`, `session_states`/`SessionStates` (signed out, password kept, problems, plugins), push (`push_status`, `set_push_server`, `set_push_distributor`; `PushStatus`, `PushServer`), `unread_counts`, `list_dms`, `select_server` (waits for its join place, counting down on Shiver's page through `shiver://join-wait`; a newer choice made meanwhile wins), `app_version`, `update_settings`, `forget_trusted_links`, rail (`reorder_rail`, `create_folder_with`, `set_server_folder`, `rename_folder`, `delete_folder`, `set_folder_expanded`) |
| `model` | `ServerEntry` (+`push_token`, `retired_push_endpoints`), `Settings`, `Registry` (`registry!` helpers, `entry_for_push_token`, `ensure_push_tokens`) |
| `store` | as desktop |
| `icons` | logos as files (`icons/<entry id>`, a `data:` uri each): `save` (none deletes), `load`, `load_one`, `key` (names the file's contents by size and time, unread), `restore` (prune the gone, fetch the missing) |
| `rail` | the quick rail over the server page: `QuickRail` (whether it is up, the logo keys it holds), `wanted` (a page's `#home`), `open`, `refresh` (unread changed), `close`, `chosen` (another server or one of Shiver's screens leave through `#open=`, `#dms`, `#add`, `#settings`; the same server just closes it; a rail that cannot be drawn falls back to `#home`) |
| `inbox` | core sockets for servers not on screen + secret storage: `Inbox` (tokens, problems, plugins, dms, unread, signed_out, baselines), `sync` (each server left goes unwatched for `WATCH_GRACE`, one timer for all), `set_background` (the server on screen is watched by the core after `WATCH_GRACE` in the background), `restart`, `restore` (holds back the socket of the server about to open), `session_for` (renews a due session in the background, an expired one first), `remember_session`, `remember_password`, `forget_password`, `forget_everywhere`, `Watch` (each connection takes a join place first), `harvest_token` (never over a session that outlasts it), `replace_mutes`, `watch_mutes` (not in the background), `collect_dms`, `DmEntry`, `INBOX_EVENT` |
| `webview` | the single webview (`main_window`): `Showing` (home, current server and whether it loaded, a stray page from history, pending DM user, when each server was left, whether Shiver is in the background; `at_home`, `just_left`, `hold_back`, `begin_opening`/`still_opening`, `set_background`, `in_background`, `kept_by_page`), `show_server`, `show_failed` (back to Shiver's page with `#failed=<id>`), `go_home`, `emit_home` (events only while Shiver's page is up), `without_seed`, `install_bridge`/`PageContext` (that entry's own config only), `read_mutes` (the page's mutes and outside links), navigation guard (`is_allowed`, `navigation_allowed`; allows only, since Android also asks it for frames and cancelled navigations), arrival on load (`is_home`, `landed_home`), `background_color`, `document_start` (bundle wrapped with the per-launch seed secret; `seed_key_for` derives each origin's key), `Openings` |
| `push` | UnifiedPush per chosen server: `Push`, `start`, `register_wanted`, `set_wanted` (turning off retires the endpoint; the page clears it with the plugin), `unregister`, `migrate_tokens`, `PUSH_EVENT` |
| `update` | notify-only: `start` (announces `shiver://update`), `check_for_update`, `update_available`, `skip_update`, `open_releases`, `open_repository` |

Page hooks (mobile): `__SHIVER__` (config), `__SHIVER_MUTED__`, `__SHIVER_OPEN__`, `__SHIVER_BACK__`,
`__SHIVER_MOBILE_INSTALLED__`, `__SHIVER_SESSION_SHIM__`.

Android plugins: `PushExt` (`distributors`, `set_distributor`, `register`, `unregister`, `on_event`,
`PushEvent`); `SecretsExt` (`set`, `get`, `remove`, `keys`, `wipe_origin`: that origin's webview storage and camera/mic consent);
`RailExt` (`show`, `refresh`: a `RailView` of `Row`s and `Tile`s, answering the logo keys it lacks; `hide`; `on_event` → `RailEvent`
{Open, Dms, Add, Settings, Closed}; Kotlin `RailPlugin`: the overlay laid out as Shiver's rail (direct messages, servers, add,
settings pinned at the bottom), its back press and animations; icons in `android/src/main/res/drawable`). Kotlin in `android/src/main/java`.

## shared/web (TS)

| file | exports |
|---|---|
| `types.ts` | `Folder`, `MutedChannel`, `ServerInfo`, `ServerCheck`, `TrustedLink`, `NotifyLevel` (re-exported by each app's `types.ts`) |
| `settings.ts` | `DEFAULT_THEME_COLOR`, `DEFAULT_ACCENT_COLOR`, `DEFAULT_RAIL_COLOR`, `MAX_SOUND_VOLUME` |
| `rail.ts` | `initials`, `byPosition`, `membersOf`, `railOrder` (every server as the rail shows them) |
| `dms.ts` | `DmRow` (a conversation as either client lists it), `dmKey`, `filterDms` (person or server), `dmMeta`, `dmInitial`; tested in `dms.test.ts` |
| `time.ts` | `relativeTime` ("5m ago": the feed and the DM list) |
| `components/DmList.tsx` | `DmList`: the DM list both clients draw (search, rows with avatar, account · server, time; `touch` sizes it for a finger), styled by `components/dm-list.css` |
| `colors.ts` | `automaticTextColor`, `lift` |
| `theme.ts` | `applyTheme` (`--shiver-*` vars on Shiver's own pages) |
| `session.ts` | `installSessionShim`, `takeSeedFromLocation`, `AUTO_LOGIN*` (session kept off disk) |
| `bridge/dom.ts` | `ensureStyle`, `defineHook`, `onDomSettled` (hands callbacks what changed), `touched`, `isTopFrame`, `whenDocumentReady`, `installExternalLinks` |
| `bridge/sharkord.ts` | Sharkord store types, test-id selectors (`SIDEBAR`, `CHANNEL_ITEM`, `DM_ITEM`…), `sharkordStore`, `watchStore`, pure store reads (`notificationTarget`/`NotificationTarget`: a notification's author and channel from its title, `dmPartnerId`, `findDmChannelIdByUserName`, `readDms`/`DmChannel`, `fileUrl`; tested in `sharkord.test.ts`), `notificationFlags` (Sharkord's own notification switches for a level), `notifyAllows`, `rowName`, `channelOfRow`, `markAllChannelsRead`, `installMuteStyles`, `paintMuted`, `pressEscape`, `closeDialog` (Sharkord's topmost open dialog, as Escape), `IMAGE_VIEWER`, `RECONNECTING_OVERLAY`, `SIDE_PANEL`, `NARROW`, `SHIVER_PLUGIN_ID` |
| `bridge/plugin.ts` | `callPlugin`, `waitForPlugin`, `syncMutesWithPlugin`, `pushMutesToPlugin`, `storeReadFloor` |
| `bridge/channel-menu.ts` | the channel menu both bridges open (right-click, long press): mute or unmute, mark all as read; Shiver's items join Sharkord's own menu when it opens, else Shiver draws its own. `installChannelMenu` (`ChannelMenuSetup`, `ChannelMutes`), `channelPressed`, `requestChannelMenu`, `closeChannelMenu`, `isInChannelMenu` |
| `bridge/features.ts` | `installSoundVolume`, `installAttachmentCards`, `installVoiceColors`, `installSidePanels` (Sharkord's voice chat and thread panels cover a narrow page instead of staying hidden), `closeSidePanel` |
| `bridge/theme.ts` | `ShiverTheme`, `applyPageTheme` |

## Frontends

- **desktop/src**: `main.tsx` (label → `App` | `Bell` | `NotificationsPopup`; drops refused wherever the rail does not take them, so none navigates a chrome webview), `api.ts` (`api.*` invoke
  wrappers, `errorMessage`), `events.ts` (`EVENTS`, `useCoreEvent`; `App` answers `shortcut` in rail order), `types.ts` (`*_PAGES_KEPT`), `sounds.ts` (`playNotificationSound`), `components/`:
  `ServerRail`, `AddServerPanel`, `SignInPanel`, `SettingsPanel`, `HotkeyField`, `DirectMessagesPanel` (the shared `DmList` in a sidebar),
  `NotificationList`, `RenameFolderPanel`, `RemoveServerPanel`, `ConnectingPanel`, `WelcomePanel`,
  `VoiceTile`, `UpdateNotice`, `icons`.
- **desktop/bridge/index.ts**: one file: session seeding, DM list (`openDmChannelId`, `resolveDmChannels`), conversation mode (`showConversation`; an open dialog such as
  Sharkord's settings is closed first, as for a clicked notification's channel), voice read/control/lock,
  the channel menu on right-click (`installChannelRightClick`; its mutes and "Mark all as read" go to the core through the drain), notification capture (filtered by the server's `NotifyLevel`: Sharkord's switches seeded to match by `applyNotifyLevel`, which marks the level it wrote in `shiver-notify-level`; `__SHIVER_SET_NOTIFY__` for a change), drain queue.
- **mobile/src**: `App.tsx` (screens; `boot` opens last server, or waits after `#home` (the quick rail could not be drawn); `#open=` opens one; `#dms`, `#add`, `#settings` (`RAIL_SCREENS`, from the quick rail) open that screen; its own rail is not drawn while the page only passes through to a server, unless that takes `PASSING_THROUGH_MS`; `__SHIVER_BACK__` reopens the last), `api.ts`, `types.ts`, `components/`:
  `Boot` (confirms rail menu actions; after `#home`, a button back to the server left; counts down a wait for a join), `Rail` (`RailRef`), `ServerList` (a notification level per server), `AddServer`, `SignInServer`,
  `SettingsScreen`, `BackgroundNotifications`, `Sessions` (+`TrustedLinks`), `DirectMessages` (the shared `DmList`, `touch`), `UpdateNotice`, `icons`.
- **mobile/bridge**: `index.ts` (install, `seedSession`), `home.ts` (`setHome`, `goHome`, `openRail` and `installRailSwipe`: back (after closing a menu or
  side panel) and a swipe past the drawer ask for the rail, which the core draws over the page), `touch.ts` (`installTouchStyles`, `installReturnMakesALine`, `installReactionNames`, `installImageZoom` (pinch, pan and
  double tap in Sharkord's full-screen picture, which only knows the mouse),
  `installLongPress`: a held message shows Sharkord's toolbar, a held channel opens the channel menu; `drawerIsOpen`, `openConversation`), `reconnect.ts` (`installAutoReconnect`; `installQuietReconnect`: Sharkord's own reconnecting dialog hidden, an accent spinner above the chat box while it retries), `document-start.ts` (seed,
  `__SHIVER_OPEN__`, theme), `types.ts` (`ShiverConfig`).

## plugin (Sharkord companion)

- `server/index.js`: the whole server half, one file (so a server reloading only its entry cannot mix
  it with stale copies of others), in sections:
  - rows: `createRows`, `createLimiter` (serialised per-user rows)
  - file names: `installFileNaming`, `uniqueName` (a random suffix, separators and control characters
    replaced), `splitName`
  - settings: `createSettings`, `mutedFrom`, `floorFrom` (mutes, unread floor)
  - page options: `OPTION_SETTINGS`, `createOptions` (the admin's switches for what the client half
    draws, pushed to every page when one changes: `roleColors`)
  - statuses: `createStatuses`, `statusFrom` (custom statuses)
  - push: `createPush`, `endpointsFrom`, `normaliseEndpoint`, `deliver` (UnifiedPush delivery); SSRF
    vetting: `isPrivateAddress`, `vetEndpoint`, `REFUSED`
  - loading: `onLoad` registers actions `getOptions`, `setStatus`, `getStatuses`, `getOwnStatus`, `getMutedChannels`,
    `setMutedChannels`, `setReadFloor`, `setPushEndpoint`, `clearPushEndpoint` (reads and writes rate
    limited per user); `onUnload`, `adoptOldStore`, `primeFromUserRows`.
- `client/index.js`: client half, in everyone's page (Shiver or a browser): announces itself as `__SHIVER_PLUGIN__`
  (`{version}`), relays the bridge's calls to server actions (`callPlugin`); custom statuses (member list, profile
  card and user settings slots, and a button beside the settings gear); usernames in their role colour
  (`MemberColor` in the member list by id; message headers, replies and mentions by name) unless the admin turned
  `roleColors` off. The newest copy in a page (an update imports another) answers and draws. Tests:
  `plugin/test/plugin.test.js`.
