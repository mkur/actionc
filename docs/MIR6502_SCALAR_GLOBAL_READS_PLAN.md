# Scalar capture forwarding across ordinary global reads

Implement the next MIR6502 capture-forwarding slice, then commit it with its
regressions and measurements.

1. Preserve existing captured values across direct scalar loads wholly within
   a named global with ordinary backing. Use checked offset/width bounds.
2. Keep absolute and aliased globals, missing objects, out-of-bounds accesses,
   indirect loads, global writes, calls and all existing barriers conservative.
   Do not broaden store/home deletion or pointer/escape proofs.
3. Keep the original read and its evaluation order. Forward only the existing
   immutable/unique-definition captures, within the existing 32-operation bound.
4. Add focused positive/negative MIR regressions and a runtime case with a
   captured code address followed by a global argument read. Cover raw and
   optimized NIR, both runtimes and LF/CRLF source parsing.
5. Run affected MIR6502 checks and compare maintained TN/TNDBG and the existing
   menu-variant experiment against commit `74132299`. Retain the emitted-size,
   affected-routine, zero-page and alias gates. No shared IR contract changes.

The concrete target is `Format`'s captured density address: the intervening
load of `ans` currently clears forwarding availability. The baseline XEX sizes
are 9,968 bytes for TN, 13,073 for TNDBG and 10,120 for the menu-variant build.

## Results

The implementation changes only the forwarding availability check for direct
scalar global loads. Reads remain in place, and existing deletion proofs and
final cost/alias checks remain mandatory. No configuration or IR changes were
needed.

The focused example, with code-address and ordinary value captures followed
by word/byte global reads, shrinks from 443 to 398 XEX bytes through the normal
`--mode mir6502 --runtime cart` CLI pipeline (optimized NIR). The VM regression
checks both runtimes, raw/optimized NIR, relocated code addresses, boundary
values, repeated calls and LF/CRLF source parsing.

Validation passed: 1,123 MIR6502 unit tests (two existing tests ignored),
96 tests across all 11 `mir6502_*` integration targets, all 169 MIR6502 emission
fixtures, and 21 VM tests covering scalar captures, direct/indirect aggregate
calls, CASE validation and TN dispatch/menu/panel behavior. No fixture outputs
or shared compiler contracts changed.

Final cartridge-runtime CLI measurements:

| Program | Before XEX | After XEX | Byte-identical |
| --- | ---: | ---: | --- |
| Maintained TN | 9,968 | 9,968 | Yes |
| Maintained TNDBG | 13,073 | 13,073 | Yes |
| Menu-variant experiment | 10,120 | 10,120 | No |

The variant's `Format` routine remains 308 code bytes, and the emitted listing
still reloads the captured density address. The broader availability proof
therefore yields no TN size saving in this slice. Its remaining overhead is
152 bytes; pointer/escape proofs and further value-materialization work remain
separate follow-ups.
