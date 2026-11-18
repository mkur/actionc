# ZP homes across arithmetic helper calls in MIR6502

Proposed against `cb6849ae`. Deliver in three separately tested commits.

## What should work

In TN's `Init`, a local `CARD h` supplies the operand of `h RSH 5` and
is incremented after the shift. Both bytes of `h` are live across the call.
The cartridge `RShift` takes the word in A/X, reads the count from `$84`,
writes scratch at `$85` and returns the word in A/X. The caller stores
the count to `$84` before loading the operand. A home such as `$E2/$E3`
can survive that sequence. [Helper signatures and effects](../src/mir6502/runtime_contracts/signatures.rs) · [Argument staging and result capture](../src/mir6502/materialize/runtime.rs)

These are measured working-tree TN cartridge builds at `$3000`. Each variant
changes just the shift count in a temporary copy of the same `LIB.ACT`.

| Expression | Current lowering | Current home of `Init.h` | XEX bytes |
| --- | --- | --- | ---: |
| `h RSH 5` | `JSR $A0E6` | `$339E/$339F` | 9,839 |
| `h RSH 6` | `JSR $A0E6` | `$339E/$339F` | 9,839 |
| `h RSH 8` | `LDA $E3` | `$E2/$E3` | 9,820 |

The allocator inspects a retained MIR routine where the shift is still
`Binary(Rsh, Word)`. Its cross-call analysis already understands explicit
`RuntimeHelper` operations. The fix is to make the helper selected for an
arithmetic operation visible to that analysis, including the instructions that
prepare its arguments. The first implementation covers constant word shifts
in either direction that select a single `LShift` or `RShift` call. Both
counts 5 and 6 exercise the same rule. Native shifts keep their existing
recipes, including high-byte extraction and the four-bit unrolling limit.
[Home admission](../src/mir6502/parameter_homes/admission.rs) · [Cross-call planning](../src/mir6502/parameter_homes/cross_call.rs)

## Slice 1 — Share the selected expansion

1. Reuse the materializer's word-shift recipe selection, typed helper
   selection and argument/result staging. Give home planning a read-only
   expansion for a retained shift whose fallback is determined. It should
   contain the same ordered MIR operations the emitter path uses, with the
   actual `RuntimeHelper` instruction between setup and result capture.
   Keep contracts in the existing helper catalog.
2. Respect production selection order and configuration. A widened byte
   shift, direct high-byte load, or fused store can supersede the general
   helper. Only expose a helper expansion when the shared selector can
   establish it for this operation and its operands. Leave ambiguous shapes
   on their current path. Finding a helper declaration elsewhere in the
   program is insufficient evidence for a particular shift.
3. Test selection on a full-word operand for counts 5, 6 and 7 in each
   direction. Include a count-4 native shift, `RSH 8`, a widened-byte case,
   and disabled helper selection. Compare setup, invocation and result
   capture with what materialization emits. This refactor should produce
   byte-identical TN images before enabling admission.

## Slice 2 — Use the expansion in ZP home planning

1. In cross-call discovery, substitute the qualified expansion in a
   temporary planning view of the routine. Run helper enumeration and
   logical liveness on that same view. Validate the invocation with
   `helper_invocation` against the current selected executable. Count each
   actual call once, including a routine whose only call comes from
   arithmetic. Apply proposed bindings to the retained routine by parameter
   or local ID. Operation indices belong to their own routine snapshot.
   [Invocation validation](../src/mir6502/runtime_contracts/invocation.rs) · [Logical liveness](../src/mir6502/parameter_homes/logical.rs)
2. Account for the caller's ordered setup writes, the helper's returning
   writes, and result capture. In particular, a word overlapping `$84` can
   lose an operand byte *before* the helper reads A/X, even if the source
   value dies at the call. A word live across the helper must also survive
   the write to `$85`. Derive these exclusions from the staged operations
   and contract. Preserve the existing source-address and pointer checks.
3. Reuse the existing combined placement trial. Recompute contracts,
   liveness and physical conflicts on the resumed candidate and accept only
   a strictly smaller complete image. Cover missing or stale bindings and a
   forced candidate-time conflict, including exact restoration of the
   previously selected image and bindings. Exercise helper-only, mixed
   ordinary/helper calls, repeated helper sites and the existing feature
   gates. [Residence verification](../src/mir6502/parameter_homes/lifetimes.rs) · [Trial and rollback](../src/mir6502/parameter_homes.rs)

## Slice 3 — Qualify the placement on TN

1. Execute a word-shift control on cartridge and standalone runtimes. Cover
   counts 5 and 6, both result bytes, high bits crossing the byte boundary,
   a live neighboring word, and a byte-array store consuming just the low
   result. Exercise an intentionally conflicting home in a focused MIR test.
   [Home-admission VM tests](../tools/vm-runtime-tests/tests/home_admission.rs)
2. Build and execute the three TN variants above on the same source and
   settings as their respective baselines. For each of the 24 rows, check
   the first table byte against the low byte of `40 * row` and the second
   against the low byte of `(40 * row) RSH count`. The existing TN dispatch
   test exercises the production count of 8. Keep 5 and 6 as temporary
   variants and normalize host newlines if the test instruments source.
   Verify LF and CRLF through that path. [TN VM controls](../tools/vm-runtime-tests/tests/tn_dispatch.rs)
3. For 5 and 6, demonstrate that the emitted code still calls `RShift`,
   `Init.h` receives a safe two-byte ZP home, and the image shrinks against
   its own 9,839-byte baseline. Record the measured address, `Init` code
   size and packaged size. The 19-byte difference from `RSH 8` includes
   the cost of the helper invocation. Use the production `RSH 8` binary
   as a byte-for-byte control.
4. Run the focused MIR6502 helper-contract, shift, and home-placement unit
   tests, the parameter-home and pointer integration targets, and the
   affected VM targets. Compare the broader home suite with the recorded
   pre-change carry-flag failure in `handoff_tests`. Update the home
   selection design with the qualified scope and measurements.

The same interface can subsequently cover variable shifts, multiply and
divide when their selected expansions are qualified. Division in particular
can select different helpers or fuse quotient and remainder, and has a
separate fault path. Each extension should demonstrate the chosen expansion
and its effects before widening admission. [Runtime helper contract design](MIR6502_RUNTIME_HELPER_CONTRACT_DESIGN.md)
