"""Fails if mobile's two rails stop agreeing about how a badge looks.

Mobile draws its rail twice and has to. One is React, on Shiver's own pages, with IPC and an
ordinary stylesheet. The other is vanilla dom inside a *server's* page, where there is no IPC and
where it lives in a closed shadow root — because Sharkord ships Tailwind's preflight, which would
restyle anything Shiver rendered, and Shiver's own rules would leak back into a client it exists not
to reimplement.

Neither stylesheet can reach the other. That is the whole reason they drifted: the React badge was
`--shiver-danger`, a red kept for things that have gone wrong, while the bridge's was white. The
same badge in the same corner of the same tile, a different colour depending on which screen you
were looking at.

Nothing structural stops that happening again, so this does.
"""

import io
import os
import re
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
CSS = os.path.join(ROOT, "mobile", "src", "styles.css")
BRIDGE = os.path.join(ROOT, "mobile", "bridge", "rail.ts")

# the properties that decide what the badge *looks* like; geometry is checked too, because a badge
# that matches in colour and not in size is still two badges
# `padding` earns its place here: it is what made the badge an oval at two digits while every
# property below still matched, so the guard passed on a badge that was visibly wrong.
WATCHED = (
    "background",
    "color",
    "min-width",
    "width",
    "height",
    "padding",
    "border-radius",
    "font-size",
    "font-weight",
)


def declarations(block: str) -> dict:
    # Comments come out first. A rule's own comment can perfectly well mention a property — the one
    # explaining why the badge stopped being a pill names `padding` — and a guard that reads prose
    # as a declaration reports a difference between two rules that are identical.
    block = re.sub(r"/\*.*?\*/", "", block, flags=re.DOTALL)

    found = {}

    for name in WATCHED:
        match = re.search(rf"(?<![\w-]){re.escape(name)}\s*:\s*([^;}}]+)", block)

        if match:
            found[name] = " ".join(match.group(1).split())

    return found


def rule(path: str, marker: str, what: str) -> dict:
    """The declarations of the block `marker` starts, or a clean exit saying it is gone.

    `str.index` raises `ValueError`, so a renamed selector used to end the run in a Python
    traceback rather than in the message below — which was written for exactly this case and was
    unreachable, because `declarations` is only ever called on a block that was found.
    """
    with io.open(path, encoding="utf-8") as handle:
        body = handle.read()

    start = body.find(marker)

    if start < 0:
        sys.exit(f"could not find the {what} badge rule ({marker!r}) — has it been renamed?")

    end = body.find("}", start)

    if end < 0:
        sys.exit(f"the {what} badge rule is not closed — is {path} valid?")

    return declarations(body[start:end])


react = rule(CSS, ".rail-badge {", "react")
bridge = rule(BRIDGE, ".badge { position: absolute", "bridge")

if not react or not bridge:
    sys.exit("neither badge rule declared any of the watched properties — has the guard drifted?")

differences = [
    (name, react.get(name), bridge.get(name))
    for name in WATCHED
    if react.get(name) != bridge.get(name)
]

for name, left, right in differences:
    print(f"  {name}:\n    react rail:  {left}\n    bridge rail: {right}")

if differences:
    sys.exit(f"\nthe two mobile rails disagree on {len(differences)} propert(ies)")

print(f"mobile's two rail badges agree on all {len(WATCHED)} properties")
