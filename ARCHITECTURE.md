# Architecture

The map of Shiver for agents and contributors. Read it before searching the tree, and update it in
the same commit whenever you add, remove, rename or move a module, command, hook or exported item.

## Overview

Shiver is a multi-server client for Sharkord: a Tauri 2 desktop app and a Tauri 2 Android app, built
around shared Rust crates and shared TypeScript, plus a Sharkord server plugin.

In both apps:

- **Shiver's own pages** (the rail, the bell, settings and the DM list) talk to the Rust core over
  Tauri commands and events. Only they may; commands from anywhere else are refused.
- **Each server's own web client** runs in a webview pinned to that server's origin, **with no
  IPC**. Shiver injects a bridge script into it. The bridge and the core talk through
  `window.__SHIVER_*__` hooks: the core calls them, and polls the page for what it has queued.
- **The core** holds the registry (`servers.json`), the secrets (the OS keychain, or the Android
  Keystore), and a read-only socket to every server that isn't on screen, so the inbox and unread
  counts are complete whichever server is showing.

The desktop keeps a webview per open server. Android has a single webview, which it navigates
between Shiver's own page and one server at a time; the quick rail is a WebView of its own, drawn
over the server's page.

```
 ┌──────────────────────────────── Shiver ────────────────────────────────┐
 │                                                                        │
 │  Shiver's own pages  ◄── Tauri commands and events ──►  Rust core      │
 │  rail, bell, settings, DMs                              registry       │
 │                                                         secrets        │
 │  A server's page (one per server, on its origin)        feed, voice    │
 │  Sharkord's own client + the bridge  ◄── hooks, drain ──►  sockets     │
 │                │                                            │          │
 └────────────────┼────────────────────────────────────────────┼──────────┘
                  │ https: the server's web client             │ wss: read-only tRPC
                  ▼                                            ▼
          Sharkord server, optionally running the companion plugin
```

What each boundary in that picture guards against is in
[SECURITY.md](SECURITY.md#trust-boundaries).

## Layout

```
icon.png                   1024px master icon, the input to `tauri icon` for both apps
Cargo.toml                 workspace (rust 1.90): shared/*, desktop/src-tauri, mobile/src-tauri, mobile/plugins/*
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
mobile/rail/               the quick rail's page (the shared `Rail`, and over the page Shiver's menus, settings and
                           DMs), built by `bun run build:rail` into the rail plugin's assets (generated, ignored);
                           part of `bun run build` and `beforeDevCommand`
mobile/plugins/            tauri-plugin-shiver-push (UnifiedPush), tauri-plugin-shiver-secrets (Keystore),
                           tauri-plugin-shiver-rail (the quick rail and the screens it opens: direct messages,
                           servers and their menus, settings; that page in a WebView of its own over the server page)
plugin/                    Sharkord companion plugin (server/ + client/, plain JS, node tests)
docs/                      standards.md (the Shiver standard, shared by four repos), decisions.md
scripts/check.sh           every local check (--quick: the ones that need no build)
scripts/check-docs.py      checks the docs follow docs/standards.md (identical in every Shiver repo)
scripts/check-version.py   checks the workspace and tauri.conf.json versions agree, the plugin's manifest and
                           package.json agree, and CHANGELOG.md has an entry for the version
scripts/check-sharkord.py  checks Sharkord still has the test ids, storage keys, classes and conventions the bridges
                           and the plugin's client half match (`SHARKORD_REF`, the commit last checked against;
                           `--latest` for its default branch)
.github/workflows/checks.yml  CI; sharkord.yml: the Sharkord check against its latest, weekly
```

## Modules

### shared/shiver-core (`shiver_core`)

- **`error`**: `Error` {InvalidOrigin, Storage, Unreachable, NotSharkord, Refused, InvalidInput,
  UnknownServer, UnknownFolder}, `Result`.
- **`origin`**:
  - `normalize_origin`;
  - `is_same_origin`, the one webview-boundary comparison;
  - `add_server_link` (`SHARKORD_SCHEME`): the server a `sharkord://add-server?url=…` link offers,
    as an origin. Both apps' `deep_link` use it.
- **`store`**: `Store<R>` (`load`, `registry`→`ReadGuard`, `edit`), and `LockExt::locked`, a
  poison-tolerant lock.
  - An edit applies only once written, and one that changes nothing is not written.
  - Readers never wait on the disk.
  - Writes are atomic, and owner-only on Unix.
- **`model`**:
  - `account_label`: a typed label trimmed and bounded (both apps' `add_server`).
  - `NotifyLevel` {All, Mentions, Dms}, with `allows(is_dm, mentions_me)`; an unreadable value reads
    as All.
  - `Folder`, `MutedChannel`, `DEFAULT_THEME_COLOR`, `DEFAULT_ACCENT_COLOR`, `MAX_SOUND_VOLUME`,
    `default_true`, `default_sound_volume`, `sanitised_color`, `sanitised_optional_color`.
  - `Colors`: the background, accent, text, accent text and sidebar colours as a set (`sanitised`,
    `Default`). Both apps' `Settings` hold one, flattened, as `colors`.
  - `ColorPreset` (a named `Colors`), `MAX_COLOR_PRESETS`, `sanitised_presets` (each name once,
    bounded).
  - `rgb`, `theme_payload(&Colors)`, `muted_for`, `normalized_mutes`, `set_muted_for`.
- **`rail`**:
  - The `RailServer` trait and the `rail_server!(Type)` macro.
  - `registry!(Registry, Server)`: that, plus `server`, `server_mut`, `muted_for`, `set_muted_for`,
    `next_position` and `rail`.
  - `RailRef {kind: RailKind (Server/Folder), id}`, and `next_position`.
  - `Rail {servers, folders}`:
    - `create_folder` (two different servers at least), `rename_folder`, `set_folder_expanded`;
    - `delete_folder`: its servers take its place, in order;
    - `set_server_folder`: last in a folder, or just after the one it left. It and `create_folder`
      dissolve a folder they leave too thin;
    - `place`, `reorder` (items left out follow in their old order), `reorder_servers`,
      `prune_folders`.
  - Each change to the top level renumbers it 0, 1, 2…
- **`http`**: `client()` (pooled, no redirects), `bytes_within_limit`, `MAX_BODY`.
- **`login`**: `sign_in` (`POST /login` → token).
- **`jwt`**: reads `exp` only.
  - `freshness`→`Freshness` {Fresh, Due (use it, renew in the background), Expired}, and `outlasts`.
  - `Renewals` (`begin`/`succeeded`): one background renewal per entry per 5 min.
- **`text`**:
  - `presentable`: server words made safe to show, bare domains dropped too;
  - `presentable_name`: the same for a server's name and description, dotted words kept;
  - `clamp`: bounded, invisible marks dropped;
  - `notice_line`: `author in #channel: text`, for both apps' system notifications.
- **`probe`**: `ServerInfo`; `fetch_info` (`GET /info`, name and description cleaned and bounded);
  `fetch_icon` (a data URI); `public_file_url` (a file under `/public/`, one segment, with a signed
  link's token).
- **`limit`**:
  - `Openings`: `take(key, n)` (5/s and 10/10s per key), `grant`, `forget`.
  - `Joins`/`JoinPlace`: `take` gives the next place for a join of a server, keyed by origin, 4 per
    62 s, under Sharkord's 5 a minute per user. Places queue in order, and are taken by the core's
    sockets, `check_server`, and Android's page loads. `give_back`.
- **`links`**: the links a page asks to open.
  - `decide`→`Decision` {Refuse, Open, Ask{site, question, always}}.
  - `Answer::from_choice`, from the dialog's buttons: `OPEN`, `always`, `CANCEL`.
  - `site`: http(s) only, and none for a link carrying credentials. It is the bare host for https on
    its usual port, and the whole origin otherwise, so trust never spans schemes or ports.
  - `question`: names the site first.
  - `TrustedLink` {entry_id, site}: `is_trusted`, `trust` (bounded, per server), `forget_entry`.
- **`hash`**: `java_string` (Java's `String.hashCode`).

### shared/sharkord-client (`sharkord_client`)

Modules: `lib` (the public types), `wire`, `html`, `unread`, `connection`, `check` and `error`.

- **`wire`**: frames, request ids and the payload parsers. `join_dms` takes the join's
  `directMessages` (Sharkord 0.0.28), or else asks `dms.get`. `parse_dms` leaves out a conversation
  the user closed.
- **`html`**: `plain_text` and `mentioned`→`Mentions {users, roles}`: a message's text and mentions,
  each `>` looked for once.
- **`connection`**: `open`, the join and subscriptions, `Session::next_event`, `call`, and the watch
  loop.
- **Types**:
  - `Joined`: the join payload (read states, user names and `user_avatars`, `own_role_ids`,
    `plugin_version`…).
  - `Avatar` (`url`), and `DirectMessage` (with the partner's `avatar_url`).
  - `Event`: `Unread`, `UnreadSet`, `Posted`, and `ServerChanged` (`others.onServerSettingsUpdate`,
    so a name or logo may have changed).
  - `NewMessage`: `id` and `thread_id` (where its notification leads); `is_own`, `author`, `body`;
    `mentioned`/`mentioned_roles`/`mentions_me`, the user or a role they hold, read from the html as
    Sharkord's `hasMention` does.
  - `Error`: `TooLarge`, `Refused` and `Busy` matter. `Busy` is a rate limit, retried later, never a
    refused session. It converts into `shiver_core::Error`. `readable_size`.
- **The watch loop**: `watch(key, impl Watcher)`.
  - The `Watcher` trait: `target()→Option<Target{origin,token,accept_any_size}>`, `joined`, `event`,
    `refused(n)→retry now?`, `too_large`, `disconnected`.
  - It reconnects with backoff (`retry_delay`: 30 s doubling to 10 min, jittered per key; the
    desktop also uses it for its own retries). The backoff resets only after a connection that
    delivered an event or lasted a minute.
  - A socket whose every subscription was refused is closed.
- **Unread maths**: `set_unread`, `apply_delta`, `unread_total`; `choose_floor` (the shared floor,
  else this device's, else the join; both apps); `settle_floor` (a channel read below its floor
  lowers it, so what arrives afterwards counts; it is never raised).
- **`check_server(origin, identity, password, &CheckedSessions, &Joins) -> ServerCheck {info,
  plugin}`**: plugin absent means not asked; its join takes a `Joins` place.
  `CheckedSessions::take` hands `add_server` the check's session for the same credentials.

### desktop/src-tauri (`shiver`)

- **`lib`**: `run()`.
  - Plugins: the single instance first (a second launch shows the first's window, and hands its
    `sharkord://` link to the deep-link plugin); autostart with `tray::AT_LOGIN_ARG`; the log plugin
    (`logs::plugin`).
  - `RustOnly`: a plugin without its page script. Used for the dialog and notification plugins,
    which would otherwise replace `alert`/`confirm` and `Notification` in server pages.
  - State, command registration (`only_shiver_chrome`: commands from Shiver's own webviews only),
    and startup.
- **`commands`**: every `#[tauri::command]` for the shell, bell and popup, one file per area under
  `commands/` (`servers`, `rail`, `menu`, `navigation`, `voice`, `settings`, `feed`, `popup`).
  `mod.rs` keeps the shared lookups and re-exports them all.
  - Registry and servers: `list_registry`, `check_server`, `add_server`, `remove_server`,
    `log_out_server` (its page comes back at the sign-in only if it was on screen),
    `sign_in_server`, `forget_password`, `refresh_server_info`, `set_accept_any_size`,
    `reset_media_permissions`.
    - `refresh_info_soon`: re-read on a watch's join, a page becoming ready and `ServerChanged`; the
      shell gets `shiver://servers` if the name or logo changed.
    - `set_notify_level`: closes the page if it is out of sight and not in the call, so it reloads
      with Sharkord's switches to match.
  - Rail: `reorder_servers`, `reorder_rail`, `create_folder_with`, `rename_folder`, `delete_folder`,
    `set_server_folder`, `set_folder_expanded`, `server_menu_facts`, and `show_menu` (a rail menu
    from `shared/web/menus.ts` as a native menu, its ids and size checked; the choice comes back as
    `shiver://server-menu`).
  - Navigation: `select_server`, `prepare_server` (an existing page that stalled is revived),
    `show_shell`, `exit_dm_split`, `open_dm`, `open_message` and `select_channel` (with a
    notification's `Target`: the message, or a plugin's notification, for a page whose client speaks
    Sharkord's desktop api).
  - Voice: `voice_status`, `voice_control`.
  - Settings: `app_version`, `get_settings`, `update_settings`, `forget_trusted_links`.
  - Feed: `list_notifications`, `list_dms`, `feed_summary`, `unread_counts`, `mark_server_read`,
    `mark_notifications_read`, `clear_notifications`, `set_channel_muted`.
  - Popup: `toggle_popup`, `close_popup`, `dismiss_popup`.
- **`logs`**: the log file (`shiver.log` in the app's log folder, 5 MB, and the one before it).
  `plugin` (the core and shared crates from info, other crates from warnings); `start` (version and
  system, and a panic hook); `page_errors` (what a page threw, under the `page` target, bounded);
  `open_logs_folder`.
- **`model`**: `ServerEntry` (`label`); `Settings` (`sanitised`, `pages_kept`; `spellcheck`, pushed
  with the theme); `Registry` (the `registry!` helpers, and `view`→`RegistryView`: what the shell
  gets, without the floors); `*_PAGES_KEPT`.
- **`store`**: the `Store` alias, `RegistryStore::update`, `load`.
- **`webviews`**: window and webview layout, and the page lifecycle.
  - Labels: `MAIN_WINDOW`, `SHELL_WEBVIEW`, `OVERLAY_WEBVIEW`, `POPUP_WEBVIEW`, `webview_label`,
    `is_shiver_chrome`.
  - `ActiveServer`: `is_on_screen`, `is_being_read` (on screen and the window in sight),
    `set_window_hidden`, `conversation_with` (who Shiver asked a page to open a DM with).
  - Windows: `create_main_window` (a close goes to the tray when its icon is up),
    `set_window_hidden` (minimised or in the tray; tells the pages), `is_in_front` (on Windows the
    foreground window, since focus in a webview reads as unfocused), `main_window`.
  - `BROWSER_ARGS`: every webview's WebView2 command line. It keeps wry's defaults, and stops
    Chromium slowing a covered or unfocused window, for screen sharing.
  - `chrome_webview`: Shiver's own webviews, pinned to Tauri's exact origin by `is_shiver_page`.
  - Pages: `show_shell_only`, `set_popup_open`, `set_page_fullscreen`, `relayout`, `show_server`,
    `preload_server`, `trim_pages`, `close_server`; conversations in the server's own page beside
    the DM list (`show_conversation`, `end_conversation`); `Openings`.
  - Submodules: `layout` (where each webview goes); `profiles` (`discard_profiles`,
    `prune_profiles`; a directory each, a data store on macOS 14+); `page` (what is evaluated in the
    pages: `push_visibility`, `push_theme`, `push_voice_lock`, `push_muted`, `push_notify_level`,
    `run_voice_action`, `mark_all_read`); `links` (`ask_to_open`: a native dialog before a page's
    link opens).
- **`drain`**: polls pages' `__SHIVER_DRAIN__`.
  - Pages on screen, connecting or in a call are polled every 0.75 s, the rest about every 2 s. An
    unchanged page answers null.
  - Events: `shiver://feed|dm-failed|server-ready|voice|signed-out|status|mismatch|open-message`.
    `VOICE_EVENT` and `OPEN_MESSAGE_EVENT` are also sent by commands.
  - A page's new shortcuts go to `page_hotkeys::set`, answered through `__SHIVER_HOTKEYS_SET__`, and
    its errors to `logs::page_errors`.
  - `Readiness`, `ServerStatus`, and `Mismatch`/`Mismatches`: a client on another desktop api
    version. One needing a newer Shiver checks for an update at once (`update::look_now`).
  - `Broadcast`: statuses and mismatches, each sent when it changes. `spawn`, `notify_feed_changed`.
  - Answers are applied one at a time, in order, on one `shiver-drain` thread.
  - `Stalls`: pages whose document never loaded (the drain answers `"absent"`) or whose client gave
    up reconnecting (`stalled`). They are probed with `retry_delay` backoff and reloaded once the
    server answers; `revive_now` does it at once, for "Try again".
- **`watch`**: the core's sockets to servers without an open page (`sharkord_client::watch`). A
  server that can't be reached for a session is retried with backoff; one that signed out parks.
  `sync`, `restart`, `forget`, `Missed`, `ReadStates`, `Plugins`, `Reported`, `Watcher`,
  `channel_read` (also lowers the stored floor), `mutes_changed`, `mark_read`, `publish_floor`.
- **`feed`**: `Feed`, the notifications and DMs.
  - `push` returns what it added. One message seen by both `Route`s, page and socket, is one entry.
  - `push_update`; `set_dms` (keeps the latest time a conversation has had, and a page's is never
    past now); `dm_active` (a message arrived); `mark_*`, `clear`, `forget_entry`, `unread_*`,
    `summary`.
  - Types: `FeedSummary` (the feed event's payload), `Notification`, `Target` (where a notification
    leads beyond its channel: `message_id`, `thread_id`, `plugin_ref`; ids that aren't positive are
    dropped), `DmEntry`, `DmChannel`, `DrainResult` (+`hotkeys`), `RawNotification`, `QueuedMute`.
- **`notify`**: system notifications for what reaches the feed while the window isn't in front
  (`announce`; a DM also asks for attention); `Posted` (channel messages, spaced per server).
- **`session`**:
  - `session` → `Result<_, NoSession>`: the session for a page or socket. A due one is renewed in the
    background; only a missing or expired one is waited on. `Unreachable` versus `Signed` out tells
    a socket whether to retry or park.
  - `CREDENTIALS`: held while a renewal stores its session, and while a log-out or removal clears
    one, so a renewal can't sign a logged-out server back in.
  - `token` (its `ok()`), and `token_for_new_page` (none when the page exists: rail clicks never
    touch the keychain).
  - `Recovery` and `recover`: signing in again when a page's seeded session is refused.
- **`secrets`**: `Secret` (Session/Password) in the OS keychain, read as `SecretString` (zeroised),
  with `*_off_thread` helpers.
- **`voice`**: `VoiceState` (one call across servers, started only by the page on screen),
  `VoiceStatus`, `VoiceSnapshot`.
- **`hotkey`**: global shortcuts.
  - The mic mute: `apply`, which replaces only its own; `is_rail_shortcut` (a rail key, which
    `update_settings` refuses as the mute key).
  - The rail's (Ctrl+1…9, Ctrl+Alt+Up/Down; `shiver://shortcut` to the shell), registered only while
    Shiver is in front (`watch_front`, which also moves the page shortcuts with the server shown).
  - `is_shivers`: a page may not bind Shiver's own.
- **`page_hotkeys`**: Sharkord's own shortcuts, from a page whose client speaks its desktop api.
  `PageHotkeys`; `set` (a page's bindings, `Binding`; `BindingError` `unsupported`/`taken`);
  `shortcut_for` (`ctrl+shift+KeyM` as a `Shortcut`); `follow_active` (only the server last shown
  holds its keys); `forget_entry` (from `close_server`). A press goes to `__SHIVER_HOTKEY__` in that
  page, and its release (`up`, for push-to-talk) to the page that had the press.
- **`deep_link`**: `sharkord://add-server?url=…` links (`plugins.deep-link` in tauri.conf.json;
  registered at startup only on Linux and in debug builds): `start`, `PendingLink` (the latest
  offer, kept until the shell asks), `take_server_link`; `shiver://server-link` to the shell.
- **`badge`** / **`badges`**: taskbar badge `refresh`; `channel_viewed`, `is_on_screen`,
  `is_being_read` (only then does the channel shown count as read).
- **`permissions`**: camera, microphone and downloads for pages. `gate` (WebView2: the on-screen page
  only; camera and microphone after a per-server yes in a native dialog, nothing saved in the
  profile); `gate_downloads` (elsewhere: Tauri's download hook, the on-screen page only);
  `forget_consents`.
- **`tray`**: the tray icon while "close to the tray" is on (`apply`, `show`,
  `hide_instead_of_closing`, `show_unread`: the badge's total as its tooltip, and an unread dot on
  the icon, `with_dot`); start at login (`set_start_at_login`, `AT_LOGIN_ARG`: started minimised or
  in the tray); `start`.
- **`update`**: signed self-update. `start` (announces `shiver://update`), `look_now`,
  `check_for_update`, `install_update`, `available_update`, `skip_update`, `open_repository`,
  `Available`.

### mobile/src-tauri (`shiver-mobile`)

- **`lib`**: `run()`.
  - `only_home`: commands are refused unless Shiver's own page is on screen.
  - The navigation guard: a page's `#home` becomes `rail::open`, and a page starting to load closes
    the rail.
  - `on_window_event`: `onPause`/`onResume` → `inbox::set_background`.
  - Page loads Shiver did not open are sent home. `ask_to_open`: a native dialog before a page's
    link opens. `pub use sharkord_client as sharkord`.
- **`deep_link`**: `sharkord://add-server?url=…` links (an intent filter from `plugins.deep-link` in
  tauri.conf.json): `start`, `PendingLink` (the latest offer, until Shiver's page takes it; over a
  server's page other than the one it names, the webview goes home for it), `take_server_link`;
  `shiver://server-link`.
- **`commands`**: one file per area under `commands/` (`servers`, `push`, `rail`, `navigation`,
  `feed`, `settings`; `mod.rs` re-exports them).
  - Servers: `list_registry`, `probe_server`, `check_server`, `add_server`, `remove_server`,
    `refresh_server_info`, `server_icons`, `set_accept_any_size`, `set_notify_level`,
    `log_out_server`, `sign_in_server`, `forget_password`, `forget_sessions`.
    - `refresh_info_soon`: re-read on an inbox join and `ServerChanged`; `shiver://servers` to
      Shiver's page, and the rail redrawn, if the name or logo changed.
    - `session_states`/`SessionStates`: signed out, password kept, problems, plugins.
  - Push: `push_status`, `set_push_server`, `set_push_distributor`; `PushStatus`, `PushServer`.
  - Feed: `unread_counts`, `list_dms`.
  - `select_server`: waits for its join place, counting down on Shiver's page through
    `shiver://join-wait`. A newer choice made meanwhile wins. Without a DM user, it takes one the
    screens over the last page queued for it.
  - Settings: `app_version`, `update_settings`, `forget_trusted_links`.
  - Rail: `reorder_rail`, `reorder_servers`, `create_folder_with`, `set_server_folder`,
    `rename_folder`, `delete_folder`, `set_folder_expanded`.
- **`model`**: `ServerEntry` (+`push_token`, `retired_push_endpoints`, `retire_push_endpoint`: once
  each, the last 10), `Settings`, `Registry` (the `registry!` helpers, `entry_for_push_token`,
  `ensure_push_tokens`).
- **`store`**: as on desktop.
- **`icons`**: logos as files (`icons/<entry id>`, a `data:` uri each). `save` (none deletes),
  `load`, `load_one`, `key` (names the file's contents by size and time, unread), `restore` (prunes
  the gone and fetches the missing).
- **`rail`**: the quick rail over the server page.
  - `QuickRail`: whether it is up, or still wanted while drawing, and the logo keys it holds.
  - `wanted` (a page's `#home`), `open`, `refresh` (unread changed), `close` (also takes down one
    still being drawn), `is_open`.
  - `chosen`:
    - another server, or adding one, leave through `#open=` and `#add`;
    - a conversation on the server behind opens in its page, and one elsewhere is queued for it
      (`Showing::queue_dm`);
    - a folder opened or shut is stored and the rail redrawn;
    - a call from the screens goes to `overlay`;
    - a rail that can't be drawn falls back to `#home`.
  - Choosing the server already on screen just closes the rail: `mobile/rail/main.tsx` sends
    `closed` for it.
- **`overlay`**: Shiver's screens over the server page. The rail's page draws a tile's menu, the
  settings and the DMs, so the page behind stays loaded.
  - `call`: a command by name, from the listed ones only (`run`), none of which opens or adds a
    server. Removing or logging out of the server behind sends that page home. Answered
    `{ reply, ok, value }`, and the rail is redrawn after a change it shows.
  - `emit`: the core's events, while the rail is up; `emit_home` sends them.
- **`inbox`**: the core's sockets for servers not on screen.
  - `mod.rs`: `Inbox` (tokens, problems, plugins, dms, unread, signed_out, baselines); `sync` (each
    server left goes unwatched for `WATCH_GRACE`, one timer for all; tasks started under the lock that
    found them missing); `set_background` (the server on screen is watched by the core after
    `WATCH_GRACE` in the background); `restart`; `Watch` (each connection takes a join place first);
    `replace_mutes`; `watch_mutes` (not in the background); `INBOX_EVENT`.
  - `inbox/sessions.rs`: sessions and secret storage, on one writer thread, in order.
    - `restore` holds back the socket of the server about to open.
    - `session_for` renews a due session in the background, and an expired one first. A renewal is
      kept only while the password still is, so a log-out meanwhile wins.
    - `remember_session`, `remember_password`, `forget_password`, `forget_everywhere`,
      `forget_credentials` (session and password only, the floor kept: "Forget sessions").
    - `harvest_token`: never over a session that outlasts it.
  - `inbox/dms.rs`: `collect_dms` (each conversation with the latest time any source gave),
    `DmEntry`; the page on screen's own list (`page_dms`, `PageDm`: bounded, names cleaned, pictures
    from its origin only) and times (`messages_seen`), kept per entry and never past now.
- **`notify`**: the Android notification per server for what the core's connections hear
  (`Notices`: one post per burst, summarised). `announce`, `clear`, `forget`,
  `report_watch_problem`; ids shared with `PushReceiver`.
- **`webview`**: the single webview (`main_window`).
  - `Showing`: home; the current server and whether it loaded; a stray page from history; a pending
    DM user; the DM queued for the next server opened; when each server was left; whether Shiver is
    in the background. With `at_home`, `just_left`, `hold_back`, `begin_opening`/`still_opening`,
    `loading_ticket` (one server page load, so a later check knows it is still that load),
    `queue_dm`/`take_queued_dm`, `set_background`, `in_background`, `kept_by_page`.
  - `show_server`; `show_failed` (back to Shiver's page with `#failed=<id>`); `go_home`;
    `open_dm_in_page` (`__SHIVER_OPEN_DM__` in the loaded page on screen); `emit_home` (events only
    while Shiver's page is up, and to the screens over a page); `without_seed`.
  - `install_bridge`/`PageContext`: that entry's own config only.
  - `read_mutes`: the page's mutes, outside links, and the retired push endpoints the plugin
    confirmed clearing, which only then leave the registry.
  - The navigation guard (`is_allowed`, `navigation_allowed`): it only allows, since Android also
    asks it for frames and cancelled navigations. Arrival on load: `is_home`, `landed_home`.
  - `background_color`; `settings_script` (colours, volume and attachment cards applied in place to
    the page on screen, by `update_settings`).
  - `document_start`: the bundle wrapped with the per-launch seed secret; `seed_key_for` derives
    each origin's key. `Openings`.
- **`push`**: UnifiedPush per chosen server. `Push`, `start`, `register_wanted`, `set_wanted`
  (turning off retires the endpoint; the page clears it with the plugin), `retire_endpoints` (every
  endpoint, before switching distributor), `unregister`, `migrate_tokens`, `PUSH_EVENT`.
- **`update`**: notify-only. `start` (announces `shiver://update`), `check_for_update`,
  `update_available`, `skip_update`, `open_releases`, `open_repository`.

### mobile/plugins (Android plugins)

Kotlin in each plugin's `android/src/main/java`.

- **`PushExt`**: `distributors`, `set_distributor`, `register`, `unregister`, `on_event`,
  `PushEvent`.
- **`SecretsExt`**: `set`, `get`, `remove`, `keys`, `wipe_origin` (that origin's webview storage, and
  its camera and microphone consent).
- **`RailExt`**: `show`, `refresh` (a `RailView`: the user's colours, `Tile`s and `RailFolder`s,
  answering the logo keys it lacks), `hide`, `post` (an answer or event to the page), and `on_event` →
  `RailEvent` {Open (with a DM user), Add, Closed, Folder, Call}.
- **Kotlin `RailPlugin`**: the host.
  - A WebView made at start-up and kept, loading only `mobile/rail`'s page from the assets
    (`WebViewAssetLoader`). It loads nothing else but https pictures, which the page's policy
    limits to images, and doesn't navigate.
  - It talks to the page over `shiverRail` (`addWebMessageListener`, its own origin only). Only
    known choices with well-formed ids pass, and calls with a command-shaped name, bounded.
  - The overlay, scrim, slide-in once the page has drawn, `expand` (the rail's width, or the whole
    overlay for a menu or screen), and back press (the page's, while it fills the overlay).

### shared/web (TS)

| file | exports |
|---|---|
| `types.ts` | `Folder`, `MutedChannel`, `ServerInfo`, `ServerCheck`, `TrustedLink`, `NotifyLevel` (re-exported by each app's `types.ts`) |
| `settings.ts` | `DEFAULT_THEME_COLOR`, `DEFAULT_ACCENT_COLOR`, `DEFAULT_RAIL_COLOR`, `MAX_SOUND_VOLUME`, `Colors` (both apps' `Settings` and the quick rail's view are `Colors &` the rest), `DEFAULT_COLORS`, `ColorPreset`, `MAX_COLOR_PRESETS`, `MAX_PRESET_NAME`, `BUILT_IN_PRESETS` (Sharkord, and Cord-like: Discord's default dark theme, its accent the text's white), `sameColors` |
| `rail.ts` | `initials`, `byPosition`, `membersOf`, `railOrder` (every server as the rail shows them), `RailRef`, `DropZone`, `railDrop`→`RailStep`s (what a drag does, as the calls to make; mirrors `shiver_core::rail`, folders dissolving included), `runRailSteps` (each client's `RailCalls`); tested in `rail.test.ts` |
| `events.ts` | `coreEventHook`: each app's `useCoreEvent` (the latest handler for each payload of a core event while mounted), over the `listen` it passes |
| `menus.ts` | the rail's menus as one item model: `MenuEntry`, `serverMenu` (`MenuServer`, `ServerMenuFacts`: a platform leaves out what it lacks; moves in a "Move to folder" submenu), `folderMenu`, `MenuAction`, `readMenuId` (item ids are `action:target`), `notifyLevelOf`; tested in `menus.test.ts` |
| `dms.ts` | `DmRow` (a conversation as either client lists it), `dmKey`, `filterDms` (person or server), `sortDms` (newest message first across servers, unknown last, then by name), `dmMeta`, `dmInitial`; tested in `dms.test.ts` |
| `time.ts` | `relativeTime` ("5m ago": the feed and the DM list) |
| `errors.ts` | `errorMessage` (a rejected call as words worth showing; re-exported by each app's `api.ts`) |
| `components/DmList.tsx` | `DmList`: the DM list both clients draw (search, rows with avatar, account · server, time; `touch` sizes it for a finger), styled by `components/dm-list.css` |
| `components/ui.css` | the look both clients share (fields, buttons, the dialog `card`, the server preview, the update bar, the connecting screen), the desktop client's; a `.touch` ancestor (mobile's root) sizes it for a finger. Imported first by each app's `main.tsx` |
| `components/icons.tsx` | inline `currentColor` icons: `BellIcon`, `MessagesIcon`, `SettingsIcon`, `PlusIcon`, the voice set, the settings set (`ChevronLeftIcon`, `MenuIcon`, `PaletteIcon`, `ShieldIcon`, `ServerIcon`, `InfoIcon`) |
| `components/SettingsLayout.tsx` | settings in the shape of Sharkord's own (styled by `settings.css`): `SettingsLayout` (`SettingsEntry`: Back and the title, the sections with an icon each, a drawer behind a menu button below 700px of room, the chosen one in a centred column), `SettingsCard`, `Setting` (a name, what it does, its controls), `Toggle` (a switch), `useSettingsDraft` (the draft against what was saved, its save bar once they differ, and `leave`, which keeps the screen open while there is something unsaved) |
| `components/UpdateNotice.tsx` | `UpdateNotice`: a newer release (take it: install or download, per app; Later; Skip this version) |
| `components/AddServerForm.tsx` | `AddServerForm` (`NewServer`): check the address, then sign in and add, or add for signing in on the server's own page; account label; `initialOrigin` (a `sharkord://` link's address, still checked) |
| `components/SignInForm.tsx` | `SignInForm`: sign a server in again, keeping the password if asked |
| `components/Confirm.tsx` | `Confirm`: a question as a dialog card; `armMs` holds the button back from the touch that opened it |
| `components/SettingsSections.tsx` | the settings sections both clients have, each a `SettingsCard` editing its fields of the app's draft: `AppearanceSection` (`AppearanceFields`: the presets, built-in and saved, the five colours, saving these as a preset), `NotificationsSection` (volume; the platform's own switches as children), `AboutSection` (version, check for updates, take it) |
| `components/Rail.tsx` | `Rail` (`RailServer`, `RailScreen`): direct messages, servers and folders (collapsing to a 2×2 preview), add, settings; badges, the offline mark, `notices` (an amber mark and a tooltip line, for a desktop api mismatch), the active ring; one drag on Pointer Events (a mouse lifts after a small move, a finger after a hold, which released still asks for the menu); children above settings. Styled by `components/rail.css` |
| `components/Menu.tsx` | `Menu`: a `MenuEntry` list drawn in the page (Android), kept on screen, closed by a press outside or Escape. Styled by `components/menu.css` |
| `components/RenameFolderForm.tsx` | `RenameFolderForm`: a folder's new name, as a dialog card |
| `components/Connecting.tsx` | `Connecting`: a spinner while a server comes up (with the countdown of a wait for a join), or the failure with a retry |
| `colors.ts` | `automaticTextColor`, `lift`, `liftHex` (`lift` worked out as `#rrggbb`), `contrastRatio`, `readableAccent` (the accent moved towards the text until it reads on the background) |
| `theme.ts` | `applyTheme` (`--shiver-*` vars on Shiver's own pages), `accentTextColor` |
| `session.ts` | `installSessionShim`, `takeSeedFromLocation`, `AUTO_LOGIN*` (session kept off disk) |
| `bridge/dom.ts` | `hasLucideIcon`, `ensureStyle`, `defineHook`, `onDomSettled` (hands callbacks what changed), `touched`, `isTopFrame`, `whenDocumentReady`, `installExternalLinks` |
| `bridge/sharkord.ts` | Sharkord store types; test-id selectors (`SIDEBAR`, `CHANNEL_ITEM`, `DM_ITEM`…); `sharkordStore`, `watchStore`; pure store reads (`notificationTarget`/`NotificationTarget`: a notification's author and channel from its title, in any of the client's languages, the channel before or after the author; `notificationAuthor`: the author of one that names its channel, the desktop api's; `dmPartnerId`, `findDmChannelIdByUserName`, `readDms`/`DmChannel`, `fileUrl`; tested in `sharkord.test.ts`); `notificationFlags` (Sharkord's own notification switches for a level), `notifyAllows`, `rowName`, `dmRowName`, `isDmRowFor` (a DM row by its exact name), `channelOfRow`, `markAllChannelsRead`, `installMuteStyles` (a muted row's unread pill hidden, by Sharkord's own dimming), `pressEscape`, `closeDialog` (Sharkord's topmost open dialog, as Escape), `IMAGE_VIEWER`, `RECONNECTING_OVERLAY`, `SIDE_PANEL`, `NARROW`, `SHIVER_PLUGIN_ID` |
| `bridge/messages.ts` | `watchDmActivity`: each DM conversation's latest message, and which the user closed, from the page's own connection (the join's `directMessages`; on a server older than 0.0.28 it asks `dms.get` once joined, under an id Sharkord's client ignores; then `messages.onNew`, sent and received alike, and `dms.onConversationUpdate`); `readDmTimes`, `noteClosed`, `closedDms` (on `window`, so every bridge script reads one list); installed before the page opens its socket |
| `bridge/mutes.ts` | Sharkord's own mute (0.0.27) as the face of Shiver's synced list: `watchSharkordMutes` (Storage reads and writes of `SHARKORD_MUTES_KEY`; a write diffed against what Sharkord last read or wrote names the channel toggled), `seedSharkordMutes` (Shiver's list, for Sharkord's next load), `readMutes`, `toggledBetween` |
| `bridge/plugin.ts` | `callPlugin`, `waitForPlugin`, `syncMutesWithPlugin`, `pushMutesToPlugin`, `storeReadFloor` |
| `bridge/channel-menu.ts` | "Mark all as read" added to Sharkord's channel menu (right-click, long press), which since 0.0.27 everyone gets, with its own mute item. `installChannelMenu` (`ChannelMenuSetup`), `channelPressed`, `requestChannelMenu`, `closeOpenMenu` (any Sharkord menu, as Escape) |
| `bridge/features.ts` | `installSoundVolume`, `installAttachmentCards`, `installVoiceColors`, `installSidePanels` (Sharkord's voice chat and thread panels cover a narrow page instead of staying hidden), `installComposerWrap` (a long pasted line wraps instead of pushing the send button off a phone), `closeSidePanel` |
| `bridge/theme.ts` | `ShiverTheme`, `applyPageTheme` (Sharkord's variables, the sidebars and top bars (`--card`) in the sidebar colour, and its accent-coloured text in the accent text colour) |

### Frontends

- **desktop/src**:
  - `main.tsx`: label → `App` | `Bell` | `NotificationsPopup`. Drops are refused wherever the rail
    doesn't take them, so none navigates a chrome webview.
  - `api.ts` (the `api.*` invoke wrappers, `errorMessage`); `events.ts` (`EVENTS`, and `useCoreEvent`
    bound to Tauri's `listen`). `App` answers `shortcut` in rail order, and `serverLink` with
    `followServerLink`: the server a link names, else the add screen filled in; also at boot.
  - `live.ts`: `useLive`, what the shell mirrors from the core's events: the feed, unread counts,
    the call, statuses, `mismatch` as the rail's `notices` (`MISMATCH_NOTICES`), and the update
    offer.
  - `rail-menu.ts` (`useRailMenu`: the rail's native menus, and each chosen item carried out;
    `AskPanel`); `types.ts` (`*_PAGES_KEPT`); `sounds.ts` (`playNotificationSound`).
  - `components/`: `SettingsPanel` (the shared `SettingsLayout` and its draft: the shared sections,
    and from `DesktopSections` `DesktopNotifications` (two switches over the shared volume),
    `VoiceSection`, `PermissionsSection`, `ServersSection` (spellcheck among them) and `LogFolder`
    under About), `HotkeyField`, `DirectMessagesPanel` (the shared `DmList` in a sidebar),
    `NotificationList`, `WelcomePanel`, `VoiceTile`.
  - `App` draws the shared `Rail` (with `VoiceTile` in it; its menus native, through `show_menu`),
    `AddServerForm`, `SignInForm`, `Confirm` (removing a server), `RenameFolderForm`, `Connecting`
    and `UpdateNotice`.
- **desktop/bridge/**:
  - `desktop-api.ts`: Sharkord's desktop api, version `DESKTOP_API_VERSION`, put on
    `window.sharkordDesktop` by `installDesktopApi`. See [Contracts](#sharkords-desktop-api).
    Tested in `desktop-api.test.ts`.
  - `index.ts`: `install`, the hooks and the drain queue, with `stalled` (Sharkord's own
    reconnecting gave up, or no client after `STALL_MS`); `installSpellcheck` (`spellcheck` off on the
    root, which every text box inherits; `__SHIVER_SET_SPELLCHECK__`); the channel menu on
    right-click, `installChannelRightClick`, whose "Mark all as read" goes to the core through the
    drain, as each mute toggled in Sharkord does.
  - `types.ts`: `ShiverConfig`, the queued shapes, the `window` hooks.
  - `storage.ts`: session seeding. `applyNotifyLevel` seeds Sharkord's switches to the server's
    `NotifyLevel` and marks the level it wrote in `shiver-notify-level`; `__SHIVER_SET_NOTIFY__` for a
    change.
  - `notifications.ts`: capture, filtered by that level; `ownMentionNames`/`mentionsAny` (the user's
    name and roles, for a level changed on a loaded page); `resolveDmChannels`; the message ping
    silenced.
  - `conversations.ts`: `openDmChannelId`, conversation mode (`showConversation`,
    `openDirectMessage`, `selectChannelWhenReady`; an open dialog such as Sharkord's settings is
    closed first), `onOpenDmFailure`.
  - `page-state.ts`: signed out, stalled, ready, fullscreen.
  - `dm-tracker.ts` (`createDmTracker`: the DM list for the core, each conversation's latest message,
    redrawn only when it changed) and `hotkey-handoff.ts` (`createHotkeyHandoff`: the page's
    shortcuts held for the drain and the core's answer passed back); both tested in
    `handoff.test.ts`.
  - `page-errors.ts`: `capturePageErrors` (uncaught errors, rejections and `console.error`, tokens
    blanked by `redact`, held for the drain by `takePageErrors`); tested in `page-errors.test.ts`.
  - `voice.ts` (read, control, and the lock: `setVoiceLocked`; a text row is told by its `#` icon)
    and `visibility.ts` (`setWindowHidden`).
- **mobile/src**:
  - `App.tsx`: the screens.
    - `boot` follows a `sharkord://` link first (`followServerLink`, also on `EVENTS.serverLink`),
      else opens the last server, or waits after `#home` (the quick rail couldn't be drawn).
    - `#open=` opens one; `#add` (`RAIL_SCREENS`, from the quick rail) opens that screen.
    - Its own rail isn't drawn while the page only passes through to a server, unless that takes
      `PASSING_THROUGH_MS`.
    - `__SHIVER_BACK__` reopens the last; `openAbout`: the sign-in or rename screen for one
      `subject`.
  - `api.ts`; `events.ts` (`EVENTS`: what the core tells Shiver's page; `useCoreEvent`); `types.ts`;
    `rail-menu.ts` (`railMenu`, `chooseMenuItem` with each screen's `MenuActions`,
    `runConfirmed`); `sessions.ts` (`useSessions`: the session states, polled while settings is
    open).
  - `components/`: `Boot` (rail menu confirmations and `Connecting`; after `#home`, a button back to
    the server left), `ActionConfirm` (`ConfirmAction`: the shared `Confirm` for remove, log out and
    forget password), `SettingsView` (the shared `SettingsLayout` and its draft, one for every
    section, saved from its bar), `ServerSignIn` (the shared `SignInForm`), `ServerList` (a
    notification level per server), `BackgroundNotifications`, `Sessions` (+`TrustedLinks`),
    `DirectMessages` (the shared `DmList`, `touch`).
  - `App` draws the shared `Rail` and its `Menu` (a held tile), `AddServerForm`, `RenameFolderForm`
    and `UpdateNotice`.
- **mobile/rail**:
  - `main.tsx`: the quick rail, and over the page what it opens: a tile's `Menu` once the WebView is
    wide, `ActionConfirm`, `RenameFolderForm`, `ServerSignIn`, `SettingsView`, `DirectMessages` and
    an error. Back closes the topmost, and a screen's back the overlay.
  - `channel.ts`: `shiverRail` (`say`, `onView`, `onBack`), and `invoke`/`listen`, which stand in
    for `@tauri-apps/api`'s in this build (`vite.rail.config.ts`), so `api.ts` reaches the core's
    `overlay`.
- **mobile/bridge**:
  - `index.ts`: install, `seedSession`, `installDmList` (`__SHIVER_DMS__`: this server's
    conversations once joined, when they changed), `__SHIVER_OPEN_DM__`.
  - `edit.ts`: `installEditInComposer`, an open edit's box over the composer's text area. Send saves
    it, with a bar with Cancel above.
  - `home.ts`: `setHome`, `goHome`, `openRail` and `installRailSwipe`. Back (after closing a menu or
    side panel) and a swipe past the drawer ask for the rail, which the core draws over the page.
  - `touch.ts`: `installTouchStyles`; `installReturnMakesALine` (Shiver's own untrusted Enter
    passes); `installLongPress` (a held message shows Sharkord's toolbar, and a held channel gets
    "Mark all as read" in the menu Sharkord opens; `HOLD_MS`/`HOLD_SLOP`); `drawerIsOpen`;
    `openConversation` (then closes the drawer over it).
  - `zoom.ts` (`installImageZoom`: pinch, pan and double tap in Sharkord's full-screen picture, which
    only knows the mouse), `reactions.ts` (`installReactionNames`: a held reaction shows who
    reacted), and `reconnect.ts` (`installAutoReconnect`; `installQuietReconnect`: Sharkord's own
    reconnecting dialog hidden, and an accent spinner above the chat box while it retries).
  - `document-start.ts`: the seed, `__SHIVER_OPEN__`, `__SHIVER_SEEN__` (each channel's latest
    message, read by the core with the mutes), the mute watch (so it sees Sharkord read its mutes),
    and the theme.
  - `types.ts` (`ShiverConfig`).

### plugin (Sharkord companion)

- **`server/index.js`**: the whole server half, in one file, so that a server reloading only its
  entry can't mix it with stale copies of others. Its sections:
  - rows: `createRows` (serialised per-user rows), `createLimiter` (a per-key rate limit);
  - file names: `installFileNaming`, `uniqueName` (a random suffix; separators and control
    characters replaced), `splitName`;
  - settings: `createSettings`, `mutedFrom`, `floorFrom` (mutes, unread floor);
  - page options: `OPTION_SETTINGS`, `createOptions` (the admin's switches for what the client half
    draws, pushed to every page when one changes: `roleColors`);
  - statuses: `createStatuses`, `statusFrom` (custom statuses);
  - push: `createPush`, `endpointsFrom`, `normaliseEndpoint`, `deliver` (UnifiedPush delivery), and
    the SSRF vetting: `isPrivateAddress`, `vetEndpoint`, `REFUSED`;
  - loading: `onLoad` registers the actions (see [Contracts](#the-companion-plugin)), reads and
    writes rate limited per user. A banned user's endpoints are cleared (`user:banned`), and
    wake-ups skip banned users and users without `USE_PLUGINS`. `onUnload`, `adoptOldStore`,
    `primeFromUserRows`.
- **`client/index.js`**: the client half, in everyone's page (Shiver or a browser).
  - It announces itself as `__SHIVER_PLUGIN__` (`{version}`), and relays the bridge's calls to
    server actions (`callPlugin`).
  - Custom statuses: the member list, profile card and user settings slots, and a button beside the
    settings gear.
  - Usernames in their role colour (`MemberColor` in the member list by id; message headers, replies
    and mentions by name), unless the admin turned `roleColors` off.
  - The newest copy in a page (an update imports another) answers and draws.
- Tests: `plugin/test/plugin.test.js`.

## Flows

### Opening a server on desktop

1. The rail calls `select_server`. A server with no page yet gets one: `token_for_new_page` reads
   its session from the keychain, through `session`, which renews a due session in the background
   and waits for a missing or expired one.
2. `build_page_webview` creates the page, pinned to the entry's origin (`on_navigation` with
   `is_same_origin`, new windows refused), in the entry's own profile. Its initialisation script is
   `bridge_script`: that entry's config, session included, then the bridge, run only in the top
   frame on that origin.
3. In the page, `installSessionShim` (`shared/web/session.ts`) serves the session to Sharkord's
   auto-login from memory, so it never reaches the page's storage.
4. The drain polls the page through `__SHIVER_DRAIN__` and applies what it reports: ready, signed
   out, notifications, mutes, voice, links, shortcuts and errors.

### Opening a server on Android

1. `select_server` waits for the server's join place (`Joins`), counting down on Shiver's page.
2. `show_server` navigates the one webview to the server, with the session in a
   `#shiver-seed=<key>.<token>` fragment. The key is derived from a per-launch secret and the page's
   origin (`seed_key_for`).
3. `document-start.ts` runs before the page's own scripts. It takes the seed out of the URL, checks
   the key, and installs the session shim.
4. Once the page has loaded, `install_bridge` evaluates the bridge with that entry's config
   (`PageContext`), checked again against the page's origin.

### Notifications, unread counts and direct messages

- **A server with a page**: its bridge captures Sharkord's own notifications, filtered by the
  server's notification level (`desktop/bridge/notifications.ts`), and the drain hands them to the
  feed (`Feed::push`). `notify::announce` raises a system notification while Shiver isn't in front.
- **A server without a page**: the core's socket (`watch` on desktop, `inbox` on Android, both over
  `sharkord_client::watch`) counts unread from read states, and reports new messages, which reach the
  same feed.
- **Direct messages** come from both: the page's own connection (`watchDmActivity`) and the
  socket's join. `set_dms` on desktop and `collect_dms` on Android keep the latest time any source
  gave.
- **On Android while Shiver is closed**, UnifiedPush (`push`) wakes the phone for a server whose
  companion plugin has its endpoint.

### Sessions: renewal and recovery

- Sharkord's sessions last seven days and can't be refreshed, so Shiver signs in again with the
  stored password: in the background once a session is due (`jwt::freshness`; at most once per entry
  per five minutes, through `Renewals`), and first when it has expired.
- A page whose seeded session is refused reports itself signed out. `session::recover` signs in
  again and rebuilds the page (`Recovery`). If it can't, the shell is told once
  (`shiver://signed-out`) and asks the user.
- A log-out or removal holds `CREDENTIALS`, so a renewal finishing meanwhile can't store a session
  again.

### A link from a page

1. The bridge queues what the user opens away from the page (`installExternalLinks`).
2. The drain takes the queue from the page on screen only, rationed per server (`Openings`).
3. `links::decide` refuses anything but http(s), and links carrying credentials. It opens a site
   the user trusts from that server, and otherwise asks in a native dialog that names the site first
   (`webviews/links.rs` on desktop, `ask_to_open` on Android).

## Contracts

### Page hooks: desktop

Desktop bridge ↔ core: `__SHIVER_DRAIN__` (page→core queue) and core→page
`__SHIVER_SET_THEME__`, `_SET_MUTED__`, `_SET_HIDDEN__`, `_CONVERSATION__`,
`_SELECT_CHANNEL__`, `_MARK_ALL_READ__`, `_VOICE__`, `_SET_VOICE_LOCK__`, `_SET_SOUND_VOLUME__`,
`_SET_ATTACHMENT_CARDS__`, `_SET_READ_FLOOR__`, `_SET_NOTIFY__`, `_HOTKEY__`, `_HOTKEYS_SET__`.
Sharkord's desktop api (`window.sharkordDesktop`, `desktop/bridge/desktop-api.ts`) is the page's side
of those last two and of `_SELECT_CHANNEL__`'s target.

### Page hooks: Android

`__SHIVER__` (config), `__SHIVER_MUTED__`, `__SHIVER_OPEN__`, `__SHIVER_SEEN__`, `__SHIVER_DMS__`,
`__SHIVER_CLEARED__`, `__SHIVER_BACK__`, `__SHIVER_OPEN_DM__`, `__SHIVER_MOBILE_INSTALLED__`,
`__SHIVER_SESSION_SHIM__`, `__SHIVER_SET_THEME__` (with the shared `_SET_SOUND_VOLUME__` and
`_SET_ATTACHMENT_CARDS__`: a settings change applied in place).

### Sharkord's desktop api

`desktop/bridge/desktop-api.ts` puts version `DESKTOP_API_VERSION` on `window.sharkordDesktop`
(`installDesktopApi`).

- A client asking for another version stays a web page, and its mismatch (`desktopMismatch`) is
  reported to the core.
- A client that connects at that version hands over:
  - its notifications whole (`readNotification`: the message, or a plugin's, remembered by a
    number);
  - its connection state (`connectionState`);
  - its shortcuts (`readBindings`, `readHotkeyErrors`, and `pressHotkey`: presses and releases,
    `keyUp: true`).
- `openNotification` passes a click back to the client's own handling, waiting for a client that is
  still starting.
- The top bar's server-list and desktop-settings buttons are hidden, since the rail always shows and
  has its own settings.

### Deep links

`sharkord://add-server?url=<address>` (`SHARKORD_SCHEME`), the link a Sharkord sign-in page offers
for its desktop app. `add_server_link` turns it into an origin under the same rules as a typed
address. It only ever fills in the add screen, or opens the server if it is already in the rail.
Shiver takes the scheme over from Sharkord's own desktop app.

### The companion plugin

- **Actions** (`onLoad` in `plugin/server/index.js`): `getOptions`, `setStatus`, `getStatuses`,
  `getOwnStatus`, `getMutedChannels`, `setMutedChannels`, `setReadFloor`, `setPushEndpoint`,
  `clearPushEndpoint`.
- **The announcement**: the client half sets `__SHIVER_PLUGIN__` (`{version}`) in every page.
  `shared/web/bridge/plugin.ts` waits for it (`waitForPlugin`) and calls the actions through it
  (`callPlugin`).
- **Releases**: `shiver-plugin-<version>.zip` and `SHA256SUMS.txt`, which the server manager
  installs.

### With the other Shiver repos

See [docs/standards.md](docs/standards.md#contracts-between-the-repos).
