#!/usr/bin/env python3
"""Checks what the bridges and the companion plugin rely on in Sharkord's client still exists there.

The bridges (and the plugin's client half) read Sharkord's page rather than an API, so a Sharkord
release can break them without any Shiver test noticing. This checks, against a Sharkord checkout:

- every `data-testid` they match is one of Sharkord's test ids,
- every `sharkord-*` storage key Shiver reads or seeds is one the client uses,
- every class they match with `[class~="..."]` appears in the client's source (or its UI package),
- and the other conventions named in `CONTRACTS` (notification titles, DM channel names, the
  plugin store, the image portal, the member-list slot, the DM list's times, the new-message
  subscription and who it reaches, and the screens that tell a page gave up connecting),
- and, once Sharkord's client has its desktop api, the parts of it the desktop bridge speaks
  (`DESKTOP_CONTRACTS`), so a change of its version fails here rather than in a user's app.

Usage: check-sharkord.py [SHARKORD_CHECKOUT] [--ref REF | --latest]
Without a checkout, Sharkord is cloned at `SHARKORD_REF` (or `--ref`, or its default branch with
`--latest`) into a temporary directory.
"""

import argparse
import json
import pathlib
import re
import subprocess
import sys
import tempfile

# The Sharkord commit the bridges were last checked against (client 0.0.28). Bump it after checking
# the bridges against a newer release.
SHARKORD_REF = "4b9282303b558fe7df53ec0a49fdf2ccc84d11a9"
SHARKORD_REPO = "https://github.com/sharkord/sharkord"

ROOT = pathlib.Path(__file__).resolve().parent.parent

# What Shiver reads Sharkord's page with: the bridges, the shared page code, the plugin's client half,
# and the Android core's token read.
SHIVER_SOURCES = [
    *ROOT.glob("shared/web/**/*.ts"),
    *ROOT.glob("desktop/bridge/*.ts"),
    *ROOT.glob("mobile/bridge/*.ts"),
    *ROOT.glob("plugin/client/*.js"),
    ROOT / "mobile/src-tauri/src/inbox/sessions.rs",
]

# (what Shiver relies on, a Sharkord path relative to the checkout, a regex that must match there)
CONTRACTS = [
    (
        "notification titles `Author (DM)` / `Author in #channel`, or their translations (notificationTarget)",
        "apps/client/src/features/server/messages/actions.ts",
        r"\(DM\)`[\s\S]*in #\$\{|'common:notificationTitleDm'[\s\S]*'common:notificationTitleChannel'",
    ),
    (
        "DM channels named `DM - <userA>:<userB>` (dmPartnerId)",
        "apps/server/src/routers/dms/open-direct-message.ts",
        r"`DM - \$\{[^}]+\}:\$\{[^}]+\}`",
    ),
    (
        "text channel rows draw lucide's `Hash` icon (the voice lock tells text rows by it)",
        "apps/client/src/components/left-sidebar/channels.tsx",
        r"import \{[^}]*\bHash\b[^}]*\} from 'lucide-react'[\s\S]*<Hash className",
    ),
    (
        "the plugin store on `window.__SHARKORD_STORE__` (sharkordStore)",
        "apps/client/src/features/server/plugins/plugin-store.ts",
        r"window\.__SHARKORD_STORE__\s*=",
    ),
    (
        "the full-screen image portal `#imagePortal` (IMAGE_VIEWER)",
        "apps/client/src/components/fullscreen-image/content.tsx",
        r"getElementById\('imagePortal'\)",
    ),
    (
        "every DM conversation's latest message time (watchDmActivity, the core's `parse_dms`)",
        "apps/server/src/db/queries/dms.ts",
        r"lastMessageAt:\s*max\(messages\.createdAt\)",
    ),
    (
        "each conversation carrying when this user closed it (`closedDms`, `parse_dms`)",
        "apps/server/src/db/queries/dms.ts",
        r"closedAt:\s*row\[getClosedAtKey\(row, userId\)\]",
    ),
    (
        "the join listing the conversations as `directMessages` (watchDmActivity, the core's `join_dms`)",
        "apps/server/src/routers/others/join.ts",
        r"getDirectMessageConversations\(ctx\.user\.id\)[\s\S]*\bdirectMessages\b",
    ),
    (
        "`dms.onConversationUpdate` publishing a conversation that opened, closed or moved (watchDmActivity)",
        "apps/server/src/routers/dms/index.ts",
        r"onConversationUpdate:\s*onDmConversationUpdateRoute",
    ),
    (
        "Sharkord reading its mutes from storage once, at load (`watchSharkordMutes` sees that read)",
        "apps/client/src/features/app/slice.ts",
        r"mutedChannelIds:\s*getLocalStorageItemAsJSON<number\[\]>\(LocalStorageKey\.MUTED_CHANNEL_IDS\)",
    ),
    (
        "each mute toggle writing the whole list back to storage (`watchSharkordMutes` diffs the writes)",
        "apps/client/src/features/app/actions.ts",
        r"export const toggleChannelMuted[\s\S]*setLocalStorageItemAsJSON\(\s*LocalStorageKey\.MUTED_CHANNEL_IDS",
    ),
    (
        "a muted channel's name span dimmed `opacity-50` (its unread pill hidden, `installMuteStyles`)",
        "apps/client/src/components/left-sidebar/channels.tsx",
        r"cn\('flex-1', \{ 'opacity-50': isMuted \}\)",
    ),
    (
        "the channel menu given to everyone, with its mute item (Shiver adds only \"Mark all as read\")",
        "apps/client/src/components/context-menus/channel/index.tsx",
        r"<ContextMenuItem onClick=\{onToggleMuteClick\}>",
    ),
    (
        "role mentions as `data-type=\"role-mention\"` spans with `data-role-id` (the core's `mentioned`)",
        "packages/shared/src/helpers/has-mention.ts",
        r"ROLE_ID_PATTERN = /\\bdata-role-id=",
    ),
    (
        "the client subscribing to new messages only once it has joined (watchDmActivity asks then)",
        "apps/client/src/features/server/actions.ts",
        r"joinServer\.query[\s\S]*?initSubscriptions\(\)",
    ),
    (
        "new messages on the `messages.onNew` subscription (watchDmActivity)",
        "apps/server/src/routers/messages/index.ts",
        r"onNew:\s*onMessageRoute",
    ),
    (
        "a new message published to everyone who can see its channel, its author included (watchDmActivity)",
        "apps/server/src/db/publishers.ts",
        r"getAffectedOnlineUserIdsForChannel\([\s\S]*?VIEW_CHANNEL[\s\S]*?publishFor\(affectedUserIds, targetEvent",
    ),
    (
        "the page's client speaking over one WebSocket (watchDmActivity)",
        "apps/client/src/lib/trpc.ts",
        r"wsLink\(",
    ),
    (
        "the plugin store exposed as the client starts, before any join (isStalled: none yet is a page that never loaded)",
        "apps/client/src/main.tsx",
        r"(?m)^exposePluginStore\(\);",
    ),
    (
        "the screen shown once the client gives up reconnecting, with the `WifiOff` icon (isStalled)",
        "apps/client/src/screens/disconnected/index.tsx",
        r"<WifiOff ",
    ),
    (
        "the member-list slot rendered inside the member's row, after the name (the plugin's MemberColor)",
        "apps/client/src/components/right-sidebar/index.tsx",
        r"TestId\.MEMBER_ITEM\}[\s\S]*?\{name\}[\s\S]*?slotId=\{PluginSlot\.MEMBER_LIST_ITEM\}",
    ),
    (
        "shortcuts recorded as `ctrl+alt+shift+meta` then the key's code (the core's `page_hotkeys::shortcut_for`)",
        "packages/ui/src/lib/hotkey.ts",
        r"parts\.push\('ctrl'\);[\s\S]*parts\.push\('alt'\);[\s\S]*parts\.push\('shift'\);[\s\S]*parts\.push\('meta'\);"
        r"[\s\S]*withModifiers\(event, event\.code\)",
    ),
]

# Sharkord's desktop api, which the desktop bridge offers (`desktop/bridge/desktop-api.ts`). Checked
# only against a client that has it (`DESKTOP_API`); one without it is a web page to the bridge.
DESKTOP_API = "packages/shared/src/desktop.ts"

DESKTOP_CONTRACTS = [
    (
        "desktop api version 1 (`DESKTOP_API_VERSION`)",
        DESKTOP_API,
        r"export const DESKTOP_BRIDGE_VERSION = 1;",
    ),
    (
        "the api found on `window.sharkordDesktop` and connected with the client's version",
        "apps/client/src/helpers/desktop-api.ts",
        r"window\.sharkordDesktop[\s\S]*\.connect\(DESKTOP_BRIDGE_VERSION\)",
    ),
    (
        "every method the bridge's api offers, and none more the client calls",
        DESKTOP_API,
        r"export type TDesktopBridge = \{\s*getServerListVisible:[\s\S]*?onServerListVisibleChange:[\s\S]*?"
        r"toggleServerList:[\s\S]*?openSettings:[\s\S]*?setUnreadState:[\s\S]*?setConnectionState:[\s\S]*?"
        r"setVoiceActive:[\s\S]*?startSystemAudio:[\s\S]*?stopSystemAudio:[\s\S]*?showNotification:[\s\S]*?"
        r"setHotkeys:[\s\S]*?getHotkeyCapabilities:[\s\S]*?onHotkey:[\s\S]*?onHotkeyErrors:[\s\S]*?"
        r"onNotificationClick:[^;]*;\s*\};",
    ),
    (
        "hotkey capabilities as `keyUp` and `systemManaged` (`getHotkeyCapabilities`)",
        DESKTOP_API,
        r"export type TDesktopHotkeyCapabilities = \{\s*keyUp: boolean;\s*systemManaged: boolean;\s*\};",
    ),
    (
        "a press handed over with its phase, `down` (`pressHotkey`)",
        DESKTOP_API,
        r"export type TDesktopHotkeyPhase = 'down' \| 'up';[\s\S]*listener: \(actionId: string, phase: TDesktopHotkeyPhase\) => void",
    ),
    (
        "a message's notification naming the message and the thread it replies in (`readNotification`)",
        "apps/client/src/features/server/messages/actions.ts",
        r"type: 'message',\s*channelId,\s*messageId: message\.id,\s*parentMessageId: message\.parentMessageId",
    ),
    (
        "a click passed to the client leading to the message, or to the plugin (`openNotification`)",
        "apps/client/src/components/notifications-controller/index.tsx",
        r"dispatchPluginNotificationClick\(target\.pluginId, target\.data\)[\s\S]*jumpToMessage\(target\)",
    ),
    (
        "the server-list button first in the top bar's left cell (hidden: Shiver's rail is always shown)",
        "apps/client/src/components/top-bar/index.tsx",
        r"grid-cols-\[1fr_minmax\(320px,1\.4fr\)_1fr\][\s\S]*?<div className=\"flex min-w-0 items-center gap-2\">\s*"
        r"\{isDesktop && \(\s*(?:<>\s*)?<Button[^>]*onClick=\{toggleServerList\}",
    ),
]


def checkout(directory, ref, latest):
    def git(*args):
        subprocess.run(["git", *args], cwd=directory, check=True, stdout=subprocess.DEVNULL)

    git("init", "--quiet")
    git("remote", "add", "origin", SHARKORD_REPO)

    if latest:
        git("fetch", "--quiet", "--depth", "1", "origin", "HEAD")
    else:
        git("fetch", "--quiet", "--depth", "1", "origin", ref)

    git("checkout", "--quiet", "FETCH_HEAD")

    return directory


def shiver_text():
    return "\n".join(path.read_text(encoding="utf8") for path in SHIVER_SOURCES if path.is_file())


def client_text(sharkord):
    """The client's source and the UI components it is built from."""
    sources = [path for part in ("apps/client/src", "packages/ui/src") for path in (sharkord / part).rglob("*")]

    return "\n".join(
        path.read_text(encoding="utf8", errors="replace")
        for path in sources
        if path.suffix in {".ts", ".tsx", ".css"} and "__tests__" not in path.parts
    )


def main():
    parser = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    parser.add_argument("checkout", nargs="?", type=pathlib.Path, help="an existing Sharkord checkout")
    which = parser.add_mutually_exclusive_group()
    which.add_argument("--ref", help=f"commit or tag to clone (default {SHARKORD_REF})")
    which.add_argument("--latest", action="store_true", help="clone Sharkord's default branch")
    args = parser.parse_args()

    if args.checkout:
        if args.ref or args.latest:
            parser.error("a checkout is checked as it is; --ref and --latest only choose what to clone")

        return check(args.checkout)

    # removed afterwards, however the check ends (git's read-only objects do not stop that)
    with tempfile.TemporaryDirectory(prefix="sharkord-", ignore_cleanup_errors=True) as directory:
        return check(checkout(pathlib.Path(directory), args.ref or SHARKORD_REF, args.latest))


def check(sharkord):
    ours = shiver_text()
    theirs = client_text(sharkord)
    test_ids = set(
        re.findall(r"=\s*'([^']+)'", (sharkord / "packages/shared/src/test-ids.ts").read_text(encoding="utf8"))
    )
    problems = []

    for test_id in sorted(set(re.findall(r'data-testid="([^"]+)"', ours))):
        if test_id not in test_ids:
            problems.append(f"test id {test_id!r} is not in packages/shared/src/test-ids.ts")

    their_keys = set(re.findall(r"""['"`](sharkord-[a-z0-9-]+)['"`]""", theirs))

    for key in sorted(set(re.findall(r"""['"](sharkord-[a-z0-9-]+)['"]""", ours))):
        if key not in their_keys:
            problems.append(f"storage key {key!r} is not used by the client")

    for token in sorted(set(re.findall(r'class~="([^"]+)"', ours))):
        # a whole class, inside a class string (`lg:flex` must not match `flex`)
        if not re.search(r"(?<![\w:/\[-])" + re.escape(token) + r"(?![\w:/\]-])", theirs):
            problems.append(f"class {token!r} no longer appears in the client")

    # translated titles keep their shape: a bracketed marker, or one word before the channel
    for locale in sorted((sharkord / "apps/client/src/i18n/locales").glob("*/common.json")):
        titles = json.loads(locale.read_text(encoding="utf8"))
        shapes = {
            "notificationTitleDm": r"\{\{author\}\} ?[(（][^()（）\s]{1,8}[)）]",
            "notificationTitleChannel": r"\{\{author\}\} \S+ #\{\{channel\}\}",
        }

        for key, shape in shapes.items():
            if key in titles and not re.fullmatch(shape, str(titles[key])):
                problems.append(f"{key} in {locale.parent.name} is not the shape notificationTarget reads")

    has_desktop_api = (sharkord / DESKTOP_API).is_file()

    for what, path, pattern in CONTRACTS + (DESKTOP_CONTRACTS if has_desktop_api else []):
        file = sharkord / path

        if not file.is_file() or not re.search(pattern, file.read_text(encoding="utf8")):
            problems.append(f"{what}: not found in {path}")

    if problems:
        print("Sharkord no longer matches what the bridges and the plugin expect:", file=sys.stderr)

        for problem in problems:
            print(f"  - {problem}", file=sys.stderr)

        return 1

    checked = len(CONTRACTS) + (len(DESKTOP_CONTRACTS) if has_desktop_api else 0)
    desktop = "with" if has_desktop_api else "without"

    print(
        f"Sharkord matches the bridges and the plugin ({len(test_ids)} test ids, {checked} conventions checked; "
        f"a client {desktop} the desktop api)"
    )

    return 0


if __name__ == "__main__":
    sys.exit(main())
