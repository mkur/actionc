# MIR6502 terminal cleanup qualification — 2026-11-13

The shared terminal transaction removes stores made dead by late consumer
changes. It preserves frames and producers, validates the final placement and
earlier selected decisions, and accepts only a smaller packaged image.
The [design](../../MIR6502_LATE_CLEANUP_DESIGN.md) and
[implementation plan](../../MIR6502_LATE_CLEANUP_IMPLEMENTATION_PLAN.md) describe
the ownership and acceptance contract.

The frozen [phase-tracing corpus](../6502-phase-tracing/baseline.json) supplies
the original artifacts. Its historical report and receipts are preserved.
[qualification.json](qualification.json) records the new hashes, exact sizes,
proof outcomes and work counts. Large output files and complete trace bundles
remain under ignored `build/mir6502-late-cleanup/qualification/`.

## Results

| Case | Runtime | Before | After | Saving |
| --- | --- | ---: | ---: | ---: |
| TN | cart | 10,000 | 9,982 | 18 bytes |
| TN | standalone | 10,107 | 10,089 | 18 bytes |
| Argument staging | cart | 334 | 330 | 4 bytes |
| Argument staging | standalone | 416 | 412 | 4 bytes |

All sizes are complete XEX images. TN removes four stores in
`FormatDirectoryRow` and two in `SetWin`; their instruction footprint falls by
18 bytes. All six home declarations remain allocated. Argument staging removes
two zero-page stores, saving four instruction bytes without reallocating homes.
Other CLI corpus images are unchanged. Full-fixture API controls also establish
no image growth, including shifted origins and checked-range retention.

TN keeps three full materializations. Top-level emissions increase from 41 to
43, inner emission probes from 246 to 258, and shared rewrite-analysis builds
from 24,626 to 24,787. Six earlier selected decisions are revalidated against
the new placement. The filtered trace contains 17,044,538 encoded detail bytes
for the cart case, compared with 16,756,477 in the frozen tracing report.

Separate TN process measurements with Darwin `/usr/bin/time -l` were:

| Compiler / diagnostics | Wall time | Peak RSS |
| --- | ---: | ---: |
| Frozen baseline, tracing off | 7.76 s | 80.3 MiB |
| Current compiler, tracing off | 8.72 s | 83.4 MiB |
| Current compiler, filtered tracing on | 9.64 s | 225.7 MiB |

This observed run adds about 0.96 seconds and 3.1 MiB with tracing disabled.
It is a single comparison on a shared machine, so the timing is indicative.
These separate measurements ran after backend qualification; the corpus's first
TN measurements overlapped the tail of the VM run. The resumed harness RSS
counter covers only its completing invocation, as labeled in the receipt.

## Validation

- 1,296 MIR6502 library tests passed; 13 existing optional checks were ignored.
- 127 integration tests passed across all 27 `mir6502_*` targets and
  `cli_emit_modes`; ten optional assembler/frozen-artifact checks were ignored.
- 28 VM tests passed across eleven targets: argument staging, terminal cleanup,
  pointer retention, bounded-call retention, checked ranges, byte-plus-one,
  pointer-load sharing, word-load sharing, TN reproducers, TN arena and TN
  dispatch. One optional frozen unchecked allocator measurement was ignored.
- All 54 CLI artifacts match with tracing enabled and disabled; each case's
  complete journals agree across XEX/listing/map modes. All 20 full API
  comparisons pass. Sixteen MADS LF/CRLF roundtrips reconstruct the exact images.

The validation receipt records targets, commands and counts. Local execution was
scoped to MIR6502 and its runtime consumers; NIR and other backend contracts did
not change. The repository's full CI matrix remains applicable.

## Reproduce

```sh
CARGO_INCREMENTAL=0 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_DEV_OPT_LEVEL=1 \
  CARGO_BUILD_JOBS=2 CARGO_TARGET_DIR=build/checked-arena/shared-target \
  cargo build --bin actionc-emit
python3 tools/compare6502/late_cleanup.py --mads /usr/local/bin/mads
```

The default reference directory is
`build/mir6502-phase-trace/baseline/<case-id>/output.{xex,asm,map}`.
`--reference` selects another archive. If these ignored artifacts are absent,
build `actionc-emit` at the frozen receipt's `baseline_revision` in a separate
checkout and pass its executable as `--reference-compiler`. That regenerates
reference bytes in memory and checks their frozen hashes before comparison.
The current and reference compilers consume the same manifest sources.

`--compiler`, `--api-rlib` and `--output` override the current executable,
matching library and fresh output directory. `--resume` can continue an incomplete
run after checking its compiler identity and completed artifact prefix. Resumed
RSS accounting explicitly excludes earlier subprocesses. Omit `--mads` to skip assembler
roundtrips. The compiler and API library must be from the same build.
The corpus runner checks normalized source hashes, so unrelated fixture changes
cannot silently become a new baseline.

## Coverage and limits

CLI qualification compares load images, listings and maps with tracing enabled
and disabled. Complete journals must agree across all three output modes.
Every accepted transaction must retain identical frame declarations. The six
TN homes must still exist with zero final reads and writes. Materialization
counts must remain equal, and terminal cleanup may add at most two top-level
emissions. Inner branch/layout probes and analysis builds are recorded separately.

The Rust probe uses complete fixture APIs for home invalidation, argument
staging, checked ranges and word sharing, in cart and standalone runtimes at
`$3000` and `$41C7`. It toggles only terminal cleanup, compares complete
`CodegenOutput` for trace equivalence, and requires exact output equality when
no saving is selected. This preserves controls that CLI reachability would prune,
including a semantic memory read that emits zero bytes and the T10 endpoint.

Safety tests cover successor/backedge reads, physical aliases, unknown readers,
callee-pointer and caller-after-return observers, definite overwrites, explicit
SArgs overrides, required callee facts, selected partial rollback, changed data
placement, code overlap and missing preservation information. VM checks compare
public writes, argument values and final registers/flags. LF/CRLF sources pass
through the actual include loader; listing checks feed both encodings to MADS.

Ordinary private frames keep the existing non-escape contract. Public zero-page
backing receives the additional physical read proof. The first implementation
retains immutable selected snapshots and replays family-owned proofs; its extra
memory and analysis work are measured here. Frame compaction and removal of dead
register producers remain separate work.

Wall times are observations from a shared development machine, without a
statistical performance threshold. Maximum child RSS covers compiler, Rust API
probe build/execution and MADS, rather than isolated sidecar allocations.
