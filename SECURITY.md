# Reporting a security problem

**Please do not open a public issue for a security problem.** Use GitHub's private reporting
instead: **Security → Advisories → Report a vulnerability** on
[this repository](https://github.com/GenericD00d/Shiver/security/advisories/new). That opens a
thread only you and the maintainer can see.

If that is not available to you for any reason, open an ordinary issue saying only that you have
something to report and asking for a way to send it — no details in the issue itself.

Expect a first reply within a week. This is one person's side project, not a company with an
on-call rota, and saying so is more useful than a service level I cannot meet.

## What this project is

Shiver is a desktop and Android client for [Sharkord](https://github.com/sharkord/sharkord). It
handles session tokens and, by default, passwords; both live in the OS keychain on desktop and in
`EncryptedSharedPreferences` on Android, never in a file.

**No independent security audit has been done.** The README says so and it is still true. What
exists is a full internal review of the source, whose findings are fixed in the current tree.

## What is in scope

Roughly, anything that breaks one of the boundaries Shiver claims:

- **Origin pinning.** A server's page should not be able to navigate its webview somewhere Shiver
  still treats as that server, reach another server's data, or reach Shiver's own core.
- **Credentials.** A session or a password reaching a file, a log, another origin, or a host the
  user did not name.
- **The companion plugin.** It is optional and server-side. The endpoint it fetches is supplied by
  a user, so anything that gets this server to make a request it should not is in scope — see
  `plugin/server/push.js`, which is where that check lives.
- **The updater.** Anything that would get code onto a user's machine without a signature that
  verifies against the key built into the binary.
- **The bridge.** It is injected into every Sharkord page and is the only Shiver code that runs on
  a server's origin.

## What is out of scope

- **A malicious Sharkord server showing you misleading content.** Shiver renders a server's own
  client; a server can always lie to its own users. What is in scope is that lie reaching *past*
  that server — to another server, to Shiver's chrome, or to the machine.
- **The installers being unsigned.** Known, stated in the README, and a money problem rather than a
  code one.
- **Anything requiring an attacker who can already read or write the app's data directory or the
  OS keychain.** At that point they are running as the user.
- Reports from automated scanners with no working path to an impact.

## Things worth knowing before you look

- **Servers are checked against Mozilla's root set only**, not the OS trust store —
  `rustls-tls-webpki-roots` for the websocket and reqwest's `rustls-tls` for http. A server behind a
  private CA will not connect even if the machine trusts it. That is a deliberate trade and it is
  also why "could not reach the server" is sometimes a certificate problem. The desktop updater is
  the exception: `tauri-plugin-updater` brings its own client, which uses the OS trust store; what it
  downloads must still carry a valid signature.
- **https is required everywhere**, with no exemption for localhost or a private address.
- **Server pages cannot call Shiver.** Tauri refuses commands from remote origins, and desktop also
  refuses any command not sent by Shiver's own webviews. Android reports a new page's origin late,
  so it refuses every command while a server is on screen or being opened, or while a page Shiver
  did not open (a step back through history) is loading on its way home.
- **Camera and microphone go only where you said yes.** On Android they go only to the server page
  on screen (never a frame inside it), after Shiver has asked you once for that server; a log out or
  removal forgets the answer. WebView2 on Windows asks per site itself, and WebKitGTK on Linux
  refuses. macOS is not yet covered: wry grants every page there, so a macOS build must not ship
  until Shiver handles it.
- **Only the user opens the browser.** A page's own navigations off its origin and its new windows
  are refused; the browser gets only links the user clicked and one window per click, rationed per
  server.
- **Sessions never reach a webview's storage.** A small script that runs before the page's own
  patches `Storage.prototype` so Sharkord's auto-login token and live session are served from
  memory (`shared/web/session.ts`). On desktop it is part of the initialization script; on Android
  the token arrives in a `#shiver-seed=` fragment that the document-start script removes before any
  page script runs, and is only accepted with a key derived from a per-launch secret held in that
  script and the page's origin, so a link cannot seed a session, even one from a server that has seen
  its own key. A URL carrying it is never handed to the browser, and an off-origin redirect
  before the server's first load is refused. It steps aside if the server refuses the token or the
  user signs in on the page; logging out of or removing a server clears what its page stored
  (on Android every account on one address shares that storage, so it is cleared for all of them).
- **What a server's page can learn on Android.** There is one webview, but the rail is on Shiver's
  own page, not in the server's. A server's page is handed its own entry's settings, mutes, session
  and push endpoint, and the address of Shiver's page (going home is a navigation there); nothing
  about other servers. It can navigate to Shiver's page, whose fragment can only ask to show the
  rail, or reopen a server (or say it failed) by id; nothing is changed from a URL. The core's
  events (unread counts, push, updates) are sent only while Shiver's page is up, because the one
  webview keeps that page's event listeners after it navigates to a server.
- **The plugin's push delivery is pinned.** The endpoint is resolved once, every address is
  checked, and the request is sent over TLS on port 443 to that vetted address with the hostname as
  SNI, so DNS rebinding cannot redirect it. Deliveries in flight are capped server-wide.
