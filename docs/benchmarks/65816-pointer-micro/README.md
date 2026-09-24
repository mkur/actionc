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
