# MIR6502 wide routine ABI implementation plan

Status: complete, 2026-11-17. All four slices implemented and qualified.

Implement the [design](MIR6502_WIDE_ROUTINE_ABI_DESIGN.md) in four vertical
slices, committing each after its focused checks. Commit the design and this
plan first. Preserve unrelated working-tree changes.

## Baseline and scope

Start at `7c8110aa`. The [runtime-contract qualification](benchmarks/6502-runtime-contracts/README.md)
records TN at 9,913 bytes with ten named-ZP routines; Huffman portable/driver/
no-inline at 3,851/3,781/3,851 bytes, with zero named-ZP routines. Keep these
sources, origins and configurations for the final comparison.

Changes stay in MIR6502 ABI metadata and its consumers. Retain emitted calling
conventions, byte/word instructions, helper bodies, capture strategy, source
arity rules, ZP eligibility/pool, effects, final-cost acceptance and rollback.
No source/NIR or unrelated allocator changes. The existing six MIR6502 unit
failures and one callee-facts VM placement failure are recorded with baseline
reproductions in the qualification above; do not relax them.

## Slice 1 — Represent complete integer ABI values

- Introduce `MirAbiIntegerWidth` for byte/word/long and extend the existing
  direct parameter/result types. Keep one LONG parameter and one ParamId.
- Provide explicit byte extent and machine transfer projections, keeping
  `MirWidth` byte/word. Migrate narrow constructors and consumers mechanically.
- Derive parameter and result metadata independently from verified NIR types;
  validate integer bits/extent and pointer address space. Reuse the integer
  width projection for `MirScalarSignature`.
- Print LONG results readably and include all four canonical result bytes.
  Retain conservative rejection wherever a consumer cannot yet validate Long.
- Extend metadata tests for signed/unsigned wide values, pointer mixtures,
  narrow/void results with wide parameters, independent unknown facts and
  inconsistent type/width declarations.

Run the affected lowering/result tests and current runtime-contract tests.
Commit the representation slice.

## Slice 2 — Validate and compose the complete ABI

- Centralize complete logical transfer groups, byte counts and bounded physical
  argument projection. Use these in call planning and structural verification;
  preserve legal omitted-argument prefixes without admitting half a LONG.
- Extend direct result validation to the existing primary/additional word
  layout, including unused/partial captures and materialized calls. Reject
  missing, duplicated, displaced or wrongly sized high-result declarations.
- Validate known frame/signature consistency, including a scalar four-byte slot
  without a single machine width. Keep optional unknown evidence conservative.
- Admit validated four-byte direct capture and larger SArgs frames to entry
  analysis. Use byte extents for ABI observation accounting and retain scratch
  and input-supply checks.
- Extend current every-return-path result proofs to four bytes through the
  shared result projection. Test partial definitions, old high-byte reads,
  nested calls, absent normal returns and stale metadata/dependencies.
- Check consistency with integer-only scalar signatures without turning pointer
  signatures into integers or creating another fact catalog.

Run focused ABI, entry/result, verifier and observation tests plus existing
`mir6502_call_abi` integration checks. Commit the proof-consumer slice.

## Slice 3 — Exercise public APIs and caller preservation

- Inspect real module/provider declarations for `SqrWide`, `AsrLI`, `ValLC`/
  `ValLI` and long input/print/conversion entries. Inspect `u32toa`'s target
  applicability; use a LONG/pointer/SIZE control for MIR6502 because CSTRING is
  a native 65816 library. Assert complete
  logical metadata independently of whether remaining effects permit entry
  composition. Disable inlining where calls need to survive.
- Reuse wide nested direct/typed-indirect execution and long parser function-
  pointer tests in both runtimes. Exercise first/shifted LONG arguments, following
  arguments, signed values, unused results and SArgs capture.
- Add one focused control for a live narrow caller home across a wide-signature
  callee, checking actual placement/results; use analysis controls for high
  argument/result-byte collisions. Do not broaden four-byte home eligibility.

Run these explicit VM targets/filters and affected library-unit cases. Commit
the public/runtime qualification slice.

## Slice 4 — Measure and close

- Repeat the existing TN/Huffman fact probes at matching origins/settings.
  Keep old receipts intact and save a new compact comparison with hashes,
  entry/result facts, homes, sizes, remaining blockers and fallback events.
- Require ReadBits to retain its CARD parameter and LONG result; report actual
  composed certificates rather than promising downstream ZP or size changes.
- Run the Huffman cartridge/standalone vector controls with LF/CRLF loading.
  If TN's image changes, exercise its affected VM controls.
- Broaden once to the MIR6502 library-unit filter and affected ABI/runtime
  targets. Investigate new failures; distinguish known baseline failures.
- Confirm tracing does not change images, remove temporary audit hooks and
  update design/plan status. Check documentation links and commit the receipt.

## Validation discipline

Use the existing `build/checked-arena/shared-target`, optimized dev/test builds,
disabled debug information, two build jobs and disabled incremental caching to
limit disk growth. Do not repeat passing suites after documentation-only edits.
Shared frontend/NIR changes would require the AGENTS.md shared-contract checks;
they are outside this plan. Preserve exact binary/ATASCII fixtures and normalize
host LF/CRLF text where required by the existing test paths.

## Implementation receipts

- Slice 1: `cargo test --lib mir6502::lower::direct --no-run` built the test
  runner with the documented profile. That runner passed the combined
  `mir6502::lower::direct` and `mir6502::runtime_contracts` filters: 13 tests.
  Existing narrow/pointer behavior, the new LONG metadata and independent
  result discovery pass. Wider call admission remains conservative until slice 2;
  the expanded metadata/layout matrix is covered with its proof consumers.
- Slice 2: the focused ABI/entry/verifier/runtime-contract/observation run passed
  132 tests. After adding high-word return liveness, the broader MIR6502 library
  run passed 1,531 tests, with 15 ignored and the same six baseline failures
  recorded above; no new failures. `mir6502_call_abi` passed all 11 integration
  tests. Wide result declarations now survive materialization, and both high
  result bytes participate in return liveness and caller-home observations.
- Slice 3: `wide_abi_homes` passed two tests: twelve real SYS/math API signatures
  plus a LONG/pointer/SIZE control, and live caller-home execution across first
  and shifted LONG/SArgs calls in both runtimes. Re-exported the ABI width and
  parameter kind alongside the existing public result kind. The native 65816
  CSTRING module is outside MIR6502 scope; corrected that inventory entry.
  Existing nested-wide-call and parser-function-pointer filters each passed;
  `cross_call_homes` passed 14 tests and `runtime_library_contracts` passed three.
  Huffman passed its two MIR6502 provider tests and LF/CRLF vector control
  (370 executions), covering the final qualification's execution obligation.
- Slice 4: the combined fact audit passed both tests and retained matching
  source/image hashes. `ReadBits`, `ReadHeader` and `Decode` gain composed entry
  certificates in all three Huffman configurations; no certificates are lost.
  TN remains 9,913 bytes and Huffman remains 3,851/3,781/3,851 bytes. All images,
  named homes and temporary homes are byte-for-byte unchanged. The existing
  Pull/WriteByte remapping fallbacks remain. The slice 2 backend-unit run and
  slice 3 execution controls complete validation; no duplicate suites were
  needed. Temporary hooks were removed and documentation links checked.
  See the [qualification and remaining blockers](benchmarks/6502-wide-routine-abi/README.md)
  for reproducible facts, hashes and exact check results.
