# Releasing Shiver

Releases are built locally and deliberately, on one machine, because they need the Android SDK, JDK
17 and two signing keys that are not in this repository and are not in CI. That is a defensible
choice and it has one consequence worth being honest about: **those keys are a single point of
failure, and losing either is not recoverable.** Most of this file is about that.

## The two keys, and what each one protects

They protect different things and neither substitutes for the other.

| | **Desktop update key** | **Android release key** |
| --- | --- | --- |
| What | minisign keypair | Java keystore (`.jks`) |
| Where the public half lives | `pubkey` in `desktop/src-tauri/tauri.conf.json`, compiled into every binary | inside every APK ever shipped |
| Where the private half lives | one machine, outside the repo | `keystore.properties` → a `.jks`, outside the repo |
| Passed to the build by | `TAURI_SIGNING_PRIVATE_KEY` and `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | `keystore.properties` at the Android project root |
| It stops | a bad release installing itself on someone's machine | an APK from anyone else replacing Shiver on a phone |
| If it leaks | an attacker can sign an update — but still has to publish it where the updater looks | an attacker can build an APK that overwrites Shiver on any device that sideloads it |
| If it is lost | **no existing desktop install can ever be updated again** | **no existing Android install can ever be updated again** |

That last row is the important one, and it is the same on both sides for the same reason: the
verifier is baked into what the user already has. A desktop Shiver checks updates against the public
key compiled into it, and Android identifies an app by its package name *and* its signing
certificate together. Sign with a different key and the system does not see an update — it sees a
different app, and refuses to install over the top.

For Android there is no recovery at all. Key rotation exists (APK Signature Scheme v3 lineage, fine
on `minSdk 24`) but it works by proving continuity *from the old key*, so it helps when a key is
compromised and still held, not when it is gone. Play App Signing can reset a lost upload key, but
Shiver ships APKs directly rather than through Play, so that route does not apply either.

For the desktop the position is slightly better only because it is recoverable by brute force: ship
a new version with a new public key compiled in, and tell every existing user to download it by
hand, because their installed copy will reject anything signed with the new key. That is a release
nobody can be automatically told about.

## What to do about that, today

**Back up both private keys, off that machine, before anything else in this file matters.**

1. Two copies, in two places that will not burn down together.
2. Encrypted — `age` or `gpg` — with the passphrase stored *separately* from the key. A key and its
   password in the same backup is one secret, not two.
3. Paper is legitimate here. Both keys are small; a printed armoured block in a safe or a deposit
   box survives things that disks do not.
4. **Test the restore once.** On a machine that has never seen the key, restore it and build a
   release. An untested backup is a belief.

Also back up, because they are needed to *use* the keys and are not secret:

- the keystore's alias (`keyAlias` in `keystore.properties`)
- the minisign public key, so a restored private key can be checked against what is already shipped

## Building a release

Bump the version in **one** place: `version` under `[workspace.package]` in the root
`Cargo.toml`. Every crate inherits it, and both `tauri.conf.json` files omit `version`, so Tauri
(including Android's `versionCode`) takes it from the crate.

The companion plugin is versioned separately. It installs separately and its
`plugin/manifest.json` version is its own, bumped when the plugin changes rather than to match an
app release.

### Desktop

```bash
cd desktop
export TAURI_SIGNING_PRIVATE_KEY="$(cat /path/to/shiver.key)"
export TAURI_SIGNING_PRIVATE_KEY_PASSWORD='…'
bun run app:build
```

`createUpdaterArtifacts` is on, so this produces the installer *and* a `.sig` beside it, four files
in two directories:

```
target/release/bundle/nsis/Shiver_<version>_x64-setup.exe        (+ .sig)
target/release/bundle/msi/Shiver_<version>_x64_en-US.msi         (+ .sig)
```

**That `target/` is the one at the repository root, not under `desktop/`.** The crates are a cargo
workspace, and a workspace has a single build directory that every member writes into — so the
bundle moved there when the workspace landed, and `desktop/src-tauri/target/` no longer exists.
Worth stating because the failure is quiet: a script still pointing at the old path finds whatever
that directory happened to hold, which for a while was the *previous* release's installers, and
ships those instead.

Both go in the GitHub release, along with a `latest.json` naming the version, the download URL and
that signature — that file is what the updater fetches from
`https://github.com/GenericD00d/Shiver/releases/latest/download/latest.json`.

A release whose `latest.json` points at an installer whose `.sig` does not verify is not a broken
update, it is a *silent* one: the updater refuses it and users simply never hear about the version.
Check the signature verifies before publishing.

### Android

```bash
cd mobile
./node_modules/.bin/tauri android build --apk   # call the binary directly, not through bun
```

The build now refuses to produce an unsigned release APK — `failOnUnsignedRelease` in
`gen/android/app/build.gradle.kts` fails the build, naming what is missing, if
`keystore.properties` is absent or incomplete. Debug builds are unaffected and still work on a
machine that has never held the key:

```bash
./node_modules/.bin/tauri android build --apk --debug
```

The APK is Gradle's output rather than cargo's, so it did not move with the workspace:

```
mobile/src-tauri/gen/android/app/build/outputs/apk/universal/release/app-universal-release.apk
```

It is named after the variant rather than the version, so rename it on the way into the release —
`Shiver_<version>_arm64.apk`, matching every release so far.

An unsigned or differently-signed APK is the failure this guards against, and it is not obvious
when it happens: it installs perfectly well on a phone that has no Shiver on it, and cannot update
one that does. Check it before publishing, against the certificate every release since 0.1.0
carries:

```bash
apksigner verify --print-certs <apk>
# Signer #1 certificate SHA-256 digest: 49e78c33db2056082cca67262ba1b7ef064b920606e08376b430164f0e51874b
```

A different fingerprint there cannot upgrade an installed Shiver, whatever else is right about it.

## What is deliberately not signed

Shiver's installers carry no OS-level code signature — no Authenticode on Windows, no Developer ID
on macOS — which is why Windows SmartScreen warns on first run. The README says so plainly, and it
is a cost problem rather than a technical one.

**It is worth being precise about what that does and does not mean**, because the two are easy to
conflate. Updates *are* signed and verified against the key compiled into the binary, so an attacker
who takes over the GitHub account still cannot push code to an existing install. What is missing is
the assurance for the *first* download — the one where the user has nothing of Shiver's to check
against and is relying on the operating system to vouch for the publisher.

If that is ever worth fixing:

- **Windows.** Since the CA/Browser Forum's 2023 baseline change, publicly trusted code-signing
  private keys must live on FIPS 140-2 Level 2 hardware, so a downloadable `.pfx` is no longer
  something any real CA will sell. That leaves a hardware token (which cannot be plugged into a CI
  runner) or a cloud signing service that holds the key in an HSM. **For an open-source project the
  first thing to look at is the SignPath Foundation**, which issues free certificates to OSS
  projects.
- **macOS.** Apple Developer Program, then Developer ID signing *and* notarization — two separate
  steps, both required. Only worth it if macOS builds are actually shipped.
- **Linux.** Nothing required.

Verify current pricing and eligibility rather than trusting this paragraph; these terms move.

## If a key is compromised rather than lost

Different problem, better options, because you still hold the key.

- **Android:** rotate with `apksigner`'s signing lineage (`--lineage`), which signs with the new key
  while proving descent from the old. Devices on Android 9+ accept the rotation; anything older
  keeps trusting the old certificate, which is a reason to treat rotation as damage limitation
  rather than a clean fix.
- **Desktop:** generate a new minisign keypair, ship a version carrying the new public key signed
  with the *old* one — existing installs will accept that update, and from then on they verify
  against the new key. This only works while the old key is still usable, which is exactly the
  window a compromise gives you. Do it before revoking anything.

In both cases, say so publicly. A signature is a claim about provenance; a compromised key makes
past claims unreliable, and only an announcement fixes that.

## If releases ever move to CI

They are local today and that is a deliberate trade — fewer moving parts, and the keys never leave
one machine. If that changes:

- Prefer a signing service that holds the key (Azure Trusted Signing, a cloud KMS) over putting the
  raw key in repository secrets. The key never existing on the runner is worth more than any amount
  of care about who can read the secret.
- If a raw key must be a secret, put it in a GitHub *environment* with required reviewers, and scope
  the workflow to tag pushes only. A secret readable by any workflow run is readable by any pull
  request that can change a workflow.
- Keep the checks workflow as it is — no secrets, safe to run on a fork's pull request. Signing
  belongs in a separate workflow with a separate trigger. The two should never be the same job.
