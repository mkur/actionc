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

The [signed Q8.8 library](../../docs/FIXED_POINT_Q8_8.md) is checked by
`cargo test --locked --test fixed_q8_8`. Five tests cover 3,810 VM executions:
150 conversion/constant cases, 3,462 boundary/random arithmetic cases, 72
division-fault cases, 120 staged/nested composition cases, and six sample
output runs. Common source runs in Compatibility, Optimized classic, and
MIR6502 with both runtimes; nested expressions additionally exercise the four
modern lanes. Numerical standalone runs load no ROMs. Host i64 oracles check
complete guarded regions, including page-crossing INT stores; fault tests
verify Error(101) and non-returning behavior even with a returning Error hook.

The [signed Q4.12 library](../../docs/FIXED_POINT_Q4_12.md) is checked by
`cargo test --locked --test fixed_q4_12`. Two tests cover 3,774 VM executions:
617 arithmetic pairs across six configurations, plus 72 division faults.
They check truncation, explicit floor rounding, wrapping, and exact LONGCARD
squares stored across a page boundary, using the shared fixed point harness.

The [Oscar64 Mandelbrot port](../../samples/graphics/mandelbrot/README.md) has
a dedicated `cargo test --locked --test oscar64_mandelbrot` target. Eight tests
cover 5,352 executions: 2,424 original numerical cases, 2,904 display-coordinate
cases, six printing probes, six selected-row renders, and one native-resolution
160x192 image in standalone MIR6502. The independent
integer oracle preserves Oscar64's floor rounding and wide radius test. Graphics
checks compare every modeled CIO pixel and palette values, including untouched
regions; they do not emulate ANTIC scanout or verify OS screen-memory packing.
Another eleven executions cover VBXE: four selected-row renders in Optimized
classic/MIR6502 on both register pages, one complete 320x192 MIR6502 image, and
six missing/incompatible-hardware runs. A focused bus-event model checks
revision reads, palette writes, XDL bytes, MEMAC banking, all framebuffer pixels
and padding, plus untouched local memory. It does not emulate VBXE scanout.

The [LET code-generation audit](../../docs/Action_2027/LET_CODEGEN_AUDIT.md)
compares a reused mutable local, distinct mutable locals, and sequential LET
bindings. `cargo test --locked --test let_codegen_audit -- --nocapture` checks
1,296 executions and reports XEX sizes and CPU cycles in both modern backends
and both runtimes. Set `ACTIONC_LET_AUDIT_DIR` to an existing empty directory to
retain sources, IR, listings, objects and CSV measurements. Performance numbers
are reports, not golden assertions; functional results use independent oracles.

The [IF/CASE codegen audit](../../docs/Action_2027/IF_CASE_EXPRESSIONS_CODEGEN_AUDIT.md)
compares statement and expression forms, including known-SOME and dynamic
variant selectors. `cargo test --locked --test if_case_codegen -- --nocapture`
checks 384 executions and reports XEX bytes and cycles. It also asserts that
verified optimization removes known-tag dispatch while preserving dynamic
validation. `cargo test --locked --test if_case_expressions` covers 20 tests
for consumers, effects, exact-width joins, invalid tags and the published sample
across the supported classic/raw-MIR/optimized-MIR runtime matrix.

The accompanying optimizer regressions can be run with:

```sh
cargo test --locked --test static_table_pointer_copy --test connected_scalar_relays
```

They check 1,008 captured-pointer/index copy cases and 24,576 connected-relay
executions respectively, with raw/optimized MIR, both runtimes, two origins,
byte wrapping, all byte values and page crossings. They check complete guarded
output regions; the relay test also requires elimination of the staging homes.

The [Oscar64 behavioral ports](../../fixtures/runtime/oscar64/README.md) cover
array indexing, word-pointer transfers, loop bounds, comparisons, masks,
shift/add/sub composition, signed multiplication, reverse-copy loops, nested
calls, signed intervals, mixed INT/BYTE comparison values, record-array copies,
inline record-member arrays, signed division, full-range unsigned div/mod,
mixed-width IF selection, enum CASE dispatch, and 8/16/32-bit rotations using
independent host-side oracles. Run them separately with:

```sh
cargo test --locked --test oscar64_conformance
```

The original 14 Oscar64 tests retain 258 passing VM cases, including the
formerly failing MIR6502 word-vector initialization checks. The second batch
and focused IF/CASE and rotation ports bring the total to 32 active tests and
16,438 VM cases, including the 512 repaired Compatibility nested-call cases and 120 repaired
classic reverse-copy cases. Stage 4 adds 408 branch/count cases across all modes
and 264 numeric comparison-value cases across modern classic and MIR6502.
Compatibility's semantic rejection of the extension is checked separately.
Stage 5's first port adds 198 record-array copy cases with observable calls,
mixed-width fields, page-boundary layouts and independent complete-memory oracles.
The second port adds 120 modern inline-member/vector cases, preserving the
original record layouts. Its two Compatibility rejection checks are not
counted as VM executions. No pointer-backed workaround replaces inline members.
No Oscar64 tests are ignored. See the fixture README for the mode/case matrix
and the additional 1,344 mixed-width IF cases, including explicit arm widening,
outer narrowing, repeated calls, full-page guards and separate Compatibility
rejection checks, as well as resolved compiler regressions. The enum-switch
port adds 1,024 VM cases covering statement and expression CASE over every
BYTE enum representation, including default dispatch for unnamed values.
Both forms retain the original four-call sum check and have independent result
oracles; Compatibility rejection is checked separately from the VM count.
The rotation ports add 1,542 BYTE/CARD cases across all modes and runtimes and
456 LONGCARD cases across all modes and both runtimes. They retain the original
seeds and inverse/cycle checks, with independent expected values for every
left/right table entry, walking-bit patterns, odd bases and guards. Classic
LONG coverage also includes typed arithmetic, calls, captures, loops, selections,
volatile byte traces, decimal I/O and the maintained sample.
`cargo test --locked --test comparison_values`
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
