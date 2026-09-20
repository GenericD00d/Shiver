"""Fails if the four copies of the "is this colour light?" test stop agreeing.

Shiver decides whether to draw text dark or light on the user's chosen background, and it has to
make that decision in four places that cannot import from one another:

    desktop/src/theme.ts        Shiver's own chrome, desktop
    desktop/bridge/index.ts     injected into a server's page, desktop
    mobile/src/theme.ts         Shiver's own chrome, mobile
    mobile/bridge/index.ts      injected into a server's page, mobile

Two separate Bun packages, and inside each one a bundle that is `include_str!`'d into the Rust
binary rather than imported. There is no module all four can share without a build-time hop that
neither bridge config has.

**They have already drifted once**, and the comment above the mobile bridge's copy records how it
looked: the fallback for a value that was not six hex digits returned `false` there and `true` in
`theme.ts`, so a hand-edited colour gave Shiver's chrome one foreground and the server page the
opposite one — in the two halves of a single window, under a comment asserting they agreed.

Nothing structural stops that happening again, so this does. It compares the bodies rather than
the names, because each copy is called something different in its own file.
"""

import os
import re
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))

# Where each copy lives, and what it is called there. The names differ on purpose — `theme.ts`
# keeps a module-private `isLight`, the bridges export nothing and call theirs `isLightColor` —
# so the function is found by name per file and compared by body.
COPIES = (
    (os.path.join("desktop", "src", "theme.ts"), "isLight"),
    (os.path.join("desktop", "bridge", "index.ts"), "isLightColor"),
    (os.path.join("mobile", "src", "theme.ts"), "isLight"),
    (os.path.join("mobile", "bridge", "index.ts"), "isLightColor"),
)


def body(path, name):
    """The named function's body, as a list of code lines with comments and blanks dropped.

    Both spellings have to be handled: `const isLight = (hex: string) => {` in the theme modules,
    `function isLightColor(hex: string) {` in the bridges. The point of comparison is what the
    code does, so the surrounding syntax and any prose around it are not part of it.
    """
    with open(os.path.join(ROOT, path), encoding="utf8") as handle:
        source = handle.read()

    opening = re.search(
        r"^(?:const %s\s*=\s*\([^)]*\)\s*(?::[^=]+)?=>|function %s\s*\([^)]*\))\s*\{"
        % (re.escape(name), re.escape(name)),
        source,
        re.M,
    )

    if not opening:
        sys.exit("%s: could not find %s — has it been renamed?" % (path, name))

    depth = 0
    lines = []

    for line in source[opening.end() - 1 :].splitlines():
        depth += line.count("{") - line.count("}")

        lines.append(line)

        if depth == 0:
            break
    else:
        sys.exit("%s: %s is never closed" % (path, name))

    keep = []

    for line in lines[1:-1]:
        stripped = line.strip()

        if not stripped or stripped.startswith("//") or stripped.startswith("*"):
            continue

        keep.append(stripped)

    return keep


def main():
    bodies = [(path, name, body(path, name)) for path, name in COPIES]
    reference_path, reference_name, reference = bodies[0]

    failed = False

    for path, name, lines in bodies[1:]:
        if lines == reference:
            continue

        failed = True

        print(
            "%s: %s does not match %s in %s" % (path, name, reference_name, reference_path),
            file=sys.stderr,
        )

        for index in range(max(len(lines), len(reference))):
            here = lines[index] if index < len(lines) else "(nothing)"
            there = reference[index] if index < len(reference) else "(nothing)"

            if here != there:
                print("    %s has: %s" % (path, here), file=sys.stderr)
                print("    %s has: %s" % (reference_path, there), file=sys.stderr)

    if failed:
        print(
            "\nAll four decide the same thing about the same colour, including the fallback for a"
            "\nvalue that is not six hex digits. Fix every copy, not the one the test named.",
            file=sys.stderr,
        )

        return 1

    print("all %d copies of the luminance test agree (%d lines)" % (len(bodies), len(reference)))

    return 0


if __name__ == "__main__":
    sys.exit(main())
