# MIR6502 homes across multiple leaf calls

Focused qualification, 2026-11-15, against `0880cd2f`. Selector slice:
`6e2a37d4`. Implements the [design extension](../../MIR6502_PARAMETER_HOME_SELECTION_DESIGN.md#proposed-extension-multiple-calls-to-leaf-routines)
and [two-slice plan](../../MIR6502_MULTIPLE_LEAF_CALL_HOMES_IMPLEMENTATION_PLAN.md).

`enable_multiple_leaf_call_homes` is enabled in optimized configuration and off
in conservative configuration. Eligible callers with one or several direct leaf
calls enter the same ordinary-call batch. Every call site must supply the callee's
entry inputs, and placement avoids the union of all callee writes. Expanded
lifetime verification, invocation-home certificates, preservation, final-image
costing and exact rollback retain their existing authority.

## Independent savings

The new `multiple_leaf_calls.act` control uses a pointer loop with a conditional
call to Bump and an unconditional call to Hop, a repeated call to AddOne, and a
single-call control. Small-leaf inlining is disabled. All other optimized gates,
including scoped observation, remain enabled. ActionCart `$3000`:

| Metric | New gate off | New gate on |
| --- | ---: | ---: |
| Packaged image | 197 | 173 |
| SumWalk code + ordinary homes | 80 + 4 | 65 + 0 |
| SumWalk ZP homes | 0 | 4 |
| Twice code + ordinary homes | 38 + 2 | 35 + 0 |
| Twice ZP homes | 0 | 2 |
| Once code + ordinary homes | 24 + 0 | 24 + 0 |
| Once ZP homes | 2 | 2 |

The complete image saves **24 bytes**. SumWalk saves 19 code/storage bytes and
Twice saves five. No routine grows. The gate-off batch accepts Once alone, from
200 to 197 bytes. The enabled batch accepts SumWalk, Twice and Once together,
from the same 200-byte pre-trial baseline to 173 bytes. It does not run a separate
single-call trial first. Traced/untraced executable artifacts match in both arms.

## TN outcome and remaining blocker

TN remains **9,791 bytes** in both arms. Images, complete maps and relocations
match each other and the retained whole-image-cost baseline exactly. All 101
routine footprints are unchanged; there are no growing routines.

| Routine | Code | Ordinary homes | ZP homes |
| --- | ---: | ---: | ---: |
| FindItem | 74 | 3 | 0 |
| Ord | 51 | 0 | 5 |
| Items | 37 | 0 | 3 |

FindItem now reaches multiple-call discovery, but call index 0, Key, fails with
`unknown residence callee writes`. The [recorded rejection](tn-finditem-rejection.txt)
is an eligibility failure before placement/costing. Next is the second call.
The existing Key write summary is therefore the next boundary to investigate;
this extension does not relax unknown-write handling. Ord and Items remain in
the accepted combined batch, preserving their existing 56-byte saving against
the 9,847-byte pre-trial image.

## Work and artifact evidence

| Work | Control off / on | TN off / on |
| --- | ---: | ---: |
| Ordinary-call trials | 1 / 1 | 1 / 1 |
| Core materializations | 3 / 3 | 6 / 6 |
| Emission invocations | 15 / 15 | 163 / 163 |
| Inner emission probes | 60 / 60 | 978 / 978 |
| Preservation replays | 0 / 0 | 24 / 24 |
| Rewrite analyses | 1,714 / 1,713 | 50,800 / 50,800 |

The independent candidate changes downstream code and needs one fewer rewrite
analysis. It adds no trial, continuation or emission. TN work counts are identical.
One TN observation recorded 20.75 / 20.55 seconds traced and 19.66 / 19.73 seconds
untraced (off/on), with peak process RSS 756,596,736 / 751,910,912 bytes. Part of
qualification overlapped a test build. These samples do not establish a speedup
or regression; no repeated timing benchmark was run.

Sources and configuration match between gate arms except for the new boolean.
The off configuration matches the historical settings after omitting that added
false field. The off TN image/map/relocation hashes reproduce the old receipts;
no old compiler was rebuilt. Receipts retain library/probe hashes, the compiler
revision plus its pending default-enable diff hash, decisions and every routine's
footprint:

- [Control off](control-off.json), [control on](control-on.json)
- [TN off](tn-off.json), [TN on](tn-on.json)
- [Complete comparison](comparison.json)

The TN-only receipts include the earlier control-source hash as suite bookkeeping;
that file was not compiled in either TN run. TN.ACT/LIB.ACT hashes are identical
and current. The final control receipts record the corrected fixture used by the
passing integration/VM tests. Large images, maps, listings and logs remain under
ignored `build/multiple-leaf-calls/`.

## Minimal validation

Nine selected tests passed:

- Four grouped library tests for distinct/repeated targets, the later callee's
  word-lane/local clobber, per-site argument supply and exact failed-trial rollback.
- The existing exact selected-input/executable rollback control.
- Two integration tests for actual savings, initialized local/parameter residence,
  one combined trial, default/parent gates and traced/untraced equality.
- The edited unsupported-call control, retaining nested/recursive, exposed,
  unknown and unsupported callees.
- One VM group: cart off/on, actual LF/CRLF loading, one standalone smoke case,
  call arguments/order, results, stack balance, repeated invocations, conditional
  execution, page crossing, wrapping arithmetic and source-memory sentinels.

[Slice 1 checks](slice1-checks.json), [slice 2 checks and fixture corrections](slice2-checks.json).
The initial fixture used the reserved name Step, then a pointer-reading leaf with
an unknown write summary. The corrected positive control uses supported leaves;
its required residence assertions and compiler proof rules remain intact.
Passing tests were repeated only when those fixture corrections affected them.

The shared build target used opt-level 1, no debug information, incremental off
and two jobs. Full backend suites, NIR sweeps, raw-NIR/origin matrices, MADS and
full TN runtime scenarios were deferred under the user's minimal-testing direction.

## Reproduce

Build the current rlib using the plan's shared-target settings, then run:

```sh
python3 tools/compare6502/parameter_homes.py \
  --api-rlib build/checked-arena/shared-target/debug/deps/libactionc-bf811c1ef8b2df31.rlib \
  --output build/multiple-leaf-calls/recheck-off \
  --enabled --handoff --cross-call --scoped-observation \
  --suite multiple-leaf-calls --tn-only
```

Use a separate output directory and add `--multiple-leaf-calls` for the enabled
arm. Replace `--tn-only` with `--skip-tn` for the control. The rlib filename can vary.
