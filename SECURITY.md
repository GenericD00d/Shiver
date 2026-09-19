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

- **Only Mozilla's root set is trusted**, not the OS trust store — `rustls-tls-webpki-roots` for the
  websocket and reqwest's `rustls-tls` for http. A server behind a private CA will not connect even
  if the machine trusts it. That is a deliberate trade and it is also why "could not reach the
  server" is sometimes a certificate problem.
- **https is required everywhere**, with no exemption for localhost or a private address.
- **DNS rebinding is not defeated** in the companion plugin's endpoint check. The endpoint is
  re-resolved and re-checked immediately before every send, which narrows the window to that
  request rather than closing it; pinning the connection to the vetted address would close it, and
  has not been done.
