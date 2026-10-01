# Shiver companion plugin

Optional. Shiver works fine against a stock Sharkord server; installing this adds what the client
cannot do alone: **your Shiver settings are kept on the server, per user, so they follow you to
every device.** Today that means your muted channels, your custom status, and the UnifiedPush
endpoints that let this server wake your phone while Shiver is closed. It also draws usernames in
their role colour, for everyone on the server. Statuses, push and colours have sections of their own
below.

Needs Sharkord **0.0.25 or newer** (plugin SDK 2). A server on 0.0.24 refuses to load it, and the
0.0.24 version of this plugin refuses to load on 0.0.25 — the SDK version is checked both ways.

## Installing

1. Copy this directory into your server's plugin folder as `shiver`:

   ```
   <sharkord data dir>/plugins/shiver/
   ```

   The data directory is `~/.config/sharkord` by default, or `apps/server/data` when running from
   source. The folder name must match the manifest id, which Sharkord requires.

2. Restart the server, then enable **Shiver** in Settings → Plugins.

That is the whole install. Nothing else about the server changes.

**Updating:** replace the folder and restart the server. The server half is one file,
`server/index.js`, so no older copy of a part of it can be left running beside a newer one.

## What it stores

One row per user, in Sharkord's own per-plugin storage (the `plugin_user_data` table):

```json
{
  "mutedChannels": [4, 17],
  "status": "back on Thursday",
  "pushEndpoints": ["https://ntfy.example.com/up1a2b3c…"]
}
```

Three fields, and a row carries only the ones that user has actually set:

| Field | What it is | Who can see it |
| --- | --- | --- |
| `mutedChannels` | channels that user has muted in Shiver | them, and anyone who can read the database |
| `status` | the line they wrote about themselves | **everyone on the server** — that is the point of it |
| `pushEndpoints` | URLs this server posts an empty body to, to wake their devices | them, and anyone who can read the database |

**No message content ever passes through the plugin**, and nothing else about a user is recorded.

`pushEndpoints` is the one worth treating carefully. An endpoint is a capability: anyone holding
it can make that person's phone buzz, through a relay outside this server. It is never shown to
other users and never leaves the server except as the request that uses it.

The row is the host's to keep: it is size-capped by the server, deleted with the user, and every
row is deleted when the plugin is removed. That last part is Shiver's rule — it forgets a user once
they leave — enforced by Sharkord rather than by us. An earlier version of this plugin kept a JSON
file and deleted rows on the `user:left` event instead; on 0.0.25 that event turned out to fire on
every *disconnect*, so it would have wiped a user's settings each time they closed the app. The file
is read once on load and carried into the host's storage, then deleted.

**A server admin can read the database.** Which channels you muted is not especially sensitive; a
push endpoint is more so, because it can be used. Both stop being private to your machine once the
plugin is installed, which is the trade for having them follow you between devices.

## How it works

Sharkord's plugin *settings* are a single key/value space for the whole server, so they cannot hold
per-user data. Since 0.0.25 `plugins.getUserData` / `setUserData` can: they read and write a row
belonging to the calling user, authenticated by the server, so a user can only ever reach their own.

Shiver's bridge cannot call those itself — they live on Sharkord's plugin store, and only code served
out of `/plugin-bundle/shiver/` is this plugin. So `client/index.js` is a relay: it answers messages
posted inside the same page and passes them on to one of this plugin's **server actions**. It accepts
only same-window, same-origin messages, and only named operations — it cannot be talked into reading
or writing anything else.

Every write to a user's row goes through the server, one change per user at a time. The host's
`setUserData` replaces the whole row, so two writers (a mute from the page, a status from the server)
used to be able to lose each other's change; they cannot now. A row that cannot be read is never
overwritten.

| Action | Does |
| --- | --- |
| `getOptions` | What the admin turned on for everyone's page (today: role colours) |
| `getMutedChannels` / `setMutedChannels` | Reads or replaces the caller's muted channels |
| `setReadFloor` | Stores the caller's shared unread floor, so their devices agree on one badge |
| `setStatus` | Sets the caller's status line, and pushes it to everyone connected (rate limited) |
| `getStatuses` / `getOwnStatus` | Every status currently set, or the caller's own |
| `setPushEndpoint` | Registers an endpoint **after checking it** — see the section below (rate limited) |
| `clearPushEndpoint` | Drops one endpoint, or all of the caller's when none is named |

Each acts on `invoker.userId`, which Sharkord authenticates, so none of them can be aimed at
another account. The other writes share a limit of 30 a minute per user, and the reads one of 60.

## Merge behaviour

The first time a device meets the plugin, Shiver unions that device's mute list with the server's —
a mute made before the plugin was installed and a mute made on another device are both deliberate.
After that first reconcile the server's copy is the source of truth, and every later change is
pushed to it.

If the plugin is not installed, Shiver detects that (`window.__SHIVER_PLUGIN__` is absent), keeps using
its local list, and nothing else changes.

## Waking a phone that has Shiver closed

Shiver's Android client notifies from its own connections, which only run while its process does —
so once Android closes it, the phone goes quiet. The usual fix is Google FCM, which would make a
self-hosted client depend on Google. This plugin does it the other way, with **UnifiedPush**.

Installing the plugin is all a server admin has to do. Each user's phone needs a UnifiedPush
distributor (ntfy is the usual one, self-hosted or not) and picks it under
*Settings → Notifications while Shiver is closed*; Shiver registers one endpoint per server and hands it
to this plugin, which stores it against that user.

What it then does, on every message:

- Does **not** skip people who look connected. It used to, until 0.4.4: Sharkord reports a user
  as gone only when their *last* socket closes, and Shiver holds one per server from every device
  it runs on — so "online" meant "Shiver is installed somewhere", and no push was ever sent. The
  phone decides instead, which is where the knowledge is.
- Skips channels that user has muted, using the same row this plugin already keeps.
- Skips channels Sharkord says that user cannot view.
- Posts an **empty body** to each remaining endpoint, at most once every ten seconds per user.

The body is empty on purpose. The distributor is a relay outside this server, so anything in the
payload is something a third party reads. Shiver only needs "look again"; it finds out what actually
arrived over its own connection. A leaked endpoint is therefore worth spam, not messages.

**One thing to be aware of as an admin.** The endpoint is a URL supplied by a user that this server
then fetches, so it is checked before it is stored *and again immediately before every send*: https
on port 443 only, the hostname resolved, and refused unless every address it resolves to is public. IPv6 is
checked as an allow-list of global unicast, so NAT64, 6to4-to-private, Teredo, IPv4-mapped and similar
tunnelled forms are refused too.

**The request is made to the exact address that passed the check.** Each wake-up is a TLS connection
to that address, with the certificate verified against the endpoint's hostname — so a name that
changes its answer between the check and the connection (DNS rebinding) cannot move the request.
Redirects are never followed, and at most 64 wake-ups are in flight at once across the server; the
rest wait their turn (oldest first, up to 4096) rather than being dropped.

A refused registration is told only that it was refused, never why, so the check cannot be used to
map which hostnames exist on this server's network.

Nothing here is required. A server without this plugin simply cannot wake a closed phone, which is
exactly how Shiver behaved before.

## Custom statuses

A line each user writes about themselves, shown beside their name in the member list while they are
online. Sharkord's own `status` is presence — online, idle, offline, set by the server — so this is
a different thing rather than a replacement for it.

It is stored in that user's row on this server, capped at 100 characters and flattened to a single
line, and pushed to everyone currently connected so open member lists update without a refresh.

**Shown** through Sharkord's own `member_list_item` slot, so it reaches anyone on the server — in a
browser as much as in Shiver.

**Set** in either of two places, both ending at the same server action, and both drawn by the plugin
for everyone: a button beside Sharkord's settings gear that does it in one click (centred above the
keyboard on a touch screen), and a Status field in Sharkord's own user settings. Both appear only to
users Sharkord lets use plugins.

## Role colours

Sharkord colours a role's badge but never the names of the people who hold it. The plugin draws
names in the colour of their role: in message headers, reply previews, mentions (except your own,
which keep Sharkord's "you were mentioned" colour) and the member list. A user's colour is their
first coloured role in the server's role order, else the default role's; white counts as no colour,
as it does in Sharkord.

Member-list names are coloured by user id, through the `member_list_item` slot. Messages, replies and
mentions carry no id in Sharkord's page, so those are matched by the name shown.

An admin can turn it off for everyone under **Settings → Plugins → Shiver → Colour usernames by
role**. Open pages follow the change without a reload. Shiver's apps draw no colours of their own, so
on a server without this plugin names stay Sharkord's plain colour.

## Conversation order

Shiver lists every server's direct messages together, newest message first. A server's page cannot
read when each conversation last had a message (Sharkord's client keeps that to its own DM list), so
on desktop, for a server whose page is open, Shiver asks the plugin: the `getDmTimes` action answers
with the time of the newest message in each of the caller's own conversations. Only DMs the caller
takes part in are answered (for a DM, Sharkord's channel permission is membership alone, so not
even an owner can ask about anyone else's), only the time is read, never the message, and nothing
is stored. Reads are rate limited per user like the plugin's others.

## Updating while people are connected

Sharkord imports the client half by a url that carries the plugin's version, so after an update a
page that stayed open holds two copies of it. The newest one answers and draws; older ones stand
down.
