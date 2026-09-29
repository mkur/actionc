# Native forwarding wrappers

Compiler baseline `4bf1b101`; candidate `8b9799b6`. Exec remains at
`134130b`, with the same 133 generated source/layout inputs. This is a
compile-only comparison using a local compiler override; the recorded compiler
pin, ROM/emulator configuration and play image have not changed.

| Measurement | Before | After |
| --- | ---: | ---: |
| Generated Exec routine code, 960 routines | 489,875 bytes | 485,991 bytes |
| `IsMinListEmpty` code | 94 bytes | 4 bytes |
| `IsMinListEmpty` frame / local peak | 8 / 14 bytes | 0 / 0 bytes |
| `NewMinList` code | 90 bytes | 4 bytes |
| `NewMinList` frame / local peak | 8 / 14 bytes | 0 / 0 bytes |

Forty-one wrappers shrink, saving **3,884 bytes**. No other routine changes
size; 39 frames shrink, none grow. Every selected wrapper is exactly a far
jump to a retained routine entry, with no local storage or call reservation.
All 960 maps pass Exec's frame-map validator. These are routine code sizes,
not packed XEX/container sizes or a hosted-system qualification.

Both list wrappers now have the expected bodies:

```asm
IsMinListEmpty:
    JML IsListEmpty

NewMinList:
    JML NewList
```

The largest individual saving is 128 bytes in each of `DOS.Seek` and
`PROGRAMAPI.Seek` (132 to 4 bytes). Chains such as
`PROGRAMAPI.Seek -> DOS.Seek -> DOSCALLS.Seek` retain separate public addresses
and share the original caller's argument area and return address.

Notable fallbacks retain their existing code sizes: `PROGRAMAPI.Yield` targets
an external Exec gateway, so it stays an ordinary call (73 bytes).
`PROGRAMAPI.ReadArgs` obtains arguments, parses them, sets IoErr and transforms
the result; it performs work beyond forwarding (293 bytes). Their ordinary
instruction selection, guards and native gateway contract remain unchanged.

Reserved bank-zero delta: **0 fixed bytes and 0 bytes per task**. The local
stack savings do not change stack reservations, interrupt headroom or a
callee's stack requirements. Whole-task stack bounds remain unknown.

The [per-wrapper table](exec-routines.csv) lists all 41 targets, byte counts,
frames and local peaks. [Totals](exec-results.json) and
[provenance](provenance.json) retain the compiler revisions, executable/image
hashes and source/layout digest. Digests hash sorted relative paths as path,
NUL, content SHA-256 and LF. The source input list and module paths match the
[previous measurement](../../MIR65816_STORAGE_DEMAND_PLAN.md#measurements).

Reproduce from the unchanged generated Exec inputs:

```sh
../actionc-public-release/target/debug/actionc-65816 \
  --layout build/demo/layout.json \
  --module-path build/demo/task-kernel --module-path build/demo \
  --module-path examples/shell --module-path lib/exec --module-path lib/dos \
  --module-path lib/fs --module-path lib/console --module-path lib/mydos \
  --module-path lib/spartados --module-path lib/io \
  -o /tmp/exec816-forwarding-wrappers.a816.json build/demo/kernel-program.act
```

Validation: 322 MIR65816 unit tests (one existing ignored), 39 emission/o65/
snapshot integrations, and 21 focused native runtime tests. Raw and optimized
execution covers argument identity/padding, native result lanes, indirect
entry to wrappers, cross-bank jumps, two o65 placements, stack-floor/ceiling/
underflow checks and IRQ/NMI restoration in both task domains. See the
[implementation plan](../../MIR65816_FORWARDING_WRAPPERS_PLAN.md#slice-results).
