# Call flow: native outputs and immediate Return

Stage 2 admits sole adjacent native Returns at widths 1–4. The Call remains an
incoming register/flag/scratch barrier. Its verified callee-return contract
creates fresh A/X tokens; tracked publication checks actual lane preservation
through outgoing cleanup at body S. Return consumes the output at its own source
point before ordinary teardown. Replay derives the permission again; placement
requires complete logical uses and exactly one matching publish/read pair under
the sealed generation. Missing origins, clobbered lanes, wrong consumers and
changed routes are rejected. No useful callee flags are assumed.

Allocation trials compare complete conservative/candidate demand, mixed
residence and final affinity layouts. A rejected trial restores conservative
ownership. Affinity validation now consumes the same demand plan; recursively
replanning demand during trial verification is forbidden. Native output homes
are absent from allocation maps and result preflight has no hidden home lookup.
Existing terminal JML forwarding retains its separate contract.

| Profile | Code bytes | Saved vs stage 0/1 | Owned output intervals | Frames shrinking | Sum of frame/spill/peak deltas |
| --- | ---: | ---: | ---: | ---: | ---: |
| Optimized release | 429,819 | 272 | 438 | 50 | −170 each |
| Optimized guarded | 579,049 | 716 | 438 | 50 | −170 each |
| Raw guarded | 648,569 | 394 | 441 | 40 | −130 each |

No routine grows in code, frame, spill or local peak. The one materialized
adjacent indirect Return retains capture/read fallback. Removal of reserved
homes is distinct from the earlier optimization that already omitted most
Return captures/reloads. Savings here come from allocation and its emitted
frame/guard consequences, not from counting nonexistent captures.

Validation: 391 emission tests, 45 root ABI/contract/emission tests, 19 tooling
tests, and 16 focused qualified native tests pass. Native Return coverage includes
full A/X values and zero extensions, nonzero outgoing areas/frames, recursion,
relocation, task switching and IRQ/NMI interruption through cleanup/teardown.
The native Return fixture now requires absent result homes and independently
checks zero-frame teardown. Actual LF/CRLF images match.

All 147 vectors in each frozen native profile pass with unchanged values,
arguments, exact external accesses, cycles/private traffic and stack peaks.
These vectors do not claim dynamic benefit from the new Return forms; application
size and independent Return allocation checks establish this slice's benefit.
[Native results](native-results.json), [provenance](provenance.json),
[results](results.json) and compressed routine changes bind the evidence.
A first application collection exposed the recursive validator and aborted; the
published generation and all listed checks use its corrected implementation.

Reproduction uses the stage-0 commands with `target/call-flow-stage2`, stage `2`
and `--reference target/call-flow-stage0`. Focused native qualification:

```sh
python3 -B tools/native65816-runtime-tests/qualify.py \
  --test call_returns --test home_demand --test replay \
  --test stack_allocation --test call_flow -j2
```

Compiler-cost acceptance remains the final serial comparison against stage 0.
The earlier stage-7 open application gates remain open.
