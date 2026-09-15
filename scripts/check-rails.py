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
BRIDGE = os.path.join(ROOT, "mobile", "bridge", "index.ts")

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


def react_rule() -> dict:
    body = io.open(CSS, encoding="utf-8").read()
    start = body.index(".rail-badge {")

    return declarations(body[start : body.index("}", start)])


def bridge_rule() -> dict:
    body = io.open(BRIDGE, encoding="utf-8").read()
    # the rule is inside a template literal, written on several lines
    start = body.index(".badge { position: absolute")

    return declarations(body[start : body.index("}", start)])


react = react_rule()
bridge = bridge_rule()

if not react or not bridge:
    sys.exit("could not find one of the badge rules — has it been renamed?")

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
