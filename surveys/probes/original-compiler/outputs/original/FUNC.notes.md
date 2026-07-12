# FUNC.COM Notes

Captured from `functions.act` with origin forced to `$3000`.

Load-file structure:

- code segment: `$3000-$309F` (`160` bytes)
- RUNAD segment: `$02E2-$02E3`, value `$305E`

Current `actionc --emit-load functions.act` comparison:

- compatible codegen now uses inline routine storage followed by `JMP body`
  trampolines, matching the broad original layout.
- RUNAD now points at `Main`'s trampoline instead of its body.

Important deltas observed:

- The original stores globals and routine parameter frames inside the emitted
  code/data segment near `$3000`; compatible actionc now does the same, using
  deterministic zero-filled storage bytes.
- The original emits jumps around inline storage/frame data. Compatible actionc
  now emits the same broad trampoline shape.
- Multi-byte argument functions use the runtime `SArgs` helper (`JSR $A0F5`)
  with inline frame metadata before the function body. `actionc` currently saves
  incoming bytes directly to fixed local slots.
- The simple call ABI shape is close: calls still load constants into `A`/`X`/`Y`
  and use `$A3+` for later bytes.

The next likely compatibility step is modeling the original parameter-frame
metadata and `SArgs` prologue for wider/multi-byte argument functions.
