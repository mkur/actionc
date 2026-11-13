# MIR6502 phase tracing qualification — 2026-11-13

TN's six reviewed stores survive because **pointer retention removes their last
explicit readers after core cleanup has finished**. They have no hidden final
MIR loads. The accepted transaction preserves their frame declarations and
producer stores, and the remaining late pipeline does not rerun private-store
cleanup or prune these homes.

This change adds diagnostics only. TN's cart image remains **10,000 bytes**.
The pre-change reference is `c00f37a84b8c0943c81565723420ace2c0d4f8d2`.
[baseline.json](baseline.json) records frozen SHA-256 hashes;
[qualification.json](qualification.json) records the final comparisons and
scoped evidence references. Large journals and MIR dumps stay under ignored
`build/mir6502-phase-trace/qualification/`.

## Reproduce

Build `actionc-emit`, then run the harness against that executable and its
matching library. For the shared target used during this work:

```sh
CARGO_INCREMENTAL=0 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_DEV_OPT_LEVEL=1 \
  CARGO_BUILD_JOBS=2 CARGO_TARGET_DIR=build/checked-arena/shared-target \
  cargo build --bin actionc-emit
python3 tools/compare6502/phase_trace.py --check --mads /usr/local/bin/mads
```

`--compiler`, `--api-rlib`, `--baseline` and `--output` override their defaults.
MADS is optional; omit `--mads` when unavailable. Each run creates a fresh
subdirectory. `--check` additionally requires accepted, rejected and partial
selection coverage and all twelve TN observations (six stores, two runtimes).
Every run checks artifact hashes and complete journals for XEX, listings and
maps. The XEX journal per input/config supplies the indexed diagnosis.

The harness compares pre-change, new trace-disabled and trace-enabled XEX,
listing and map artifacts across cart/standalone and displaced origins. Its
Rust API probe also compares complete `CodegenOutput`, including relocations,
against eight frozen API results: default home/staging configurations and
optimized home/checked-range configurations, each in both runtimes. The API
probe retains the complete fixtures. Their minimal CLI entry points otherwise
let the linker prune the interesting home and checked-range routines. It never changes
compiler profitability or asks the compiler to replay for diagnostics.

## Six stores, one measured sequence

The addresses below are the cart build at origin `$2C00`. Operation numbers are
zero based and belong to the named input/final snapshot. The receipt gives the
specific event IDs for each run; those IDs are not stable compiler identities.

| Routine | Temp lane → home | Last reader at T3 input | Final store | Home address |
| --- | --- | --- | --- | --- |
| FormatDirectoryRow | t6.0 → spill12 | b2/op10 | b2/op1, `$40E5` | `$2F06` |
| FormatDirectoryRow | t6.1 → spill13 | b2/op12 | b2/op4, `$40ED` | `$2F07` |
| FormatDirectoryRow | t13.0 → spill26 | b4/op13 | b4/op1, `$4113` | `$2F08` |
| FormatDirectoryRow | t13.1 → spill27 | b4/op15 | b4/op3, `$4119` | `$2F09` |
| SetWin | t55.0 → spill110 | b17/op7 | b17/op1, `$43AD` | `$2F44` |
| SetWin | t55.1 → spill111 | b17/op9 | b17/op3, `$43B3` | `$2F45` |

For every row, the journal establishes:

1. **Initial demand:** the home plan says `coupled`, with byte-load producers
   feeding a `materialize-address` consumer. Both lanes of each word get homes.
   This is the current home planner's reason, not a claim that no alternative
   register strategy is possible.
2. **Assignment and remapping:** the initial declared spills are the six IDs
   above. None participates in a coloring/ZP remap in the selected core attempt.
   The trace preserves the temp/lane distinction; a home ID alone is not a value.
3. **Cleanup retention:** both compatibility scans stop on an explicit read.
   The second scan's reader coordinates agree with the T3 input. In these scans,
   an unknown helper/alias barrier is not the stopping reason.
4. **Consumer removal:** the accepted `pointer_retention` transaction, T3,
   changes each home's explicit-read count from one to zero. Its applied
   `pointer-retention-setup` rewrites remove the redundant pointer setup.
   This is a snapshot-to-snapshot observation plus an applied rewrite range;
   it does not invent an operation identity across subsequent rewrites.
5. **Later behavior:** T4–T10 retain zero reads and one write. Word-load sharing
   changes the first FormatDirectoryRow store sequence, accounting for the
   final high-lane operation index; it is not the pass that removes these reads.
6. **Emission:** the exact final MIR still declares and writes each home.
   Each store emits three bytes and each home occupies one byte in final layout.
   There is no final read operation whose empty emitted range could explain the
   absence of assembly loads.

Current footprint: 18 instruction bytes and six allocated bytes. Measured
saving from phase tracing: zero. Deletion still requires the existing private
memory/effect proofs on the later MIR; this investigation does not substitute
an explicit-read count for those proofs.

## Controls and selected history

The corpus also covers home invalidation, argument staging, word-load sharing,
pointer loops, checked ranges, captured arguments, spill coloring, and an
actual cost-rejection case. The receipt lists their actual cleanup stopping
reasons and typed blockers, final unused-home counts, and empty emitted ranges.
Counts aggregate observed attempts, including speculative work; they are not
reported as accepted savings.

TN exercises a real partial captured-comparison rollback: routines r14, r50
and r51 are accepted while r56 is restored. The returned composite snapshot
and its selection event are recorded. The small call-predicate control exercises
rejection; the integration tests exercise nested inlining and its selected-input
rematerialization. Equivalent source snapshots are labeled as equivalences,
not assertions of a unique supplier.

| Control | Supported observation |
| --- | --- |
| Home invalidation (full API fixture) | Explicit readers retain stores; successful compatibility deletions also occur. Optimized final private homes have no remaining written-but-unread entries. |
| Argument staging | Its accepted transaction removes the last reads of virtual-ZP slot 1 in r2 and r3, leaving two written homes. This independently demonstrates the late-cleanup gap. |
| Word sharing | Its accepted rewrite leaves no written-but-unread private homes; one final memory-read operation emits zero bytes. |
| Pointer loop | Pointer retention is accepted; no compatibility spill-store candidates are observed. Missing candidates are not reported as successful deadness proofs. |
| Checked ranges (full API fixture) | T10 changes the actual final MIR in both runtimes; complete endpoint traces agree with the range-proof integration/VM checks. |
| Captured arguments | The observed core attempts retain 12 spill stores and remove four; final private homes have no unread writes. |
| Coloring | Typed remaps cover reuse; explicit readers retain 24 observed candidate stores across attempts, with no unread private writes at emission. |
| Cost rejection | Both accepted and rejected transaction outcomes occur; returned snapshots remain authoritative. |

The unrelated controls matter in two ways. Existing compatibility and analyzed
proofs encounter genuine explicit readers, overwrites, calls and opaque effects;
those reasons must remain distinct from late consumer removal. Separately, the
word/staging tests demonstrate final MIR reads that emit zero bytes. That second
mechanism is real, but does not explain TN's six stores.

## Next compiler slice

Make cleanup invalidation part of the late-transaction contract. The first
implementation should be a **frame-preserving private-store cleanup transaction**
after late consumer-changing passes, using the existing analyzed proofs. Record
which routines changed, preserve observable callee exit facts, verify the result,
and validate it against final placement and cost before acceptance.

Keep frame compaction as a separate slice: removing declarations changes data
placement and can invalidate earlier physical-memory and alias facts. Define how
those dependent proofs are replayed or invalidated before combining compaction
with cleanup. This addresses the demonstrated phase interaction without another
TN-specific matcher or a replay of the entire materializer.

## Validation and resource accounting

Qualification passed 1,282 MIR6502 unit tests, 124 tests across 26 MIR6502
integration targets plus the CLI-mode target, and 24 tests across eight VM
targets. The writer-failure unit check and final CLI LF/CRLF/determinism checks
also passed. Existing optional measurements/assembler checks remain ignored in
the broad suites (13 unit, 10 integration, one VM); the harness separately
passed 16 MADS LF/CRLF roundtrips. All 54 CLI artifacts and eight complete API
outputs match their pre-change references. Complete journals are identical
across the three CLI emit modes for every corpus case.

TN records three materializations, 41 emission invocations, 246 inner probes
and 24,626 shared-driver analysis builds. Its filtered complete trace retains
about 16 MiB of encoded detail. One measured cart XEX run took 7.27 seconds
without tracing and 8.60 seconds with tracing; those are single-run observations.
The main harness's maximum child RSS was about 217 MiB, including its compiler,
Rust probe and MADS subprocesses. The receipt identifies this measurement scope.

The receipt contains measured wall times, detail/bundle sizes and work counts.
Encoded collector size is distinct from heap usage. The optional child-RSS
measurement covers compiler, Rust probe and MADS subprocesses; it is not an
isolated measurement of collector allocations. Timings are observations, not
correctness thresholds.

The independent unit observer verifies equal materialization, emission, inner
probe and shared-driver analysis-build counts with tracing on and off. Disabled
contexts do not evaluate diagnostic payloads. The broad backend run also caught
and fixed an extra test-only zero-work observation caused by a temporary `Work`
initializer; the emission schedule itself remained unchanged.

See [trace usage and contracts](../../MIR6502_PHASE_TRACING.md) and the
[implementation plan](../../MIR6502_PHASE_TRACING_IMPLEMENTATION_PLAN.md) for the
scope of the diagnostic API and qualification checks.
