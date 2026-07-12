# Release Plan

This note captures the current release direction: cut a conservative preview
before doing broader refactors.

## Scope

- Treat `--profile legacy --backend classic` as the supported default path.
- Document modern/classic as usable where it is known to work.
- Document `mir6502`, SemIR-native, stress probes, and runtime sweeps as
  experimental.
- Avoid broad architecture refactors before the tag; limit pre-release work to
  release hardening.

## Freeze And Triage

1. Inspect the dirty tree and split changes into release-required,
   non-release, and generated/build artifacts.
2. Commit or park only release-required work.
3. Confirm `compile-run-atr.sh` uses the in-tree ROM.
4. Confirm default profile/backend behavior is legacy/classic.
5. Confirm samples do not depend on removed libraries.

## Documentation

Update or add:

- `README.md`: quick start, default compile command, ATR workflow.
- `samples/README.md`: known-good examples.
- `KNOWN_LIMITATIONS.md`: MIR6502 experimental status and unsupported
  constructs.
- `CHANGELOG.md` or release notes: what works, what changed, and what remains
  intentionally incomplete.

## Validation Matrix

Run and record:

```sh
cargo test
cargo run --bin actionc -- --output target/release-smoke/hello-world.com samples/hello-world.act
cargo run --bin actionc -- --profile legacy --backend classic --output target/release-smoke/logo-legacy.com samples/logo.act
cargo run --bin actionc -- --profile modern --backend classic --output target/release-smoke/logo-modern.com samples/logo.act
cargo run --bin actionc -- --profile modern --backend classic --output target/release-smoke/tn-modern.com samples/tn/modern/TN.ACT
```

Also run report-only sweeps:

- stress probes across both profiles and both backends
- runtime sweep through `SYSALL.ACT`
- selected toolkit demos as smoke coverage

## Manual Smoke

Create ATRs and boot:

- `hello-world`
- `logo`
- `TN`

For TN, verify directory listing, file selection counts, delete confirmation
counts, and the basic copy path if it is stable enough.

## Cut Release

1. Choose a preview tag, for example `v0.1.0-preview`.
2. Build the release binary:

   ```sh
   cargo build --release
   ```

3. Capture commit hash, test summary, known limitations, and ROM/cart
   provenance.
4. Tag the release:

   ```sh
   git tag -a v0.1.0-preview -m "actionc v0.1.0 preview"
   ```

## After The Release

Start a refactor branch after the tag. Good first targets:

- unify proof/effects terminology
- clean SemIR vs AST backend boundaries
- decide what proof reporting should mean for MIR6502
- turn TN/debug artifacts into intentional samples or fixtures
