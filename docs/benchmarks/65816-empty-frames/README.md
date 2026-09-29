# Empty-frame entry guards

The compiler now omits entry instructions when the final allocated frame is
empty. Nonempty frames and every call reservation retain their checks. This
also applies to zero-frame arithmetic helpers and unchecked builds. Typed
emission and replay establish the body from the unchanged native-entry S,
without an artificial TSC or an invented accumulator value.

This intentionally removes invalid-entry-S diagnostics from empty leaves.
The native caller or platform stub owns entry validity, return records and
arguments; interrupt headroom remains reserved. No ABI layout or bank-zero
reservation changes: **0 fixed bytes and 0 bytes per task**.

The baseline is actionc `6455ebc7`, with the same Exec revision `134130b` and
133 generated source/layout inputs as the
[address-consumer measurement](../65816-address-consumers/README.md).

| Measurement | Before | After |
| --- | ---: | ---: |
| Exec routine code, 960 routines | 479,099 bytes | 478,763 bytes |
| NewList code | 120 bytes | 92 bytes |
| NewList frame / local peak | 0 / 0 bytes | 0 / 0 bytes |

Twelve routines each lose 28 bytes, saving **336 bytes**. No routine grows.
The other 41 empty-frame routines were already forwarding jumps. All frame
maps, spills, incoming offsets and local stack peaks are unchanged; all 960
maps pass Exec's validator. Counts exclude data, alignment and container costs.
The compiler pin and play image remain unchanged. This is a compile-only local
compiler override, not hosted-system qualification.

The [routine table](exec-routines.csv) lists every changed routine;
[provenance](provenance.json) records compiler/source and image hashes. Reproduce
with the same module paths and compile command as the preceding measurement,
using `/tmp/exec816-empty-frames.a816.json` as the output.

Focused validation covers raw/optimized empty leaves and helpers at the stack
floor, exact return/argument bytes, caller guards, nonempty-frame floor/wrap/
ceiling faults, relocated guards, divide-by-zero faults and IRQ/NMI reentry.
The byte/proof snapshot changes only for zero-frame routines; existing guard
tests continue to compare every retained guard with independent assembly.

Development checks: 333 MIR65816 unit cases across the backend batch and focused
follow-ups (one existing ignored), 39 emission/o65/state-boundary integrations,
and 28 native cases across `address_consumers`, `arithmetic_helpers`,
`call_pushes`, `guard_branches`, `stack_checks` and `stack_faults`. Retained
guards keep their independent assembly, floor/ceiling/wrap and interrupt checks.
The arithmetic target also had two stale SIZE multiplication oracles corrected
to the existing unsigned 24-bit language rule; production arithmetic is unchanged.
