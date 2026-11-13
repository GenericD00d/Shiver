#!/usr/bin/env python3
"""Checks this repository against the Shiver standard (docs/standards.md).

The same file is in every Shiver repo. Run it from anywhere:

    python3 scripts/check-docs.py

It prints every problem it finds and exits 1 if there are any. Standard library only, Python 3.8+.
"""

import hashlib
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent

# sha256 of docs/standards.md, with line endings normalised to \n. It changes only when the standard
# does: see "Changing this standard" in docs/standards.md.
STANDARDS_SHA256 = "6a25e29fe7ace179e85922d5a26ed64b98ec53b1f578b8cb5a6a8583c1cebb59"

MAX_AGENTS_LINES = 200

# file: (title rule, required second-level headings in order). A title rule is None (any title),
# "=text" (exactly that) or "*text" (ends with that).
SECTIONS = {
    "README.md": (None, [
        "A disclaimer worth reading", "What it does", "Install", "Usage", "Settings",
        "Known limitations", "Building", "The Shiver family", "Licence",
    ]),
    "AGENTS.md": ("*: agent guide", [
        "What this repo is", "The Shiver family", "Commands", "Layout",
        "Rules that must not break", "Conventions", "When you change something", "Further reading",
    ]),
    "ARCHITECTURE.md": ("=Architecture", ["Overview", "Layout", "Modules", "Flows", "Contracts"]),
    "SECURITY.md": ("=Security", [
        "Reporting a problem", "What this project handles", "Trust boundaries", "In scope",
        "Out of scope", "Design decisions worth knowing", "Review history",
    ]),
    "RELEASING.md": ("=Releasing", [
        "Versions", "Keys", "Building", "Signing and publishing", "Verifying a release",
        "If a key is lost or leaked",
    ]),
}

REQUIRED_FILES = list(SECTIONS) + [
    "CLAUDE.md", "CHANGELOG.md", "LICENSE", "docs/standards.md", "docs/decisions.md",
    ".github/dependabot.yml",
]

CHANGELOG_VERSION = re.compile(r"^\d+\.\d+\.\d+(?:[-+][0-9A-Za-z.-]+)? \(\d{4}-\d{2}-\d{2}\)$")
CHANGELOG_GROUPS = {"Added", "Changed", "Deprecated", "Removed", "Fixed", "Security"}
DECISION = re.compile(r"^(\d{4}-\d{2}-\d{2}): \S")
BUG = re.compile(r"^.+ \((?:fixed|open|documented)\)$")

HEADING = re.compile(r"^(#{1,6})\s+(.*?)\s*#*\s*$")
FENCE = re.compile(r"^ {0,3}(```|~~~)")
CODE_SPAN = re.compile(r"`([^`\n]+)`")
LINK = re.compile(r"!?\[[^\]\n]*\]\(([^)\s]+)(?:\s+\"[^\"]*\")?\)")
PATHLIKE = re.compile(r"^[A-Za-z0-9_.@-][A-Za-z0-9_./@+-]*$")

problems = []


def problem(where, message):
    problems.append(f"{where}: {message}")


def read(rel):
    return (ROOT / rel).read_text(encoding="utf-8").replace("\r\n", "\n")


def prose_lines(text):
    """Yields (line number, line) outside fenced code blocks."""
    fence = None

    for number, line in enumerate(text.split("\n"), 1):
        opened = FENCE.match(line)

        if fence is None and opened:
            fence = opened.group(1)
        elif fence is not None and line.strip().startswith(fence):
            fence = None
        elif fence is None:
            yield number, line


def headings(text):
    found = []

    for number, line in prose_lines(text):
        match = HEADING.match(line)

        if match:
            found.append((len(match.group(1)), match.group(2), number))

    return found


def tracked(pattern):
    try:
        out = subprocess.run(
            ["git", "ls-files", "-z", "--cached", "--others", "--exclude-standard", "--", pattern],
            cwd=ROOT, capture_output=True, check=True,
        ).stdout.decode("utf-8")

        return [name for name in out.split("\0") if name]
    except (OSError, subprocess.CalledProcessError):
        skipped = {"node_modules", ".repos", "target", ".git"}

        return [
            str(path.relative_to(ROOT)).replace("\\", "/")
            for path in ROOT.rglob(pattern)
            if not skipped.intersection(path.relative_to(ROOT).parts)
        ]


def ignored(path):
    """Whether git ignores this path (generated files a clean checkout doesn't have)."""
    try:
        return subprocess.run(
            ["git", "check-ignore", "-q", "--no-index", str(path)], cwd=ROOT, capture_output=True
        ).returncode == 0
    except OSError:
        return False


def resolves(path):
    """Exists, or is a generated file git ignores. A Markdown file always has to exist."""
    return path.exists() or (path.suffix != ".md" and ignored(path))


def check_files():
    for rel in REQUIRED_FILES:
        if not (ROOT / rel).is_file():
            problem(rel, "missing (see docs/standards.md, Files every repo has)")
        elif ignored(ROOT / rel):
            problem(rel, "is ignored by .gitignore, so it would never be committed")


def check_title(rel, found, rule):
    if not found or found[0][0] != 1:
        return problem(rel, "should start with a '# ' title")

    title = found[0][1]

    if sum(1 for level, _, _ in found if level == 1) > 1:
        problem(rel, "has more than one '# ' title")
    if rule is None:
        return
    if rule.startswith("=") and title != rule[1:]:
        problem(rel, f"title should be '# {rule[1:]}', not '# {title}'")
    if rule.startswith("*") and not title.endswith(rule[1:]):
        problem(rel, f"title should end with '{rule[1:]}', not be '# {title}'")


def check_sections():
    for rel, (rule, required) in SECTIONS.items():
        if not (ROOT / rel).is_file():
            continue

        found = headings(read(rel))
        check_title(rel, found, rule)

        seconds = [text for level, text, _ in found if level == 2]
        last = -1

        for name in required:
            if name not in seconds:
                problem(rel, f"missing section '## {name}'")
                continue

            position = seconds.index(name)

            if position < last:
                problem(rel, f"section '## {name}' is out of order")

            last = max(last, position)


def check_changelog():
    rel = "CHANGELOG.md"

    if not (ROOT / rel).is_file():
        return

    found = headings(read(rel))
    check_title(rel, found, "=Changelog")
    seconds = [(text, number) for level, text, number in found if level == 2]

    for index, (text, number) in enumerate(seconds):
        if text == "Unreleased":
            if index != 0:
                problem(f"{rel}:{number}", "'## Unreleased' must come first")
        elif not CHANGELOG_VERSION.match(text):
            problem(f"{rel}:{number}", f"'## {text}' should be '## <version> (<YYYY-MM-DD>)'")

    for level, text, number in found:
        if level == 3 and text not in CHANGELOG_GROUPS:
            problem(f"{rel}:{number}", f"'### {text}' isn't one of {', '.join(sorted(CHANGELOG_GROUPS))}")


def check_decisions():
    rel = "docs/decisions.md"

    if not (ROOT / rel).is_file():
        return

    found = headings(read(rel))
    check_title(rel, found, "=Decisions")
    seconds = [(text, number) for level, text, number in found if level == 2]
    names = [text for text, _ in seconds]
    previous = ""

    if "Bug log" not in names:
        problem(rel, "missing section '## Bug log'")
    elif names[-1] != "Bug log":
        problem(rel, "'## Bug log' must be the last section")

    for text, number in seconds:
        match = DECISION.match(text)

        if match:
            if match.group(1) < previous:
                problem(f"{rel}:{number}", "decisions should be oldest first")

            previous = match.group(1)
        elif text not in ("Open items", "Bug log"):
            problem(f"{rel}:{number}", f"'## {text}' should be '## <YYYY-MM-DD>: <decision>'")

    in_bug_log = False

    for level, text, number in found:
        if level == 2:
            in_bug_log = text == "Bug log"
        elif level == 3 and in_bug_log and not BUG.match(text):
            problem(f"{rel}:{number}", f"'### {text}' should end with (fixed), (open) or (documented)")


def check_agent_files():
    if (ROOT / "CLAUDE.md").is_file() and read("CLAUDE.md").strip() != "@AGENTS.md":
        problem("CLAUDE.md", "should contain exactly '@AGENTS.md'")

    if (ROOT / "AGENTS.md").is_file():
        lines = read("AGENTS.md").count("\n") + 1

        if lines > MAX_AGENTS_LINES:
            problem("AGENTS.md", f"is {lines} lines; keep it under {MAX_AGENTS_LINES}")


def candidate_path(span):
    """The repo path a backticked span names, or None if it isn't one."""
    text = re.sub(r":\d+(?:-\d+)?$", "", span.strip())

    if "/" not in text or "://" in text or not PATHLIKE.match(text):
        return None

    first = (text[2:] if text.startswith("./") else text).split("/")[0]

    return text if first and (ROOT / first).exists() else None


def check_paths():
    for rel in ("AGENTS.md", "ARCHITECTURE.md"):
        if not (ROOT / rel).is_file():
            continue

        for number, line in prose_lines(read(rel)):
            for span in CODE_SPAN.findall(line):
                path = candidate_path(span)

                if path and not resolves(ROOT / path):
                    problem(f"{rel}:{number}", f"`{path}` doesn't exist")


def check_links():
    for rel in tracked("*.md"):
        if not (ROOT / rel).is_file():
            continue

        base = (ROOT / rel).parent

        for number, line in prose_lines(read(rel)):
            for target in LINK.findall(line):
                if re.match(r"^(?:[a-z][a-z0-9+.-]*:|#)", target, re.I):
                    continue

                target = target.split("#")[0]

                if target and not resolves(base / target):
                    problem(f"{rel}:{number}", f"link to '{target}' doesn't resolve")


def check_standard():
    rel = "docs/standards.md"

    if not (ROOT / rel).is_file():
        return

    digest = hashlib.sha256(read(rel).encode("utf-8")).hexdigest()

    if digest != STANDARDS_SHA256:
        problem(rel, f"differs from the standard this script expects (sha256 {digest}); "
                     "see 'Changing this standard'")


def step_block(lines, index):
    """The lines of the workflow step that contains lines[index]."""
    def indent(line):
        return len(line) - len(line.lstrip(" "))

    start, dash = index, indent(lines[index])

    if not lines[index].lstrip().startswith("- "):
        for back in range(index - 1, -1, -1):
            stripped = lines[back].lstrip()

            if stripped.startswith("- ") and indent(lines[back]) < indent(lines[index]):
                start, dash = back, indent(lines[back])
                break

    end = start + 1

    while end < len(lines) and (not lines[end].strip() or indent(lines[end]) > dash):
        end += 1

    return lines[start:end]


def check_workflows():
    for path in sorted((ROOT / ".github" / "workflows").glob("*.y*ml")):
        rel = str(path.relative_to(ROOT)).replace("\\", "/")
        text = path.read_text(encoding="utf-8").replace("\r\n", "\n")
        lines = text.split("\n")

        if not re.search(r"^permissions:", text, re.M):
            problem(rel, "sets no top-level 'permissions:'")
        if "pull_request_target" in text:
            problem(rel, "uses pull_request_target")

        for index, line in enumerate(lines):
            match = re.match(r"^\s*(?:-\s+)?uses:\s*[\"']?([^\"'\s#]+)[\"']?\s*(#.*)?$", line)

            if not match:
                continue

            action, comment = match.group(1), match.group(2)

            if action.startswith(("./", "docker://")):
                continue
            if not re.search(r"@[0-9a-f]{40}$", action):
                problem(f"{rel}:{index + 1}", f"'{action}' isn't pinned to a full commit SHA")
            elif not comment:
                problem(f"{rel}:{index + 1}", f"'{action}' has no '# <version>' comment")

            if action.startswith("actions/checkout@") and not any(
                re.search(r"persist-credentials:\s*false", step) for step in step_block(lines, index)
            ):
                problem(f"{rel}:{index + 1}", "checkout without 'persist-credentials: false'")


def main():
    check_files()
    check_sections()
    check_changelog()
    check_decisions()
    check_agent_files()
    check_paths()
    check_links()
    check_standard()
    check_workflows()

    if problems:
        print(f"check-docs: {len(problems)} problem(s)")

        for line in problems:
            print(f"  - {line}")

        return 1

    print("check-docs: ok")

    return 0


if __name__ == "__main__":
    sys.exit(main())
