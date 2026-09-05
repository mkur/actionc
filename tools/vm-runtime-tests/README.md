# VM runtime tests

This isolated harness runs generated Action! objects through the reusable
`actionc-vm` library. Its VM dependency is pinned to an exact Git
revision and is deliberately absent from the root `actionc` manifest, so normal
compiler builds do not resolve or compile the VM.

Run the tests from this directory so Cargo reads `.cargo/config.toml` and uses
the pinned `actionc-vm` revision:

```sh
cargo test --locked
```

The [Oscar64 behavioral ports](../../fixtures/runtime/oscar64/README.md) cover
array indexing, word-pointer transfers, loop bounds, comparisons, and masks
using independent host-side oracles. Run them separately with:

```sh
cargo test --locked --test oscar64_conformance
```

All 14 Oscar64 tests are active: 258 VM cases, including the formerly failing
MIR6502 word-vector initialization checks. See the fixture README for the
mode/case matrix and resolved compiler regressions.

The test-enforced coverage ledger in `src/sys_coverage.rs` maps every public
`SYS` routine to a fixture that invokes it and is wired into this harness. Any
new interface routine must gain VM execution coverage or an explicit deferral.
The current ledger has no deferred routines.

The selectively linked standalone-library groups can be exercised together:

```sh
cargo test --locked selectively_linked_
```

Self-contained fixtures use the VM's standalone-object profile and need no
ROMs. Fixtures whose generated code calls Action! or OS services use the
cartridge-object profile; the harness reads the repository's ROM files and
passes their bytes to the VM library itself.

Every `fixtures/runtime/run-*-vm.sh` compatibility entry point now selects its
corresponding library test. Scripts that also assert a compiler selection keep
that preflight before invoking the harness. Examples:

```sh
fixtures/runtime/run-initialized-arrays-vm.sh
fixtures/runtime/run-kalscope-contracts-vm.sh
fixtures/runtime/run-direct-word-compares-vm.sh
fixtures/runtime/run-direct-byte-array-indexes-vm.sh
fixtures/runtime/run-scaled-card-indexes-vm.sh
fixtures/runtime/run-ordered-absolute-sub-vm.sh
fixtures/runtime/run-paired-word-arithmetic-compare-vm.sh
```
