# Native address consumers

Baseline compiler `979205e4`; candidate is the four address-consumer slices.
Exec remains at `134130b`, with the same 133 generated source/layout inputs.
This compile-only comparison uses a local compiler override. The compiler pin
and play image remain unchanged; this is not hosted-system qualification.

| Measurement | Before | After |
| --- | ---: | ---: |
| Exec routine code, 960 routines | 485,991 bytes | 479,099 bytes |
| NewList code | 133 bytes | 120 bytes |
| NewList frame / local peak | 12 / 12 bytes | 0 / 0 bytes |
| AddHead code / frame | 170 / 16 bytes | 162 / 8 bytes |
| AddTail code / frame | 182 / 16 bytes | 183 / 8 bytes |
| Insert code / frame | 369 / 16 bytes | 361 / 12 bytes |
| Enqueue code / frame | 373 / 20 bytes | 361 / 12 bytes |

Net saving: **6,892 bytes** (about 1.42%). 405 routines shrink and 461 frames
and local stack peaks shrink. No frame or local peak grows. All 960 final
maps pass Exec's frame-map validator. These are routine code sizes, excluding
data, alignment and container overhead; they do not imply a whole-task bound.

NewList loses its temporary frame and stores its computed address from registers.
Its [generated assembly](newlist.asm) retains the existing 28-byte entry guard
and stages the base again after each unresolved indirect store. Such a store
may alias the cached pointer bytes, so this series adds no nonaliasing assumption.
Internal address expressions currently use the same canonical A/X result
schedule as wide returns; narrower internal register schedules remain possible
future work. No source-name or list-layout special case is used.

Ten routines grow, by 32 bytes in total:

| Routine | Before | After |
| --- | ---: | ---: |
| TASKPOLICY.DosPrepare | 434 | 442 |
| O65WIRE.Header | 3,040 | 3,048 |
| O65RELOCATIONS.Walk | 3,726 | 3,734 |
| TASKPOLICY.SetState | 130 | 132 |
| SIODRIVER.PrepareWaiter | 600 | 601 |
| SDFSFILE.Begin | 282 | 283 |
| FSPORTS.Create | 362 | 363 |
| EXECLISTS.AddTail | 182 | 183 |
| DOSRAW.Initialize | 598 | 599 |
| DEMO.ShellByte | 222 | 223 |

The [complete table](exec-routines.csv) includes unchanged and growing routines,
frames, spills and local peaks. [Totals](exec-results.json) and
[provenance](provenance.json) record compiler source/executable hashes and both
image hashes. Digests hash sorted relative paths as path, NUL, content SHA-256
and LF. The input list and module paths match the
[previous measurement](../65816-forwarding-wrappers/README.md).

Reproduce from Exec's unchanged generated inputs:

```sh
../actionc-public-release/target/debug/actionc-65816 \
  --layout build/demo/layout.json \
  --module-path build/demo/task-kernel --module-path build/demo \
  --module-path examples/shell --module-path lib/exec --module-path lib/dos \
  --module-path lib/fs --module-path lib/console --module-path lib/mydos \
  --module-path lib/spartados --module-path lib/io \
  -o /tmp/exec816-address-consumers.a816.json build/demo/kernel-program.act
```

Validation: 329 MIR65816 unit cases (one existing ignored), 39 emission/o65/
state-boundary integrations and 59 focused native cases. Native checks cover
raw/optimized execution, fixed and relocated images, bank carry/wrap, exact
three-byte stores, argument/result lanes, mutable pointer snapshots, source/
scratch invalidation, stack guards, LF/CRLF input and two-task IRQ/NMI reentry.
The selected runtime targets are `address_consumers`, `address_selection`,
`home_demand`, `pointer_forwarding`, `pointer_values`, `call_pushes`,
`call_returns`, `call_copies`, `stack_checks`, `stack_faults`, `memory`,
`indirect`, `wide_returns`, `terminal_pointer_stores` and `terminal_pointer_calls`.

Reserved bank-zero delta: **0 fixed bytes and 0 bytes per task**. Existing task
stack reservations, DP regions, native ABI and interrupt headroom are unchanged.
See the [implementation plan](../../MIR65816_ADDRESS_CONSUMERS_PLAN.md).
