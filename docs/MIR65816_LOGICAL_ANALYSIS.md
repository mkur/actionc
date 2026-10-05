# MIR65816 logical routine analysis

`mir65816::analysis::RoutineAnalysis` describes typed computation and invocation
storage before physical placement. Native emission constructs and checks it after
arithmetic preparation and before forwarding or allocation. Stage 1 changes no
operation selection, ABI reservation, temporary home map, or emitted bytes.

This analysis consumes MIR alone. SemIR retains source meaning and NIR retains
promotion legality. Existing lowering projects NIR storage facts into frame
ownership, mutability, addressability and parameter plans. No new frontend
contract or SemIR lookup is needed for this slice. Selected-instruction analyses
remain authoritative for physical homes, machine resources and actual effects.

## Immutable input and checked queries

An analysis borrows its entire MIR routine immutably. Editing the input while
facts remain live is rejected by Rust's borrow checker. Each construction gets
a distinct generation, even when analyzing an identical clone. Block, temporary,
point, edge and storage handles contain that generation and the routine ID;
queries reject foreign routines and previous generations. Handles expose readable
stable IDs but callers cannot construct proof handles from those IDs.

A program point is before an operation, or before its terminator. Block parameters
are already defined at point zero. Structural census queries include unreachable
code. Availability, dominance, liveness, edge substitution and storage proof
queries reject unreachable points. Missing identities and invalid points/ranges
are errors, not empty successful proofs. Unknown value representations and
unknown storage versions cannot satisfy the equality proof queries.

Analysis construction does not replace the program ABI/frame verifier. Native
emission first verifies those plans, legalizes arithmetic, and then checks logical
facts. External entries and descriptor-defined arithmetic helpers have opaque
implementations; their ABI and selected effects remain separately checked.
Machine blocks have no executable logical MIR form in this backend and retain
their existing lowering diagnostics or external-call boundary.

## Values and control flow

The exhaustive operand census is shared with existing stack interference and
home-demand consumers. It includes indirect address bases, indices, indirect
callees, every repeated input occurrence, edge arguments and return values.
Every declared temporary has exactly one correctly sized definition. Every
operand refers to a known correctly sized temporary or parameter. Every reachable
use must follow its definition in the same block or be dominated by it.

The CFG uses stable block IDs and the existing ordered successor/fallthrough
rules. Edges additionally retain source-block and edge ordinal identities:
parallel branches to the same target keep different simultaneous assignments.
Arity and widths must match target parameters. Routine entries cannot require
edge arguments, and fallthrough cannot supply them or leave the routine.
Shared dominance and dataflow solvers operate on structurally reachable blocks;
unreachable predecessors cannot corrupt executable dominance facts.

Backward liveness reaches a fixed point through loops. Incoming block parameters
are definitions, outgoing edge arguments are predecessor uses. Queries at a
block's first point include its live parameters. The allocator retains its
existing complete-census, closed-operation interference policy, including its
conservative treatment of unreachable uses.

Each load is a distinct captured value, including repeated reads of the same
field. Complete casts of captured temporaries may share identity only when both
width and target representation match. Representations retain integer facts,
boolean identity, address spaces and callable signature facts; display names and
type-summary text do not establish equality. Identity casts can cross CFG block
order, but need verified dominance. Edge parameters retain their own identities
and explicit incoming mappings rather than guessing a common capture.

A captured pointer carries no proof of pointee extent, alignment, disjointness
or unchanged field contents. Calls and writes do not change a previously captured
logical temporary; its required physical preservation is a later placement/ABI
obligation. This slice neither forwards memory loads nor removes accesses.

## Invocation storage

Storage IDs distinguish frame objects and uncopied incoming parameters. Owner
and extent checks reconcile parameter/frame associations. Uncopied inputs are
immutable; automatic homes are invocation-private unless the projected
address-required flag or any direct address formation makes them addressable.
The exposure census includes unreachable blocks. Addressable does not claim
that an escape occurs, and private does not describe the memory reached through
an object holding a pointer.

Parameters start initialized; automatic locals require writes. A forward fixed
point intersects initialization across all incoming paths. Initialization means
these bytes have an incoming value or an executed MIR write, not a proof about
an unknown source value or source-language undefined behavior.

Contents versions include generation, storage identity and exact byte position.
Exact stores and copies update only their declared destination bytes, including
partial and overlapping writes. Constant-index frame accesses are checked against
the object's extent. Dynamic indices do not establish an extent or alias proof.
A version names an incoming byte or an acyclic write occurrence. Strongly
connected components include irreducible cycles: writes inside a cycle have
unknown versions because a static site cannot name a dynamic iteration.
Different predecessor versions join to unknown without losing valid initialization.

Calls invalidate addressable contents; unexposed invocation homes and immutable
inputs retain their versions under the existing ownership and ABI contracts.
Unknown writes, absolute/indirect writes, dynamic destination extents and volatile
accesses invalidate every tracked version. Such uncertainty cannot initialize
previously unwritten bytes. Exact volatile writes initialize their destination
but do not create reusable versions. Unknown storage is not presumed disjoint.

`same_storage_version` checks nonempty exact ranges of the same logical home
within one invocation; every byte must have a known matching version. It never
proves anything about a pointee or permits external accesses to move. Stronger
memory equivalence and placement consumers require later contracts.

## Qualification

Independently authored graphs cover malformed definitions, partial writes,
parallel edges, loops, irreducible cycles, escaping/addressable homes, uncertain
accesses and stale/foreign handles. Integration tests exercise real mixed record
code in raw/optimized mode and both LF and CRLF, and ensure native emission rejects
forged use-before-definition before placement. A compile-fail example checks the
immutable borrow.

The [stage-1 evidence](benchmarks/65816-record-placement-stage1/README.md) compares
the frozen Exec816 workload and all representative routines with stage 0. The
[implementation plan](MIR65816_RECORD_VALUE_PLACEMENT_PLAN.md) defines subsequent
resource and placement verification work.
