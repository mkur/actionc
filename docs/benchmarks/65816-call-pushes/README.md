# Native outgoing argument pushes

Direct native calls may construct their existing outgoing argument area from
high addresses down using exact BYTE/word PHA chunks. Complete-call selection
accounts for padding, width changes and final A16 restoration, and chooses
pushes only when smaller than reservation/stores. Argument evaluation, public
ABI layout, cleanup and the complete pre-write stack guard remain unchanged.

The internal `ArgumentPush` instruction distinguishes outgoing payload from
indirect-transfer pushes. Its physical effects are ordinary PHA effects;
tracked depth, selected-CFG validation and replay retain that distinction.
Direct call summaries also retain the checked outgoing extent and verify it
against the actual depth before JSL. All caller-home reads use the current S.

## Slice 4: exact BYTE/word operands

Captured and numeric operands with exact one/two-byte ABI widths, including
empty calls, are eligible. Wider, symbolic and mixed-width-extension operands,
and all indirect calls, retain their previous construction.

Against `8824f00b`, the same frozen Exec workload (631 routines and 120 verified
input hashes) shrinks **385,714 → 380,648 B**: **5,066 B saved**. The 689 changed
call spans save 5,050 bytes; 16 BRL-to-BRA relaxations save the rest. There are
254 smaller routines and no larger ones. Frames, temporary homes, public ABI
metadata, local stack peaks, initialized data and all 2,676 guards are unchanged.
The guards still occupy 72,252 bytes; guard-subtracted code is **308,396 B**.
This is an inventory build, not a separately compiled guard-disabled release.

[Size and hashes](byte-word/exec-summary.json),
[routine deltas](byte-word/exec-routines.csv), and
[changed spans](byte-word/exec-spans.csv) retain the evidence.

Validation: 14 focused emitter call tests, 37 selected analysis tests, 33 native
emission/o65 integration tests and the reviewed snapshot pass. The snapshot's
eight changed call constructions were independently symbolically executed to
check identical payload/padding and final S; frame contracts and other spans
agree after position remapping. The snapshot reader passes with LF and CRLF.
The stack oracle enumerates 32 mixed layouts independently of selector choices.

Twenty-two native debug tests pass across call pushes/padding/copies/results,
interop, replay and state tracking. Five push/padding tests pass in release.
Independent assembly observes the full area, including maximum displacement,
padding canaries and retained caller storage. Guards fail before any write for
empty, word and mixed calls. Both frontend modes, LF/CRLF source compilation,
two o65 placements and indirect fallback are covered. IRQ/NMI injection checks
each reached construction instruction in both task domains and I states against
independent execution, including reentry of the same routine from the dispatcher.

Full/final backend and hosted Exec qualification were not run.

## Slice 5: mixed native widths

Exact three/four-byte captures and numeric constants now participate in the
same complete-call plan. Chunks stay inside their source argument; a pointer
uses exactly three bytes and never reads a fourth. Width-mismatched operands
and symbolic byte fixups still retain full reservation/stores.

Against `348f2f04`, frozen Exec shrinks **380,648 → 370,027 B**: **10,621 B**,
including 10,583 bytes in 1,351 call spans and 38 branch relaxations. All 120
input hashes, frames, temporary homes, ABI metadata, initialized data and guards
are unchanged; guard-subtracted code is **297,775 B**. No routine grows.
[Size and hashes](mixed/exec-summary.json), [routines](mixed/exec-routines.csv),
and [spans](mixed/exec-spans.csv) retain the evidence.

Validation: 14 emitter call tests (including 1,024 independently decoded mixed
layouts), 22 emission and 11 o65 integration tests, and the unchanged boundary
snapshot pass. Eighteen focused native debug tests cover copies, padding,
pushes, replay and state tracking; five push/padding tests pass in release.
IRQ/NMI injection now covers CARD, ADDRESS and LONGCARD construction at every
reached instruction, both task domains, both I states and both frontend modes.
The independent callee observes exact pointer writes and padding, including
bank-boundary source canaries and maximum outgoing displacement. Full/final
qualification was not run.

## Slice 6: symbolic byte relocations

Exact-width static/global/routine addresses now use their existing LDA byte
fixups in complete push plans. Symbols are never packed into word relocations.
Symbol-plus-offset producers keep their checked addends and push the captured
three-byte value through slice 5's path. Width mismatches still fall back.

This slice changes **zero bytes** in frozen Exec: its symbolic call operands
were already captured before the call. Code remains **370,027 B**, with unchanged
frames, guards, data and all 120 input hashes. [Evidence](symbolic/exec-summary.json).
Phase 2 in total selects **2,040 of 2,045 calls** and saves **15,687 B**: 15,633
in call spans and 54 from branch relaxation. The 15,695-byte call-only model
exceeded the measured call saving by 62 bytes: four direct calls have U24 numeric
operands in word ABI slots and intentionally retain the mismatch fallback.
The fifth unchanged call is indirect. No profitability fallback remains in
this workload among the admitted exact-width direct calls.

Validation: 15 emitter call tests, 11 o65 integration tests and the unchanged
boundary snapshot pass. Sixteen focused native debug tests cover argument
pushes/padding, address selection and replay; both argument-push tests pass in
release. Tests inspect all nine byte selectors/targets of a three-symbol plan,
execute static-string and symbol-plus-offset calls at two rebased o65 placements,
and preserve bank carry, neighboring canaries and exact widths. Full/final
qualification was not run.
