# MIR65816 storage demand

## Objective

Decide whether an intermediate value needs memory before allocating its home.
Keep a short-lived value in the accumulator when its consumer can use it
directly. Never emit a store/reload pair for that value or reserve its stack
slot. Keep NIR semantics, the native ABI and stack-check policy unchanged.

The model is MIR6502's `materialize/home_census.rs`: explicit decisions,
bounded register lifetimes and a reason for retaining each memory home.
MIR65816 must account for A8/A16 and the hidden accumulator byte itself.

## Executable slices

1. **Demand planning and allocation.** Classify every MIR temporary before
   allocation. Initially admit a single definition with exactly one use in
   the immediately following operation of the same block. The producer is a
   nonvolatile direct BYTE/CARD load, native CARD Add/Sub/And/Or/Xor, or an
   unsigned CARD shift by 1–3 constant bits. The consumer is an unsigned integer
   widening cast to 2–4 bytes. Record the
   producer/consumer sites and accumulator width. Allocate only the remaining
   values. Independently recompute admission during allocation verification;
   a missing unapproved home is an error. Recompute alignment, incoming
   displacements, staging and peak stack usage from the resulting frame.
2. **Select accumulator producers and consumers.** Emit the load/arithmetic
   into A at its original site. The adjacent cast reads A directly, stores the
   widened result and explicitly establishes zero high bits. Source spans
   retain their original operation identities. These decisions happen before
   instruction emission, without a cleanup pass over generated instructions.
3. **Validate and measure.** Test raw and optimized NIR, BYTE/CARD boundaries,
   all admitted arithmetic/shift operators, dirty hidden B, stack/DP guards and
   calls elsewhere in the routine. Reject volatile, indirect/indexed,
   signed, multi-use, cross-block and nonadjacent candidates. Verify absent
   temporary map entries and reduced frames, plus malformed sparse maps.
   Run the affected MIR65816 units, emission/artifact integrations and focused
   native runtime tests. Measure the `MetadataBytes` expression and existing
   Exec generated inputs using a documented local compiler override.

## Boundaries

The initial consumer deliberately stays small. Other casts, stores, call
arguments/results, returns and branches continue to use their current
selection. Later consumers can extend the same demand decision rather than
introducing unrelated instruction peepholes. No value remains register-only
across a call, label, intervening operation or CFG edge in this implementation.
Frame objects and parameter homes remain authoritative. Existing pointer-leaf
and scalar-DP allocation keep their contracts; their closed whitelists exclude
the widening casts admitted here.

Memory-backed values retain closed-operation interference. A register-only
value has no memory extent and must not appear as a fictitious stack or DP
home in the image. Artifact schema and public ABI do not change. Register
decisions are compiler-internal and checked against the original typed MIR.

Reserved bank-zero delta: **0 fixed bytes, 0 bytes per task**. Individual
routine frames may shrink inside existing task stack reservations.

## Status

All three slices are implemented. The demand plan lives in
[`home_demand.rs`](../src/mir65816/emit/home_demand.rs); allocation omits its
register-only values and
[`accumulator_homes.rs`](../src/mir65816/emit/accumulator_homes.rs) selects their
producer and consumer instructions. BYTE widening stays in A8 and writes zero
high bytes directly, avoiding both hidden-B leakage and extra mode switches.

Validation covers raw and optimized NIR: 311 MIR65816 unit tests pass (one
pre-existing ignored test), plus 39 emission/o65/state-boundary integrations.
Focused native tests pass for storage demand (4), integer casts (2), stack
checks (2) and direct assignments (4). The new interrupt test checks both task
domains at instruction boundaries through BYTE/word widening; the three
existing parameter/frame/accumulator forwarding interrupt checks also pass.
Fixture instrumentation checks LF and CRLF input. Default-feature library
compilation passes. No 6502/68k suite or hosted Exec qualification is claimed.

Test maintenance reconciles the state-boundary snapshot and X-residency oracle
with the existing upper-half DP ABI. Snapshot review established that all 146
changed instruction bytes were DP operands increased by `$80`; only DP home
offsets changed otherwise. Forwarding probes now distinguish missing captures
from stored values, and their fixtures still exercise positive, zero and
negative flags after incoming-load/direct-assignment selection.

No Exec compiler-pin update or play-image refresh is part of this slice.

## Measurements

The baseline is the same local compiler checkout with the earlier direct
assignment work, before storage-demand planning. Figures are emitted routine
bytes, including normal guards, with optimized NIR.

| Probe | Code before → after | Fixed frame before → after |
| --- | --- | --- |
| `SIZE(blocks)*12+SIZE(blocks RSH 2)` | 179 → 171 | 12 → 12 |
| `LONGCARD(value)`, CARD parameter | 59 → 51 | 8 → 6 |
| Existing generated Exec program, 960 routines | 492,863 → 492,302 | 6 routines shrink; none grow |

The Exec total saves **561 bytes**. Sixty routines change; none grow in code
size. Exec's frame-map validator accepts all 960 routine maps. This is a
compile-only comparison, not a booted-image qualification or a claim about
padding/container size. The multiplication helper and the load before the
shift in `MetadataBytes` remain outside this slice's consumer scope.

Local override provenance:

- Compiler HEAD `c64e0c50772d354ae74e7d096d770cc97baf33dc`, with the direct
  assignment and storage-demand changes in the working tree.
- Compiler executable SHA-256:
  `ed29bd5d452167ecb3029543b9032886a0faf23c238b252e87df48533b05db28`.
- Compiler `src/**/*.rs` tree digest:
  `1e5186bdfabc473b639fd1f289ca9fb180c04212965ed278dc7640fe9568f075`.
- Exec HEAD `134130bdb8ce1374ca7d5e6011ed98c4ccd62151`; its recorded compiler
  pin remains `bcabe0a4cbb8bb57b389a8596aa8fd72cb9ee0c7`.
- The 133 Exec source/layout inputs have tree digest
  `751059513e938ac69f5d578219029b39347388dccda46797916d30d03eb7898d`.

Tree digests hash sorted relative paths as `path`, NUL, content SHA-256 and LF.
The Exec inputs are `build/demo/layout.json`, `build/demo/kernel-program.act`
and immediate `*.act` files in the module paths below. Reproduce from Exec's
existing generated inputs using the locally built compiler:

```sh
../actionc-public-release/target/debug/actionc-65816 \
  --layout build/demo/layout.json \
  --module-path build/demo/task-kernel --module-path build/demo \
  --module-path examples/shell --module-path lib/exec --module-path lib/dos \
  --module-path lib/fs --module-path lib/console --module-path lib/mydos \
  --module-path lib/spartados --module-path lib/io \
  -o /tmp/exec816-home-demand.a816.json build/demo/kernel-program.act
```
