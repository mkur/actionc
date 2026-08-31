# MIR6502 General Codegen Optimization Plan

Status: active. Created 2026-08-31.

This plan turns the code-generation gaps exposed by the Action!/Mad Pascal
benchmark comparison into general MIR6502 improvements. The benchmark suite is
an observation workload, not an optimization contract: no matcher may depend
on a benchmark routine name, a particular array address, or source spelling.

The three goals are:

1. recognize normalized counted loops and select compact induction latches;
2. preserve loaded byte values and index-register values when machine effects
   and liveness prove that they remain available;
3. preserve narrow operand facts when a byte computation produces a word
   result, and feed simple word expressions directly to their consumers.

## Architectural boundary

NIR remains target-independent. It owns typed casts, arithmetic, explicit
loads/stores, and normalized CFG. It must not acquire A/X/Y placement, 6502
flags, addressing modes, or a source-level `FOR` operation.

MIR6502 owns all changes in this plan:

- counted-loop recognition operates on typed MIR CFG and never consults SemIR;
- load forwarding and register reuse use pre-home/post-home analyses;
- byte-versus-word helper choice uses types already carried by NIR and MIR;
- all rewrites use the analyzed rewrite workflow and phase verification;
- calls, machine blocks, volatile storage, absolute memory, pointer writes, and
  unknown effects remain conservative barriers unless structured effects prove
  a narrower result.

The existing contracts in `MIR6502_PSEUDO_MACHINE_CONTRACT.md` and
`MIR6502_REWRITE_WORKFLOW_PLAN.md` remain authoritative.

## Validation policy

Each implementation slice must include focused, generic coverage and one
commit. A slice is complete only after its relevant unit/integration tests and
`cargo test` pass. If a slice changes NIR lowering, optimization, verification,
or printing, it must additionally run:

```sh
cargo test nir_fixtures_match_snapshots
cargo run --bin actionc-nir-sweep -- fixtures/nir
cargo test
```

The benchmark suite is recompiled after each material code-generation slice.
Routine byte counts and selected helpers are reported, but exact benchmark
sizes are not test assertions.

Focused acceptance fixtures cover:

- ascending and descending byte loops, including zero and one iteration;
- volatile loads immediately consumed by compares and arithmetic;
- indexed byte loads sharing an index across a branch;
- unsigned `u8 * u8 -> u16` expressions with store, call, and arithmetic uses;
- zero-extend/shift/add expressions consumed by word stores;
- negative cases containing calls, volatile induction homes, aliasing stores,
  signed extensions, multiple uses, and ambiguous CFG joins.

## Slice 0: plan and baseline

Commit this document and retain current emitted MIR/opcode observations for the
focused fixtures and benchmark routines. Baselines describe current behavior;
they do not make inefficient instruction sequences contractual.

Telemetry names introduced by later slices should distinguish candidates,
applied rewrites, and proof failures. Suggested applied counters are:

- `widening-byte-multiply-selected`;
- `loaded-byte-forwarded-to-consumer`;
- `counted-loop-latch-selected`;
- `machine-value-a-reload-elided`;
- `machine-value-y-reload-elided` (already present through the X/Y census);
- `direct-word-shift-store-consumer`;
- `register-carried-induction-selected`.

## Slice 1: unsigned byte multiply with a word result

Recognize this typed value graph before generic word helper selection:

```text
t0:u16 = zext t_byte0:u8
t1:u16 = zext t_byte1:u8
t2:u16 = mul t0, t1
```

Add an analyzed pre-home selector under `mir6502/materialize`. Resolve both
multiply operands through unsigned byte-to-word `Extend` definitions and
materialize the existing `MultB` helper with byte operands and a word A:X
result. The result continues through the ordinary word destination, so the
optimization is independent of whether the consumer is a store, call
argument, comparison, or later arithmetic.

The first implementation selects the target-owned helper for standalone
output. Cartridge output retains resident `MultI`, because embedding a new
helper for an isolated call loses size; a future whole-program cost model may
select it when enough cartridge call sites amortize that cost.

Do not add a new NIR operation or persistent MIR operation. Remove extension
producers only when shared use/def proves them dead. Reject signed extension,
duplicated evaluation, unsupported definitions, and any case where the full
word result would not be preserved.

Acceptance criteria:

- `CARD(byte) * byte` selects `MultB`, not `MultI`;
- products above 255 retain their high byte;
- each operand is evaluated once;
- store, call-argument, add, and subtract consumers work;
- signed and non-byte operands retain the general word path.

## Slice 2: immediate loaded-value forwarding

Extend pre-home demand selection so a unique-use byte load can be forwarded to
an immediate consumer without a private home:

```text
load -> temp; store temp-home; reload temp-home -> A; compare
```

becomes:

```text
load -> A; compare
```

Use the existing `ForwardToConsumer` home decision. Initially support compares,
bitwise operations, byte arithmetic, and stores in the same block. The original
load remains at the original program point. A volatile or hardware load may be
forwarded as a value, but may never be removed, repeated, or moved across a
barrier.

Reject multiple uses, A clobbers, calls, machine blocks, unknown effects, and
additional effect barriers. Focused volatile tests must count exactly one
hardware read.

Implementation note: the initial selector permits either no intervening
operation or the single compiler barrier emitted immediately beside a volatile
access. The barrier is retained, as is the original load; only the private temp
home and reload are removed. All longer or effectful ranges remain rejected.

## Slice 3: counted-loop analysis and memory-resident latches

Add `mir6502::analysis::counted_loops`. Its result describes a normalized loop
using stable MIR identities:

- preheader, header, body, latch, and exit blocks;
- induction home and width;
- initial value, bound, direction, step, and signedness;
- whether the initial guard is required;
- whether the final induction value is observable.

The first selector supports byte steps of `+1` and `-1`, constant bounds, and a
memory-resident induction home. Generalize and then replace
`rotate_initialized_byte_countdowns`; do not add an emitter peephole.

Select the cheapest semantically exact latch:

- `DEC/BNE` for inclusive unsigned countdowns ending at one;
- `DEC/CMP/BCS` for other proven-safe descending bounds;
- corresponding `INC` forms for ascending loops.

Dynamic starts retain their entry guard. Remove a guard only when the initial
value is proven in range. Preserve wrap protection and the source-visible
post-loop value. Reject volatile/address-taken induction homes, unsupported
steps, ambiguous backedges, or side effects that invalidate the proof.

Boundary coverage includes 0, 1, 127, 128, and 255, zero/one iteration,
overflow/underflow, `EXIT`, nested loops, and signed `INT` loops.

Implementation note: the initial analysis recognizes both conventional
head-tested byte loops and Action!'s bottom-guarded inclusive descending loop.
The selector rotates proven-entered `INC`/`DEC` head latches and selects a
direct `DEC` for descending-to-zero bottom latches. Dynamic entries, signed or
non-unit loops, hardware-backed storage outside zero-page RAM, and live flags
remain on the general path. The old initialized-countdown scanner is removed;
its `DEC/BNE` case is now selected from the shared counted-loop facts.

## Slice 4: exact A/X/Y facts across simple CFG edges

Extend machine-value availability with exact byte identities for:

- direct memory;
- indexed memory with a proven base and index value;
- opaque load results, including volatile reads, which are reusable as values
  but are never repeatable memory reads.

Reuse the existing must-agree dataflow and post-home rewrite driver. Begin with
single-predecessor successors, then allow ordinary meets only when every
reachable predecessor agrees. Extend redundant reload elimination from X/Y to
A and allow store/compare consumers to use the already available register.

Kill relevant facts on possibly aliasing writes, index changes, unproven calls,
machine blocks, edge argument materialization, and effect barriers. Physical
memory-map facts may prove non-aliasing after final layout; unresolved aliases
remain conservative.

Acceptance criteria:

- a comparison-loaded indexed byte remains usable in its taken successor;
- an unchanged index remains available in Y;
- an aliasing store or call blocks reuse;
- volatile source reads are neither deleted nor duplicated.

## Slice 5: destination-aware word shift/add chains

Recognize single-use expressions such as:

```text
zext.u8(value) << constant
(zext.u8(value) << constant) + constant
```

before generic word-shift expansion. Extend the existing store/call expression
consumer machinery and use routine-wide use/def to prove a unique consumer.
Emit the low/high carry chain directly into the destination rather than
materializing an intermediate absolute word scratch.

Start with constant shift one and direct word stores. Add other small constant
shifts and call arguments only when the same proof and cost model apply.
Multiple uses, volatile destinations, overlapping source/destination storage,
or unsupported addressing forms retain the general word path.

## Slice 6: register-carried induction

After counted-loop recognition and cross-edge register facts are stable, allow
a counted-loop candidate to request X or Y as its induction carrier. Feed the
request into existing home planning rather than creating a second allocator.

Choose the carrier from body demand: Y is valuable for absolute/indirect
indexed byte accesses, while X may be preferable when Y has another live role.
Keep the value across the backedge and emit an explicit final writeback when
the induction home is live after the loop.

Reject calls without a preservation proof, conflicting register demands,
address-taken or volatile induction homes, machine blocks, and loops whose
observable post-loop value cannot be reproduced exactly.

Implementation note: the first selector is a post-home target-home decision,
not a general register allocator. It consumes the existing counted-loop,
machine-liveness, structured-effect, and final-layout facts, tries X and Y, and
accepts only a lower-cost canonical head-tested byte loop. It introduces the
explicit post-home `UpdateReg` latch and lets `CompareDirectIndexedBytes` name X
or Y. A Y-indexed access is converted to X only after an exact induction load
in the same block; all ambiguous indirect accesses, aliases, calls, barriers,
machine blocks, carrier conflicts, and noncanonical exits are rejected. A
possibly read final induction value is written back at the canonical exit.

## Delivery order and expected signals

The commit order is Slice 0 through Slice 6. Slices 1, 2, and 5 are locally
useful; Slice 4 builds on Slice 2; Slice 6 builds on Slices 3 and 4.

The benchmark validation signals are:

- Monte Carlo selects `MultB` and avoids staging immediate hardware reads;
- BSort reuses comparison A/Y values and later carries its induction value;
- Sieve avoids scratch words for simple shift/add address calculations;
- countdown loops use compact update-and-branch latches.

These are expected consequences of general rules, never matching conditions.
