# MIR65816 mode-state optimization plan

## Objective and baseline

Reduce redundant accumulator-width instructions in native 65816 output while
preserving instruction widths, flags, hidden B, stack contracts and interrupt
safety. This implements priority 1 from the Exec output analysis.

The baseline is compiler `de51e276` with Exec source revision
`154bcf5a45605aeee57de5420ec4604e4ff7b30c` and the existing generated demo inputs.
The measured workload contains 899 routines from 166 unique source files.
Compiler code occupies 350,372 bytes optimized without stack guards, 457,661
bytes optimized with guards, and 399,348 bytes raw without guards. Initialized
compiler data occupies 1,385 bytes. Platform assembly is outside these totals.

The disassembly model identified 1,778 immediately overwritten mode instructions
(3,556 bytes) and 4,107 redundant mode instructions across control flow (8,214
bytes). These are candidate counts, not promised savings. Re-emission and branch
relaxation can change the final savings. Do not compare this workload's totals
with older Exec reports using different routine sets.

Baseline images, inventories, source hashes, analysis scripts and the probe are
retained locally in `target/exec-priorities-de51e276/`. Record compact final
measurements in this document so the committed result does not depend on those
ignored artifacts.

## Ownership and invariants

All implementation belongs to MIR65816 selection, tracked state, selected-code
verification and replay. There is no SemIR/NIR contract, ABI, allocation or Exec
source change. Use typed instructions and requests; do not edit encoded opcode
bytes to discover or remove mode changes.

Only accumulator changes with mask `$20` qualify for removal. Mixed masks,
index-width changes, unknown status effects and calls remain protected unless
the existing structured contract explicitly proves the required width. A mode
instruction does not alter A/B or arithmetic flags; changing X width can truncate
registers and is excluded.

Every omitted instruction needs a proof grounded in executable predecessors or
an immediately following overwrite. A stored desired width or a circular loop
assumption is not sufficient. Validate incoming edges, including late backedges,
and reject incompatible widths. Keep memory/value facts conservative at joins.

Regenerate bytes, effects, traces, spans, fixups and labels through tracked
emission. Rebuild and verify the selected CFG before publishing changed code,
then run normal branch relaxation and metadata reconciliation. Replaying the
result must reproduce its bytes and proof observations exactly.

## Slice 1 — immediately overwritten requests

Identify a mode request whose emitted `$20` instruction is overwritten by the
next mode request before any width-sensitive instruction or control-flow entry.
Permit only explicitly harmless bookkeeping between the requests. Labels,
transfers, calls, stack operations and status instructions with other masks end
the window.

Implement a narrow checked transformation over selected actions, followed by
fresh tracked emission. Preserve the second request and its demanded width.
Verify that all subsequent physical instructions retain their required
environment. Keep the original code when the proof does not apply.

Add positive and negative tests for both width directions, repeated requests,
source spans, labels, mixed masks, hidden B and live condition flags. Check fresh
replay and finalization. Measure the same Exec inputs and commit this slice
separately.

## Slice 2 — width facts at control-flow joins

Extend the existing checked entry contracts to retain accumulator-width
knowledge at eligible internal labels and continuations. Use predecessor
agreement and existing native call contracts; do not propagate register values
or memory facts merely because widths agree. If the present edge checker is
insufficient for a boundary, add explicit selected-CFG analysis rather than
assuming a width.

Cover diamonds, loops and late backedges, unreachable code, entry/return paths,
direct and indirect calls, and compiler-generated guard labels. Add rejection
tests for a missing or incompatible predecessor and forged selected boundaries.
Document the resulting guarantees in the state-tracker contract, measure Exec
again, and commit the second slice separately.

## Validation and completion

During each slice, run focused MIR65816 library tests and native runtime targets
for state tracking, replay, control flow and guards. Update exact byte
expectations only where the smaller output is intentional and independently
covered. Do not relax semantic, ABI or proof assertions to accept new output.

For final qualification, run the MIR65816 library and integration targets and
the full native 65816 runtime suite through
`python3 -B tools/native65816-runtime-tests/qualify.py`. Include a release run of
the focused state/control-flow/replay/guard/preemption targets. The runner pins
the corrected VM and checks that compiler and fixture inputs remain unchanged
during execution. Tests must cover raw and optimized compilation, guards on and
off, IRQ/NMI, relocation and byte/word boundaries. For new newline-sensitive host
fixtures, exercise LF and CRLF through the actual instrumentation path.

Rebuild the original three Exec profiles with unchanged source hashes. Report
actual code/data totals and savings, any remaining candidates, and qualification
scope. Compiler VM tests do not constitute hosted Exec boot qualification.
Shared NIR checks are required only if implementation changes those contracts;
other backends and unrelated dirty workspace files remain outside this patch.

## Results

Slice 1 reduces optimized unchecked Exec code from 350,372 to 344,150 bytes:
6,222 bytes saved across the unchanged 899-routine workload. All 166 source
hashes match the baseline. Removing an overwritten request can also make its
successor unnecessary, explaining savings above the single-instruction model.

The first slice passes 341 active MIR65816 library tests (one existing ignored
test), plus native control-flow, guard, replay and state-tracking targets. The
VM probe independently checks both store widths, hidden B, flags and cycles.
Replay inventory coverage includes the existing pointer request families, and
the trace test now checks actual entry/return coverage instead of requiring a
historical instruction count. The native test lockfile uses the corrected CPU
path expected by the qualification runner, without dependency version updates.

Slice 2 retains the checked width at internal labels and native call
continuations. Late incoming edges still have to satisfy the original execution
contract; memory and register-value facts remain conservative. Re-emission also
preserves consume decisions, since selection may have relied on a decision to
omit a later load or pointer setup. A changed decision retains the original
routine.

| Exec profile | Baseline code bytes | Final code bytes | Saved bytes |
| --- | ---: | ---: | ---: |
| Optimized, guards off | 350,372 | 334,256 | 16,116 (4.60%) |
| Optimized, guards on | 457,661 | 444,271 | 13,390 (2.93%) |
| Raw, guards off | 399,348 | 381,526 | 17,822 (4.46%) |

All three final builds retain the same 899 routines, signatures, argument/result
layouts, frames, spill sizes, temporary homes and stack bounds. Initialized data
(1,385 bytes) and zero-fill descriptors are identical. All 166 loaded source
hashes and the original layout hash match the baseline. The optimized unchecked
disassembly leaves one redundant and 193 adjacent overwritten mode candidates
(388 modeled bytes); these are outside the implemented proof and do not justify
extending this slice. All routine byte streams and MIR span totals reconcile.

The reviewed emission snapshot changes intentionally: mode instructions disappear
and labels, spans, fixups and branch offsets follow the smaller output. Frame
lines are unchanged. This is an emission optimization, with no NIR or printer
contract change. The snapshot passed with actual LF and CRLF fixture bytes.

Final validation passes 346 active MIR65816 library tests (one existing ignored
test), 86 integration tests across 13 targets (four existing ignored tests),
and 53 debug runtime tests for control flow, state tracking, replay, guards and
preemption. Release runtime qualification passes 332 tests across all 87 targets,
with six existing opt-in tests ignored and ten baseline failures excluded by
name. The qualified runner confirms unchanged compiler and fixture hashes.

The initial unfiltered runtime run exposed ten failures also reproduced at
`de51e276` in an isolated checkout with the same corrected CPU. These concern
existing forwarding/home assumptions, old DP ranges, a guard expectation,
relocation and coverage counts. They remain enabled in source and are explicitly
excluded only from the final qualification command. This is not an unfiltered
green runtime suite. Both preemption exclusions also apply to the focused debug
run. Exact test names, baseline failures, commands, image hashes and qualification
manifest hashes are in the [measurement report](benchmarks/65816-mode-state/results.json).

No shared frontend/NIR contract changed, so validation is scoped to MIR65816.
Hosted Exec boot qualification remains separate from these compiler/runtime
checks and compilation measurements.
