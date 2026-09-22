# Direct native frame store/load forwarding

The emitter forwards a stored word in A16 into the next load's retained temporary
capture. Selection is independent of frontend optimization mode. This implements
the first recommendation from the [movement inventory](MIR65816_MOVEMENT_INVENTORY.md).

## Eligibility and state

Both MIR operations must be nonvolatile, two-byte, direct accesses to the same
non-addressable `AutomaticFrame` object and byte displacement. The store source
must be a word temporary; the load destination must have a complete stack home.
The selector checks object extent, both stack-relative bytes and the retained
destination store before requesting omission. Incoming parameter reloads and
address-taken, external, indexed, indirect, byte and wider accesses are excluded.

The existing state tracker uses a distinct `Frame(object, displacement)` identity
alongside `Temp(id)`. The witness requires A16, full-word N/Z, the exact physical
home and generation, zero transient S displacement and an unchanged instruction
and label cursor. A matching numeric address does not substitute for identity.
Overlapping writes, unknown writes, calls/helpers, other operations, labels,
width changes and stack movement invalidate the witness or make its cursor stale.
The witness is consumed once. The retained load capture can then publish the
existing temporary witness for arithmetic, comparison, store or return.

For the measured rotation, the final sequence changes from:

```asm
LDA $0A,S
STA $02,S
LDA $02,S       ; omitted
STA $0C,S
```

Both stores and all homes remain. The rule does not allocate registers across
calls or joins, change DP use, move memory effects or remove stack guards.
The ABI remains `action65816.native.v1`, image version 3 and o65 profile version 1.

## Qualification

The [frozen forecast](benchmarks/65816-frame-forwarding/baseline.json) identifies
one instruction in the qualified selective-staging image. The
[exact checker](../tools/compare65816/check_frame_forwarding.py) constructs the
expected complete image by deleting that LDA and relocating later positions.
It checks all executable segments, including the uncounted driver, all metadata,
all 28 Action images, vbcc artifacts and every measurement field. Existing
adjacent-temp forwarding counters keep their meaning; `frame_forwarded_loads`
and `frame_forwarded_load_sites` report the new proof family separately.

The test-only frame index combines verified MIR object facts and operation spans
with independently decoded final instructions. It walks only retained word
stores back to a word LDA/ADC/SBC and rejects labels inside the proof window.
Byte mutations invalidate each proof. The VM checks A against the stored object
and checks complete N/Z at every reached forwarding boundary.

Generated raw/optimized probes exercise zero, sign and wrap boundaries with both
incoming IRQ masks, flat images and two o65 placements. Tracker trace-on/off
checks cover bytes, labels, spans, fixups and actual home generations through
rotation loops, including rebased o65 execution. Two-task probes inject IRQ and
NMI at each retained store, independently execute the interrupted instruction,
and require exact restoration of every register and the live invocation frame.
Seeded IRQ/NMI schedules also retain correct task results. Negative selector and
state tests cover identity, generation, partial writes, aliases, volatility,
flags, mode, labels, calls, transient S and source/destination extents.

The historical movement inventory remains frozen; its incoming-parameter
candidates and edge coalescing are separate future slices.
