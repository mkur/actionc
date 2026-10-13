# MIR6502 record base reuse implementation plan

Status: Stages 0–3 committed; Stage 4 qualification in progress. Starting compiler revision:
`2e8aaa06efefa24d24e8881450bd858970c5640e`.
See the [maintained measurements](record-bases/README.md).

Keep a complete record address in its existing zero-page pointer pair while
successive field operations use that address. Reduce repeated setup without
changing pointer evaluation, field access order, source snapshots or the public
Action! ABI. Start with operations inside one basic block. TN and TNDBG are the
application scorecards; generic fixtures define admissible compiler behavior.

The [qualified size audit](size-audit-2026-10-13/README.md) measures TN at
16,617 file bytes and TNDBG at 20,354. TN has 132 full pointer setups occupying
1,310 code bytes. Twenty-eight repeat the same source and pair in a simple
region, occupying 280 bytes; twenty-seven follow indirect writes. `BatchBegin`
is 86 MIR6502 code bytes, `TagsReset` 97 and `TagsExtent` 479.
These are candidate-pattern budgets, not savings estimates. The listing matcher
does not prove aliases, value availability or live machine state.

For a sequence such as `batch.valid=0 batch.count=0 batch.ended=0`, the desired
shape prepares the base once and changes Y for each field. Each field write
still executes at its original point. The optimization must distinguish two
independent facts: the pair still contains the earlier address, and a later
evaluation of the source pointer would still produce that address. Neither
fact follows merely from the fields belonging to the same record type.

Use MIR6502's typed storage identities, address operands and byte offsets.
SemIR owns source meaning and callable facts; verified NIR carries those facts;
MIR6502 owns pointer-pair selection, liveness, layout and target effects.
Emission writes the selected instructions and final accounting. Recognizers
must not consult SemIR, parse printed field names or special-case TN routines.
The initial work uses existing IR forms. Any required shared fact must cross
the typed NIR boundary explicitly, with verifier and boundary documentation.

The existing implementation already provides much of the framework:

| Component | Existing responsibility | Extension needed |
| --- | --- | --- |
| [Address uses](../../src/mir6502/analysis/address_uses.rs) and [address values](../../src/mir6502/analysis/address_values.rs) | Captured values, mutable cells, offsets and complete Y windows | Stable base identity and bounded origins for reuse |
| [Temp materialization](../../src/mir6502/materialize/temps.rs) | Exact repeated setup, retaining the high-byte load for A/Z/N | Replace local invalidation assumptions with shared proofs where layout is available |
| [Machine values](../../src/mir6502/analysis/machine_values.rs) and [effects](../../src/mir6502/analysis/effects.rs) | Register/fixed-cell availability and conservative invalidation | Precise pair and source-cell preservation through proven writes |
| [Post-home snapshot](../../src/mir6502/analysis/posthome.rs) and [rewrite context](../../src/mir6502/rewrite/context.rs) | Immutable, generation-scoped facts and machine liveness | Queries exposing complete base availability and blockers |
| [SSA-lite materialization](../../src/mir6502/materialize/ssa_lite.rs) | Remove a redundant pointer-byte store when its value is already resident | Consume the stronger proofs without creating a second cache |
| [Rewrite workflow](../../docs/MIR6502_REWRITE_WORKFLOW_PLAN.md) | Validated transactions and rollback | Base-reuse proof validation and final budget accounting |

Today, temp materialization discards staged addresses after indirect stores;
machine-value analysis clears fixed-pointer facts after indirect writes and
conservatively invalidates memory dependencies after many named stores. An
ordinary field store can change the pointer source itself. Source aliases can
also place distinct declared cells at the same physical address. Keep these
fallbacks until a replacement proof establishes noninterference.

The reusable fact should identify the complete low/high value, its defining
sites, the pointer pair and any base adjustment. A captured value and a fresh
read of a mutable pointer cell require different identities. For cell-based
reuse, record both source bytes and prove their values unchanged; equality of
cell names is insufficient. A surviving captured value does not become a
license to substitute it for a later mutable-cell read.

Write proofs must cover both the destination pair and all memory dependencies
needed to establish the repeated address. Resolve named and absolute aliases
through final placement. Use exact BYTE/WORD footprints and explicitly model
16-bit wrapping ranges; an unresolved range remains unknown. Preserve ordinary
parameter captures and source pointer assignments. An unexposed home, private
ABI classification or record type alone is not a proof that an arbitrary
numeric pointer cannot reach that home.

Calls, runtime helpers, machine blocks, barriers, hardware/volatile accesses
and unproved indirect writes end the initial reuse region. Pair advancement,
one-lane clobbers and nested address preparations invalidate the affected
pair. RHS evaluation can prepare a different pointer: inspect its actual
effects before retaining the destination base. Reusing an address must never
cache, remove, duplicate or speculate a pointee read or write.

Apply the plan in these vertical slices, committing each implemented slice
with its focused checks and measurements:

1. **Stage 0: pin candidates and establish the proof boundary.** Reuse the
   audit's qualified compiler and four root images. Add a generic
   `record-base-reuse.act` fixture and a measurement driver that emits typed MIR
   at the relevant phases, final listings, image hashes and per-routine budgets.
   Cover read/read, store/store and mixed field sequences, mutable pointer
   cells, captured values and nested RHS accesses. Map each audited repeat to
   typed operation sites and classify the actual blocker. Identify where
   source identity is lost during lowering and materialization, and pin a
   phase where identity and physical alias checks are both available. Check
   the automatic-storage and scratch ownership contracts before proposing any
   stronger separation rule. This slice changes diagnostics and coverage;
   production output must remain byte-identical. Its deliverable is a split
   between immediately provable cases and cases needing stronger origin facts.

2. **Stage 1: add shared base and write-preservation proofs.** Extend the
   existing generation-scoped analyses with complete base availability, exact
   source dependencies and write footprints. Expose narrow queries through
   the rewrite context rather than adding recognizer-specific alias scans.
   Start with final-layout disjoint direct writes and bounded addresses already
   derived from storage identities. Track both pair lanes; partial knowledge
   cannot satisfy a WORD query. Preserve unknown-write invalidation in the
   general effect summary, and use a separate positive proof for an individual
   reuse candidate. Unit coverage must distinguish logical identity from
   physical aliases, same-value rewrites from mutations, wrapped ranges and
   missing facts. Production selection stays unchanged. Document the precise
   guarantees in the
   [pseudo-machine contract](../../docs/MIR6502_PSEUDO_MACHINE_CONTRACT.md).

3. **Stage 2: select reuse across field reads and disjoint direct stores.**
   Add an opt-in selector for a complete resident base established earlier in
   the same block. Retain the existing pair and schedule each original field
   access with its own Y displacement. `ActivateLocation` is the first TN
   sentinel: it captures `directory` into a local, rebuilds the global record
   base, then reads `unit`. Final placement must prove those local stores do
   not mutate the global pointer or scratch pair. Preserve all pointee loads
   and source captures. Remove setup loads only when their ordinary-memory
   nature, dependencies and A/Z/N effects permit it; otherwise retain the
   high-byte state repair or the entire fallback. Use the existing transactional
   machinery and independently validate the effect delta. Require a real final
   code saving, with no growth in homes, image bytes or occupied zero page.

4. **Stage 3: admit field writes with proven destination ranges.** Extend the
   same selector to BYTE/WORD stores whose complete footprint is proved
   disjoint from the pair and any mutable source cells. Begin with storage
   address origins already available in typed MIR; unknown parameter values
   remain barriers. Retain RHS captures, store byte order and all field memory
   effects. Keep existing Y-window rebasing when a WORD spans offset 255; reset
   reuse when the required base adjustment changes rather than introducing a
   new arithmetic scheduler. Generic header initialization and reset sequences
   should demonstrate savings. Measure `BatchBegin`, `TagsReset`, `TagsInit`
   and `TagsExtent`, recording which candidates still fail and why. Do not
   weaken the unknown-pointer fallback to force a TN result.

5. **Stage 4: recover bounded parameter origins where justified.** Use the
   Stage 3 blocker census to implement the smallest missing provenance slice.
   Where existing storage facts do not establish disjointness, a bounded
   caller-origin analysis may supply it: trace resolved argument values from
   known storage addresses, preserved captures and bounded offsets. A callee
   fact must cover every possible admitted entry and caller, retaining a
   conservative union of origins. Escaped/observable entries, indirect or
   unresolved callers, recursive uncertainty, pointer-cell mutation and
   unexplained numeric casts retain unknown facts. A direct-call-only census
   is insufficient if machine or external entry remains possible. Do not
   infer an index bound from a record or array capacity. If the required
   callable or storage fact is absent from verified NIR, add it in a separate
   shared-contract slice before consuming it in MIR6502. Rerun affected shared
   checks for that slice. This stage may recover parameter-based header
   sequences; it does not promise that all 27 write-barrier candidates become
   legal. Stop at the proved subset, with explicit fallback reasons.

6. **Stage 5: qualify and enable the measured subset.** Compare feature-off
   and feature-on output through the same final emission path. Rebuild facts
   after accepted transactions, reject stale plans, and roll back the complete
   candidate on proof, verification, placement or cost failure. Require
   non-growing final code and parameter/local/spill storage for each selected
   routine, no increase in whole-image bytes, and no added occupied zero-page
   bytes. Report measured cycles and pointer-scratch traffic separately from
   static bytes. Enable optimized selection only after generic boundary cases,
   focused TN behavior, both runtimes and backend qualification pass. Publish
   the applied/blocked census, root sizes and artifact hashes. Keep subsequent
   CFG-wide, loop or call-crossing reuse as separately scoped work.

The semantic regression matrix must exercise these distinct boundaries:

| Boundary | Required result |
| --- | --- |
| Consecutive BYTE/WORD fields and mixed reads/writes | Identical values, memory order and untouched neighbor bytes |
| Pointer points into its own source cell; either lane changes | Later cell-based access observes the mutation |
| Earlier captured base followed by source-pointer mutation | Later captured-value access retains the earlier address |
| Named aliases, fixed/absolute homes and frame-address exposure | No inferred disjointness without a valid range proof |
| Pair low/high clobbers and RHS prepares another address | Re-establish the base or retain fallback |
| Calls, helpers, machine blocks and volatile accesses | Initial selector retains barriers and observed access count |
| BYTE offset 255, WORD offsets 254/255/256, page crossing and `$FFFF` wrap | Exact effective addresses and original byte order |
| Branches, joins, loops and live A/X/Y/C/Z/N/V | No unsupported region extension or loss of live state |
| ABI captures, source pointer contents and `$A0/$A1` results | Public behavior and retained source storage remain correct |
| Feature off, stale proof, verifier failure and unfavorable final layout | Complete, deterministic fallback with unchanged reservations |

During development, run the affected analysis/materialization unit filters,
MIR6502 integrations and the focused VM targets
`address_values`, `record_consumer_boundaries`, `comparison_loops`,
`tn_codegen_reproducers`, `tn_directory_model`, `tn_mydos_directory` and
`tn_size_profile` as their consumers change. Add a focused `record_base_reuse`
VM target for the new generic cases. Exercise cartridge and standalone
runtimes, feature off/on, and raw/optimized IR on the distinct proof cases.
Use actual emitted execution to check memory and machine behavior; counting
matching instruction strings is not sufficient.

Final substantial backend qualification runs the full MIR6502 unit and
integration suite, its emission sweep and the required 6502 VM/runtime suite.
Preserve existing comparison-loop, argument/result and address-value behavior.
Normalize host fixture text before newline-sensitive instrumentation and
compile actual LF/CRLF fixture and TN/TNDBG source copies. Both variants must
produce equivalent artifacts and execution results. Record an intentional
MIR fixture change as target strategy or a bug fix; do not silently refresh
snapshots. Changes to NIR, semantic lowering, verification or printing also
require the repository's shared-contract checks:

```sh
cargo test nir_fixtures_match_snapshots
cargo run --bin actionc-nir-sweep -- fixtures/nir
cargo test
```

The scorecard records code, loaded homes, whole XEX size, occupied zero page,
pointer setups, executed scratch writes and cycles. Keep TN and TNDBG separate
and retain classic images as controls. Report raw candidate bytes separately
from qualified net savings. Retain the small compiler/fixture/image/listing
artifacts and remove disposable build caches after qualification; the plan
does not require another large historical compiler checkout.
