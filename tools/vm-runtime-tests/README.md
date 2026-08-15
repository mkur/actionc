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

The compatibility entry point from the repository root remains:

```sh
fixtures/runtime/run-initialized-arrays-vm.sh
```
