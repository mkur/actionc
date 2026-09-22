# Native 65816 analysis and checked-rewrite foundation

Status: in progress. Implementation slices 0 and 1 (baseline gate and typed
physical effects) are [complete](MIR65816_ANALYSIS_EFFECTS.md), as is slice 2
([selected actions and CFG](MIR65816_SELECTED_ACTIONS.md)) and slice 3
([physical homes and backward liveness](MIR65816_HOME_ANALYSIS.md)). Slice 4
([stored definitions and read attribution](MIR65816_HOME_DEFINITIONS.md)) is also
complete, as is slice 5 ([register/flag liveness](MIR65816_MACHINE_LIVENESS.md)).
Typed replay and checked rewrites remain planned.
This foundation takes priority over further temporary-store
elimination, mutable-counter promotion and broader register/DP allocation.
The qualified baseline is `a73dab7`, with compiler selection at `1ce9624` and
[saved INX results](MIR65816_LOOP_INX.md). The foundation must preserve that
output; it makes no code-size or cycle-saving forecast.

The [implementation plan](MIR65816_ANALYSIS_REWRITE_IMPLEMENTATION_PLAN.md)
defines module ownership, commit-sized delivery slices, the unchanged-output
baseline, replay integration and the first production consumer.

## Purpose and existing contracts

Provide shared answers to three questions: which stored definitions can still
be read, which register lanes and flags can still be observed, and whether a
proposed rewrite preserves every required effect. Later optimizations should
consume those answers through checked plans.

The existing [temporary interference analysis](../src/mir65816/emit/liveness.rs)
already supports home reuse. It does not establish that an individual emitted
store is dead. The [state tracker](MIR65816_STATE_TRACKER_DESIGN.md) records
forward value/mode/stack facts; backward liveness answers a different question.
Keep both, including closed-operation interference and the current X reservation
contract. A dead register value does not release a selector's reservation.

Native MIR contains typed computation but leaves many register uses and flag
dependencies implicit until selection. The
[tracked emitter](../src/mir65816/emit/tracked.rs) writes bytes immediately,
while [Code](../src/mir65816/emit/code.rs) retains fixups and proof metadata.
Instruction boundaries and optional state traces are insufficient as the input
to general machine-state rewrites. Establish a compiler-owned typed view of
selected instructions, effects and control flow first.

## What to adapt from MIR6502

| Reference | Native adaptation |
| --- | --- |
| [Home liveness](../src/mir6502/analysis/home_liveness.rs) and [home definitions](../src/mir6502/analysis/home_definitions.rs) | Byte-range liveness and identity of each particular write, including physical overlap, reused slots and partial word writes. |
| [Machine liveness](../src/mir6502/analysis/machine_liveness.rs) | Native register lanes, independent flags, mode-dependent effects and ABI-specific boundaries. |
| [Sites and generations](../src/mir6502/analysis/sites.rs), [analysis snapshots](../src/mir6502/analysis/posthome.rs) | Immutable routine/allocation/selection snapshots; reject stale sites and facts. |
| [Proof context](../src/mir6502/rewrite/context.rs), [plans](../src/mir6502/rewrite/plan.rs) and [driver](../src/mir6502/rewrite/driver.rs) | Explicit obligations, effect changes, rejected-plan reasons and transactional application. |
| [Shared dataflow solver](../src/analysis/dataflow.rs) | Reuse the existing finite monotone solver with a native selected-code CFG adapter. |

Adapt contracts and regression scenarios. Keep target effect models separate;
6502 byte registers, zero-page ownership and ABI boundaries are not native
65816 contracts. Generic register-value availability across joins, known-callee
preservation summaries and profitability models remain later extensions.

## Ordered delivery

### 1. Typed selected-code view and effects

Record typed operations, symbolic targets, instruction sites and non-instruction
obligations at the existing emission boundary. Start with an immutable sidecar
of the actual selected sequence so selection and encoding remain unchanged.
Effects must come from the same admitted forms that update the tracker; avoid
a separately maintained opcode decoder or high-level MIR approximation.

The view must include prologues, epilogues, guard paths, staging/copies, helper
sequences, internal comparison labels, branch fallthrough, and call/continuation
edges. Model a compound dispatch exactly or expand it into typed internal nodes.
Give sites stable identities within a generation, independent of byte offsets
and later branch relaxation. Preserve source-MIR attribution separately.

Each effect distinguishes reads, definite writes and possible writes, including
memory observability, registers, flags and execution-environment requirements.
Cover every emitted form; an unsupported effect blocks a proof. Call summaries
must include argument reads, result writes, scratch clobbers and memory effects.
Unknown effects cannot silently become empty read sets or definite overwrites.

Acceptance: deterministic records reconcile with the complete serialized code,
fixups and control flow in every corpus build. No bytes, maps, frame layouts,
existing proof events or measurements change.

### 2. Home liveness and stored-definition facts

Use backward may-liveness over concrete home bytes: a byte is live if any
reachable path reads its current contents before a definite overwrite. Take
the union at joins and iterate through loops, including backedges that read a
candidate store before the next execution of that store. Unknown may-writes do
not kill liveness; unknown reads conservatively observe possible aliases.

Distinguish invocation-owned stack bytes, domain-relative DP bytes, ABI homes,
fixed/addressable objects and unknown memory. Normalize stack accesses through
the verified entry-S equation; the same displacement at different S values
is not the same byte. Reused logical homes can overlap physically. Track each
write by site as well as byte range so a later write to the same home cannot
justify deleting a still-observed earlier definition. Byte stores kill only
the bytes they definitely replace.

Protect incoming/outgoing arguments, return homes, addressable/volatile storage,
runtime metadata and stack guards. Private scratch is not automatically dead
at a call: argument and helper reads occur before their clobbers. Model calls
using the existing ABI and helper contracts; leave uncertain aliases blocked.

Acceptance: independently specified liveness/definition tests cover diamonds,
loops, partial overlap, reused homes, stack movement, calls and unknown aliases.
Expose read-only queries and blocked reasons; remove no stores or reservations.

### 3. Register and flag liveness

Use backward liveness on the same selected CFG. Track A low/high lanes, X/Y
lanes and individual N/Z/C/V flags. Model A8 writes preserving A's high byte,
index narrowing clearing high bytes, transfers and arithmetic at their actual
widths, carry-chain inputs and branch flag reads. Matching N/Z alone does not
prove C/V equivalent, and dead flags do not prove a memory read removable.

Treat S, the direct-page register, DBR, native mode, M/X width bits, decimal mode
and interrupt-mask requirements as protected environment obligations initially.
Seed exits and calls from the native ABI, including defined result lanes and
restoration requirements; do not import 6502 return-home assumptions. A call's
clobber is not permission to ignore values it reads on entry.

Retain full CPU/frame/DP restoration under preemption. The liveness model may
rely on that existing domain-isolation contract, but cannot reduce saved state
or interrupt reserves, or assume an interrupt inspects arbitrary private temps.

Acceptance: test live-through joins, individual flag consumers, carry chains,
width changes, hidden accumulator lanes, helper/call boundaries and return
lanes. Analysis results remain observational; compiler output stays identical.

### 4. Checked rewrite plans and transactional application

Add a native proof context returning `Proven` or an explicit blocker, plus plans
bound to a routine, allocation and selected-code generation. Plans name the
original typed window, replacement, removed home definitions, changed register
lanes/flags and invalidated facts. Liveness is one obligation, not a general
proof of equivalence: replacement value semantics, memory ordering and every
observable effect also require a checked rule.

Initially preserve CFG, modes, allocation, guards, calls and ABI placement.
Reject a removed definition if it is read outside the replaced computation;
also reject replacement reads that depended on a removed definition. Require
changed live machine values to be proven equivalent. Existing tracker events
such as capture witnesses and X refreshes remain protected dependencies until
replacement proofs explicitly re-establish them.

Mutation requires a replayable typed sequence, built from the selected-code
view and passed through the tracked emission boundary. Establish that replay
is byte-identical before accepting edits. Never splice raw machine bytes using
analysis of a different sequence. Rebuild offsets, fixups, MIR spans, traces
and branch layout from the accepted sequence.

Apply one plan to a scratch candidate, reverify its obligations and emission,
then commit it atomically and rebuild analyses. Stale plans and failed checks
leave the original untouched. Begin with full invalidation and deterministic
bounded iteration; selective invalidation and interacting batches are later
work. Invalid source MIR remains an error rather than a silent fallback.

Acceptance: test stale generations, wrong routine/allocation, overlapping
windows, live removed definitions, introduced reads, undeclared clobbers,
changed flags, invalid stack/mode transitions and rollback. Synthetic/no-op
transactions exercise the driver before any new optimization is enabled.

### 5. Migrate one existing optimization without expanding it

Route an existing local A16 forwarding case through the checked machinery,
retaining its current eligibility and exact A/N/Z/home requirements. Establish
the candidate window before its omission; the emitted stream alone no longer
contains the eliminated load. Keep stores, homes and allocation unchanged.

Acceptance: identical selection/rejection cases and output across the complete
baseline, plus proof-failure fallbacks. This demonstrates that the foundation
serves production selection. Only then choose and forecast a new optimization,
such as private temporary-store elimination, in its own implementation plan.

## Qualification and completion

Commit each completed slice. Freeze hashes of the INX baseline before compiler
work. Use the existing [strict equality gate](../tools/compare65816/check_state_tracker.py)
with a fresh baseline manifest: all 28 Action images, artifact contracts and
264 corpus records must remain equal, including the known vbcc unlink failure.
Qualify raw/optimized output, both incoming I states and debug/release VM hosts.
Keep ABI v1, image v3, o65 profile v1, guards and Exec816's pin unchanged.

Port MIR6502 proof/alias/CFG regression scenarios and add native width, bank,
stack and domain cases. Validate declared effects against independent assembly
and VM execution; production records are claims to check, not their own oracle.
Run affected library/integration checks and native execution coverage for calls,
helpers, relocation, faults and preemption as each boundary changes. Rebuild
newline-sensitive fixtures in an isolated CRLF checkout. Follow contributor
checks if implementation introduces any NIR or semantic boundary changes.

Completion means stable analysis queries, exhaustive conservative effects,
checked stale-plan rejection, atomic verified rewrites and one unchanged
production consumer. Larger optimization gains are subsequent work.
