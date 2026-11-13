# The Shiver standard

Standard version **1** (2026-11-13).

This file is identical in all four Shiver repos, and `scripts/check-docs.py` fails if it is changed
in one of them only. The copy in
[GenericD00d/Shiver](https://github.com/GenericD00d/Shiver/blob/main/docs/standards.md) is the
canonical one. See [Changing this standard](#changing-this-standard).

It sets what every Shiver repo has in common: its files, how its docs are written for people and
for AI agents, and the security baseline its code, CI and releases meet.

## The Shiver family

| Repo | What it is | Runs where |
|---|---|---|
| [Shiver](https://github.com/GenericD00d/Shiver) | Desktop and Android client for Sharkord, and the companion plugin in `plugin/` | Users' devices; the plugin on Sharkord servers |
| [shiver-server-manager](https://github.com/GenericD00d/shiver-server-manager) | Sets up and runs Sharkord servers, and installs the Shiver plugins | A Linux server, as root |
| [shiver-media-player](https://github.com/GenericD00d/shiver-media-player) | Sharkord plugin: watch videos together in a voice channel | Inside the Sharkord server |
| [shiver-temp-share](https://github.com/GenericD00d/shiver-temp-share) | Sharkord plugin: file links that expire | Inside the Sharkord server |

### Contracts between the repos

When one side of a contract changes, the other side changes in the same release.

| Contract | Made by | Relied on by |
|---|---|---|
| Release assets `shiver-plugin-<version>.zip` and `SHA256SUMS.txt` | Shiver (`plugin/`) | The manager's `internal/plugins/shiver.go` and `plugins.json` |
| Release assets `temp-share-<version>.tar.gz` and `<version>.json` | shiver-temp-share | The manager's `plugins.json` |
| Release assets `shiver-media-player-<version>.tar.gz` and `<version>.json` | shiver-media-player | The manager's `plugins.json` |
| Plugin ids `shiver`, `temp-share` and `shiver-media-player`. They are never renamed, because Sharkord keys a plugin's stored data by its id | Each plugin's `manifest.json` | The manager, and Sharkord's plugin data |
| The companion plugin's actions and its `__SHIVER_PLUGIN__` announcement | Shiver's `plugin/` | Shiver's bridges |
| The Sharkord release everything is tested against, and its plugin SDK version | Shiver's `scripts/check-sharkord.py` | The manager's `internal/sharkord/tested.go`, and each plugin's `scripts/setup.ts` |

## Files every repo has

```
README.md               For users: what it is, the disclaimer, install, use, settings
AGENTS.md               For agents and contributors: start here
CLAUDE.md               One line, @AGENTS.md, so that Claude Code loads AGENTS.md
ARCHITECTURE.md         How it works: layout, modules, flows, contracts
SECURITY.md             Reporting, threat model, review history
RELEASING.md            Versions, building, signing, publishing, verifying
CHANGELOG.md            One entry per release
LICENSE
docs/standards.md       This file, identical in every repo
docs/decisions.md       Dated decisions, open items and a bug log
docs/<guide>.md         Guides that belong to one repo
scripts/check.sh        The repo's local checks; --quick runs the fast subset
scripts/check-docs.py   Checks this structure; identical in every repo
.github/dependabot.yml  Keeps the pinned GitHub Actions current
```

- **One home per fact.** Other files link to it instead of repeating it, so copies can't drift.
- **No spaces in file names.** Names under `docs/` are lowercase with hyphens.
- **Component READMEs** are allowed for parts that ship on their own, such as Shiver's
  `plugin/README.md`.
- **British spelling**, as the docs already use: colour, behaviour, licence.

## Required sections

Each file has these headings, in this order. Other headings may sit between them. A section that
doesn't apply stays, with one line saying so.

| File | Title | Sections, in order |
|---|---|---|
| `README.md` | `# <Name>` | A disclaimer worth reading · What it does · Install · Usage · Settings · Known limitations · Building · The Shiver family · Licence |
| `AGENTS.md` | `# <Name>: agent guide` | What this repo is · The Shiver family · Commands · Layout · Rules that must not break · Conventions · When you change something · Further reading |
| `ARCHITECTURE.md` | `# Architecture` | Overview · Layout · Modules · Flows · Contracts |
| `SECURITY.md` | `# Security` | Reporting a problem · What this project handles · Trust boundaries · In scope · Out of scope · Design decisions worth knowing · Review history |
| `RELEASING.md` | `# Releasing` | Versions · Keys · Building · Signing and publishing · Verifying a release · If a key is lost or leaked |
| `CHANGELOG.md` | `# Changelog` | An optional `## Unreleased`, then `## <version> (<YYYY-MM-DD>)`, newest first. Entries group their bullets under `### Added`, `### Changed`, `### Deprecated`, `### Removed`, `### Fixed` and `### Security` |
| `docs/decisions.md` | `# Decisions` | `## <YYYY-MM-DD>: <what was decided>` entries, oldest first, each saying why and what follows from it; an optional `## Open items`; then `## Bug log`, last, with entries `### <title> (fixed)`, `(open)` or `(documented)` |

## Writing for agents and people

- **AGENTS.md stays under 200 lines.** It is read at the start of every session, so it links out
  for detail.
- **Commands are exact**, and are the ones CI runs. `scripts/check.sh` runs all the local ones.
- **Rules that must not break** each name the file that enforces the rule and the test that guards
  it, so an agent editing that code knows what not to undo.
- **Plain sentences, one idea each.** No chains of nested parentheses.
- **Use the code's names**, in backticks, spelled exactly as in the code, so a search finds them.
- **Paths must exist.** Every repo path in backticks in AGENTS.md and ARCHITECTURE.md, and every
  relative link in a Markdown file, must resolve; `check-docs.py` fails otherwise.
- **Contracts are written on both sides.** See
  [Contracts between the repos](#contracts-between-the-repos).
- **Comments say why.** A security check's comment names the threat it stops and points at the test
  that guards it.
- **Commits** have a plain-English subject saying what changed, and a body saying why. Security
  fixes also get a `### Security` changelog entry.

## Security baseline

### Repository and CI

1. Every workflow:
   - starts with `permissions: contents: read`, or tighter, and raises it only in the job that
     needs more;
   - checks out with `persist-credentials: false`;
   - pins every `uses:` to a full commit SHA, with the version in a comment;
   - never uses `pull_request_target`, and never gives secrets to a workflow that runs on pull
     requests.
2. Toolchains are pinned to exact versions, and container images by digest.
3. Installs come from lockfiles only: `bun install --frozen-lockfile`, `npm ci`, `go mod verify`.
   Anything cloned at build time is pinned to a tag or a commit.
4. CI runs a dependency audit: `cargo audit`, `bun audit` or `govulncheck`. Exceptions are listed
   with a reason.
5. `.github/dependabot.yml` updates the pinned actions monthly.
6. On GitHub, which only the owner can change: private vulnerability reporting, secret scanning
   with push protection, and Dependabot alerts are on.

### Releases

1. Anything installed or updated automatically is signed with a key that never enters the repo,
   and is verified against a public key compiled into whatever installs it.
2. What gets signed is the checksum file. An installer checks the signature first, then each file
   against the checksums.
3. Third-party binaries fetched at runtime are pinned to a version and a SHA-256 in the source.
4. Every release is verified before it is published, with the commands in RELEASING.md.

### Code

1. **Requests made on a user's behalf** go only to public addresses. The check runs on the address
   actually connected to, at connect time, not just on the link beforehand. Shiver's
   `plugin/server/index.js` (`isPrivateAddress`, `vetEndpoint`) is the reference, and copies keep
   its tests.
2. **Privileged code never works by path inside a folder a less-privileged user can write.** It
   stops the service that owns the folder first. In Go it writes through `os.Root`. It changes
   ownership last, without following links.
3. **Archives** may hold only plain files and folders, with relative paths, under size caps, and are
   unpacked with `O_EXCL`.
4. **Commands** run as argument lists, never through a shell, with `--` before anything a user
   supplied.
5. **HTML output** is escaped and served with a strict CSP. Users' files are served with
   `Content-Disposition: attachment` and `X-Content-Type-Options: nosniff`.
6. **Text from users that other users will see** has control, bidi and invisible characters
   removed.
7. **Permissions and limits.** Every action states the permission it needs. A failed login or
   permission check doesn't say why it failed, so its answer can't be used to test passwords.
   Expensive actions have per-user and server-wide caps.
8. **Secrets** never go in logs, errors or URLs.

### Documentation and review

1. SECURITY.md describes the trust boundaries honestly, including what isn't protected.
2. Every README carries the disclaimer below.
3. SECURITY.md's Review history records every review: its date, the commit, and its kind.
4. Every security fix comes with a regression test.
5. Before a release, the Trust boundaries table is re-read against what changed. A change that adds
   a boundary updates SECURITY.md and gets a test.

### The README disclaimer

Adapt the bracketed parts. Shiver's own wording is the original and may stay longer.

```markdown
## A disclaimer worth reading

**This project is vibecoded.** Almost all of it was written by an LLM working from one person's
direction, review and bug reports.

- **No independent security audit has been done.** It handles [what it handles]. Read the code
  before you trust it with that. Its internal reviews are listed in SECURITY.md; they are not an
  outside audit.
- **Releases are [how they are signed and verified].**
- It is early software. Expect rough edges, and expect things to change.

It is offered as-is.
```

## Enforcement

`scripts/check-docs.py` runs in CI and in `scripts/check.sh`. It checks that:

- the files above exist, and their required headings appear in order;
- `CLAUDE.md` is exactly `@AGENTS.md`, and AGENTS.md is under 200 lines;
- repo paths in backticks in AGENTS.md and ARCHITECTURE.md exist, and relative links in Markdown
  files resolve;
- this file matches the hash the script expects;
- every workflow sets `permissions`, pins its actions to commit SHAs with a version comment, checks
  out with `persist-credentials: false`, and doesn't use `pull_request_target`.

## Changing this standard

1. Edit this file in Shiver, and bump the version line.
2. Update `STANDARDS_SHA256` in `scripts/check-docs.py`. The script prints the new value.
3. Copy both files to the other three repos on the same day.
4. Add a `docs/decisions.md` entry in Shiver saying what changed and why.
