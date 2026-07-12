# SemIR Native Emission Layer Plan

Owner: `NativeTrackedEmitter` and opcode-specific emitter helpers.

Emission answers which exact 6502 bytes are written and how tracked processor
state changes.

## Responsibilities

- opcode selection;
- absolute versus zero-page instruction forms;
- label binding and patching;
- register, flag, and memory-state tracking;
- raw data barriers;
- source-range and map recording;
- proof and guardrail hooks tied to concrete emitted code.

Emission should not decide whether a SemIR expression is an array, pointer
deref, record field, call result, or computed value. Those decisions belong to
classification and materialization. Emission should receive concrete actions:
load this byte source, store A to this address, write this JSR, bind this label,
or patch this byte.

## Current Direction

SemIR-native already routes instruction writes through `NativeTrackedEmitter`
and local emitter helpers. The next work is mostly boundary hygiene: keep shape
decisions out of emission and keep concrete opcode choices close to tracked
state updates.

Initial execution has started in `src/codegen/semir_native/native_emit.rs`.
The concrete address-form helper family and Y-register state helpers have moved
there without behavior changes. X/Y load-store address helpers now also route
through tracked zero-page-aware facades. High-level SemIR-native storage loads
and stores in `semir_native.rs` and `native_materialize.rs` now use these
address helpers instead of direct absolute opcode calls; remaining direct
zero-page ABI byte moves for `ARGS`, `ARRAY_ADDR`, `ELEMENT_ADDR`, and
`AFCUR` are now centralized in `native_emit.rs`. Remaining high-level
runtime zero-page references are mostly indirect indexed array-element
instructions and descriptor construction. Inline-array indexed loads/stores now
route through indexed address helpers, currently preserving absolute-indexed
opcode form to avoid zero-page indexed wraparound changes. Branch and jump
label emission now has named helpers in `native_emit.rs`, so condition and loop
lowering no longer spell branch opcodes directly. Absolute `JMP`, routine-entry
`JSR`, and runtime helper `JSR` emission also route through `native_emit.rs`,
including the conservative native Y-state invalidation after calls. Raw byte,
word, label, and zero-fill emission for storage, descriptors, helper operands,
string literals, and machine blocks now use explicit raw emission helpers.
Immediate A/X/Y loads, `RTS`, carry setup, and immediate `ADC`/`SBC` also route
through emission helpers. Compare/EOR immediates, AND/ORA immediate and address
forms, `TAX`, and accumulator ASL are also centralized. The remaining direct
ordinary instruction calls are mostly the tightly-coupled materialization
sequences for pointer-address staging, carry-preserving shifts, Y restoration,
and stack save/restore. These final materialization sequences now also route
through emission helpers, leaving direct `self.emitter.emit_*` calls contained
in `native_emit.rs` for the high-level SemIR-native backend files.

Current state: strong plateau. Continue here only for focused missing helpers,
tracked-state corrections, or raw-data/label guardrails discovered by another
layer. The stress backlog should normally be implemented in materialization;
emission changes should be helper extraction or state tracking needed by those
materializers, not semantic shape support.

Near-term plan:

1. Audit direct `self.emitter.emit_*` calls in `semir_native.rs` and
   `native_materialize.rs`.
   - Leave raw data/storage writes alone when they are intentionally bytes,
     labels, or map/source-range payloads.
   - Prefer helper wrappers for ordinary instructions whose opcode form depends
     only on concrete address/register operands.
2. Finish the absolute-versus-zero-page helper family.
   - Existing helpers cover `LDA`, `STA`, `STY`, `ADC`, `CMP`, `EOR`, `INC`,
     `DEC`, `SBC`, `LSR`, and `ROR` address forms.
   - `LDX`, `LDY`, and `STX` address helpers now exist for SemIR-native call
     and parameter paths.
   - Add helpers only when there are at least two callers or when direct calls
     obscure tracked-state behavior.
3. Normalize register and zero-page ABI moves.
   - Concrete helpers now cover repeated `ARGS`, `ARRAY_ADDR`, `ELEMENT_ADDR`,
     and `AFCUR` byte moves when they are pure emission operations.
   - Keep value-shape staging decisions in materialization.
4. Strengthen tracked-state guardrails.
   - Ensure every ordinary instruction still passes through
     `NativeTrackedEmitter`.
   - Add lightweight tests or source guardrails for any new raw-byte escape
     hatch.
5. Centralize branch, label, and patch emission where possible.
   - Named branch and jump helpers now keep common branch opcodes in emission.
   - `JSR` helpers now keep routine/runtime-helper call emission and Y-state
     invalidation together.
   - Keep label binding, relative-branch patching, and unresolved-label
     diagnostics owned by emission.
   - Do not mix semantic shape decisions into branch helpers.
6. Preserve map, proof, and raw-data barriers.
   - Storage bytes, machine blocks, literal data, and descriptor operands now
     route through explicit raw emission helpers.
   - Keep tracked processor state conservative across raw data and machine
     blocks.
7. Support stress-backed materializers with focused concrete helpers only when
   repeated or state-sensitive:
   - runtime helper call sequences for word `*`, `/`, `MOD`, `LSH`, and `RSH`;
   - bytewise word logic helpers if materialization repeats `AND`, `ORA`, or
     `EOR` load/store patterns;
   - stack save/restore and zero-page pointer preservation helpers when
     materializers must protect `ARRAY_ADDR` or another pointer home;
   - builtin/runtime/indirect call target emission once call materialization
     owns argument packing.

Suggested first slices:

1. Move the local address-form helpers (`emit_lda_addr`, `emit_sta_addr`,
   `emit_adc_addr`, and siblings) into an emission-focused module or section
   without changing behavior. Done in `native_emit.rs`.
2. Add tests/guardrails that ordinary instruction helpers remain tracked while
   storage/raw-data emission remains allowed.
3. Replace repeated direct `emit_lda_abs`/`emit_sta_absolute` call pairs with
   concrete helpers where the only decision is zero-page versus absolute. Done
   for current high-level SemIR-native storage paths.
4. Audit `native_materialize.rs` after those helpers move, and switch
   materializers to the new emission helpers in small commits. Initial
   materializer storage loads are now routed through address helpers.
5. Tackle branch/label helpers after address/register helpers are stable.
6. For stress backlog slices, add emission helpers only after a materializer
   exposes repeated concrete instruction sequences or tracked-state behavior
   that would be easy to get wrong locally.

## Boundary Checks

Emission code should not:

- inspect SemIR to classify value shapes;
- choose semantic lowering strategies;
- reimplement materialization staging;
- bypass state tracking for ordinary instructions.

Emission code may:

- choose zero-page versus absolute opcode forms for known addresses;
- maintain known-register and known-memory state;
- bind labels and record relocations;
- expose small helper operations for materializers and lowerings.
