#!/usr/bin/env python3
"""Checks what the bridges and the companion plugin rely on in Sharkord's client still exists there.

The bridges (and the plugin's client half) read Sharkord's page rather than an API, so a Sharkord
release can break them without any Shiver test noticing. This checks, against a Sharkord checkout:

- every `data-testid` they match is one of Sharkord's test ids,
- every `sharkord-*` storage key Shiver reads or seeds is one the client uses,
- every class they match with `[class~="..."]` appears in the client's source (or its UI package),
- and the other conventions named in `CONTRACTS` (notification titles, DM channel names, the
  plugin store, the image portal, the member-list slot, the DM list's times, and the new-message
  subscription and who it reaches).

Usage: check-sharkord.py [SHARKORD_CHECKOUT] [--ref REF | --latest]
Without a checkout, Sharkord is cloned at `SHARKORD_REF` (or `--ref`, or its default branch with
`--latest`) into a temporary directory.
"""

import argparse
import pathlib
import re
import subprocess
import sys
import tempfile

# The Sharkord commit the bridges were last checked against (client 0.0.25). Bump it after checking
# the bridges against a newer release.
SHARKORD_REF = "0eea660e02b0eb7c8784baa27580552fa4e1231f"
SHARKORD_REPO = "https://github.com/sharkord/sharkord"

ROOT = pathlib.Path(__file__).resolve().parent.parent

# What Shiver reads Sharkord's page with: the bridges, the shared page code, the plugin's client half,
# and the Android core's token read.
SHIVER_SOURCES = [
    *ROOT.glob("shared/web/**/*.ts"),
    *ROOT.glob("desktop/bridge/*.ts"),
    *ROOT.glob("mobile/bridge/*.ts"),
    *ROOT.glob("plugin/client/*.js"),
    ROOT / "mobile/src-tauri/src/inbox.rs",
]

# (what Shiver relies on, a Sharkord path relative to the checkout, a regex that must match there)
CONTRACTS = [
    (
        "notification titles `Author (DM)` / `Author in #channel` (notificationTarget)",
        "apps/client/src/features/server/messages/actions.ts",
        r"\(DM\)`[\s\S]*in #\$\{",
    ),
    (
        "DM channels named `DM - <userA>:<userB>` (dmPartnerId)",
        "apps/server/src/routers/dms/open-direct-message.ts",
        r"`DM - \$\{[^}]+\}:\$\{[^}]+\}`",
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
        "every DM conversation's latest message time from `dms.get` (watchDmActivity)",
        "apps/server/src/db/queries/dms.ts",
        r"lastMessageAt:\s*max\(messages\.createdAt\)",
    ),
    (
        "`dms.get` routed and asked of the page's own connection after it joins (watchDmActivity)",
        "apps/server/src/routers/dms/index.ts",
        r"get:\s*getDirectMessagesRoute",
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
        "the member-list slot rendered inside the member's row, after the name (the plugin's MemberColor)",
        "apps/client/src/components/right-sidebar/index.tsx",
        r"TestId\.MEMBER_ITEM\}[\s\S]*?\{name\}[\s\S]*?slotId=\{PluginSlot\.MEMBER_LIST_ITEM\}",
    ),
]


def checkout(ref, latest):
    directory = pathlib.Path(tempfile.mkdtemp(prefix="sharkord-"))

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
    which.add_argument("--ref", default=SHARKORD_REF, help="commit or tag to clone")
    which.add_argument("--latest", action="store_true", help="clone Sharkord's default branch")
    args = parser.parse_args()

    sharkord = args.checkout or checkout(args.ref, args.latest)
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

    for what, path, pattern in CONTRACTS:
        file = sharkord / path

        if not file.is_file() or not re.search(pattern, file.read_text(encoding="utf8")):
            problems.append(f"{what}: not found in {path}")

    if problems:
        print("Sharkord no longer matches what the bridges and the plugin expect:", file=sys.stderr)

        for problem in problems:
            print(f"  - {problem}", file=sys.stderr)

        return 1

    print(f"Sharkord matches the bridges and the plugin ({len(test_ids)} test ids, {len(CONTRACTS)} conventions checked)")

    return 0


if __name__ == "__main__":
    sys.exit(main())
