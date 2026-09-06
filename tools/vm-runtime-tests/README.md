# VM runtime tests

This isolated harness runs generated Action! objects through the reusable
`actionc-vm` library. Its VM dependency is pinned to an exact Git
revision and is deliberately absent from the root `actionc` manifest, so normal
compiler builds do not resolve or compile the VM.

Run the tests from this directory so Cargo reads `.cargo/config.toml` and uses
the pinned `actionc-vm` revision:

```sh
cargo test --locked --no-fail-fast
```

The [Oscar64 behavioral ports](../../fixtures/runtime/oscar64/README.md) cover
array indexing, word-pointer transfers, loop bounds, comparisons, masks,
shift/add/sub composition, signed multiplication, reverse-copy loops, nested
calls, signed intervals, mixed INT/BYTE comparison values, record-array copies,
inline record-member arrays, signed division, and full-range unsigned div/mod
using independent host-side oracles. Run them separately with:

```sh
cargo test --locked --test oscar64_conformance
```

The original 14 Oscar64 tests retain 258 passing VM cases, including the
formerly failing MIR6502 word-vector initialization checks. The second batch
now brings the total to 28 active tests and 12,072 VM cases,
including the 512 repaired Compatibility nested-call cases and 120 repaired
classic reverse-copy cases. Stage 4 adds 408 branch/count cases across all modes
and 264 numeric comparison-value cases across modern classic and MIR6502.
Compatibility's semantic rejection of the extension is checked separately.
Stage 5's first port adds 198 record-array copy cases with observable calls,
mixed-width fields, page-boundary layouts and independent complete-memory oracles.
The second port adds 120 modern inline-member/vector cases, preserving the
original record layouts. Its two Compatibility rejection checks are not
counted as VM executions. No pointer-backed workaround replaces inline members.
No Oscar64 tests are ignored. See the fixture README for the mode/case matrix
and resolved compiler regressions. `cargo test --locked --test comparison_values`
also runs 24 modern consumer cases checking widths, calls, eager composition,
indexed destinations and captured pointers.
`cargo test --locked --test compound_assignments` adds 30 public-language VM
executions (five input sets, three modes, both runtimes), each checking ten
compound operators for BYTE, CARD and INT with neighboring-byte guards and
counted RHS calls. Its signed MOD inputs are nonnegative; negative inputs are
covered by the modern arithmetic target below. This coverage is
separate from the Oscar64 case count and from the modern embedded-array tests.
Use `--no-fail-fast` to run the remaining test binaries after a failing one.

`cargo test --locked --test modern_arithmetic` checks all 16 ordered integer
type pairs over 28 boundary inputs in all six mode/runtime combinations (2,688
executions), plus constants/static images, signed multiplication composition,
narrow helper selection and division-by-zero behavior. Fault tests distinguish
the specified non-returning stop from a watchdog hang. These are independent
modern oracles, not expectations copied from the original cartridge. Both
linking modes use compiler-owned division/remainder helpers, including in the
Compatibility profile. See the
[arithmetic contract and implementation record](../../docs/MODERN_INTEGER_ARITHMETIC_IMPLEMENTATION_PLAN.md).

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
