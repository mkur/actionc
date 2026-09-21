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
