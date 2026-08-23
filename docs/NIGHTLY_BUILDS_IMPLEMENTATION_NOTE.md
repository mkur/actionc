# Nightly Builds Implementation Note

Status: slices 1 through 5 complete. A manual dispatch builds all four targets
and updates the moving `nightly` prerelease. Real-emulator rollout checks and
the daily schedule remain.

Snapshot date: 2026-08-11.

## Goal

Publish installable `actionc` nightlies for Linux, Windows, and macOS from the
latest tested commit on `main`. A nightly is a release channel, not a request to
compile with the Rust nightly toolchain. Release builds use a pinned stable Rust
toolchain.

The first public nightly contains these user-facing executables:

- `actionc`;
- `actionc-run`;
- `actionc-emit`.

Internal sweep, comparison, map, and quality tools are not release artifacts.

## Supported Packages

| Package | GitHub runner | Rust target | Archive |
| --- | --- | --- | --- |
| Linux x86-64 | `ubuntu-24.04` | `x86_64-unknown-linux-musl` | `.tar.gz` |
| Windows x86-64 | `windows-2025` | `x86_64-pc-windows-msvc` | `.zip` |
| macOS Apple Silicon | `macos-15` | `aarch64-apple-darwin` | `.tar.gz` |
| macOS Intel | `macos-15-intel` | `x86_64-apple-darwin` | `.tar.gz` |

The first implementation publishes separate Apple Silicon and Intel archives.
A universal macOS binary can be added later after the two native packages are
stable.

The Linux package uses musl because the public executables currently depend
only on Rust code and embedded assets. If a native dependency is introduced,
the workflow must either preserve static musl compatibility or change the
published target and archive name explicitly.

## Publication Model

One workflow, `.github/workflows/nightly.yml`, owns the nightly channel. Its
final triggers are:

- a daily schedule away from the top of the hour;
- `workflow_dispatch` for rollout, diagnosis, and forced rebuilds.

During rollout, only `workflow_dispatch` is enabled. The daily schedule is
added after the published packages pass the remaining manual checks. The
embedded-asset license gate is closed and the publisher is enabled.

The workflow uses a four-entry build matrix. Each entry tests, builds, smoke
tests, and packages one target, then uploads its archive as an intermediate
workflow artifact. A separate publisher job runs only after all four entries
succeed.

The publisher updates one moving prerelease and tag named `nightly`. Its assets
therefore have stable download URLs. Stable version tags and releases must
never be modified by this workflow.

The moving tag is incompatible with immutable GitHub releases. If immutable
releases are enabled later, replace it with dated tags of the form
`nightly-YYYYMMDD-g<sha>` and add an explicit retention policy.

Scheduled builds may skip publication when `nightly` already points at the
current commit. A manually dispatched build can force repackaging the same
commit.

## Permissions and Concurrency

Workflow permissions default to:

```yaml
permissions:
  contents: read
```

Only the publisher job receives `contents: write`. It uses the workflow's
`GITHUB_TOKEN`; a personal access token is not required.

The workflow has a `nightly` concurrency group and cancels an older in-progress
nightly when a newer run starts. This prevents two publisher jobs from racing
to move the tag or replace release assets.

Third-party release actions are not required. Publication uses the GitHub CLI
available on GitHub-hosted runners.

## Reproducible Build Identity

The repository pins a stable Rust version in `rust-toolchain.toml`. Toolchain
updates are deliberate changes tested by regular CI before they affect the
nightly channel.

The three public executables support `--version` and `-V`. The version output
contains:

- Cargo package version;
- release channel when supplied by the build;
- source commit SHA when supplied by the build;
- UTC build date when supplied by the build;
- compilation target when supplied by the build.

The workflow passes this metadata at compile time. Ordinary local builds remain
valid and report at least the Cargo package version.

Each package also contains `BUILD-INFO.txt` with the complete build identity and
the `rustc --version` output. Archive identity must not depend solely on an
executable being runnable on the machine inspecting it.

## Archive Contract

Archive names are stable and target-specific:

```text
actionc-nightly-x86_64-unknown-linux-musl.tar.gz
actionc-nightly-x86_64-pc-windows-msvc.zip
actionc-nightly-aarch64-apple-darwin.tar.gz
actionc-nightly-x86_64-apple-darwin.tar.gz
```

Every archive has one root directory and contains:

- `actionc`, `actionc-run`, and `actionc-emit` with the host executable suffix;
- `README.md`, `USAGE.md`, and `LICENSE`;
- `docs/ACTIONC_RUN.md`;
- `BUILD-INFO.txt`;
- the notices and license texts for embedded third-party components.

The release also contains the pinned Action! corresponding-source snapshot and
`SHA256SUMS` covering all four platform archives plus that source snapshot.
Generated `.xex` files, ATR files, and emulator executables are not packaged.

A single portable repository script assembles every package. It receives the
target, input executable directory, output directory, and build metadata as
arguments. It must verify the complete archive inventory rather than accepting
arbitrary contents from `target/`.

## License Gate

`actionc-run` embeds three redistributable artifacts:

- the MyDOS ATR supplied by `atrcopy-rs`;
- AltirraOS;
- the Action! cartridge image in `roms/action.rom`.

The AltirraOS license and provenance are recorded in `roms/README.md` and
`roms/ALTIRRAOS-LICENSE`.

The Action! cartridge gate is closed by `roms/ACTION-ROM-NOTICE.md`. It records
the GPL-3.0-or-later license, pinned corresponding-source revision, upstream
build path, complete CAR and payload hashes, and a byte-for-byte match between
the bundled CAR payload and the upstream Action! 3.6 reference ROM. The release
publisher attaches `roms/source/action-3.6-source-0b8bcedb.tar.gz` alongside the
binary archives.

The MyDOS gate is closed by `atr/MYDOS-NOTICE.md` and the preserved
`atr/source/MYDOS453.ARC` source release. The notice records the complete
historical distribution terms, original and 4.53/3 authorship, source archive
and binary hashes, and the configuration relationship between the upstream
single-density release disk and the embedded double-density ATR. Every binary
package containing `actionc-run` must include both the notice and source
archive.

The packager enforces the embedded-asset license-material gate by default. The
nightly workflow does not bypass it. `--allow-incomplete-license-notices`
remains available only for local prepublication checks; such an archive
contains a conspicuous `licenses/INCOMPLETE-LICENSING.md` warning and must not
be attached to a public release.

## Test and Smoke-Test Contract

Regular CI establishes the same host-portable baseline used by the nightly
workflow:

```text
Linux:   cargo test --locked
Windows: cargo test --locked
macOS:   cargo test --locked
```

Tests for maintained shell helpers are Unix-only. Runner behavior itself is
tested through Rust adapter and CLI tests on all hosts; it must not require
Bash. Windows-specific discovery and replacement behavior needs native Windows
coverage.

Each nightly matrix entry performs these native smoke tests:

1. execute `--version` for all packaged binaries;
2. compile `samples/hello-world.act` with `actionc`;
3. emit one textual representation with `actionc-emit`;
4. run `actionc-run --no-run` and parse or otherwise validate the resulting ATR;
5. verify the archive inventory.

The Linux job additionally compiles the sample in compatibility, optimized,
and MIR6502 modes. Hosted CI does not launch a graphical emulator. A real
Altirra launch on Windows and Atari800 launches on the supported hosts are
manual release smoke tests.

## Signing Policy

Initial Windows and macOS nightlies are unsigned. Release notes must say that
Gatekeeper or SmartScreen may warn about them. Apple Developer ID signing and
notarization, and Windows code signing, are later slices requiring protected
repository secrets and a documented certificate-rotation process.

Checksums provide download-integrity information but are not a substitute for
code signing. Artifact attestations may be added after the basic channel is
stable.

## Implementation Slices

### Slice 1: cross-platform CI baseline

- add push, pull-request, and manual CI for Linux, Windows, and macOS;
- isolate legacy Bash-only tests to Unix;
- retain host-independent `actionc-run --no-run` coverage on every platform;
- add focused Windows behavior where current tests are Unix-only.

### Slice 2: build identity

- pin the stable Rust toolchain;
- add a shared build-information module;
- add `--version` and `-V` to the three public executables;
- test local fallback and injected nightly metadata.

### Slice 3: portable packaging

- add the cross-platform packaging script;
- assemble the exact archive contract;
- add package-inventory and executable-mode tests;
- produce `BUILD-INFO.txt` and license notices.

The embedded-asset license gate that originally blocked this slice is now
closed by the notices and corresponding-source material described above.

### Slice 4: nightly build workflow

- add the four-target build matrix;
- install the musl target and tools on Linux;
- run tests, builds, native smoke tests, and packaging;
- upload intermediate artifacts with seven-day retention.

This slice exposes only a manual trigger. The packager enforces the complete
embedded-asset license material without an override. Its artifacts passed the
initial cross-platform build and packaging inspection.

### Slice 5: publisher

- aggregate all matrix artifacts;
- generate and verify `SHA256SUMS`;
- create or update the moving `nightly` prerelease;
- restrict write permission to this job;
- skip unchanged scheduled commits and support forced manual publication.

Implemented. `tools/prepare-nightly-release.py` rejects missing, unexpected,
corrupt, or mixed-commit matrix archives; checks their complete inventories and
build identities; verifies the pinned Action! source snapshot; and generates
and rechecks `SHA256SUMS`. The publisher alone receives `contents: write`, moves
the `nightly` tag, replaces the prerelease assets, removes stale assets, and
verifies the final asset inventory. Manual dispatches always publish; once the
schedule is enabled, an unchanged commit is skipped.

### Slice 6: rollout

- manually run the complete build and publication workflow;
- inspect every archive on its target host;
- verify that every archive contains the required license and source material;
- inspect the first moving `nightly` prerelease and its checksums;
- perform real-emulator smoke tests;
- enable the daily schedule.

## Follow-ups

After the nightly channel is stable, consider:

- macOS universal archives;
- Linux ARM64 and Windows ARM64;
- macOS notarization and Windows signing;
- GitHub artifact attestations;
- reuse of the tested packaging jobs by stable tagged releases.
