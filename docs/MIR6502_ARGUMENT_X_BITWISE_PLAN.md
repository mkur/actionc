# MIR6502 argument X forwarding through immediate bitwise setup

Prepared 2026-11-11. Baseline: `9a43c963`. Status: slice 1 complete; selection remains unchanged.
Implement three vertical slices and commit each after its checks pass.
This extends the existing target scheduling contract; no separate design
note or executable IR form is needed.

Allow immediate BYTE AND/OR/XOR operations in the unchanged setup gap of
retained-store A to X forwarding. Preserve the producer, every store, source
access order, call, parameter home and ABI placement. The current
[callee entry flag measurements](benchmarks/6502-callee-entry-flags/README.md)
put TN/TNDBG cartridge images at 9,858/12,953 bytes and `Strcat` at 113 bytes /
355 sampled cycles. Those values are controls for this extension.

## Scope and proof contract

Admit only typed `MirOp::Binary` operations with BYTE width, destination A,
left operand A, right operand `ConstU8`, operation And/Or/Xor, no carry input
and ignored carry output. Keep memory/variable operands, WORD operations,
other registers, shifts, helpers and indexed address construction on their
current paths. This slice widens the setup gap G; the existing tail vocabulary
and window limits stay unchanged.

```text
store.b H, A                 store.b H, A
G                           X = move.b A
X = load.b H                G
T                           T
call (... X ...)            call (... X ...)
```

The bitwise operation remains in G at its original position, with its original
operands and result. Its immediate instruction defines N/Z, preserves C/V and
neither reads nor clobbers X. Identity masks still emit a real instruction;
do not delete one while relying on its flag definition. Use shared typed
effects and the existing driver/physical-memory proof. Inserting TAX still
requires N/Z dead after the retained store, and removing LDX separately
requires its N/Z definition unobserved or restored by T. The new operations
do not waive either obligation or any explicit callee contract.

Keep home equality, ordinary RAM, complete physical write ranges, incoming X
deadness, X survival, bounded regions, callee-body witnesses and final input/
exit contracts. The proof records whether it used the basic or bitwise gap
policy; driver reconstruction must use that policy and recheck the actual
operations. Existing restricted argument staging keeps the basic policy.
No optimizer consumes printed IR or source syntax.

Introduce `enable_argument_x_bitwise_forwarding`, initially false in both
config constructors, under the existing peephole/argument-X switches. Off
reproduces the baseline, including precise callee entry proofs. On shares the
same discovery and costed batch. Enable only the optimized default after
qualification; the conservative default remains off. Reuse the existing
strict final XEX, no routine/home growth, physical replay and rollback gates.
No additional full materializations or per-site emission trials are needed.

## Confirmed candidates

`Tag` contains a retained high-byte store, a later `XOR #$7F`, and a zero-page
X reload before the call. Its final Y load restores flags. This is a candidate
for one byte/one cycle in each TN variant/runtime, subject to the full proof
and image costing. `Sort` retains an unsupported indexed setup and is separate
work. The nine unknown callee-entry cases retain their current eligibility.

The neutral fixture uses `Observe(p^, tag OP constant)` for each operation,
with a real callee recording the CARD, BYTE and flags. The frozen optimized
MIR confirms private high-byte captures and reloads for all three operations.
Keep normal preceding optimizations enabled; supplement source fixtures with
typed MIR for shapes that source normalization removes.

## Slice 1 Freeze controls and model immediate operations

1. Save compiler/source/dependency hashes, images, assembly, MIR and frame/
   storage facts under ignored `build/argument-x-bitwise/before/`, with a
   compact manifest in `docs/benchmarks/6502-argument-x-bitwise/`. Reuse the
   preceding corpus artifacts only after checking hashes and current inputs.
2. Add `fixtures/runtime/argument_x_bitwise.act`, an inert typed predicate
   and focused classification tests. Keep selection unchanged.
3. Add integration and VM targets `mir6502_argument_x_bitwise` and
   `argument_x_bitwise`. Confirm all three optimized misses and freeze both
   runtimes at `$3000` and `$41C7`; include raw/optimized NIR and actual
   LF/CRLF parsing in execution checks.

Acceptance: the baseline images match, each operation is represented and
unsupported operand/register/width/carry forms reject.

Suggested commit: `Freeze immediate bitwise argument forwarding controls`.

## Slice 2 Select and qualify bitwise gap forwarding

1. Add the independent default-off control and an explicit gap policy to the
   existing proof/discovery path. Keep the basic entry point used by earlier
   restricted staging and its existing tests.
2. Admit only the three immediate forms in G. Carry the policy in the typed
   witness, reconstruct it in driver validation and reuse the current batch,
   proposed-callee facts, final physical replay and cost gates.
3. Test all operations and immediate boundaries `$00/$01/$7F/$80/$FE/$FF`,
   all byte inputs/forwarded bytes in focused emitted execution, both incoming
   C/V states, and identity masks. Compare an independent host oracle, ABI
   bytes and restored A/X/Y/N/Z/C/V/SP. Include live N/Z, X inputs/clobbers,
   writes aliasing H, stale/altered witnesses and unsupported tails/forms.
4. VM tests compare retained writes, ordered indirect reads, guards and return
   state across both settings, NIR modes, runtimes/origins and LF/CRLF text.
   Batch alias and feature-composition cases instead of multiplying every
   boundary dimension. Check report modes, repeated selection and rollback.

Acceptance: neutral routines remove one X reload each while retaining stores
and homes; negative cases retain their strategies. Defaults remain off.

Suggested commit: `Forward argument bytes across immediate bitwise setup`.

## Slice 3 Measure and qualify the optimized default

Add an opt-in `argument_x_bitwise_measure_corpus` exporter using normal linked
compilation. Compare only the new flag across the previous 32 runtime/origin
pairs and the new neutral fixture in both runtimes/origins. Write exact XEX,
assembly, MIR and maps under ignored `build/argument-x-bitwise/measured/` and
commit compact results with input/image hashes, sizes, frames, reservations,
zero page, applications/blockers, loads/stores, selector costs and timings.
Historical exporters and baseline-sensitive helpers pin the new control off.

Execute the exact TN/TNDBG images, including `Tag` behavior, panel/dispatch
coverage and existing counted-string/overlap/parameter-home alias controls.
Use `ACTIONC_ARGUMENT_X_BITWISE_MEASURED_IMAGES` to bind fresh neutral and TN
compilation to these images. Confirm the sampled Tag/neutral saving and keep
`Strcat` as an unchanged control. Require no corpus growth, unchanged storage
capacities and final input/exit contracts. Report remaining blockers without
assuming the candidate saving must materialize.

Run the full affected backend qualification: filtered MIR6502 library tests,
all `tests/mir6502*.rs` integration targets, the 169 emission/load fixtures,
existing argument/pointer/callee-entry runtime targets plus the new target,
TN dispatch, LF/CRLF MADS reassembly/relocation, scoped arithmetic/indexed/
indirect-field runtime/selection checks and frozen Strcat static/VM audits.
Avoid unfiltered VM Cargo fixture wrappers. Preserve the known indexed-byte
selection-count discrepancy separately from runtime correctness.

Use `CARGO_INCREMENTAL=0` to bound build artifacts. Development checks are
scoped to the new targets, argument-X analysis/materialization and the rewrite
driver. No SemIR/NIR/verifier/printer contract change is planned; if those
contracts change, also run the shared checks required by `AGENTS.md`.

Enable the optimized default only after qualification. Update the existing
pseudo-machine contract and documentation index, record results here and
leave unrelated working changes untouched.

Suggested commit: `Qualify immediate bitwise forwarding and enable defaults`.

## Completion record

| Slice | Status | Evidence |
| --- | --- | --- |
| 1 Baselines and typed forms | Complete | Frozen controls, typed classification, integration and independent execution |
| 2 Selection and boundary checks | Planned | |
| 3 Corpus and default | Planned | |

Slice 1 freezes eight controls in
[baseline.json](benchmarks/6502-argument-x-bitwise/baseline.json).
The neutral images are 205 bytes in either runtime/origin. Two focused tests
confirm all three immediate forms and reject unsupported shapes while the
normal proof still rejects their gaps. Integration and 1,152 VM calls pass,
including raw/optimized NIR, both runtimes/origins and actual LF/CRLF source
parsing. Each source word read remain ordered, once per byte, with
independent bitwise results and preserved C/V.
