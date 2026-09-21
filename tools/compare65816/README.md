# actionc / vbcc native 65816 comparison

The [analysis and findings](../../docs/MIR65816_VBCC_COMPARISON.md) accompany
14 equivalent Action!/C kernels, 66 independently calculated input vectors,
raw/optimized machine code, and execution on the existing independent native VM.
This corpus does not change either compiler or the public Action ABI.

The checked-in sources live in
[`tests/fixtures/code_quality`](../native65816-runtime-tests/tests/fixtures/code_quality).
Generate them with `python3 tools/compare65816/corpus.py`; use `--check` to verify
them. C compile-time assertions check integer, pointer, and record sizes.

From the repository root, with Rust, Python 3.12+, vbcc65816, vasm6502_oldstyle,
and vlink installed:

```sh
cargo build --release --bin actionc-65816
python3 tools/compare65816/corpus.py --check
python3 tools/compare65816/build.py --verify-crlf
```

`build.py` defaults to `~/atari/vbcc/bin`; override `--vbcc-bin` if needed.
`--actionc` and `--output` override the compiler and artifact paths. A missing
tool, unknown emitted stack-check sequence, unresolved reference, or different
LF/CRLF output fails the build. The defaults emit to `target/code-quality-65816`:

- Action: serialized image and disassembled final bytes in `code.asm`.
- C: compiler assembly, vasm object/listing, vlink map/raw binary, and
  `code.linked.lst`, combining assembler instruction boundaries with the actual
  relocated binary bytes. Coverage must account for every linked byte.
- Manifest: exact commands, tool hashes/banners, source hashes, entry points,
  counted code ranges, physical arguments, and recognized stack checks.

Execute **both commands**, even if the first reports a compiler correctness
failure. Each run saves all completed measurements before reporting wrong
outputs. Infrastructure errors, illegal accesses, ABI violations, or exhausted
execution budgets abort the run.

```sh
A816_COMPARISON_MANIFEST="$PWD/target/code-quality-65816/manifest.json" \
  A816_COMPARISON_RESULTS="$PWD/target/code-quality-65816/debug.json" \
  CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 \
  python3 tools/native65816-runtime-tests/qualify.py \
  --test code_quality -- --ignored --nocapture

A816_COMPARISON_MANIFEST="$PWD/target/code-quality-65816/manifest.json" \
  A816_COMPARISON_RESULTS="$PWD/target/code-quality-65816/release.json" \
  python3 tools/native65816-runtime-tests/qualify.py \
  --release --test code_quality -- --ignored --nocapture

python3 tools/compare65816/report.py
```

Use the qualification runner, not bare `cargo test`; it prepares the pinned VM
with the committed CPU timing correction. The comparison test is ignored by
default because it requires external compiler artifacts. `debug` and `release`
refer to the **host VM harness**, independently of each compiler's raw/optimized
target code.

The original 13-kernel 2026-09-21 baseline reports a failing comparison test in both
host modes: optimized vbcc unlink corrupts an adjacent pointer field. This is a
compiler result, not an expected-success exemption in the test. All 240 records
(480 executions, including both interrupt-mask states) are saved. `report.py`
requires matching debug/release results and the same build manifest, verifies
artifact hashes, and retains incorrect results explicitly. Its success means a
complete report was generated, not that all compiler outputs were correct.

The report defaults to `target/code-quality-65816/snapshot`. The committed
snapshot was written using `--output docs/benchmarks/65816-vbcc`; it contains all
vector measurements in CSV, representative tables, provenance, and raw/optimized
final listings for add, sum-loop, and unlink. Refresh it only when deliberately
recording a new comparison, not as an assembly golden test.

The [word-arithmetic baseline](../../docs/benchmarks/65816-word-arithmetic/before/tables.md)
adds subtraction: 264 paired-mask records, or 528 executions per host build.
The original snapshot remains unchanged. New snapshots also retain subtraction
listings alongside add, sum-loop, and unlink.

The [word-arithmetic results](../../docs/MIR65816_WORD_ARITHMETIC.md) include a
separate [post-change snapshot](../../docs/benchmarks/65816-word-arithmetic/after/tables.md)
and [before/after table](../../docs/benchmarks/65816-word-arithmetic/delta.md).
Given both saved build directories, regenerate the comparison with:

```sh
python3 tools/compare65816/delta.py \
  target/word-arithmetic-before target/word-arithmetic-after \
  --output docs/benchmarks/65816-word-arithmetic
```

The script requires identical debug/release measurements, matching cases and
external tools, unchanged vbcc results, no Action correctness/size/cycle
regression, and unchanged ABI/storage maps, stack traffic, peaks, and guard
costs. Build the baseline with its historical compiler when reproducing from
scratch (`7907b77`, supplied to `build.py --actionc`); the current compiler is
the post-change side. Each directory needs its own manifest and both execution
reports before running `delta.py`. The known vbcc unlink failure remains visible.

The [native word return results](../../docs/MIR65816_WORD_RETURNS.md) reuse the
word-arithmetic `after` snapshot as their immutable baseline. New snapshots also
retain identity listings. Reproduce the delta from the saved build directories:

```sh
python3 tools/compare65816/delta.py \
  target/word-arithmetic-after target/word-returns-after \
  --output docs/benchmarks/65816-word-returns \
  --title 'Native word returns: before / after'
```

When reconstructing historical builds, run their compiler and build runner from
an isolated historical checkout (`257e5f9` for this baseline). The manifest
records the runner's checkout revision as well as the supplied binary hash;
`--actionc` alone does not update that revision.

The [native word comparison results](../../docs/MIR65816_WORD_COMPARISONS.md)
use the return `after` snapshot as their immutable baseline. New snapshots also
retain maximum listings. After rebuilding and running both host modes in
`target/word-comparisons-after`, reproduce the report and delta with:

```sh
python3 tools/compare65816/report.py \
  --input target/word-comparisons-after \
  --output docs/benchmarks/65816-word-comparisons/after
python3 tools/compare65816/delta.py \
  target/word-returns-after target/word-comparisons-after \
  --output docs/benchmarks/65816-word-comparisons \
  --title 'Native word comparisons: before / after' \
  --stack-read-deltas docs/benchmarks/65816-word-comparisons/stack-read-deltas.json
python3 -m unittest discover -s tools/compare65816 -p 'test_delta.py'
```

`--stack-read-deltas` accepts an explicit JSON list keyed by
case/mode/compiler/vector, with an exact positive `delta`. The committed four
+2 entries were predicted before implementation: native CMP reads both private
words where bytewise maximum stopped after unequal high bytes. Missing/unused
keys, other read differences, and all other contract changes still fail; vbcc
records must remain identical. Without the option, every stack-read count must
match. Reconstruct this baseline from the isolated `943ed21` checkout if its
saved artifacts are unavailable. Historical snapshots are never rewritten.

The [compare-to-branch fusion results](../../docs/MIR65816_COMPARE_BRANCH_FUSION.md)
use the word-comparison `after` snapshot as their immutable baseline. Reproduce:

```sh
cargo build --release --bin actionc-65816
python3 tools/compare65816/build.py --output target/compare-branch-after --verify-crlf
# Execute both host comparison commands above, using compare-branch-after paths.
python3 tools/compare65816/report.py \
  --input target/compare-branch-after \
  --output docs/benchmarks/65816-compare-branch/after
python3 tools/compare65816/delta.py \
  target/word-comparisons-after target/compare-branch-after \
  --output docs/benchmarks/65816-compare-branch \
  --title 'Native compare-to-branch fusion: before / after' \
  --fused-branch-counts docs/benchmarks/65816-compare-branch/fused-branch-counts.json
```

`--fused-branch-counts` accepts positive integer `count` entries keyed by
case/mode/compiler/vector. Each count must match reached, decoded fused machine
sequences and exactly that many fewer stack byte reads **and** writes. Unlisted
records retain equality. Invalid, duplicate, missing/unused or external-compiler
entries fail. DP traffic and every other invariant remain strict; this option
is mutually exclusive with `--stack-read-deltas`. The strict default and the
older positive-only read accounting retain their original behavior. Action
measurements include `fused_branches` and per-PC `fused_branch_sites`; these
fields are absent from vbcc records, which remain identical to baseline.
Reconstruct the baseline in an isolated `4d54b81` checkout if saved artifacts
are unavailable. Both host runs retain the known optimized vbcc unlink failure.

The [native word edge-copy results](../../docs/MIR65816_WORD_EDGE_COPIES.md)
use the fusion `after` snapshot as their immutable baseline. Reproduce:

```sh
cargo build --release --bin actionc-65816
python3 tools/compare65816/build.py --output target/word-edges-after --verify-crlf
# Execute both host comparison commands above, using word-edges-after paths.
python3 tools/compare65816/report.py \
  --input target/word-edges-after --output docs/benchmarks/65816-word-edges/after
python3 tools/compare65816/delta.py target/compare-branch-after target/word-edges-after \
  --output docs/benchmarks/65816-word-edges --title 'Native word edge copies: before / after'
python3 tools/compare65816/check_word_edges.py target/compare-branch-after target/word-edges-after \
  --counts docs/benchmarks/65816-word-edges/expected-edges.json \
  --output docs/benchmarks/65816-word-edges/coverage.json
```

Use the **strict default** delta: stack reads and writes must match exactly.
Do not reuse earlier fusion/read exceptions. The additional check compares
predeclared counts to decoded `word_edges`, `edge_words` and `word_edge_sites`,
checks unchanged DP/fusion traffic and storage contracts, verifies raw/unaffected
emitted-file hashes and enforces the three size/cycle ceilings. New fields exist
only on Action records; vbcc records remain identical. Reconstruct missing
baseline artifacts using the compiler and runner from isolated `01ea393`.
The new snapshot also retains rotation and byte-sum listings. Both host commands
must run and preserve the known optimized vbcc unlink failure.

The [empty-edge cleanup results](../../docs/MIR65816_EMPTY_EDGES.md) use the
word-edge `after` snapshot as their immutable baseline. Reproduce:

```sh
cargo build --release --bin actionc-65816
python3 tools/compare65816/build.py --output target/empty-edges-after --verify-crlf
# Execute both host comparison commands above, using empty-edges-after paths.
python3 tools/compare65816/report.py \
  --input target/empty-edges-after --output docs/benchmarks/65816-empty-edges/after
python3 tools/compare65816/delta.py target/word-edges-after target/empty-edges-after \
  --output docs/benchmarks/65816-empty-edges --title 'Native empty edges: before / after'
python3 tools/compare65816/check_empty_edges.py target/word-edges-after target/empty-edges-after \
  --baseline docs/benchmarks/65816-empty-edges/baseline.json \
  --output docs/benchmarks/65816-empty-edges/coverage.json
```

Use strict default traffic accounting. The additional check proves that every
Action instruction stream differs only by removed empty-edge SEP/REP instructions
and relocated JSL/JML targets. It requires exactly three saved cycles per removed
executed instruction, unchanged DP/fusion/word-copy counts and storage contracts,
identical unaffected emitted files and exact representative forecasts. Both host
commands must run and retain the known optimized vbcc unlink failure. Reconstruct
missing baseline artifacts from an isolated `932a0cf` checkout.


The [direct single-word edge results](../../docs/MIR65816_SINGLE_WORD_EDGE_COPIES.md)
use the empty-edge snapshot as baseline. Reproduce with:

```sh
cargo build --release --bin actionc-65816
python3 tools/compare65816/build.py --output target/single-word-edges-after --verify-crlf
# Execute both host comparison commands above, using single-word-edges-after paths.
python3 tools/compare65816/report.py \
  --input target/single-word-edges-after --output docs/benchmarks/65816-single-word-edges/after
python3 tools/compare65816/delta.py target/empty-edges-after target/single-word-edges-after \
  --output docs/benchmarks/65816-single-word-edges --title 'Native direct single-word edges: before / after' \
  --direct-word-edge-counts docs/benchmarks/65816-single-word-edges/expected-copies.json
python3 tools/compare65816/check_single_word_edges.py target/empty-edges-after target/single-word-edges-after \
  --counts docs/benchmarks/65816-single-word-edges/expected-copies.json \
  --baseline docs/benchmarks/65816-single-word-edges/baseline.json \
  --output docs/benchmarks/65816-single-word-edges/coverage.json
```

The dedicated accounting option is mutually exclusive with existing read/fusion
exceptions. Each predeclared direct copy must remove exactly two instructions,
ten cycles and two stack byte reads/writes, with unchanged DP/fusion/word totals
and complete storage/guard contracts. Unlisted records remain strict. New metrics
`direct_word_edges` and `direct_word_edge_sites` appear only on Action records.
The runner rebuilds each Action artifact from its recorded source/mode/layout and
requires identical serialized image bytes before using typed edge-site evidence;
CPU execution still consumes the saved artifact.

The focused checker proves that only selected staging STA/LDA pairs and relocated
JSL/JML addresses changed. Unaffected Action artifacts and vbcc code/records are
identical; vasm's listing source-header path changes with the build directory and
is compared after replacing that exact header. Both host commands retain the
known vbcc optimized unlink failure. Reconstruct missing baseline artifacts using
an isolated `192b6d8` checkout and its runner.

The [local accumulator-forwarding results](../../docs/MIR65816_LOCAL_ACCUMULATOR_FORWARDING.md)
use the direct-edge `after` snapshot as their immutable baseline. Reproduce:

```sh
cargo build --release --bin actionc-65816
python3 tools/compare65816/build.py --output target/local-accumulator-forwarding-after --verify-crlf
# Execute both host comparison commands above, using local-accumulator-forwarding-after paths.
python3 tools/compare65816/report.py \
  --input target/local-accumulator-forwarding-after \
  --output docs/benchmarks/65816-local-accumulator-forwarding/after
python3 tools/compare65816/delta.py \
  target/single-word-edges-after target/local-accumulator-forwarding-after \
  --output docs/benchmarks/65816-local-accumulator-forwarding \
  --title 'Native local accumulator forwarding: before / after' \
  --forwarded-word-load-counts docs/benchmarks/65816-local-accumulator-forwarding/expected-reloads.json
python3 tools/compare65816/check_accumulator_forwarding.py \
  target/single-word-edges-after target/local-accumulator-forwarding-after \
  --counts docs/benchmarks/65816-local-accumulator-forwarding/expected-reloads.json \
  --sites docs/benchmarks/65816-local-accumulator-forwarding/expected-sites.json \
  --baseline docs/benchmarks/65816-local-accumulator-forwarding/baseline.json \
  --output docs/benchmarks/65816-local-accumulator-forwarding/coverage.json
python3 -m unittest discover -s tools/compare65816 -p 'test_*.py'
```

The mutually exclusive `--forwarded-word-load-counts` mode requires exactly one
instruction, five cycles and two private stack-byte reads removed per predicted
execution. All stores, DP accesses, frames and guards remain equal. Action-only
`forwarded_word_loads` and `forwarded_word_load_sites` count the first reached
consumer, authenticated by typed identities, retained stores and final bytes.
Fusion and edge counts remain checked despite moved PCs.

The instruction checker requires only the 76 declared LDA removals and required
JSL/JML fixups in all 28 Action streams. Every selected site must be reached;
unselected builds and vbcc artifacts remain identical, apart from the exact
vasm source-header path. All 112 positive vector counts and representative
forecasts must match. Both host commands retain the known optimized vbcc unlink
failure. Reconstruct missing baseline artifacts with compiler and runner from
an isolated `0e8248c` checkout; preserve historical snapshots.
