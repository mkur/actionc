# Callee entry capture qualification

**Qualified:** the independent capture fixture shrinks from **375 to 337 bytes**;
TN stays at **9,853 cart / 9,960 standalone bytes**. Baseline reproduction is
committed at `4542b663`; the corrected compiler is `4ea5886d`. All four planned
slices are complete. Full results and exact commands are in
[qualification.json](qualification.json).

The baseline compiler is `72131db3`; the design and plan were committed as
`b9a76330`. Slice 0 changes only tests and measurement tools. The frozen receipt
records the actual library hash and MIR source diff (which includes the new test).
Large images, maps, listings and logs live under ignored
`build/mir6502-callee-entry-capture/`.

## Baseline and reproduction

The ordinary Action fixture keeps real forwarding calls with inlining both
disabled and enabled. `CaptureWords` receives A/X/Y/$A3 and uses selected
`$A0–$A3` homes. Before capture the old fact incorrectly reports all four working
bytes as inputs. With the real capture present, only `$A3` remains a fixed input;
A/X/Y provide the other bytes. The old liveness query also includes SP because
of an ordinary Return, so admission reports `extra-entry-flags-or-stack` before
it reaches the extra fixed-byte check. This is the implicit balanced call/return
contract, not an additional argument. A genuine extra read of `$AC` survives
capture in `ExtraFixed`. `ReadPointers` has no integer-only `scalar_signature`
and is independently rejected as unknown.

`ForwardWord` is already a three-byte jump. The two-word `ForwardWords` and
`ForwardPointers` each occupy 22 code bytes and four ordinary parameter bytes.
The whole fixture occupies 375 packaged bytes in either runtime and at either
origin. These figures are the production baseline, not forced-placement savings.

Commands use `build/mir6502-layout-census/build-env.sh` (opt-level 1, no debug
information/incremental build, two jobs, shared target directory):

```sh
cargo test --lib mir6502::parameter_homes -- --nocapture
cargo test --test mir6502_parameter_argument_handoff -- --nocapture
python3 tools/compare6502/parameter_homes.py \
  --api-rlib build/checked-arena/shared-target/debug/deps/libactionc-bf811c1ef8b2df31.rlib \
  --enabled --handoff --suite capture \
  --output build/mir6502-callee-entry-capture/baseline
```

During development the VM test is linked directly to the freshly built root
library and existing VM library to avoid a second compiler build. Final
qualification uses the VM workspace Cargo target. Both paths execute
`tools/vm-runtime-tests/tests/parameter_argument_handoff.rs`.

The actual LF and CRLF source-loading paths, two runtimes, two origins and both
inlining settings are covered by the integration and VM tests. Runtime hooks
observe canonical argument bytes before the callee captures them, independent
public arithmetic/pointer results and balanced return. The prior counted-copy
oracles continue to check ordered reads and writes.

Slice 0 passed 24 parameter-home unit tests, six handoff integration tests
(one MADS test deferred to final qualification), and all four handoff VM tests.
All four TN image/map/relocation hashes and work/upstream counts match the prior
qualification. See [baseline.json](baseline.json) and
[baseline-checks.json](baseline-checks.json).

## Shared construction and stage refactor

Slice 1 moves the unchanged direct capture builder into
[`abi/param_capture.rs`](../../../src/mir6502/abi/param_capture.rs) and requires a
planned/actual stage on shared entry queries, fact validity and handoff verification.
The verifier's production callers were audited: all pre-materialization checks
operate on retained inputs; PostHome and later checks operate after capture.
This avoids adding state to every MIR routine. Machine-entry omission and SArgs
selection remain with the existing materializer. Tests cover capture ordering,
inline/ZP/mixed/register/absent/identity lanes and stage/body/binding staleness.

Slice 1 passed 814 focused unit tests (12 existing ignored) and
91 integration tests (one existing MADS test deferred). All 12 frozen
cases match in bytes, maps, relocations, selected homes, placement outcomes and
work/upstream counts: [receipt](slice1.json), [checks](slice1-checks.json).

## Capture composition

The independent fixture now occupies **337 packaged bytes**, versus 375 with the
same handoff-enabled baseline compiler. Each of `ForwardWords` and
`ForwardPointers` changes from 22 code bytes + four ordinary home bytes to
seven code bytes + zero ordinary homes: **19 bytes saved per wrapper, 38 total**.
The two physical fragments are `$A0` and `$A3`; X and Y stay in their canonical
registers. Both runtimes, origins and inlining settings produce the same saving.
`ForwardWord`, the extra-$AC control and the post-call-use control retain their
previous costs. This is measured production selection, not a forced binding.

The shared entry fact now composes register-store capture with body liveness.
The required inputs for `CaptureWords` are A/X/Y and `$A3`, with no additional
stack or flag requirement. Pointer eligibility comes from a complete finite
`direct_parameter_abi` declaration, independently of the integer-only signature.
Memory-sourced capture and SArgs remain unknown; no expansion effects are guessed.

A separate `Sum` regression intentionally reaches an uncertified expanded carry
requirement. The existing flag proof does not certify its low-byte memory ADC
for the later high-byte carry use. The trial is rejected, restoring the selected
input identity, program, image/maps/relocations, costs and trace identity. This
keeps final verification authoritative without broadening flag certification.

Slice 2 passed 72 focused proof/unit tests (one existing census ignored), six
handoff integration tests (MADS deferred) and all four handoff VM tests. The TN
cart/standalone images remain **9,853 / 9,960 bytes**, with identical image/map/
relocation hashes, routine costs and work/upstream counters at both origins.
The new controls use one existing handoff continuation (two to three total
materializations); no upstream search is repeated. See [receipt](slice2.json)
and [checks](slice2-checks.json).

## Final backend qualification

| Scope | Targets | Passed | Existing ignored |
| --- | ---: | ---: | ---: |
| MIR6502 unit tests | 1 | 1,389 | 15 |
| MIR6502 integration tests | 33 | 150 | 12 |
| Relevant VM consumers | 24 | 60 | 0 |
| MADS reconstruction and relocation | 11 | 11 | 0 |

There were no failures or compiler changes during qualification. The explicit
MADS run executes the selected assembler-dependent tests, including the new
capture fixture and TN, with LF/CRLF listings at both origins/runtimes. It keeps
the previous inventory's historical `keeps_unknown_tn_destinations_unchanged`
skip. The 15 existing unit-test ignores remain unchanged.
The VM target inventory is unchanged; its handoff target adds the new capture
oracle. Shared SemIR/NIR contracts are unchanged, so this is a backend-scoped
qualification, not a full repository test run.

The [12 existing handoff cases](qualification-handoff.json) and
[24 leaf controls](qualification-leaf.json) reproduce their previously qualified
images, physical maps, relocations, routine costs and work/upstream counters.
All four TN configurations also match the frozen baseline exactly. The
[eight handoff-disabled capture controls](qualification-capture-off.json)
reproduce their original 375-byte artifacts without a handoff trial.

For each new positive case the existing single batch adds one continuation:
materializations 2 → 3, emission invocations 7 → 9, inner emission probes 14 → 18.
Upstream search counts remain unchanged. TN and existing controls retain all
work counters. Capture measurements include traced/untraced equality, inlining
on/off, both origins and runtimes. Timings in the receipts are diagnostic only:
builds, tests and comparison processes overlapped under the recorded opt-level-1
profile; they are not an isolated compiler-performance measurement.

No new production gate or default was introduced. Memory-sourced capture, SArgs,
aggregate/variadic/wider or incomplete declarations remain conservative. The
ordinary return-stack distinction does not relax explicit stack dependencies,
and final expanded proof plus strict cost acceptance and rollback remain mandatory.
