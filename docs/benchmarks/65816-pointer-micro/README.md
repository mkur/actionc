# Native pointer micro-optimization measurements

This series implements the [ordered plan](../../MIR65816_POINTER_MICRO_OPTIMIZATIONS_PLAN.md).
The original [three-compiler analysis](../65816-execlists/README.md) remains the
baseline, including its recorded vbcc failures. Each new slice qualifies Action
against the same independent list-state oracle; foreign compiler output is not
a correctness oracle.

| Slice | Raw list bytes | Optimized list bytes | Change from preceding slice | Remove body bytes / cycles |
|---|---:|---:|---:|---:|
| Baseline | 3,635 | 3,557 | — | 82 / 154 |
| 1: zero-offset indirect accesses | 3,566 | 3,488 | −69 in each mode | 76 / 148 |
| 2: captured pointer null reduction | 3,524 | 3,446 | −42 in each mode | 76 / 148 |
| 3: native three-byte casts | 3,464 | 3,390 | −60 raw / −56 optimized | 76 / 148 |
| 4: zero-offset captured addresses | 3,404 | 3,330 | −60 in each mode | 76 / 148 |
| 5: constant-offset captured addresses | 3,288 | 3,214 | −116 in each mode | 76 / 148 |
| 6: single captured-pointer edges | 3,288 | 3,174 | unchanged raw / −40 optimized | 76 / 148 |
| 7: acyclic captured-pointer edges | 3,288 | 3,140 | unchanged raw / −34 optimized | 76 / 148 |

Slice 1 removes 23 `LDY #0` instructions in each mode. Guards remain 540 bytes;
optimized bodies total 2,948 bytes. Per-vector cycle changes range from −75 to
zero, with no regressions. Remove saves six cycles on all nine vectors.
DP/stack read and write counts, stack peaks, frames and reservations are
unchanged. These are module measurements, not an estimate for all of Exec.

[Sizes](slice1/sizes.csv), [measurements](slice1/measurements.csv),
[deltas](slice1/delta.csv), [raw code](slice1/actionc-raw.lst),
[optimized code](slice1/actionc-optimized.lst), and
[provenance](slice1/provenance.json) retain the complete result.

Qualification for slice 1:

- 221 native library tests passed (one opt-in inventory ignored); 17 rewrite
  tests include live Y/N/Z, memory-change, nonzero-offset and event rejection.
- 22 emission and 11 o65 integration tests passed.
- Qualified `memory`, `pointer_allocation`, `pointer_preemption`,
  `state_tracking`, and `checked_rewrites`: 23 tests passed. These cover exact
  volatile access order, bank crossings, aliases, fixed/o65 execution, replay
  equality, each reached enabled pointer-leaf instruction under IRQ, and seeded
  IRQ/NMI schedules.
- The list oracle passes all 135 vectors in each compiler mode, in both I
  states: 270 paired-mask records / 540 executions per host. Debug and release
  host results are identical. Actual LF and CRLF builds are byte-identical.

Slice 2 keeps captured null tests in A16. Guards remain 540 bytes and optimized
bodies total 2,906 bytes. The size/cycle tradeoff is visible: Insert, RemHead
and RemTail save eight bytes each (−7 to +1 cycles per vector), Enqueue saves
six bytes (+8 to +16 cycles), and FindName saves twelve bytes (−7 to +30 cycles).
Other routine sizes and cycles are unchanged. Extra private stack reads range
from zero to thirteen per vector; DP traffic, stack writes, stack peaks and
reservations are unchanged.

Slice 2 [sizes](slice2/sizes.csv), [measurements](slice2/measurements.csv),
[deltas](slice2/delta.csv), [raw code](slice2/actionc-raw.lst),
[optimized code](slice2/actionc-optimized.lst), and
[provenance](slice2/provenance.json) retain that tradeoff. Qualification passed
212 native emitter unit tests (one opt-in inventory ignored), 22 emission and
11 o65 integration tests, and 28 qualified VM tests across pointer comparisons,
state tracking, checked rewrites, control flow and compare/branch selection.
The new VM trace check covers zero, all 24 individual pointer bits and all bits
set, proving private reads at exactly offsets 0, 1, 1, 2. Existing tests cover
Boolean/branch consumers, mutable parameters, aliases/calls, volatile captures,
same-target edges/backedges and relocated execution. The assembler/CPU probe
checks ORA-stack encoding and effects in both A widths. Both list oracle host
runs pass all 270 paired-mask records with identical results and LF/CRLF builds.
The disassembler and physical dispatch observer now recognize the new form.

Slice 3 uses overlapping word transfers for representation-preserving casts
between complete, identical/disjoint private homes. Optimized bodies total
2,850 bytes; guards remain 540. Per-vector cycles improve by up to 80 raw / 50
optimized, with no regressions. Private stack reads/writes increase by up to
twenty bytes per vector because the middle byte is repeated; DP traffic, stack
peaks, frames and reservations are unchanged. [Sizes](slice3/sizes.csv),
[measurements](slice3/measurements.csv), [deltas](slice3/delta.csv),
[raw code](slice3/actionc-raw.lst), [optimized code](slice3/actionc-optimized.lst)
and [provenance](slice3/provenance.json) record the complete results.

Qualification passed 214 emitter unit tests (one inventory ignored), 22 emission
and 11 o65 integration tests, and 14 qualified VM tests in `pointer_values`,
`memory`, `wide_returns` and `checked_rewrites`. Focused tests cover identity and
disjoint copies, stack/DP boundaries, A8/A16 entry, mutable parameters, partial
overlap fallback and atomic rejection. Runtime probes cover every pointer bit,
neighboring canaries, volatile/alias ordering, calls and relocated wide returns.
The list oracle passes all 270 paired-mask records in both host builds with
identical results and LF/CRLF artifacts.

Slice 4 forms captured zero-offset addresses directly in disjoint private
homes. Optimized bodies total 2,790 bytes; guards remain 540. Per-vector cycles
improve by up to 62 in each mode, with no regressions. DP reads drop by up to
six bytes and DP writes by up to eight; stack reads stay equal and stack writes
increase by at most two repeated private middle bytes. Frames, stack peaks and
reservations are unchanged. [Sizes](slice4/sizes.csv),
[measurements](slice4/measurements.csv), [deltas](slice4/delta.csv),
[raw code](slice4/actionc-raw.lst), [optimized code](slice4/actionc-optimized.lst)
and [provenance](slice4/provenance.json) retain the results.

Four focused pointer-value unit tests and 33 emission/o65 integration tests pass.
The qualified runner also passes 21 tests across `memory`, `checked_rewrites`,
`pointer_values` and `state_tracking`; the new address fixture was corrected to
use the language's required explicit pointer-to-ADDRESS cast and rerun with
state tracking. Coverage includes disjoint homes, unsupported addressing and
overlap fallback, incomplete extents, both entry widths, null/every pointer bit,
no dereference during address formation, canaries and fixed/o65 trace checks.
Both list oracle host runs pass all 270 paired-mask records, with equal results
and actual LF/CRLF build equality.

Slice 5 adds positive 16-bit offsets with an A16 low-word addition and A8 bank
carry, modulo 24 bits. Optimized bodies total 2,674 bytes; guards remain 540.
Per-vector cycles improve by up to 92 in both modes, with no regressions. Stack
reads fall by up to two bytes, DP reads by twelve and DP writes by fourteen;
stack writes, frames, peaks and reservations are unchanged. See
[sizes](slice5/sizes.csv), [measurements](slice5/measurements.csv),
[deltas](slice5/delta.csv), [raw code](slice5/actionc-raw.lst),
[optimized code](slice5/actionc-optimized.lst) and [provenance](slice5/provenance.json).

Qualification passed 217 emitter unit tests (one inventory ignored), 33 emission/
o65 integration tests and 23 qualified VM tests in `pointer_values`, `memory`,
`state_tracking` and `checked_rewrites`. Offsets 1, 3, 255, 256 and 65,535 cover
low-word carry and full 24-bit wrap; 65,536 exercises the fallback. Canaries and
both I states remain checked. A two-task probe injects IRQ at every reached
enabled instruction in address formation, runs the same computation in the IRQ
dispatcher and also injects NMI, checking complete results and domain guards.
Both list oracle host runs pass all 270 paired-mask records with matching
results and LF/CRLF artifacts.

Slice 6 copies a single captured three-byte edge argument with overlapping
native words. One private two-byte staging word preserves the original full A;
the final bank-byte load restores the bytewise fallback's exact A/N/Z state.
Identities omit both staging and copies but retain that state repair. Optimized
Enqueue and FindName each save twenty bytes; optimized bodies total 2,634 bytes,
with guards still 540. Per-vector cycles improve by up to 56, without regressions.
Stack reads increase by up to four bytes; writes, DP traffic, measured frames,
peaks and guard costs are unchanged. Other edges still require the existing
shared staging capacity in these routines. Raw output is unchanged. See
[sizes](slice6/sizes.csv), [measurements](slice6/measurements.csv),
[deltas](slice6/delta.csv), [raw code](slice6/actionc-raw.lst),
[optimized code](slice6/actionc-optimized.lst) and [provenance](slice6/provenance.json).

Qualification passed 220 emitter unit tests (one inventory ignored), 33 emission/
o65 integration tests and 20 qualified VM tests in `pointer_edges`,
`selective_staging`, `state_tracking` and `word_edges`. Focused checks cover
exact full-register state at backedges, identity repair, authoritative mutable
parameter homes, stack/DP geometry, staging bounds and atomic preflight failure.
The new loop executes across 24-bit wrap and both I states, including relocated
o65 placements. A two-task probe injects IRQ at every reached enabled loop
instruction, re-enters the same routine in the dispatcher and injects NMI,
checking results and domain guards. Both list oracle host runs pass all 270
paired-mask records with identical results and actual LF/CRLF build equality.

Slice 7 schedules complete three-byte assignments in dependency order, omits
identities and retains one A-save word whenever any real move remains. Cycles,
partial overlaps, constants and mixed widths keep the existing fallback. Only
optimized FindName changes: 511→477 bytes. Optimized bodies total 2,600 bytes;
guards remain 540. Per-vector cycles fall by up to 310, stack reads by 25 and
stack writes by 30. DP traffic, frames, stack peaks and guard costs are unchanged.
Raw output is unchanged. See [sizes](slice7/sizes.csv),
[measurements](slice7/measurements.csv), [deltas](slice7/delta.csv),
[raw code](slice7/actionc-raw.lst), [optimized code](slice7/actionc-optimized.lst)
and [provenance](slice7/provenance.json).

Qualification passed 224 emitter unit tests (one inventory ignored), 33 emission/
o65 integration tests and 21 qualified VM tests in `pointer_edges`, `word_edges`,
`checked_rewrites` and `state_tracking`. An exhaustive three-assignment oracle
covers permutations, repeated sources, identities and cycles; separate rejection
checks cover partial overlaps, late invalid homes and staging aliases before
emission. Independent ca65 sequences compare complete registers, flags and
non-staging memory with the bytewise fallback. Compiled multi-pointer backedges,
two-task IRQ/NMI re-entry and two o65 placements pass. Both list oracle host runs
pass all 270 paired-mask records with matching results and LF/CRLF artifacts.

Reproduce a slice from its commit by building and copying the CLI to a stable
path, then using the existing comparison builder:

```sh
cargo build --release --bin actionc-65816
mkdir -p target/pointer-micro/slice1
cp target/release/actionc-65816 target/pointer-micro/slice1/actionc-65816
python3 -B tools/compare65816/execlists.py \
  --actionc target/pointer-micro/slice1/actionc-65816 \
  --output target/pointer-micro/slice1/lists
python3 - <<'PY'
import json
from pathlib import Path
p = Path('target/pointer-micro/slice1/lists/manifest.json')
m = json.loads(p.read_text())
p.with_name('all-compilers-manifest.json').write_text(p.read_text())
m['artifacts'] = [a for a in m['artifacts'] if a['compiler'] == 'actionc']
p.write_text(json.dumps(m, indent=2) + '\n')
PY
A816_COMPARISON_MANIFEST="$PWD/target/pointer-micro/slice1/lists/manifest.json" \
  A816_COMPARISON_RESULTS="$PWD/target/pointer-micro/slice1/lists/debug.json" \
  CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 \
  python3 tools/native65816-runtime-tests/qualify.py \
  --test code_quality -- --ignored --nocapture \
  > target/pointer-micro/slice1/lists/debug.log 2>&1
A816_COMPARISON_MANIFEST="$PWD/target/pointer-micro/slice1/lists/manifest.json" \
  A816_COMPARISON_RESULTS="$PWD/target/pointer-micro/slice1/lists/release.json" \
  CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 \
  python3 tools/native65816-runtime-tests/qualify.py \
  --release --test code_quality -- --ignored --nocapture \
  > target/pointer-micro/slice1/lists/release.log 2>&1
python3 -B tools/compare65816/report_pointer_micro.py \
  --input target/pointer-micro/slice1/lists \
  --baseline docs/benchmarks/65816-execlists \
  --output docs/benchmarks/65816-pointer-micro/slice1
```

For subsequent slices, use a fresh output directory and the preceding slice's
committed report as `--baseline`. Keep all qualification inputs stable during
each run. The reporter requires complete, successful Action records, identical
host results and verified input/artifact hashes; it does not suppress failures.
