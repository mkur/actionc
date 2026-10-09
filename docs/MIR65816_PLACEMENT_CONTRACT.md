# Native 65816 operation resources and placement

MIR65816 owns operation resource requirements, value placement and transfers.
The immutable [logical routine analysis](MIR65816_LOGICAL_ANALYSIS.md) supplies
definitions, complete uses, widths, representations and CFG facts. Selected
instruction effects remain authoritative for physical reads, writes, clobbers
and execution state. Neither resource descriptions nor placement grant new
memory aliasing or instruction-rewrite permissions.

The [argument and result flow design](MIR65816_CALL_FLOW_DESIGN.md) defines
call-boundary ownership within the current ABI. Sole adjacent native Returns,
byte/word zero tests and exact-width private local Stores use checked output
ownership. Bounded private byte/word inputs can share a complete Direct call.
These forms retain separate input and output permissions; indirect calls,
cross-block native outputs and wide register arguments retain their existing
strategies.

## Planning and ownership

The routine placement plan constructs operation descriptions before allocating
homes. It owns the existing storage-demand decisions, final allocation, checked
pointer and scalar read bindings, accumulator intervals, redirected local loads,
deferred address components, adjacent assignments and the bounded X mirror.
Selection consumes these admission results instead of reconstructing competing
home-demand or residence plans. Existing selectors implement their qualified
forms; mixed residence extends their checked input/home choices within blocks
and through qualified acyclic control flow.

The plan borrows one immutable MIR routine and data table. Verification
reconstructs admission and value/resource obligations from these inputs and the
final allocation. It checks the complete logical operand census, including
unreachable occurrences, pointer bases, indices, indirect targets, block
parameters and distinct parallel edges. Invalid plans fail compilation; a
profitability refusal still selects the existing conservative strategy.

Each temporary has one capture owner and an explicit read location for every
logical occurrence. A canonical allocated home is separate from a borrowed
input, register interval, deferred computation or redirected destination.
Accumulator intervals name their producer and consumer and the complete A8,
A16, A16/X8 or A16/X16 payload lanes. A three-byte deferred address expression
does not promise a live A/X value at its omitted source operation.

Pure three-byte data-pointer address results may also share a deferred owner
with their sole same-block Return. Each omitted definition retains its complete
logical use and an empty source span. Checked pointer representation identities
may connect up to sixteen consecutive address/identity operations. Unsigned
literal indices contribute checked stride-scaled byte offsets; the complete
combined displacement must fit `0..65535`. Planning and ordinary indexed
selection share that interpretation. Dynamic/signed indices, incompatible
casts, hidden uses and out-of-range compositions retain complete fallback.
The original source
read point authorizes a qualified borrowed input; the final allocation supplies
an exact captured or borrowed three-byte home. Observable Loads retain their
original site and capture. The typed `ReturnAddress` request evaluates the
address at Return and establishes A16/X8 with X.high zero, before normal frame
teardown. This computed result is distinct from a callee-origin native output.
Placement reconstruction checks expression, source, consumer and omitted spans;
fresh replay regenerates the entire low-word/bank arithmetic schedule. Atomic
trial allocation preserves preceding admissions and forbids frame/spill/peak
growth. The public ABI and bank-zero reservations are unchanged.

A native output interval is distinct from accumulator residence. Its Call table
owns the complete ABI declaration independently of memory allocation. The
callee's return defines fresh A/X tokens after the incoming call barrier; actual
cleanup must preserve the declared lanes and reach body S before publishing a
read permission. A sole adjacent Return, byte/word literal-zero Eq/Ne comparison
or private local Store consumes that permission at its own logical point.
Comparisons establish fresh flags explicitly; callee or cleanup flags never
authorize the test. Each consumer family is trialled against the preceding
qualified demand so fallback retains earlier admissions. Reserved result homes
disappear only after complete-use admission and an atomic comparison of
conservative/candidate frames, spills and local peaks.
Affinity validation consumes the same demand plan rather than recursively
reconstructing admission. A rejected trial restores that family's complete
conservative demand while preserving earlier qualified families.

Borrowed pointer aliases retain separate definitions and use sets. Their exact
authoritative input homes come from the existing complete, closed read-binding
proofs; numeric slot coincidence cannot authorize a binding. Loop writes with
unknown logical storage versions do not become memory-equivalence proofs.
The existing bounded borrowing rules separately reject intervening writes,
escapes, partial accesses, volatility and call crossings. No field read is
reused because a pointer identity survived.

## Resource windows

An operation window includes address preparation and its complete access or
computation. Its description bounds register reads/writes/clobbers, flag
dependencies, protected state changes, scratch bytes and additional stack use.
Routine-local dense window IDs form a bijection over operation points. Complete
rows retain explicit empty masks, a shared immutable resource description and
the compound-access owner. Verification reconstructs the canonical tables and
rejects holes, duplicate IDs and invalid references. Instruction checking reads
the immutable row once per source window and still derives actual typed effects
for every instruction.
Register bounds are deliberately conservative whole-operation bounds, including
the hidden accumulator byte and index high lanes; they are not smaller live
ranges or grants to retain unrelated registers through an operation.

Nonvolatile scalar/record accesses, indexed accesses, aggregate copies, basic
scalar arithmetic, comparisons, casts and address formation have qualified
descriptions. Volatile, other arithmetic and call forms are explicit resource
barriers. These barriers preserve their current selectors and ABI checks and
cannot justify extending a residence interval. Helpers and terminal forwarding
remain opaque under their dedicated independently recomputed contracts.

Ordinary selector workspaces occupy the existing domain scratch from `$80`
through `$9E`; scalar and mixed pointer residence occupies `$A0..$BF`. Pointer leaves use their
existing three-byte slots inside the same scratch reservation. The shared
workspace constants define both selection operands and resource bounds. A
description never initializes scratch. Calls retain the native ABI clobber of
all compiler scratch, registers and flags; invocation homes must preserve any
capture used afterward.

Physical scratch overlap uses complete byte ranges, not temp names or numeric
stack/DP offset equality. A resource window cannot write a protected live DP
value. Declared output and simultaneous edge destinations are checked against
the existing closed-operation interference, pointer identity/reload and
overlap-safe edge-copy proofs. The pointer reload exception still requires
complete capture before overwriting its source base.

External scalar/record reads and writes have their exact declared byte extent.
An admitted adjacent assignment has one compound access owner spanning its
load and store: the current selector may emit both accesses in the store span.
The verifier counts authoritative typed memory effects across that complete
window. This check does not claim an address-equivalence or alias proof; typed
address selection and existing execution oracles retain those obligations.

## Boundaries and transfers

The plan records every logical edge ordinal, simultaneous value binding, full
source/destination width and memory home, plus allocated transfer staging.
Existing word, pointer and mixed copy schedules retain their allocation checks;
the new plan does not substitute sequential assignment for parallel copies.

Block labels bind only after the complete plan verifies. Selected predecessor
counts and reachable entries must match the logical CFG, including parallel
edges and late backedges. Every source operation starts in the native domain,
with decimal clear, word indices and the allocated body stack anchor. Block
entry also requires word accumulator mode. The existing tracked facade checks
actual edge and return state, stack balance and all X refresh obligations.
The X mirror contract must agree exactly with the common plan and every checked
predecessor; its canonical home remains authoritative.

Pointer-stage requests must use the exact input identity and complete home
admitted at that source site. Selected calls cannot conceal additional stack
use: typed state observations must stay within both the operation's outgoing
and transfer allowance and the allocated routine peak.

## Selection, replay and artifacts

After selection, a sealed immutable contract checks actual typed effects,
resource interference, source-window coverage, external access extents and
boundary obligations. It binds to the same routine and final allocation.
Replay regenerates actions and effects through a fresh tracked emitter and
rechecks the placement contract. Final instruction rewrites and layout retain
the contract and recheck it during reconciliation. The contract is shared
immutably across selected generations of the same compilation and allocation.
Another instance cannot reuse it even if its numeric routine ID and homes match.
Mutable input changes require a new logical analysis and plan.

These checks complement the existing physical home, machine-state and rewrite
proofs. They do not introduce a new raw-byte analyzer or trust selector-provided
effect masks. The instruction definition that drives emission and replay also
supplies the checked resource observations.

Image v3 retains its tagged stack/DP home representation. A DP-only interval has
its complete, actual DP home in the map; a stack-backed cache retains the stack
home in the map. Internal read locations are never serialized as fabricated
homes. Frames and local peaks follow actual stack demand; the ABI scratch
reservation does not grow. Transport validation checks complete pool geometry,
while compilation proves lifetimes and resource ownership. A routine can contain
calls outside a DP-only value's complete lifetime. The opt-in proof interface
exposes checked counts of complete DP homes and stack-backed residence intervals.

## Mixed block-local consumers

The common plan admits complete two-byte scalar and three-byte captured pointer
values in ordinary mixed blocks. Definitions and every use come from the typed
operand census. A DP-only value has one operation definition and all reads in
one closed interval within the same reachable block. Unreachable definitions
cannot establish residence. The initial block-local admission leaves block
parameters, terminator operands, edge values, unsupported consumers
and resource barriers in stack homes. Its intervals cross neither a block
boundary nor a call. Existing closed scalar
and pointer-leaf allocations retain their dedicated admissions.
The existing fused top-bit selector also retains its mask region and required
stack captures; mixed residence must not change that region's ownership.

Whole operation input/output extents coexist, including the final bank byte.
Aligned intervals are colored deterministically into the existing 32-byte
residence pool. Pressure refuses only the affected interval. Verifiers rebuild
these intervals and exact locations from immutable MIR; missing or forged
intervals, sizes, transfers and scratch geometry are errors.

Trial frame allocation includes actual edge-copy staging and incoming argument
geometry. If the mixed choices increase fixed extent or local stack peak, the
routine retains its ordinary allocation. Removing homes must not conceal new
parallel-copy cycles or a larger reservation.

A captured pointer with later reads beyond its local interval keeps its
invocation home. A profitable prefix receives a complete private stack-to-DP
capture after its producer. Selected transfer requests name the exact temp,
source home, destination and producer point, and must appear exactly once.
Replay reconstructs the complete transfer through the authoritative typed
instruction boundary. Internal reads use the DP copy only within that interval;
barriers and later blocks read the invocation home. The cache does not extend
memory-equivalence facts through writes: every source field read/write remains
at its original operation with its original extent and order.

The size-first admission counts changes of prepared address identity rather than
repeated uses of an already prepared base. A backed pointer needs at least three
proved preparations to cover its private copy and mode cost. Complete DP pointers
avoid preparation from a stack capture. Native DP words retain equal-size
instructions; narrow scalar field captures may use the existing checked adjacent
accumulator producer/consumer contract. BYTE widening clears hidden B explicitly.
Unsupported wide arithmetic/casts retain their existing strategies. Shared NIR
promotion legality and profitability policy are unchanged;
these consumers use already available typed values.

## Acyclic branch and join residence

The extension admits complete two-byte scalar and three-byte pointer captures
through acyclic live regions. A region includes every closed operation extent,
terminator use, intervening live point and simultaneous block-parameter
definition. Its producer and all incoming obligations come from typed MIR and
verified logical definition availability. Unreachable occurrences, unsupported
uses, resource barriers, fused selector ownership and cyclic live regions
refuse admission. An edge-only preheader capture sent to a cyclic target
retains its stack affinity, avoiding a DP capture followed by a required stack
transfer without a resident operation consumer. The conservative cyclic core
also retains paths between cycles; no loop residence fixed point is claimed.

Each target has an explicit entry table of complete resident homes. For each
incoming edge, a parameter home must be established by its own simultaneous
argument binding. An inherited capture must survive in the same complete home
through that predecessor's terminator. Parallel edges retain their ordinals;
different values sent to the same target do not establish an equality fact.
Entry locations survive only when every incoming obligation is satisfied.
No pointee content or address workspace is retained at a join.

Closed CFG interference colors the new complete lifetimes against one another
and existing local captures/caches. Full byte extents, including pointer bank
bytes, determine conflicts. Actual trial allocation includes incoming geometry
and all edge staging. An extension that increases the preceding block-local
frame extent or local peak retains the block-local allocation.

Existing all-word and all-pointer edge selectors retain their contracts. Mixed
edges using DP residence receive a complete parallel-copy schedule. Identities
need no transfer; a destination may be written only after every overlapping
pending source has been consumed or captured. When no move is safe, one whole
source is captured into an invocation-owned slot, replacing every pending use
of that exact range. Partial overlap and different widths use the same byte
extent rule. Dense staging is allocated from actual captures, not argument
count; incomplete slots and overlap with live homes are rejected.

The selected mixed-transfer request names the logical edge, complete move
schedule and exact allocated staging. The placement verifier reconstructs the
request and requires coverage of every such edge, including identical parallel
edges. Fresh replay emits the same canonical typed byte operations. A8 copies
preserve incoming hidden B; a final destination-byte load restores the original
edge's A/N/Z, and the existing boundary restores A16. C/V, indices, stack and
domain state are preserved. Artifact maps show actual complete DP homes and
remaining stack/staging demand. The extensions below use the same checked
entries, resource windows and complete transfers.


## Fixed-point loop residence

Complete word and pointer lifetimes may include a conservative cyclic core and
paths between cycles. Closed liveness reaches a fixed point before allocation;
header, backedge and exit entries use the same complete incoming obligations as
other CFG edges. Lexical block order never establishes a resident value. A
call, unsupported use, volatile access or fused ownership anywhere in the live
region refuses its complete DP home. Every simultaneous binding and complete
byte extent participates in interference, including a pointer's bank byte.

Loop extension preserves the admitted acyclic plan as its fallback. Actual
allocation must not grow its frame or local peak. A deterministic size budget
includes word, pointer and mixed transfer schedules, mode changes, captures and
final A/NZ repairs. It credits only provable removal of complete stack-pointer
preparations. Unsupported legacy transfer costs use conservative lower bounds;
uncertain profitability retains the preceding plan. Edge-only preheader
captures retain stack affinity when a cyclic destination cannot reside in DP.
Pointer comparison, wide arithmetic and other unsupported loop
consumers may retain their existing homes.

Existing verified Native65816 private-storage promotion already exposes legal
loop-carried values. Its ownership, escape, initialization, alias and call-effect
rules are unchanged. Residence consumes the resulting typed MIR; it does not
promote exposed storage or infer that pointee contents remain unchanged.

## Invocation-backed segments around calls

A captured three-byte pointer live across a call retains an authoritative stack
home for its complete lifetime. Independently admitted block-local segments may
reload that capture into the residence pool before their first consumer. A
segment crosses neither an edge nor a resource barrier. Calls of every target
kind retain their full ABI clobbers; no cached copy survives a call, and no new
call-save area or bank-zero reservation is introduced. Adjacent selectors owning
A and fused regions retain their existing ownership.

Each segment must cover at least three conservative address-preparation misses,
paying for its complete private reload. Closed logical interference colors its
whole byte range against all complete homes, prefix caches and other segments.
Pressure leaves the authoritative stack strategy available. Prefix residence
and later segments cannot overlap within the same block. The public artifact
continues to report the real stack home, not the cached read location.

Typed `ReloadResident` requests name the captured temp, exact invocation source,
complete destination and first consumer point. Verification rebuilds every
segment, requires exactly one reload for each first point, and rejects a DP
consumer that precedes its reload. Replay emits the canonical complete transfer:
two overlapping private words at offsets zero and one, with no fourth-byte read
or write. Result capture and caller cleanup complete before a later operation
reloads residence. Repeated external field accesses retain their separate source
operations, ordering and exact extents.

## Indexed and aggregate consumers

Indexed loads and stores own address setup, full index evaluation and every
payload byte in one resource window. Native Y indexing admits only unsigned
captured BYTE/CARD indexes whose complete scaled offset, displacement and last
payload byte fit sixteen bits. Its size qualification is shared with residence
budgets. Complete stack and DP homes, borrowed pointer inputs and checked local
pointer segments supply the same typed operands. Narrow indexes clear hidden B
before scaling. A resident pointer is read directly when the native form is
qualified; it is never stepped or overwritten as an address workspace.

Larger offsets, wider indexes and unsupported native scales retain full modular
24-bit address formation. A complete working copy in the ordinary pointer
workspace preserves the resident capture. Deferred address formation also
materializes its displacement in this working copy. Closed input/output extents
and index/scaling scratch participate in the common interference checks. No
object disjointness, bounds or memory-equivalence fact follows from residence.

Existing deferred three-byte A/X and component-store producers retain complete
stack input bindings. Their physical input consumption can occur after the
logical address operation. Those inputs remain outside DP residence until that
later consumption has an explicit placement contract; materialized indexed
addresses use the ordinary checked locations.

Nonvolatile aggregate copies use the same ordinary workspace through `$9E`.
Both complete addresses are materialized before a typed `AggregateCopy` request
names the exact MIR byte count and overlap policy. Verification requires one
request per nonempty copy at its original source point; fresh replay rebuilds
the complete direction comparison, byte loops and 24-bit count. Static payload
site counts complement that dynamic protocol rather than claiming that a loop
executes once. Zero-length copies have no transfer request or memory effects.

The existing aggregate protocol is preserved: overlap-safe copies compare the
complete addresses, use ascending or descending byte transfers as appropriate,
and perform no payload access for self-copy. Uncertain aliasing retains that
overlap-safe strategy. Exact extents, padding bytes and large counts are preserved;
no widened or reordered external access protocol is introduced. Aggregate scratch
does not overlap the residence pool. Pressure retains complete invocation homes,
and volatile aggregates retain their explicit unsupported diagnostic.

Aggregate execution qualification crosses 64 KiB bank boundaries. Modular
24-bit wrap is qualified for scalar and indexed address formation; residence
does not supply an alias guarantee for wrapping aggregate objects.

Native result Store routes require the sole adjacent use to name a nonvolatile,
unindexed range of an unexposed mutable automatic local, with matching widths
at one through four bytes. The full object range and stack access must fit;
parameter-backed frame aliases and address uses are excluded. Publication and
consumption permissions bridge actual cleanup to the Store source point. The
semantic write stays in that Store span and uses the exact final range, including
the three-byte tail. Its real final object remains allocated and mapped; only
the intermediate result home disappears. A missing mutable fact keeps capture
fallback, including current record objects whose field writes do not set it.

Bounded byte/word argument borrowing preserves a Load's logical definition while
its physical span can be empty. One complete argument occurrence at a Direct
Call reads the owned incoming parameter or unexposed local, at the final frame's
real offset. Admission requires full canonical views, complete ownership/escape
checks and a same-block interval of at most sixteen operations without ordering
barriers or external reads. Several such definitions can terminate at the same
Call. All bindings are validated together with ordinary operands, padding and
full outgoing extent, using the actual call construction selector. Its full
reservation bound also covers every partial push depth and exact-width tail;
otherwise the complete conservative demand is restored. An admitted sealed plan
that loses a source or schedule is an error. Existing four-byte scalar captures
and pointer admissions retain their separate contracts. Calls end all borrowed
input permission before their independently owned native outputs are defined.
