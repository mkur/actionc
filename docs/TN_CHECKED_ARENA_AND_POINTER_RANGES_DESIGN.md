# Checked arena allocation and pointer range proofs

Status: checked runtime implemented; compiler range proofs proposed, 2026-11-12.
Compiler baseline: `95e88790`.

The [implementation plan](TN_CHECKED_ARENA_AND_POINTER_RANGES_IMPLEMENTATION_PLAN.md)
defines the initial delivery and its commit/check gates. It uses established
entry facts or dominating guards and defers private routine variants.

Bound TN's arena operations at runtime, then derive reusable pointer ranges
from their verified success paths. Those ranges can establish that a copy
leaves routine parameter homes unchanged. This separates the allocator's
memory contract from the later compiler optimization: checks have value even
when a particular call cannot be optimized.

The current [bounded-call proof](MIR6502_BOUNDED_CALL_POINTER_RETENTION_PLAN.md)
handles known materialized addresses in owned RAM. TN's `Strcat` still has
unknown incoming pointers and a 113-byte body. Checked source code alone will
not change that result; interval propagation, call-entry handling and a target
consumer are also required. No byte saving is promised by this design.

## Current arena and cursor operations

[LIB.ACT](../samples/tn/modern/LIB.ACT) reserves a 2 KB arena at
`$2000..$27FF`. `allocp` is a CARD pointer stored at `$E8/$E9` and initialized
to the arena base. It is also modified by `Free`, `Push`, `Pull` and
`RestorePos`; guarding only `Alloc` would leave the cursor unbounded.

Define the following mathematical values:

```text
B = WINDOW_ARENA                 = $2000
S = WINDOW_ARENA_BYTES           = $0800
E = B + S                       = $2800
c = captured current cursor
```

Storage occupies the half-open interval `[B,E)`. The cursor may equal E when
the arena is full. The initial CARD-based implementation requires positive
capacity and a representable exclusive end, without 16-bit wrap. An arena
ending at `$10000` needs a different cursor representation and is outside
this initial contract.

Each operation captures and validates its inputs before performing cursor
arithmetic or accessing arena bytes. The notation below describes requirements;
it does not introduce an Action! `require` statement.

| Operation | Checks before access or arithmetic | Success behavior |
| --- | --- | --- |
| `Init` | B/E are valid constants for the configured arena | Set cursor to B |
| `Alloc(n)` | `B <= c <= E`; `n <= E-c` | Return c; set cursor to `c+n` |
| `Free(mark)` | `B <= c <= E`; `B <= mark <= c` | Set cursor to mark |
| `Push(data)` | `B <= c <= E`; `2 <= E-c` | Write the CARD at c, then advance by two |
| `Pull()` | `B <= c <= E`; `2 <= c-B` | Decrement by two, then read the CARD there |
| `RestorePos()` | `B <= c <= E`; `4 <= c-B` | Decrement by four, then restore the four position bytes |

Subtractions are evaluated only after cursor validity is established. The
capacity check precedes `c+n`, so overflow cannot turn an oversized request
into an apparently valid allocation. Underflow checks likewise precede any
decrement. Capture the cursor once for a successful operation; do not build a
proof from one load and perform the update from an unrelated later load.

Keep the existing successful read/write order and public Action ABI. Bounds
checks do not add an alignment requirement or verify that every pop matches a
particular earlier push. All writes to the cursor must participate in this
audit, including absolute aliases and direct stores outside these routines.
An unproved external write invalidates cached cursor facts; each checked
operation still validates the value it actually observes.

## Zero allocations and checkpoint semantics

`Alloc(0)` succeeds, returns the current cursor and leaves it unchanged. At
exhaustion it can return E. The numerical bound supplies no nonempty allocation
extent for a subsequent access. Consequently the universal return bound is
`[B,E]` inclusive; the stronger bound `[B,E-1]` requires a proven nonzero request.

`Free` rewinds a checkpoint. It neither clears storage nor creates a language
rule forbidding later access through an existing pointer. It checks the mark's
range, without inventing allocation headers or historical-mark tracking.

Two current uses require this behavior:

- [TN.ACT](../samples/tn/modern/TN.ACT) allocates twenty bytes in `MkDir`,
  rewinds to that address with `Free(s)`, then uses the bytes as scratch.
- `CloseWindow` rewinds to its saved image with `Free(buf)` before reading that
  image through `PutImage`.

Neither use grants freshness, exclusive ownership or non-aliasing. A later
allocation may reuse those bytes. Captured address ranges remain numerically
valid after rewind; assumptions about the bytes' contents need separate
memory-effect proofs.

## Failure behavior

Invalid cursor, exhaustion and underflow enter a dedicated non-returning arena
failure path. `ArenaStop` records a reason in `ArenaFault` and enters a stop
loop, without depending on cartridge services. Returning zero is unsuitable
because existing callers immediately use allocation results without checking
them. There is no recovery or window-operation rollback contract in this
design.

| Reason | Value |
| --- | --- |
| Invalid captured cursor | 1 |
| Insufficient remaining capacity | 2 |
| Rewind mark outside `[B,c]` | 3 |
| Pop or position-restore underflow | 4 |
| Invalid configured arena (checked by `Init`) | 5 |

A failing primitive leaves its cursor and requested destination bytes unchanged;
recording the fault reason is an intentional diagnostic effect. Earlier
successful operations in a compound UI operation may already have changed
state. In particular, a failing `Window` need not undo a preceding `SavePos`.

The failure edge must have verified non-returning control flow. An explicit
sink loop suffices. A call to `Error` or another routine is not non-returning
merely because of its name. A returning error callback must not let execution
fall through into unchecked pointer arithmetic or memory access.

Successful allocations retain existing behavior. Failure behavior is an
intentional change from unchecked wrap/corruption to a defined terminal path.
Implement the source contract consistently for TN and TNDBG before using it
as an optimization premise.

## Facts and compiler ownership

| Layer | Responsibility |
| --- | --- |
| SemIR | Resolve constants, unsigned types, pointer casts, storage identities, calls and source control flow; preserve the guards and failure path |
| NIR | Represent the executable checks and computation normally; derive numeric ranges, no-wrap conditions and return facts from verified blocks and use/def relationships |
| MIR6502 | Project those facts to physical address ranges, track complete word carriers, instantiate copy effects and select profitable pointer reuse |
| Emission | Resolve final placement, write code and provide maps used by final proof replay |

There is no allocator-name recognition, TN-specific address whitelist or new
source annotation in the initial design. Ordinary typed comparisons, branches,
loads, stores, casts and calls supply the evidence. A differently named routine
with the same verified properties must work, and a routine named `Alloc` with
unchecked arithmetic must supply no such guarantee.

Start with bounded interval analysis over the unsigned widths carried by NIR.
Facts identify values, blocks, storage and routines by stable IDs and retain
their defining guards and analysis generation. Unsupported arithmetic,
excessive graph complexity or lost provenance yields unknown. Join every
reachable predecessor conservatively; a loop backedge cannot establish its
own initial range. Use widened mathematical arithmetic while proving bounds,
then check the actual bit-vector width before asserting no wrap.
Initially, recursion and unresolved call targets supply no return-range summary.

For the checked allocator, the initial useful summary is that every normal
return lies in `[B,E]`. Stronger request-dependent bounds may be derived when
their guard relationships are proved. A claimed requested-size extent is
separate from this address bound; do not infer it solely from an interval.
The summary must account for every reachable return and exclude failure paths
only when their lack of normal continuation is verified.

Prefer derived analysis tables over new executable NIR forms. If facts are
persisted or projected between phases, validate their IDs, widths, dominance,
callee-body dependency and generations. Recompute or invalidate them after a
relevant rewrite. NIR verification remains mandatory before optimization;
an optimizer's own fact checks supplement that verification.

### Values in mutable storage

A captured value's range survives unrelated writes. Loading a mutable pointer
cell requires a current fact for that cell's reaching definition. Track whole
word stores, joins and intervening writes; unknown calls, raw aliases, OS
effects, machine blocks and possible reentry invalidate unproved storage facts.
MIR may retain a projected range through a call only using its complete
physical write footprint, including parameter prologues and scratch.

This matters for `linebuf`: its initial value is zero and its assignment from
`Alloc` is conditional on first-time initialization. Its later loads cannot
receive an unconditional arena bound from a census of visible assignments.
The same applies to the mutable global `s`. Prove the reaching bounded store,
derive a checked path, or retain unknown. Carrying a pointer type is insufficient.

Integer/pointer conversions preserve a range only when the typed conversion
preserves the represented address. Reconstructing a word from arbitrary low
and high byte facts is insufficient. MIR carrier facts must tie both lanes to
the same complete value and invalidate them after an unproved partial write.

## Entry facts and calls to Strcat

A verified allocator return range is a postcondition. It does not establish a
range for every incoming argument of every caller or callee.

At each call, instantiate the callee analysis with the actual reaching
arguments. Any restricted entry must be justified by an explicit validated
contract, dominating checks, or a compiler-created private variant whose
incoming edges all carry the required proof. An ordinary routine declaration
and a direct-call census do not by themselves establish closed entry.

Keep generic `Strcat` valid for its existing input domain. Unknown callers and
the parameter-home alias examples retain the generic implementation. A checked
variant may be used only at proved calls. Redirecting a call or creating such
a variant must preserve its source-visible effects and pass whole-image cost
checks, including any retained generic body and duplicated code. This design
does not silently narrow `Strcat` to arena-only destinations.

## Copy footprint and parameter-home disjointness

For a call with a proven nonempty-allocation bound `B <= s <= E-1`, let a and b
be the two captured unsigned length bytes. The current helper uses:

```text
destination = s + a + 1
count       = b, if b != 0; otherwise 256
last write  = destination + count - 1
```

The worst-case last write is `E+510 = $29FE`. With only the universal bound
`B <= s <= E`, it is `E+511 = $29FF`. Thus the conservative copy-write union
for either case fits in `[$2001,$2A00)`. A subsequent length-byte update at s
is bounded separately. No 16-bit address wrap occurs in these calculations.

TN's current program begins at `$2C00`, above that union. Final physical maps
must establish the actual locations of code, parameter homes and protected
scratch. A different origin or overlapping layout can invalidate the proof;
do not make `$2C00` a compiler rule. Maintained TN builds must also check that
their reserved arena and emitted image do not overlap.

An address bound is not an allocation-capacity proof. A malformed string can
make the copy leave its allocated buffer or even the arena while still staying
below the parameter homes. Preserve those possible writes in the effect
summary. Do not call the tail up to `$29FF` arena-owned or use it to justify
deleting unrelated memory effects.

The current owned-buffer proof therefore needs a distinct extension: accept a
complete, non-wrapping physical write range in supported RAM when it is
disjoint from the protected cells and executed code. Invalidate every cached
memory fact intersecting that range. Retain all direct ABI/scratch writes,
and reject hardware, stack, code or unknown effects. Source-read ranges and
their ordinary-memory behavior also need proof. This does not make arbitrary
absolute storage private or weaken the existing object-extent proof.

`MovePage` effects remain specific to its actual call. `RestorePos` copies four
bytes back to `@zx` at `$5A`, so the current `writes $0100-$FFFF` annotation is
not a complete description of that routine's real uses. Audit/correct its
metadata before using declared regions as trusted bounds. Derive narrower
copy footprints from the verified body and arguments, rather than globally
declaring that `MovePage` cannot write zero page.

### Pointer reuse after the call

Prove both resident-pair preservation and unchanged pointer-home values. Keep
the final length reads after the copy; overlap can change either length byte,
so the pre-copy byte count cannot replace the later `t^` read.

There is also a placement obligation beyond disjointness. Before the current
call, `$AE/$AF` hold s and `$AC/$AD` hold t. The post-call reconstruction gives
those pairs the opposite roles. Removing all twenty bytes therefore requires
remapping the final compound consumer to the retained pairs, or selecting
consistent roles earlier. Preserve effective addresses, access order, live
A/X/Y/flags and published callee facts. Prove changed scratch exit values
unobservable or restore them. Bounds alone do not authorize deleting the
twenty bytes while leaving the consumer unchanged.

## Qualification and acceptance

Runtime validation covers empty/full arenas, exact-fit allocations, zero
requests at both ends, oversized requests, invalid cursors, mark boundaries,
push/pop and restore underflow, and carry/word-wrap cases. Observe the fault
path before any failing primitive changes its cursor or target bytes. Preserve
the existing post-`Free` scratch/image uses and measure real UI high-water use;
do not enlarge the arena merely to conceal an unexpected failing scenario.

Compiler validation uses neutral checked routines with different names,
different arena sizes and non-page-aligned bases. Include unchecked lookalikes,
returning failure handlers, unsupported casts, partial word writes, loops,
unknown callers, changed callee bodies, initialized/uninitialized joins and
intervening writes to globals or cursor aliases. Unsupported cases must retain
the previous code. Keep generic parameter-home alias probes, including the
short-string case without length overflow.

Execute feature-off/on, raw/optimized NIR, both runtimes, relocation and actual
LF/CRLF paths for the affected compiler behavior. Runtime-library changes also
cover the classic optimized backend and MIR6502 consumers of modern LIB.ACT.
Shared NIR/lowering/verifier/printer changes require the repository's mandatory
NIR snapshots, NIR sweep and full `cargo test`, followed by affected backend
and VM checks. A documentation-only design change needs content/link checks.

Measure three configurations: existing unchecked library; checked library with
range optimization disabled; checked library with range optimization enabled.
Report complete XEX bytes, affected bodies, checks/fault-handler cost, any
specialized variants, storage/zero-page changes, runtime cycles and compiler
cost. A successful bound is a correctness result. A code-size improvement is
accepted only when the complete measured image supports that claim.
