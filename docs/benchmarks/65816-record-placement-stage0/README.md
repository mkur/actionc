# Exec816 record/value placement: stage 0

Baseline capture is complete. The unchecked hosted release gate remains open;
its provider contract rejects an unguarded kernel. This is measured input for
[stages 1 and 2](../../MIR65816_RECORD_VALUE_PLACEMENT_PLAN.md), with explicit
hosted qualification gaps below. No compiler strategy, ABI or reservation changed.

## Frozen workload

- Compiler: `d7d536c928d343932043ba3f1aea5deb107dd41d`.
- Exec816: `57df0d7850031fe7964bfef1e515e4d429e72dff`, clean at capture;
  256 compiler source files and 1,156 emitted routines. This is the current
  text-console shell/prime demo with 16 disk commands, SDFS SYS, writable WORK,
  eight public task slots, a 4 KiB global-data arena and OF816 boot.
- Frozen compiler/Exec worktrees, generated Action source, layouts, ABI consumers,
  assembly, linked bytes, listings and packages live under
  `target/record-placement-stage0/`. The live Exec checkout was not edited.
- OF816 upstream `8c92362` and pinned ROM, three Altirra bridges/SDKs, independent
  SDFS producer/reader sources, headers and libraries were copied or checked out
  separately. Input hashes, tool versions and host executable hashes are in
  [inputs.json.gz](inputs.json.gz).
- The frozen Exec compiler pin was deliberately changed from `f1ff4ce0` to the
  measured compiler revision. The original and local override are both recorded.
- Host compiler: optimized Cargo **dev** profile, `opt-level=3`, `debug=0`,
  incremental compilation disabled. Target optimization and guards are separate
  controls. This is not a claim that a Cargo release binary was used.

The older Exec `154bcf5a` reports and the original `COOKEDLINE.Recall` listing
represent different workloads and compiler versions. They are not comparison
baselines for these numbers.

## Image scorecard

| Profile | Compiler code | Native guards within code | Compiler initialized data | Hosted upper image code + initialized data |
| --- | ---: | ---: | ---: | ---: |
| Optimized release, guards disabled | 444,948 | 0 | 2,471 | Not packaged: checked-provider requirement |
| Optimized guarded | 594,828 | 147,771 | 2,470 | 615,099 |
| Raw guarded | 661,861 | 148,203 | 2,470 | 682,132 |

The compiler contributes another 277 zero-fill bytes in every profile. Guarded
packages add 12,570 assembly code bytes and 5,231 initialized platform bytes to
the compiler image. Upper-image totals exclude bank-zero loader/resident code,
OF816 boot storage, disk command bodies loaded on demand, and OS RAM. They are
components of whole loaded footprint, not complete RAM or distribution sizes.
The measured release compiler contribution alone is **447,419 bytes**, exceeding
the 262,144-byte application objective by **185,275 bytes** before platform code
is counted. No unchecked hosted total is inferred from guarded code.

The guarded program XEX files are 632,493 optimized and 700,712 raw bytes;
OF816 XEX files are 657,396 and 725,615 bytes. XEX framing/staging and boot bytes
are accounted separately from executable code. Disk commands use the current
experimental o65 checked-provider contract in every profile; raw commands are
compiled without source optimization.

All compiler bytes are covered by disassembly and nonoverlapping MIR spans plus
prologue/helper bytes. Recompilation matches every compiler-owned segment,
routine, map and import in the packaged image. Assembly/platform additions are
accounted separately. [results.json](results.json), [modules.csv](modules.csv)
and [routines.csv](routines.csv) contain the full inventory.

Release code contains 136,206 bytes of stack-relative instructions (30.6%),
37,596 bytes of direct-page instructions (8.4%), and 57,622 bytes of mode changes
(13.0%). MIR spans account for 117,389 load bytes, 60,921 store bytes and 117,051
call bytes. These are recurring cost categories with disjoint accounting within
each view; opcode categories and MIR families must not be added together.
They establish scope for coordinated placement work, rather than predict savings.

## Representative routines and native measurements

The fixed representative set contains 11 routines spanning lists, task/port
management, DOS streams, cooked input and SDFS state. It includes branches,
joins, loops and calls; its release code totals **10,262 bytes**. Exact routine
names, sizes, frames, spills, call counts and block counts are in
[representatives.csv](representatives.csv).

`COOKEDLINE.Recall` is 943 release bytes, 1,085 guarded optimized bytes, and
1,060 raw guarded bytes. The optimized routine retains a 24-byte frame, 20 spill
bytes and a 36-byte local peak. The full optimized application has a largest
fixed frame of 70 bytes and largest local peak of 90 bytes. Local peaks exclude
callee chains and asynchronous context costs.

The native corpus compiles the **actual frozen ExecLists source**, then adds an
independently authored record layout with CARD alignment padding, an embedded
array, a branch/merge, loop, mutable fields and direct/indirect calls. Independent
host oracles check complete records, pointer metadata, canaries and results at
bank-zero, far and bank-crossing placements. There are 135 list vectors and 12
mixed record vectors. Each runs with I clear and set; no IRQ/NMI is injected in
this native measurement. All **441 vector/profile pairs**, or 882 executions,
pass the memory/result and ABI checks.

| Profile | Corpus cycles | Stack byte reads + writes | DP scratch byte reads + writes | Maximum depth below entry S |
| --- | ---: | ---: | ---: | ---: |
| Optimized release | 86,797 | 15,692 | 10,015 | 42 |
| Optimized guarded | 91,417 | 15,692 | 10,015 | 42 |
| Raw guarded | 90,155 | 14,339 | 10,477 | 48 |

Cycles and traffic cover the entry routine through its return, including callees
and selected guards. Stack traffic includes incoming arguments and return/transfer
staging; it is not a count of spills alone. DP traffic counts native-v2 compiler
scratch at domain offsets `$80..$BF`; domain metadata reads are separate. Stage 0
corrects the comparison harness's previous `$00..$3F` scratch range. Historical
DP totals should be remeasured. [native-metrics.csv](native-metrics.csv) contains
per-vector measurements. Corpus totals give each vector equal weight; they are
not application frequency or hot-path estimates. Raw code's lower traffic and
cycles in this particular guarded corpus make a cycle/traffic gate necessary
alongside the whole-application code-size gate.

The corpus rebuilt from actual LF and CRLF source copies produces identical
compiler images in all three profiles. Measurement control tests reject changed
inputs, unknown encodings, incomplete guards, overlapping spans and altered
compiler projections.

## Hosted checks and reservations

Focused development scope only; this is not full Exec or backend qualification.

| Workload | Optimized guarded | Raw guarded |
| --- | --- | --- |
| Port queues/protocol, ownership and native IRQ/NMI | Pass | Fail: native interrupt/COP vectors not restored |
| Cooked editing/history independent model | Pass | Pass |
| Packaged shell, path commands and CAT/WC pipeline | Pass | Pass |
| OF816 five-second autoboot, guards and OS restoration | Pass | Pass |
| Existing instrumented far-list fixture | Build failure: bank-zero resident payload | Same failure |
| Standalone DOS stream defaults/routing fixtures | Build failure: missing generated console include | Same failure |

The DOS fixtures fail while loading
`lib/console/console-storage-action.inc`; they provide no runtime observation.
Packaged demo pipelines exercise the working hosted DOS/filesystem composition.
The raw port failure reproduced in two runs; its cause is not assigned to the
compiler or runtime. The unchecked demo attempt fails with
`Hosted o65 requires a checked kernel` after emitting its compiler image.
No failed or unsupported workload is reported as qualified.

Cooked tests observe root stack watermarks of 136 optimized and 154 raw bytes,
and kernel watermarks of 270 and 280 bytes, with no interrupt-reserve touch.
Watermarks are observed touched-byte bounds, not proven maximum depths. Both
runs restore native state and leave domain/stack guards intact. The port pass
observes native interrupts and 17 switches; hosted event counts are observations,
not isolated target cycle benchmarks.

The frozen eight-slot demo reserves 25,408 bank-zero runtime bytes excluding OS,
including 10,528 fixed runtime bytes. Its public stack sizes are 1,536; five times
1,024; and twice 2,560 bytes; idle reserves 512 and the kernel 1,536 bytes. Each
public/idle domain reserves 256 DP bytes, with a 256-byte interrupt reserve in
each task stack. The build's detailed `bank_zero_budget`, task pools, loading and
runtime phase reservations are retained in the results. Baseline compiler-work
bank-zero delta is **0**.

## Host cost and numerical gates

Five warm measured rounds per profile, after one warm-up each, reproduce the
saved compiler image exactly. Medians use per-child `wait4` accounting:

| Profile | CLI wall seconds | Peak RSS (MiB) |
| --- | ---: | ---: |
| optimized-release | 55.35 | 866.0 |
| optimized-guarded | 40.82 | 1111.2 |
| raw-guarded | 46.10 | 1029.6 |

Timing covers the compiler CLI only, excluding interface generation, assembly,
media and packaging. Source/tool/binary hashes and all 15 samples are recorded.
Other development work ran on the same macOS host; use paired, interleaved
candidate runs for overhead decisions rather than compare unrelated runs.

These are proposed acceptance gates for this plan, calibrated against the frozen
costs above, not estimates of implementation savings:

- **Foundation stages 1–2:** unchanged compiler image bytes, frame/home maps and
  runtime measurements on fixed generated inputs; no bank-zero growth. Allow at
  most 5% warm CLI wall-time and 10% per-child peak-RSS growth when comparing
  candidates interleaved on the same host. The shared-host baseline is not a
  noise-free microbenchmark.
- **First benefit tranche, stages 0–3:** at least 2% code reduction in the fixed
  representative set, a measurable independent-probe benefit and benefits in
  at least two Exec subsystems; no whole-Exec code regression.
- **Final plan:** release compiler code at most **422,700 bytes** (5% reduction),
  representative code at most **9,235 bytes** (10% reduction), and release native
  corpus private accesses at most **20,565** (20% reduction) with cycles at most
  **78,117** (10% reduction). Benefits must span at least three Exec subsystems,
  including routines with loops and calls; no individual measured vector may
  regress by more than 5% in cycles.
- Preserve every required frame/stack/domain obligation; no routine frame or
  local-peak growth, optimized native peak at most 42 bytes, and zero added
  bank-zero bytes. Retain passing hosted gates and add no new failures. Existing
  failures need explicit dispositions before final hosted qualification; the
  unchecked package must eventually build and execute under a valid provider
  contract before it can satisfy the release gate.

These targets deliberately leave the broader 256 KiB application objective open.
The joint record/placement work must earn its savings on this workload without
claiming savings from other roadmap directions.

## Reproduction

Prerequisites: Rust, Python 3.12+, git, ca65/ld65, C++, the frozen Exec revision and
its pinned ROM/Altirra bridges/SDKs, SDFS reference sources/libraries and OF816
upstream. The input manifest identifies their exact bytes. Compiler source must
be at the recorded revision for a baseline reproduction; a later comparison
must label its candidate separately and reuse the frozen generated workload.

```sh
python3 -B tools/compare65816/freeze_exec_baseline.py \
  --compiler-checkout /path/to/actionc-at-d7d536c9 \
  --exec-checkout /path/to/exec816-at-57df0d7 --base target/record-placement-stage0
for profile in optimized-release optimized-guarded raw-guarded; do
  python3 -B tools/compare65816/exec_record_baseline.py build \
    --base target/record-placement-stage0 --profile "$profile"
done
for profile in optimized-release optimized-guarded raw-guarded; do
  python3 -B tools/compare65816/exec_record_baseline.py probe \
    --base target/record-placement-stage0 --profile "$profile"
done
```

The probe runner builds the checked measurement source against the frozen
compiler dependency, then rejects mismatched emitted bytes/maps. Reproduction
uses the fixed generated source and explicit optimization/guard booleans.

```sh
python3 -B tools/compare65816/exec_record_vectors.py --base target/record-placement-stage0
# Run separately for each of the three PROFILE names above:
A816_COMPARISON_MANIFEST="$PWD/target/record-placement-stage0/native-vectors/PROFILE.manifest.json" \
A816_COMPARISON_RESULTS="$PWD/target/record-placement-stage0/native-vectors/PROFILE.measurements.json" \
  python3 -B tools/native65816-runtime-tests/qualify.py --test code_quality -- --ignored
# Focused hosted cases: lists, ports, dos-streams, dos-routing, cooked, demo, of816;
# run each with --mode opt and --mode raw. Known failing scopes remain failures.
python3 -B tools/compare65816/exec_record_hosted.py \
  --base target/record-placement-stage0 --case cooked --mode opt
python3 -B tools/compare65816/exec_record_baseline.py host \
  --base target/record-placement-stage0 --rounds 5
python3 -B tools/compare65816/exec_record_report.py \
  --base target/record-placement-stage0 --output docs/benchmarks/65816-record-placement-stage0
```

Run the focused Python controls `test_exec_record_baseline.py`,
`test_freeze_exec_baseline.py` and `test_measure_host.py` with unittest discovery
under `tools/compare65816`. Rust probe formatting/build, three native corpus
runs, LF/CRLF artifact comparisons, source integrity checks and the hosted scopes
above are the local validation for this tooling/test-only stage. Compiler/NIR
contract tests are unchanged and were not run as unrelated coverage.
