# Native 65816 record/value placement: stage 7

Stage 7 qualifies the integrated compiler against the frozen stage-0 workload.
The numerical final gates remain unchanged. Qualification infrastructure and
measured benefits do not by themselves close the size, traffic or unchecked
hosted-release acceptance gaps.

## Compiler and independent native results

The frozen inputs remain compiler `d7d536c9` and Exec `57df0d7`: 256 source files
and 1,156 routines. Candidate compiler `01cfabd5` integrates stages 1–6. The
rebuilt baseline reproduces its published compiler-image hashes and native
measurement census exactly. Candidate output reproduces stage 6; stage 7 adds
consumer integration and qualification without changing emitted code.

| Profile | Baseline code | Candidate code | Saved | Baseline cycles | Candidate cycles | Baseline private accesses | Candidate private accesses | Native peak |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| Optimized release | 444,948 | 430,091 | 14,857 (3.34%) | 86,797 | 71,833 | 25,707 | 20,655 | 33 |
| Optimized guarded | 594,828 | 579,765 | 15,063 (2.53%) | 91,417 | 76,021 | 25,707 | 20,655 | 33 |
| Raw guarded | 661,861 | 648,963 | 12,898 (1.95%) | 90,155 | 87,539 | 24,816 | 24,288 | 45 |

Native measurements cover 147 independently specified vectors per profile,
including actual frozen ExecLists and a separate mixed record-flow probe. Each
runs with both incoming interrupt masks. Ordered external accesses, results and
ABI state match the baseline. Individual measured vectors stay within 5% cycle
growth and do not increase stack depth; the largest measured cycle ratio is
1.0 in every profile, with no slower vector. Private accesses count stack and domain
DP byte traffic. These are fixed-corpus totals, not whole-Exec runtime estimates.

The release representatives total 9,711 bytes, down from 10,262. Benefits occur
in `COOKEDLINE`, `EXECLISTS`, `SDFSFILE` and `TASKPOLICY`. Placement inventory
identifies 54 improved routines with loop homes and one with call segments in
each optimized profile. Raw mode retains conservative call fallback. The frozen
Exec MIR contains no aggregate copies; aggregate correctness retains its separate
independent native qualification.

Every routine preserves or reduces its frame, spill extent and local peak.
Compiler data identities, addresses and sizes remain unchanged. Small code
growth remains in eleven release routines, four guarded optimized routines and
two raw routines; [routine-changes.csv](routine-changes.csv) records all deltas.
The largest release increase is eight bytes in `CONSOLEDISPLAY.Cells` (0.42%).
These bounded transfer costs are included in the net code totals. The vector
cycle gate covers executed vectors; it is not a cycle guarantee for every routine.

## Frozen final acceptance

| Gate | Required | Measured | Result |
| --- | ---: | ---: | --- |
| Release compiler code | ≤422,700 bytes | 430,091 | Open: 7,391 bytes |
| Representative release code | ≤9,235 bytes | 9,711 | Open: 476 bytes |
| Native release private accesses | ≤20,565 | 20,655 | Open: 90 accesses |
| Native release cycles | ≤78,117 | 71,833 | Pass |
| Optimized native peak | ≤42 bytes | 33 | Pass |
| Benefited representative subsystems | ≥3, including loops and calls | 4; loop/call benefits measured | Pass |
| Routine frame/spill/local peak growth | 0 | 0 | Pass |
| Added compiler bank-zero reservation | 0 | 0 | Pass |
| Actual unchecked hosted package | Build and execute under valid provider contract | Unsupported checked-provider combination | Open |

The release compiler contributes another 2,471 initialized data bytes and 277
zero-fill bytes. Its code plus initialized data is 432,562 bytes, already above
the separate 256 KiB application objective before linked platform assembly.
This report does not infer an unchecked hosted footprint from a guarded package.

## Hosted artifact and consumer contracts

The measured CLI is selected before importing frozen build and fixture helpers.
Build recipes bind its binary hash, compiler/runtime/tool inputs, generated
layouts, packages and command artifacts. Hosted results bind final executed
files and pinned emulator/ROM observations. The live Exec checkout is not edited.

| Guarded package | Baseline upper code + initialized data | Candidate upper code + initialized data | Candidate program XEX | Candidate OF816 XEX |
| --- | ---: | ---: | ---: | ---: |
| Optimized | 615,099 | 600,036 | 617,160 | 642,063 |
| Raw | 682,132 | 669,234 | 687,580 | 712,483 |

These packages include all sixteen disk commands and the pinned OF816 monitor.
Upper-image totals include linked platform assembly/data but exclude bank-zero
loader/resident code, boot storage and command bodies loaded on demand. XEX and
distribution framing are separate quantities. Bank-zero budget, task pools and
runtime reservations match the baseline exactly in both supported profiles.

The old frame-map consumer assumed a single DP class in call-free routines. A
versioned consumer overlay now checks current native-v2 geometry: even-aligned
two- and three-byte homes within `$A0..$BF`, or the separate legacy three-pointer
leaf pool. It retains exact home shapes, identities, bounds, incoming ABI
displacements and outgoing/local-peak accounting. Maps do not serialize live
intervals, so call preservation and interference remain compiler-owned proofs.
No extra bank-zero storage or scratch-survival guarantee is introduced.

OF816's old helper also required the historical compiler revision. An explicit
measured compiler-pin projection binds the boot package to the candidate's
revision and binary hash while preserving the original ABI fields, upstream
sources, ROM, boot arenas and frozen Exec files. Ordinary image placement,
assembly, package-hash and boot guard checks remain active.
The autoboot helper additionally verifies actual machine configuration, devices,
CPU settings and mapped ROM before execution; the original helper omitted that
readback. Verification rejection closes the emulator before running the fixture.

Unchecked packaging still reports `Hosted o65 requires a checked kernel`.
The compiler's o65 import contract also requires checked providers and uses
checked emission. Closing this gate requires a separately designed and qualified
provider contract. Bypassing its check would not produce a qualified release.

| Hosted workload | Optimized guarded | Raw guarded | Disposition |
| --- | --- | --- | --- |
| Shell/path commands, CAT/WC pipeline, dynamic command loading | Pass | Pass | Complete measured packages |
| Cooked input/history | Pass | Pass | Independent frozen fixture oracle |
| Ports/services | Pass | Pass | Raw failed at stage 0; this passing execution does not establish the cause of that change |
| OF816 autoboot, guards and OS restoration | Pass | Pass | Actual machine readback; 249 PAL frames in both runs |
| Standalone lists | Fail | Fail | Pre-existing fixture packages diagnostic payload in bank zero; Exec fixture owner must move its initialization outside resident image payload |
| DOS stream defaults | Fail | Fail | Pre-existing composition omits generated `console-storage-action.inc`; Exec fixture generation must include the required console dependencies |
| DOS routing/ownership | Fail | Fail | Same pre-existing missing generated console dependency |

Eight of fourteen hosted checks pass. The six failures match their frozen
dispositions, and all previously passing workloads still pass. New or unexplained
failures would block acceptance. The list/DOS failures are unqualified standalone
fixture paths; passing shell commands do not replace their independent oracles.
All raw hosted result records retain their diagnostics and executed artifact
hashes. Backend VM, hosted emulator and hardware evidence retain separate scopes.

## Backend qualification

The scoped MIR65816 library suite passes 415 tests (one existing ignored); all
fourteen root integration targets pass 88 tests (four existing ignored). Full
native qualification passes 379 tests across 95 targets (six existing opt-in
tests ignored). The comparison tools pass 27 tests, including forged physical
maps, resource/census changes, exact acceptance boundaries, measured boot pin
binding, machine-readback rejection and actual LF/CRLF log parsing.

Native coverage retains raw/optimized, checked/unchecked, supported relocated
images, calls, aliasing, bank crossings, task/IRQ/NMI interruption and aggregate
oracles. All six candidate/reference profile runs also pass. Seven native
qualification manifests bind the same 632 compiler/runtime/fixture inputs.
Shared frontend/NIR contracts are unchanged; other backends' suites were not run.
There is no hardware qualification claim.

## Compiler cost

Three serial measured rounds follow one warm-up per compiler/profile. CLI and
profile order alternate. Both binaries use optimized Cargo dev builds with
opt-level 3, debug 0 and incremental compilation disabled. No build, native or
hosted qualification runs alongside these measurements. Per-child `wait4`
reports CPU, wall time and peak RSS; every warm-up and measured compile must
reproduce its compiler-specific pinned image.

| Profile | Baseline wall (s) | Candidate wall (s) | Wall change | Baseline RSS (MiB) | Candidate RSS (MiB) | RSS change |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Optimized release | 54.009 | 56.654 | +4.90% | 973.3 | 1,018.7 | +4.66% |
| Optimized guarded | 39.399 | 40.324 | +2.35% | 1,002.0 | 939.8 | -6.21% |
| Raw guarded | 43.862 | 45.109 | +2.84% | 921.9 | 987.0 | +7.05% |

All per-profile medians meet the 5% time and 10% RSS review limits against
stage 0. Median corpus wall time is 137.270 seconds before and 142.086 after.
The release time result is close to its review limit. Individual samples retain
substantial RSS variation: candidate guarded peaks range from 912.0 to 1,213.8
MiB, and individual samples can exceed the median review bounds. This is a
fixed-workload cost assessment, not an asymptotic scaling guarantee or a stable
memory/speed improvement claim. All eighteen measured samples are retained;
none was removed or repeated to obtain a passing median.

## Reproduction and evidence

Restore the exact stage-0 sources and external tools from its
[input manifest](../65816-record-placement-stage0/inputs.json.gz). Use compiler
`d7d536c9`, Exec `57df0d7` and OF816 `8c92362`; a current live Exec checkout is
a different workload. Verify the restored inputs, then rebuild baseline profiles,
probes and vectors with the existing baseline tools. The unchecked baseline
package refusal is expected. Its probe images and native metrics must reproduce
the published baseline exactly.

Build the measured CLI and `record_probe --features placement-analysis` using
`CARGO_INCREMENTAL=0`, `CARGO_PROFILE_DEV_DEBUG=0` and
`CARGO_PROFILE_DEV_OPT_LEVEL=3`. Pin the CLI at
`target/record-placement-stage7/actionc-65816`. Use a fresh candidate output
directory for a new generation; keep emulator logs and observations as execution
evidence rather than prior build inputs. Compiler, runtime and tool inputs must
remain unchanged while each qualification operation runs.

```sh
python3 -B tools/compare65816/exec_record_qualification.py build \
  --base target/record-placement-stage0 --output target/record-placement-stage7 \
  --binary target/record-placement-stage7/actionc-65816
python3 -B tools/compare65816/exec_record_qualification.py probe \
  --base target/record-placement-stage0 --output target/record-placement-stage7 \
  --binary target/record-placement-stage7/rust-target/debug/actionc-exec-record-probe
python3 -B tools/compare65816/exec_record_vectors.py \
  --base target/record-placement-stage0 --output target/record-placement-stage7/native-vectors \
  --binary target/record-placement-stage7/actionc-65816
python3 -B tools/compare65816/exec_record_consumer.py references \
  --base target/record-placement-stage0 --output target/record-placement-stage7
```

Copy the three candidate vector manifests into the candidate root. For each
candidate/reference profile, set `A816_COMPARISON_MANIFEST` to its manifest and
`A816_COMPARISON_RESULTS` to its measurements file, then run
`qualify.py --test code_quality -j2 -- --ignored`. Retain each profile's
qualification log. Run the scoped library suite, the fourteen integration targets
and full native qualification; the publication tool verifies their successful
summaries and native source generation. Run comparison-tool tests with
`python3 -B -m unittest discover -s tools/compare65816 -p 'test_exec_record_*.py'`.

Run each of the seven hosted cases in both `opt` and `raw` modes. For example:

```sh
python3 -B tools/compare65816/exec_record_hosted.py \
  --base target/record-placement-stage0 --case demo --mode opt --compiler-root . \
  --binary target/record-placement-stage7/actionc-65816 \
  --profiles target/record-placement-stage7 --output target/record-placement-stage7/hosted
```

Retain nonzero exits and diagnostics for the known failed fixtures. Scoring
rejects new failures, unexplained failures, artifact/input drift, changed
reservations, changed native oracles/external traces and resource growth.

After all builds and qualification finish, measure serial costs with
`measure_host.py`, three rounds and `--expected-builds 3`. Its manifest uses the
recorded baseline commands and separately pinned before/after image hashes.
Update local command paths when restoring on another host while retaining exact
sources, layouts, flags and expected images. Preserve every sample.

```sh
python3 -B tools/compare65816/exec_record_qualification.py publish \
  --base target/record-placement-stage0 --output target/record-placement-stage7
```

Publication intentionally exits nonzero when final acceptance targets remain
open, while retaining the complete scorecard. [results.json](results.json)
contains final gates, hosted dispositions, resource checks and host samples;
[compiler-inputs.json](compiler-inputs.json) binds compiler/runtime/tool sources.
Compressed native manifests, build recipes and hosted records retain their
respective evidence. [evidence-sha256.json](evidence-sha256.json) authenticates
the published files. Rejected package combinations and failed fixtures are
published evidence, not successful qualifications.

## Development direction

The integrated placement foundation has demonstrated benefits across records,
control flow and indexed operations. Keep the open final targets as obligations
and focus the next structural work on roadmap direction 3: calls and runtime
interfaces within the current public ABI. Release MIR call spans account for
117,117 bytes (27.2% of compiler code), and call-segment benefits currently reach
only one frozen routine. This identifies an investment direction, not a forecast
of savings or an authorization to change the ABI.

Continue measuring complete application footprint alongside that work. Resolve
the unchecked-provider contract and standalone fixture composition/packaging
gaps before claiming complete release readiness. Specialized selectors and their
independent execution oracles remain useful; planner ownership is removed only
after equivalent common-model coverage exists.
