# Bounded loop-parameter residency in X

Status: implemented and qualified. Frozen evidence `0c3a4e5`, tracker
prerequisite `084cf64`, selection `3f3d6e8` and qualification probes/checker
`7a73fca` complete the four delivery slices. The forecast matches all 28 complete
images and 264 measured records; only optimized rotation changes. See
[results and qualification](MIR65816_LOOP_X_RESIDENCY.md), the
[implementation baseline](benchmarks/65816-loop-x/frozen.json), and the
[exact delta](benchmarks/65816-loop-x/delta.json). Historical
[inventory](MIR65816_REGISTER_INVENTORY.md) and
[planning evidence](benchmarks/65816-loop-x-plan/baseline.json) remain frozen.

## First slice and expected result

Keep a checked X16 copy of one private unsigned word loop parameter while
retaining its existing authoritative DP home and every store. Use TXA for its
selected arithmetic input and CPX immediate for its branch-only loop test.
Refresh X after the incoming parallel copies have established the parameter.
This extends value residency across a tightly checked loop without changing
allocation, maps, edge scheduling or closed-operation interference.

The first corpus candidate is optimized rotation. Sum-loop's counter remains
a mutable stack parameter; its private loop parameter is the accumulated sum.
Raw casts and pointer/wide/calling routines remain useful rejection controls.
Selection must use typed CFG, TempIds, home geometry and selector effects, never
kernel names, source syntax, numeric IDs, DP+$22 or a particular constant bound.

Retain ABI v1, image v3, o65 profile v1, public argument/result placement, all
home maps, frame extents, incoming displacements, guards, interrupt reserves
and Exec816's pin. This slice introduces no register location in `Location` or
`WordHome`: those continue to describe authoritative memory. Removing backing
stores/homes, mutable-object promotion, cross-call residency, Y allocation and
INX-based in-place updates require later designs.

## Admission and preflight

Build a separate immutable `LoopXPlan` after normal frame allocation and before
any routine bytes are emitted. Reuse the scalar admission contract where it
applies, but explicitly establish X-preserving effects; scalar-DP admission
alone grants no register reservation. Keep the all-stack and pointer-leaf
paths unchanged. A malformed verified contract is an error; a well-formed but
unsupported candidate gets the unchanged selector.

Initially require all of the following:

1. A whole call-free scalar routine with word/void return, an existing verified
   scalar-DP frame, and the closed supported-operation whitelist. Reject every
   explicit/implicit call, helper selector, machine block, address formation,
   pointer/indirect/indexed/external/volatile access, wide/byte arithmetic,
   signed ordering and unknown effect, including in unreachable blocks. Local
   and parameter memory accesses retain their existing alias/storage rules.
2. One simple natural loop: a header containing only a fused compare/branch,
   one straight-line body/latch ending in Goto(header), one predecessor outside
   the loop, and one exit. Header dominates latch. Reject alternate entries,
   nested cycles, extra latches and unsupported predecessor/edge shapes. Require
   the header arms to have no edge arguments and the body to have no block
   parameters in this first slice.
3. An ordinary unsigned 16-bit header parameter `p` with an aligned private
   scalar-DP home. Its only MIR uses are the header comparison and one body
   `q = p + 1`, whose result is the backedge argument for `p`. No additional
   uses, address observations or loop-exit use of `p` are admitted initially.
   Preserve `p` and `q` as distinct, interfering values with their original homes.
4. The branch comparison is unsigned `p < K` or `p <= K`, with a typed word
   immediate and a sole-use Boolean consumed by that terminator. For `<`, use
   threshold K; for `<=`, use checked K+1 and reject K=$FFFF. Reject signed,
   materialized, multiply-used, swapped or nonconstant comparisons initially.
   This normalization belongs to native instruction selection, not NIR.
5. `p` is the final logical destination on both incoming header edges. Both
   edges already have a fully checked native word-copy plan whose final A/N/Z
   represent that final assignment, including any existing self-copy repair.
   This makes an appended TAX preserve all existing edge outputs except X.
6. Within the reserved region, no selector writes X or narrows index width,
   no unmodelled memory write can alias the private home, and no different
   temporary sharing that home writes it before the latch refresh. Check exact
   address space and both bytes. Disjoint reuse after the exit is allowed only
   after releasing the binding; rotation's exit result has such a reuse.

The plan records region blocks, all incoming transfers (including arm identity),
parameter/update IDs and typed homes, condition/threshold, copy-tail obligations,
replacement operation cursors, expected widths and exit-release sites. Require
all selected uses and every predecessor to be covered. Choose at most one plan
per routine, deterministically by typed IDs after a profitability check.

Use the existing independent closed-operation liveness reconstruction to check
that the plan preserves all conflicts. Do not coalesce `p` and `q` or shorten
`p`'s lifetime to justify destructive INX. Preflight every operand, staged
capture and copy destination using the normal verifier before committing a
plan. A rejected candidate must leave the fallback bytes and maps identical.

## State-tracker contract

The current tracker already models X, TAX and TXA, but `mark()` clears values
at every label. Its MIR-entry proof presently carries execution modes and stack
equations, not arbitrary value residency. Add a separate, narrowly scoped
X-to-loop-parameter relation with explicit predecessor obligations.

On each incoming header edge, execute the normal copies and A/N/Z repair,
then TAX in M=X=16. Only that checked completed edge may establish
`X == word(D + home(p)) == incoming value of p`. Do not capture early: parallel
copies may still need the old source, and the update result remains separate.
TAX does not change A/C/V or memory, and its word N/Z equal the existing final
word assignment's N/Z. All stores and their generations remain real.

The retained store to `p` invalidates the old X/home equality immediately. Keep
X reserved but mark the binding pending refresh until TAX establishes the new
relation. No CPX/TXA consumer is authorized in that interval, and a snapshot
between STA and TAX must not claim X equals the newly written DP word. This
state is legal at an interrupt boundary and must resume with both distinct
values preserved.

At header/body entry, install only this relation after validating the declared
CFG obligations, including backedges emitted later. Mirror the existing
`prove_entries` discipline: finish must reject missing, duplicate or conflicting
predecessors. Use fresh symbolic value identity at the join, relating the current
X and current DP word; never equate values from different loop iterations.
Retain barriers for A, Y, general home values, carry, overflow and adjacency.
Do not propagate arbitrary tracker observations through a label.

Internal dispatch labels also clear ordinary value facts today. They need
explicit X-preserving transfer obligations for the admitted condition/empty-edge
path; a fallthrough observation alone is insufficient. Reservation and value
proof are separate: observing X once must not authorize later unplanned use.
The instruction facade must check reservation effects against every emitted
instruction while it is active, rather than trusting only a MIR whitelist.

Revoke permission on region exit, calls, unknown writes, D/S changes, index
narrowing or unexpected X writes. An unexpected effect in an already committed
plan fails its proof; do not silently continue with stale X or attempt a partial
fallback after emitting bytes. Ordinary forwarding barriers may clear their own
A/adjacency witnesses without revoking a separately proven X relation when the
actual instruction effects preserve it. Make that distinction explicit.

The entry guard's TAX executes before any binding. Return teardown's TAY/TYA
executes after release. Preserve their existing behavior and all guard/fault
paths. A word result still returns in A. IRQ/NMI/task switches must restore the
full live CPU state and the task's D/S/scratch under the existing ABI contract;
call-clobbered registers are not interrupt-clobbered registers.

## Instruction selection and flags

Add a typed `CpxImm16` form (`E0 lo hi`) to `tracked.rs`, validating **index**
width independently of M. Extend the compare-state helper to accept explicit
left value and width; leave current CMP behavior identical. CPX preserves A,
X, Y and V and updates C/N/Z from a 16-bit subtraction. Extend the independent
disassembler's immediate-width rules and instruction inventory explicitly;
unsupported opcodes must continue to fail closed.

For an admitted header, emit CPX #threshold followed immediately by the existing
conditional dispatch with BCC for the true arm. Keep false/true edge ordering,
fixups, relaxation and long fallback. For rotation, this changes
`LDA #7; CMP $22; BCS body` to `CPX #8; BCC body`. This preserves the unsigned
branch truth for every counter value, including $8000..$FFFF. It intentionally
does not preserve the old header A/C/N/Z values. Justify this at the fused MIR
boundary: the Boolean has no other use, C/Z are consumed immediately, and both
successor entries retain the ordinary value/flag barriers. Do not advertise the
new flags as equivalent to the old swapped CMP or reuse a materialized-compare
oracle that assumes they are.

For the checked `p + 1` input, retain existing A-forwarding priority; if a memory
load would otherwise be required, use TXA only with the current typed X relation,
M=X=16 and matching `p` identity/home. TXA establishes the same A/N/Z as LDA of
the retained DP word and leaves C/V alone. Keep CLC, ADC #1, the store to `q`,
the backedge source load and the store to `p`. Publish any subsequent adjacent-A
witness through the existing checked store path. Add separate X-forwarding
proof/counters so existing elided-load counts retain their meaning.

Append TAX only after the complete incoming edge schedule and final A/N/Z
repair. Preserve physical self-copy omissions, cyclic staging, logical copy
counts, source-capture ordering and final A/N/Z. The independent edge decoder
must recognize and authenticate the new tail; otherwise adding TAX would make
its backwards-from-transfer decoding lose existing copy evidence.

## Conditional corpus forecast

The planning evidence authenticates the old instructions and measured counts.
It is not an execution of modified compiler output. The proposed edits, named
by **old** worker PCs, are:

| Old site | Proposed change | Static bytes delta | Dynamic cycles delta |
| --- | --- | ---: | ---: |
| After STA $22 at $01003B | Add TAX after initialization copies | +1 | +2 once |
| $01003D..$010042 | Replace LDA #7 / CMP $22 with CPX #8; invert dispatch predicate | -2 | -4 × 9 |
| LDA $22 at $010056 | Replace with TXA | -1 | -2 × 8 |
| After STA $22 at $01006C | Add TAX after backedge copies | +1 | +2 × 8 |

CPX immediate takes three cycles with 16-bit indexes in the pinned VM; TAX/TXA
take two. Existing DP word LDA/CMP take four and LDA immediate takes three.
Instruction qualification must verify these independently before enabling the
selector. The new prefix shifts PCs, branches, later routines and image entry;
all references must use the normal finalizer and relocation machinery.

| Optimized rotation, each of six corpus vectors | Measured | Forecast |
| --- | ---: | ---: |
| Worker bytes | 130 | 129 |
| Cycles | 793 | 759 |
| Instructions | 218 | 218 |
| Stack peak | 8 | 8 |
| Stack bytes read / written | 21 / 32 | 21 / 32 |
| Scratch DP bytes read / written | 102 / 104 | 68 / 104 |
| Metadata reads | 4 | 4 |
| Body X reads / writes | 0 / 0 | 17 / 9 |

All vectors run eight loop iterations regardless of the rotated input value.
The header loses nine LDA executions while the two refresh sites add nine TAX
executions, so total instruction count is unchanged. The old 104-cycle counter
traffic budget is not the saving: retained stores and refreshes reduce the net
forecast to **34 cycles (4.3%)** and one byte. Memory writes, frames, copy counts,
existing forwarding counts and guards must remain equal. The other 27 complete
Action builds and all vbcc artifacts/measurements should remain unchanged.
The changed rotation image may move its uncounted Main routine and image entry.

For the admitted simple loop, with H header tests, U update loads replaced and
E incoming refreshes, savings are `4*H + 2*U - 2*E` cycles. Require a checked
nonregressing cost for zero iterations and each complete loop iteration, plus
nonincreasing static size; never use profile-specific iteration counts to admit
production code. Account for already-elided loads and any extra repair before
accepting the plan. If the actual planned encoding needs extra instructions,
revise and freeze its forecast before enabling it.

## Delivery and validation

1. **Freeze typed admission and the exact transform.** Extend the current test
   exporter with the extra binding/use/effect facts needed above, preserving
   historical schemas. Report every candidate/rejection and all incoming-edge
   obligations. Independently reconstruct the one expected whole-image change
   and all-vector deltas from the saved scalar-DP baseline. Commit that evidence
   before changing enabled production selection; preserve older inventories.
2. **Implement the tracker prerequisite with selection disabled.** Add CPX
   encoding/effects and reservation/entry APIs, with focused misuse tests and
   immutable proof snapshots. Independently assemble/execute CPX, TAX and TXA
   probes in both widths as applicable; check N/Z/C/V, I, A/X/Y and exact cycles.
   Existing corpus images and trace-on/off output must remain byte-identical.
   Commit the qualified prerequisite separately.
3. **Enable the bounded plan and independent oracles.** Integrate typed planning,
   TXA/CPX selection and TAX edge tails together. Extend fused-branch, word-edge,
   forwarding, state and counter observers to authenticate actual bytes against
   typed facts without accepting production planning decisions as an oracle.
   Update emission/temporary-allocation contracts and commit after focused tests.
4. **Qualify and record the new baseline.** Build raw/optimized final images with
   LF/CRLF verification; execute all corpus vectors in both host VM profiles
   and both incoming I states. Check the complete-image transform, all 264
   records, correct Action results, unchanged fallback builds and the known
   optimized vbcc unlink vector-0 failure. Save results separately and update the
   quality-plan baseline only after forecasts match. Commit qualification.

Positive probes must vary initial values, trip counts, high-bit words, wraparound
ADD results, bounds 0/1/$7FFF/$8000/$FFFE and both `<`/`<=`. Exhaustively compare
unsigned branch truth over all 65,536 counters for representative bounds.
Exercise zero/one/many iterations, relocated branches, direct/self/selective
copy tails and disjoint post-loop reuse of the same DP home. Include a loop
whose update source/result interference is explicitly retained.

Negative probes cover K=$FFFF with `<=`, signed/nonconstant/materialized tests,
extra parameter uses, non-final copy destinations, aliasing/partial writes,
missing or duplicate predecessor obligations, extra latch/entry, unknown
helper/call effects, index narrowing, X clobbers, invalid homes and stale
relations. Mutate actual opcode/operand, refresh site, condition threshold,
branch polarity and proof identity to ensure observers reject false witnesses.
Preserve exact old output for unsupported-but-valid cases.

Execute flat images and two o65 placements, including relaxed and long dispatch.
Inject IRQ/NMI and two-task schedules at every selected instruction, especially
between retained STA and TAX, after CPX and before TXA. Compare interrupted runs
to independent uninterrupted steps of the **new** stream, including full CPU
state and live frame/DP contents; old/new whole-register equality is not a valid
oracle when X residency and fused comparison selection intentionally change X/A.
At equivalent edge completions, retain the existing A/N/Z and memory contract.
Use distinct task values and DP windows to expose leaked X/home relations.

Run affected MIR65816 library/state-boundary/ABI/contract/emission/o65/CLI tests,
the native qualification runner in debug and release, comparison/disassembler
tests and corpus-generator checks. Rebuild affected source/embedded-fixture
consumers in an isolated CRLF checkout. No NIR/full-root sweep is required while
changes remain in native target strategy and its tests. Use the established
[qualification commands](MIR65816_EDGE_COALESCING_PLAN.md#delivery-and-checks).
Preserve unrelated local changes and commit only owned paths.
