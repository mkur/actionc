# Call flow: implementation baseline

Stage 0 is qualified. Compiler behavior remains the post-stage-7 baseline
`af7c504c`; the design/tooling commit is `ce9b1a46`. Frozen Exec816 input is
`57df0d7`, with 256 files and 1,156 routines. The original audit's four artifacts
per profile reproduce byte for byte. The new typed flow artifact adds observation.
See [provenance](provenance.json) for compiler, fixture, tool and binary hashes.

| Profile | Complete code bytes | Native cycles | Private accesses | Native peak |
| --- | ---: | ---: | ---: | ---: |
| Optimized release | 430,091 | 71,833 | 20,655 | 33 |
| Optimized guarded | 579,765 | 76,021 | 20,655 | 33 |
| Raw guarded | 648,963 | 87,539 | 24,288 | 45 |

Each profile executes 147 native vectors. All values, arguments and exact external
access traces match stage 7; cycles, private traffic and peaks are identical.
The independent call-flow fixture adds 160 executions across widths 1–4,
raw/optimized, guards and interrupt masks, checking assembly-defined argument
bytes/padding, poisoned unspecified flags/registers, canaries, result lanes and
native state. Actual LF and CRLF inputs produce identical images. Qualification
manifest hashes and vector comparisons are in [native results](native-results.json).
The 19 focused audit/candidate/measurement Python tests pass.

Host cost uses Rust 1.99, nonincremental debug builds with optimization level 3
and no debug information. One warm-up and three serial measured compiles per
profile retain all samples and per-child wait4 RSS in [host results](host-results.json).
Every compile reproduces the pinned collector image. These measurements establish
a consistent comparison baseline; this stage claims no compiler improvement.

The [implementation plan](../../MIR65816_CALL_FLOW_IMPLEMENTATION_PLAN.md) governs
acceptance. Preserve stage-7 targets: release size ≤422,700 bytes, representative
size ≤9,235, private accesses ≤20,565, cycles ≤78,117, peak ≤42, no per-routine
frame/spill/local-peak growth and no added DP. Size/private and unchecked hosted
provider gates remain open. Host medians permit at most 5% time and 10% RSS growth.
The 256 KiB application objective remains open.

## Reproduction

Keep frozen input and baseline generations immutable. The candidate path permits
changed output and authenticates its own artifacts; the original audit collectors
continue requiring frozen image identity. Later reports explicitly name this
stage-0 generation as their reference.

```sh
python3 -B tools/compare65816/exec_call_candidate.py collect \
  --output target/call-flow-stage0 --stage 0
python3 -B tools/compare65816/exec_call_candidate.py report \
  --output target/call-flow-stage0 \
  --destination docs/benchmarks/65816-call-flow-stage0 --check
python3 -B -m unittest discover -s tools/compare65816 -p 'test_exec_call_*.py'
python3 -B tools/native65816-runtime-tests/qualify.py --test call_flow -j2
CARGO_INCREMENTAL=0 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_DEV_OPT_LEVEL=3 \
  cargo build --locked --bin actionc-65816 \
  --target-dir target/exec-call-audit/rust-target -j2
python3 -B tools/compare65816/exec_record_vectors.py \
  --base target/record-placement-stage0 \
  --output target/call-flow-stage0/native-vectors \
  --binary target/exec-call-audit/rust-target/debug/actionc-65816
# For each of optimized-release, optimized-guarded and raw-guarded:
A816_COMPARISON_MANIFEST="$PWD/target/call-flow-stage0/native-vectors/optimized-release.manifest.json" \
A816_COMPARISON_RESULTS="$PWD/target/call-flow-stage0/native-vectors/optimized-release.results.json" \
  python3 -B tools/native65816-runtime-tests/qualify.py --test code_quality -j2 -- --ignored
# Stop build and qualification jobs before measuring.
python3 -B tools/compare65816/exec_call_measure.py \
  --output target/call-flow-stage0 \
  --binary target/exec-call-audit/rust-target/debug/actionc-65816 --rounds 3
```

Collection requires a fresh destination. Use a distinct generation directory
when reproducing rather than replacing the original evidence. Bulky immutable
binaries/images and qualification logs remain under ignored target directories;
committed JSON and compressed tables preserve compact evidence.
