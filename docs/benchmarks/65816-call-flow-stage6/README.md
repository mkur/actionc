# Call flow: final qualification

Stages 0–6 are implemented and qualified. Common placement now owns native
results independently of temporary memory homes and admits immediate Returns,
byte/word zero tests, exact-width private local Stores and bounded private
byte/word Direct-call inputs. Each family retains complete conservative
fallback, atomic resource admission and independent final verification/replay.
The public ABI, image format and bank-zero reservations are unchanged.

The measured compiler generation is `0226ebd2`; this final slice changes
documentation and publishes qualification. Frozen Exec816 remains `57df0d7`,
256 files and 1,156 routines. All fifteen probe artifacts are byte-identical to
stage 5. [Provenance](provenance.json) binds compiler, runtime, fixture, tool,
probe and source identities. The separate pinned CLI is bound by
[host results](host-results.json), native vector receipts and hosted records.

| Profile | Stage-0 code | Final code | Saving | Frame/spill/local-peak sum change |
| --- | ---: | ---: | ---: | ---: |
| Optimized release | 430,091 | 425,954 | 4,137 (0.96%) | −334 each |
| Optimized guarded | 579,765 | 575,068 | 4,697 (0.81%) | −334 each |
| Raw guarded | 648,963 | 639,980 | 8,983 (1.38%) | −404 each |

No individual routine grows in code, frame, spill extent or local peak against
stage 0 or any preceding slice. Release code plus initialized data is 428,425
bytes: 425,954 code and 2,471 initialized data, with another 277 zero-fill bytes.
Data identities and reservations are unchanged; guarded profiles retain their
existing 2,470 initialized bytes. Three representative subsystems benefit:
TASKPOLICY, DOSSTREAMS and SDFSFILE. The complete corpus also contains 28 improved
loop-placement routines and one improved call-segment routine. These are
complete routine savings, not a claim that call-flow changes loop strategy.

## Qualified forms and refusals

Optimized profiles own 1,416 native outputs: 438 immediate Returns, 692 zero
tests and 286 private Stores. Raw owns 1,996: 441, 654 and 901 respectively.
There are 283 optimized/374 raw narrow input bindings, of which 91/166 newly
defer storage reads; the others already borrowed storage but retained reserved
homes. See the [stage-5 binding census](../65816-call-flow-stage5/argument-bindings.json.gz).

The optimized investigation's 692 zero-test candidates are admitted. One of
287 screened Stores retains capture because its call target is indirect.
Of 121 screened bounded argument reads, 30 retain capture: 24 cross external
reads, two terminate at Helpers, three cross parameter-backed pointer reads
and one widens a word into a three-byte slot. These screen counts describe
typed workload shapes; they are not independent admission permissions.

Indirect calls, cross-block native outputs, cast chains, wide register
arguments, exposed destinations, parameter-backed local aliases and incomplete
source views retain existing strategies. Ordinary record field writes currently
do not establish the mutable-object fact required by a private Store route.
Those objects retain capture; verified MIR with an explicit fact independently
qualifies nonzero field displacements without changing frontend/NIR contracts.

## Backend and native qualification

The final scoped library run passes 422 tests, with one usual ignored test.
All fourteen root integration targets pass 88 tests, with four opt-in inventory
tests ignored. Full qualified native execution passes 385 tests across 98 result
groups, with six usual opt-in tests ignored. All 46 observer tests pass and the
candidate report's `--check` reproduces the published static tables exactly.
[Validation](validation.json) retains counts and log hashes; compressed native
qualification manifests retain compiler/fixture inputs, VM/tool identities,
execution artifacts and interruption schedules.

Independent assembly and CPU oracles cover declared result lanes and zero
extensions, poisoned unspecified registers/flags, exact argument bytes/padding,
neighboring canaries, exact three-byte Stores, fresh comparisons and native
S/M/X/I/D/DBR state. Coverage includes both construction strategies, multiple
private inputs, raw/optimized, guarded/release, supported relocations, recursion,
task/IRQ/NMI interruption at changed boundaries and actual LF/CRLF compilation.
Shared frontend/NIR contracts are unchanged; other backends' suites were not run.

All 441 frozen native vectors retain exact values, arguments, external access
traces, per-vector cycles/private accesses and peaks against stage 0.

| Profile | Vectors | Cycles | Private byte accesses | Peak below entry S |
| --- | ---: | ---: | ---: | ---: |
| Optimized release | 147 | 71,833 | 20,655 | 33 |
| Optimized guarded | 147 | 76,021 | 20,655 | 33 |
| Raw guarded | 147 | 87,539 | 24,288 | 45 |

This frozen corpus is a regression oracle and establishes no dynamic savings
from these slices. New independent fixtures execute the admitted forms.
[Native results](native-results.json) contain comparisons; compressed per-profile
vector manifests/results retain the actual oracles, image hashes and observations.

## Hosted qualification

Eight of fourteen checks pass: demo, cooked input, ports and OF816 in optimized
and raw modes. The six prior failures remain: list fixture packaging places
resident payload in bank zero, while standalone DOS streams/routing fixtures
lack generated console includes. There are no new or unexplained failures.
[Hosted results](hosted-results.json) classify every case; compressed records
retain exact diagnostics and executed artifact/emulator identities.

Both guarded packages build with all sixteen commands and the pinned OF816
monitor. Their recipes, source hashes and artifacts are authenticated separately
from compiler-only probes.

| Guarded package | Upper code + initialized data | Program XEX | OF816 XEX |
| --- | ---: | ---: | ---: |
| Optimized | 595,339 | 612,391 | 637,294 |
| Raw | 660,251 | 678,437 | 703,340 |

Upper totals include linked platform assembly/data and exclude bank-zero loader,
resident code, boot storage and commands loaded on demand. Framing sizes are
separate quantities. Bank-zero budget, task pools and runtime reservations match
the frozen baseline. Actual unchecked packaging still reports
`Hosted o65 requires a checked kernel`. Guarded execution does not qualify that
provider combination. No hardware qualification is claimed.

## Compiler cost

Rust 1.99 builds match stage 0: optimization level 3, debug information disabled,
incremental compilation disabled, and no CLI features. One warm-up per profile
precedes three serial rounds with alternating profile order. Builds and native/
hosted qualification finish before measurement. Every warm-up and sample
reproduces its pinned image; all nine measured samples are retained.

| Profile | Baseline wall (s) | Final wall (s) | Change | Baseline RSS (MiB) | Final RSS (MiB) | Change |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Optimized release | 55.750 | 56.048 | +0.54% | 1,207.0 | 1,216.6 | +0.80% |
| Optimized guarded | 41.087 | 41.518 | +1.05% | 1,290.2 | 1,326.2 | +2.79% |
| Raw guarded | 45.249 | 45.739 | +1.08% | 1,264.1 | 1,233.7 | −2.41% |

All profile medians meet the 5% wall-time and 10% RSS limits against the
consistent Rust-1.99 baseline. [Host comparison](host-comparison.json) records
exact ratios and baseline lineage; [host results](host-results.json) retain
wait4 wall/user/system/RSS samples, commands and binary/tool hashes. This is a
fixed-workload cost assessment; historical measurements using another Rust
version are not a cumulative timing baseline.

## Carried application acceptance

| Gate | Required | Final | Result |
| --- | ---: | ---: | --- |
| Release compiler code | ≤422,700 bytes | 425,954 | Open: 3,254 bytes |
| Representative release code | ≤9,235 bytes | 9,650 | Open: 415 bytes |
| Native release private accesses | ≤20,565 | 20,655 | Open: 90 accesses |
| Native release cycles | ≤78,117 | 71,833 | Pass |
| Optimized native peak | ≤42 bytes | 33 | Pass |
| Benefited representative subsystems | ≥3, with loop/call benefits | 3; both present | Pass |
| Routine frame/spill/local-peak growth | 0 | 0 | Pass |
| Added compiler bank-zero reservation | 0 | 0 | Pass |
| Per-vector cycle ratio | ≤1.05 | 1.00 | Pass |
| New/unexplained hosted failures | 0 | 0 | Pass |
| Compiler time/RSS median ratios | ≤1.05/1.10 | All profiles within limits | Pass |
| Actual unchecked hosted package | Valid provider contract and execution | Unsupported | Open |

[Final acceptance](final-acceptance.json) preserves the original numerical
targets and separates completed call-flow delivery from open application gates.
The separate 256 KiB application objective remains open. The final static
tables, [incremental resources](incremental-results.json), scorecards and
qualification receipts are authenticated by
[qualification evidence hashes](qualification-evidence-sha256.json).

## Reproduction

Restore the exact frozen inputs as described in the
[stage-0 scorecard](../65816-call-flow-stage0/README.md). Keep the original
reference and evidence immutable; collection requires a fresh candidate
generation. Substitute a new output directory when reproducing these commands.
Build the CLI with the settings above and pin it at the candidate's
`actionc-65816`. The [implementation plan](../../MIR65816_CALL_FLOW_IMPLEMENTATION_PLAN.md#validation-entry-points)
lists the full scoped backend checks.

```sh
python3 -B tools/compare65816/exec_call_candidate.py collect \
  --output target/call-flow-stage6 --stage 6
python3 -B tools/compare65816/exec_call_candidate.py report \
  --output target/call-flow-stage6 --reference target/call-flow-stage0 \
  --destination docs/benchmarks/65816-call-flow-stage6 --check
python3 -B tools/compare65816/exec_record_vectors.py \
  --base target/record-placement-stage0 \
  --output target/call-flow-stage6/native-vectors \
  --binary target/call-flow-stage6/actionc-65816
python3 -B tools/compare65816/exec_record_qualification.py build \
  --base target/record-placement-stage0 \
  --output target/call-flow-stage6/hosted-profiles --compiler-root . \
  --binary target/call-flow-stage6/actionc-65816
```

Run the comparison `code_quality` target with each profile's actual
`A816_COMPARISON_MANIFEST` and `A816_COMPARISON_RESULTS`, using the stage-0
commands with this candidate directory. Run all seven hosted cases in both
`opt` and `raw`; retain known nonzero exits and their diagnostics. For example:

```sh
python3 -B tools/compare65816/exec_record_hosted.py \
  --base target/record-placement-stage0 --case demo --mode opt --compiler-root . \
  --binary target/call-flow-stage6/actionc-65816 \
  --profiles target/call-flow-stage6/hosted-profiles \
  --output target/call-flow-stage6/hosted
python3 -B -m unittest discover -s tools/compare65816 -p 'test_exec_record_*.py'
python3 -B -m unittest discover -s tools/compare65816 -p 'test_exec_call_*.py'
# After all builds and qualification finish:
python3 -B tools/compare65816/exec_call_measure.py \
  --output target/call-flow-stage6 \
  --binary target/call-flow-stage6/actionc-65816 --rounds 3
```

Compare immutable candidate/reference images by routine identity, check all
resource dimensions and exact native vector oracles, and classify hosted
failures against stage 7. Authenticate compiler inputs, package recipes,
executed artifacts and all host samples before reporting acceptance. Bulky
images/binaries/logs stay under ignored targets; compact committed receipts
preserve their identities.
