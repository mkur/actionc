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
