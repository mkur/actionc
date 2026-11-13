# MIR6502 late producer cleanup qualification — 2026-11-13

The terminal transaction now removes proved-dead byte producers after late
consumer changes. It reuses the existing register/flag liveness analysis with
final-placement read-purity checks. Frames stay allocated; selected earlier
proofs, callee contracts, final image cost and complete rollback still govern
acceptance. The [design](../../MIR6502_LATE_CLEANUP_DESIGN.md#producer-deletion-extension)
defines the admitted load and native-transfer shapes.

This report compares against the frozen
[store-only qualification](../6502-late-cleanup/README.md). Its receipt remains
unchanged. The new [qualification.json](qualification.json) records image hashes,
work counts and proof outcomes. Detailed artifacts remain under ignored
`build/mir6502-late-producers/`.

## Results

| Program | Runtime | Store-only XEX | Producer-enabled XEX | Additional saving |
| --- | --- | ---: | ---: | ---: |
| TN | cart | 9,982 | 9,970 | 12 bytes |
| TN | standalone | 10,089 | 10,077 | 12 bytes |
| Independent pointer transform, full API | cart | 105 | 99 | 6 bytes |
| Independent pointer transform, full API | standalone | 105 | 99 | 6 bytes |

TN deletes two ordinary byte loads in `FormatDirectoryRow` and two in `SetWin`.
All four read `s` or `s+1` into A before another instruction overwrites the result
and N/Z. These four operations account for twelve instruction bytes. Combined
with the prior six store deletions, TN now saves thirty bytes from the original
10,000-byte cart image. All six spill declarations remain allocated.

The other sixteen CLI corpus images are unchanged. In particular, argument
staging retains its earlier four-byte saving; it gains no additional producer
saving. Complete-fixture API comparisons also cover shifted origins and T10
checked ranges, with exact output equality for unchanged controls.

TN's work remains three materializations, 43 top-level emissions, 258 inner
emission probes and six selected-decision preservation replays. Rewrite-analysis
builds rise from 24,787 to 24,926: 139 additional builds. The new producer loop
uses the existing transaction's two cost emissions. Cart trace detail grows
from 17,044,538 to 17,116,767 encoded bytes; that is distinct from heap usage.

Separate TN cart processes, measured after qualification jobs finished:

| Compiler mode | Wall time | Peak RSS |
| --- | ---: | ---: |
| Store-only, tracing off | 8.45 s | 81.59 MiB |
| Producer cleanup, tracing off | 7.93 s | 81.70 MiB |
| Producer cleanup, tracing on | 9.28 s | 216.69 MiB |

These are single-run observations, not evidence of a compilation speedup.
The traced run selects `FormatDirectoryRow` and `SetWin`; its image is identical
to the untraced image. The receipt also distinguishes these process measurements
from the qualification harness's aggregate maximum child RSS.

## Checks

- 1,303 MIR6502 library tests passed; 13 existing optional checks were ignored.
- 129 tests passed across all 27 MIR6502 integration targets plus CLI output
  modes. Eleven optional tests were ignored in that sweep; the new MADS test
  was then run explicitly and passed eight reconstruction/relocation roundtrips.
- 30 VM tests passed across twelve relevant targets, including the independent
  producer oracle and intact TN directory-row routine. One optional frozen
  allocator measurement was ignored.
- 54 CLI trace-on/off artifact comparisons passed, with identical complete
  journals across load/listing/map modes. All 28 complete API comparisons also
  passed. The corpus passed sixteen further LF/CRLF MADS roundtrips, for
  twenty-four in total.

Commands, target lists, counts, complete-fixture API results and resource
measurements are recorded in the receipt. Backend-scoped local validation leaves
the repository's full CI matrix applicable. No shared frontend/NIR contracts
changed.

## Reproduce

Build `actionc-emit` with the same profile used for the earlier qualification:

```sh
CARGO_INCREMENTAL=0 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_DEV_OPT_LEVEL=1 \
  CARGO_BUILD_JOBS=2 CARGO_TARGET_DIR=build/checked-arena/shared-target \
  cargo build --bin actionc-emit
python3 tools/compare6502/late_cleanup.py \
  --baseline-receipt docs/benchmarks/6502-late-cleanup/qualification.json \
  --reference build/mir6502-late-cleanup/qualification/run-eodwi7ub \
  --output build/mir6502-late-producers/qualification \
  --mads /usr/local/bin/mads
```

`--reference` names the archived store-only output directory. If that ignored
archive is absent, build the compiler at `eeaec45a` in a separate checkout and
use `--reference-compiler /path/to/old/actionc-emit` instead. Frozen artifact
hashes must agree before comparison. Omit `--mads` when unavailable.
`--compiler` and `--api-rlib` select a matching current executable/library pair.
`--resume` validates and resumes an incomplete run.

With `--baseline-receipt`, the complete-fixture API probe toggles only
`enable_late_dead_register_cleanup`, keeping store cleanup enabled. It includes
TN and the independent pointer-transform fixture in both runtimes at `$3000`
and `$41C7`. CLI comparisons use the original eighteen-case corpus and all three
output modes. Accepted transactions must retain identical frames, and the six
TN spill declarations must still have zero reads and writes. The historical
`late-private-store-cleanup` trace stage now reports separate store/producer
counts. Candidate cost emissions stay bounded by two per transaction.

## Qualification boundaries

The focused proof tests cover private-store/producer interaction, producer-only
opportunities, live A/X/Y and N/Z consumers, successor and backedge uses,
ordinary versus hardware/executable/missing placement, raw absolute and indirect
reads, native transfers, caller-observed exit facts and zero-byte cost rollback.
The core dead-register pass retains its prior purity predicate.

The independent VM test loads real LF/CRLF source files, compares both runtimes
and origins, and checks transformed bytes, ordered public writes, A/X/Y/SP and
status. A separate test executes TN's actual `FormatDirectoryRow`, including
both sector-text widths, the space-prefix branch, page crossings and surrounding
sentinels. Its results and machine state agree with store-only compilation and
a host conversion oracle. Existing TN runtime tests supply broader application
coverage; these tests do not constitute an interactive DOS end-to-end run.

Frame pruning, arithmetic deletion and indirect-read elimination remain outside
this extension. Whole-transaction rollback retains the pre-cleanup input if
any producer, callee, earlier-decision or cost obligation fails. Timings and RSS
are development-machine observations, not statistical performance thresholds.
