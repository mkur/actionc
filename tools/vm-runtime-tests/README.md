# VM runtime tests

This isolated harness runs generated Action! objects through the reusable
`action-compiler-vm` library. Its VM dependency is pinned to an exact Git
revision and is deliberately absent from the root `actionc` manifest, so normal
compiler builds do not resolve or compile the VM.

The VM repository is private. Run the tests from this directory so Cargo reads
`.cargo/config.toml` and uses the Git CLI's configured credentials:

```sh
cargo test --locked
```

Self-contained fixtures use the VM's standalone-object profile and need no
ROMs. Fixtures whose generated code calls Action! or OS services use the
cartridge-object profile; the harness reads the repository's ROM files and
passes their bytes to the VM library itself.

The compatibility entry points from the repository root remain:

```sh
fixtures/runtime/run-initialized-arrays-vm.sh
fixtures/runtime/run-kalscope-contracts-vm.sh
fixtures/runtime/run-direct-word-compares-vm.sh
fixtures/runtime/run-direct-byte-array-indexes-vm.sh
fixtures/runtime/run-scaled-card-indexes-vm.sh
fixtures/runtime/run-ordered-absolute-sub-vm.sh
fixtures/runtime/run-paired-word-arithmetic-compare-vm.sh
```
