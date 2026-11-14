# Callee entry capture qualification

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
