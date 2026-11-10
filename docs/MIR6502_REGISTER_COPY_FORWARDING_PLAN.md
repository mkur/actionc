# MIR6502 Register-to-Store Copy Forwarding Plan

Status: All slices implemented and qualified. Created 2026-11-10.

Implement a post-home copy rewrite that stores already available byte
values directly from A, X, or Y instead of routing them through private scratch.
TN's `Strcpy` is an observation workload and acceptance case, not a matcher.
Routine names, parameter names, particular scratch addresses, pointer-pair
addresses, and a particular number of copied bytes must not select the rewrite.

## Baseline and concrete target

Start from `f0524524`, following the absolute-readback and MADS-export fixes.
The maintained TN cartridge build is 9,974 XEX bytes at `$2C00`:

```sh
cargo run --locked --bin actionc -- --mode mir6502 --runtime cart \
  --listing build/register-copy/before/TN.asm \
  -o build/register-copy/before/TN.xex samples/tn/modern/TN.ACT
```

Its XEX SHA-256 is
`c1d8b354a78d6347e8c6fa1723b970257649258e4d8d7c3bc96834f8a7020676`.
Save TNDBG and generic reproducer baselines before implementation too; do not
reuse TNDBG measurements from before the correctness fixes.

`Strcpy` is 53 emitted bytes. Its post-home MIR already exposes the relevant
eight operations; they are not hidden inside an emission pseudo operation:

```text
a = y
store scratch0, a
a = load parameter_high
store scratch1, a
a = load scratch0
store destination_low, a
a = load scratch1
store destination_high, a
```

At entry to this region, Y contains the desired low byte and A contains the
desired high byte. The source parameter homes have already been written and
must remain written. The target replacement is:

```text
store destination_low, y
store destination_high, a
```

The current assembly occupies 16 bytes and 24 nominal NMOS cycles. `STY zp;
STA zp` occupies four bytes and six cycles: a local target of 12 bytes and
18 cycles saved. This would make this routine 41 bytes if nothing else changes;
do not assert a particular whole-program size when other sites also improve.
Preserve the existing final A value and any live flags, as well as both stores.

### TN candidate inventory

A static scan of the corrected TN materialized MIR finds the adjacent
`TYA; store direct,A` shape only in `Strcpy`. The broader available-register
load/store rule has another concrete candidate in `Copy`:

```asm
LDY local_Copy_files
STA local_Copy_copytab,Y
LDA local_Copy_files
STA $E4
```

Y already contains the loaded byte at the last pair. Replacing that pair with
`STY $E4` would save three bytes and four nominal cycles. The indexed store
before it must be proved disjoint from `local_Copy_files`; its table backing
and the scalar home occupy separate physical ranges in this listing. The next
instruction loads A from `$0358`, overwriting the changed A/N/Z state. Keep
that absolute read intact. This is a candidate pending the actual shared
alias/liveness proof and execution qualification, not a measured applied gain.

Use both routines to measure this family. `Push` also stages X in scratch, but
its consumer is an indirect store, outside the initial direct-store scope.
`Path` and `PrintB` have bitwise-expression staging that needs a different
rewrite. Do not count those as benefits of this plan, or expand the selector
to reach them merely to increase the TN site count.

## Existing support and actual gaps

- `analysis/machine_values.rs` already supplies routine-wide A/X/Y and N/Z
  provenance. `PostHomeRewriteContext` exposes these facts with snapshot
  generations. It is not necessary to build a replacement global tracker.
- A direct store from X or Y already emits `STX` or `STY`. No emitter change or
  new instruction form is needed.
- `discover_txa_direct_store_folds` currently recognizes only `TXA; STA direct`,
  and requires changed A/N/Z state to be unobservable. It does not cover Y or
  recover a complete multi-byte staging region.
- The generic post-home driver builds snapshots without a physical memory map.
  Machine-value analysis consequently invalidates memory-backed register facts
  on direct writes whose overlap cannot be resolved. Final register-reload
  folding already demonstrates how to provide `MirMachineMemoryMap` instead.
- Existing cleanup does not turn an available X/Y byte into the source of a
  later load/store pair. Ordinary store transfer can also rename the register's
  value after the destination home. Rebuild facts between rewrites; do not
  assume that one unchanged snapshot describes a whole sequence of edits.
- `structural_plan`, reaching home definitions, machine liveness, and the
  post-home transaction driver already check removed definitions, replacement
  dependencies, changed exit state, stale snapshots, and verification.
  They do not by themselves prove that newly selected store operands carry
  the original values. This family must establish that equality explicitly.

Relevant implementation locations:

- [machine values](../src/mir6502/analysis/machine_values.rs)
- [post-home snapshots](../src/mir6502/analysis/posthome.rs)
- [home definitions](../src/mir6502/analysis/home_definitions.rs)
- [rewrite context](../src/mir6502/rewrite/context.rs)
- [structural plans](../src/mir6502/rewrite/posthome.rs)
- [transaction driver](../src/mir6502/rewrite/driver.rs)
- [current store folds](../src/mir6502/materialize/peepholes.rs)
- [pass scheduling](../src/mir6502/materialize.rs)

## Ownership and scope

This is MIR6502 target placement over verified, typed post-home MIR. SemIR and
NIR contracts, source evaluation rules, the ABI, and emission remain unchanged.
Use existing direct byte `Load`, register `Move`, `LoadImm`, and `Store` forms.
No source-text lookup, `Strcpy` intrinsic, new pointer operation, or assembly
text peephole is permitted.

Initially rewrite adjacent byte operation pairs within one block:

```text
a = x-or-y                a = load ordinary_direct_home
store direct_dst, a       store direct_dst, a
```

Replace either pair with a store from the proven available A/X/Y register.
Compose these small rewrites and existing cleanup to a fixed point. This is
general across register choices, source homes, destination addresses, and
copy widths; multi-byte copies are sequences of byte operations. Do not require
a pointer pair, adjacent destination bytes, or an eight-operation template.

Do not match across a call, helper, machine block, barrier, arithmetic operation,
indirect/indexed access, or CFG edge. Inherited availability at block entry must
remain a shared must-agree fact; this task adds no new cross-call or loop proof.
Cross-call pointer reuse, loop promotion, arithmetic reassociation, a general
register allocator, and the other arithmetic review examples are separate work.

## Proof and rewrite contract

1. Query the immutable snapshot immediately before the first operation of the
   pair. For a register move, use that source register. For a direct load,
   compare its current byte identity with available A/X/Y values using shared
   analysis. Unknown incoming A/X/Y values are not equal merely because they
   all lack facts. Prefer A when already equal, otherwise an available X/Y.
2. Build snapshots with the resolved physical memory map, as final reload
   folding already does. Extend the driver's entry point to accept that map
   or a map-building callback; retain existing callers and conservative
   defaults. Rebuild it as necessary when a batch changes referenced homes.
   Do not introduce a second tracker or recover facts from emitter state.
3. Resolve physical aliases as well as logical identities. Source and scratch
   homes may overlap after placement. Ordinary writes must invalidate all
   overlapping value facts; unresolved addresses remain conservative. A named
   object is not automatically ordinary RAM if its backing is absolute.
4. Prove a removed memory read independently removable and its exact current
   value already available. Raw absolute/hardware reads, absolute-backed
   aliases, volatile accesses, and unknown pointer/indexed reads are excluded.
   A layout address alone is not permission to remove a read. Preserve the
   readback contract established by `c190ccfd`.
5. Keep the destination store at its original logical position, with the same
   address, width, and byte value. No store is moved across a memory operation.
   Permit existing direct addressing forms that support the selected register;
   add no indexed-store selection or new addressing mode.
6. Prove that any live final A value is unchanged, or declare A changed and
   require it dead. Do the same for N/Z: deleting a load or transfer deletes its
   flag production, whereas STX/STY/STA do not set N/Z. Matching value provenance
   can prove equality; otherwise require deadness. Preserve C/V and the other
   registers. A missing exit-change declaration is not an equality proof.
7. Use `structural_plan`, `MirPostHomeRewritePlan`, and the existing driver to
   check dependencies, snapshot generations, liveness, and verification before
   applying the pair. Recompute facts after each accepted batch; never apply
   later decisions using the register contents of the original sequence.
8. Let existing cleanup remove newly dead private scratch definitions, using
   routine-wide reaching-definition and liveness proofs. Later, successor,
   address-operand, call-argument, and terminator uses must keep the stores.
   Keep source locals, parameter stores, and public ABI/pointer destinations.
   Initially retain allocations and existing accounting; do not delete frame
   objects by hand.

For TN, A and Y are unchanged at exit, and the incoming N/Z already describe
the high byte after the ABI prologue load/store. That is a fact to prove, not a
property to assume for every superficially similar sequence.

## Implementation slices

### Slice 0: freeze neutral reproducers and diagnostics

Save listings, materialized MIR, maps, XEX sizes, home counts, and hashes at the
baseline for TN/TNDBG and small neutral programs. Include a byte copy through
scratch and a two-byte captured-address preparation followed by a real
consumer. Change names, declaration order, and destination pairs between cases.

Add candidate/blocker telemetry using typed copy facts. Record candidate pairs
and distinct proof failures: unavailable value, live changed register/flags,
observable read, physical alias, unsupported form, and cost. Existing cleanup
telemetry can explain retained scratch definitions. Revisited fixed-point candidates
must not be reported as additive realized savings.

### Slice 1: generalize direct register stores

Extend the existing X-to-A/direct-store family to Y using one register-generic
implementation, rather than adding a separate TYA pattern. Keep direct
addressing and existing liveness requirements. Add neutral X/Y cases and
negative live-A/live-flag cases. Remeasure before adding a broader selector.

### Slice 2: available-register load/store forwarding

Add the direct-load/store form using snapshot value equality and the physical
map. Preserve original A/N/Z when proven equal, otherwise use existing deadness
requirements. Run this family and the register-move/store family to a bounded
fixed point, rebuilding facts after accepted changes. Follow with existing
dead-register/private-store cleanup; repeat only while a strictly decreasing
operation/byte metric shows progress. Retain the driver's bounded convergence
mechanism; the initial operation count plus one bounds a single-plan fixed point.

In the motivating region, this should allow the following sequence of general
steps, each justified independently:

1. Replace `TYA; store scratch0,A` with `store scratch0,Y`, leaving high in A.
2. Elide the now-redundant high load before storing scratch1.
3. Replace `load scratch0,A; store low,A` with `store low,Y`.
4. Elide the high reload once A is no longer clobbered by the low transfer.
5. Remove scratch0/scratch1 stores only when existing dead-definition cleanup
   proves that they have no remaining uses.

These are an explanation of the expected fixed point, not a required operation
template. Do not implement a new copy graph or eight-operation matcher if the
shared queries and pair rewrites achieve the generic and TN acceptance cases.
If they do not, record the precise missing fact or scheduling dependency and
reassess before expanding the design.

Run after ABI prologues and byte materialization expose the copies, with
physical zero-page mapping available. Revalidate aliases if allocation later
changes; do not carry a physical proof across a remap.

Use an independent optimized-configuration switch, proposed name
`enable_register_copy_forwarding`, for feature-on/off qualification. Preserve
the existing TXA optimization when the new feature is disabled. New behavior
is initially optimized-only and remains under `enable_peepholes`.

Only accept strictly smaller resolved instruction sequences with no estimated
cycle increase. Check final emitted routine/image sizes and occupied zero page
against the same configuration with this feature disabled. A final-layout,
verification, alias, or profitability failure must retain the previously
accepted program; use the established program-level cost/rollback workflow
when a local proof cannot establish those final properties.

### Slice 3: qualification and contract update

Document admitted copy operations, rejected observable forms, live-state
guarantees, and application order in
[the pseudo-machine contract](MIR6502_PSEUDO_MACHINE_CONTRACT.md). Record final
measurements in this plan, replacing estimates with actual emitted results.
Keep implementation/proof coverage and final qualification in focused commits.

## Regression coverage

Positive cases:

- X and Y direct stores, one-byte forwarding, and multi-byte forwarding using
  incoming A plus X/Y; nonconsecutive destinations and renamed routines.
- RAM spills and virtual zero-page scratch, distinct allocations/origins, and
  equivalent values reached through more than one copy.
- Live final A preserved by avoiding its temporary clobber; live matching N/Z
  retained by proof, and dead flags permitted by normal liveness.
- Source-level address capture/indirect consumption with different valid
  pointer values and destination pairs, not just isolated instruction tests.

Negative cases:

- Scratch reads later in the block, a successor, an address operand, a call
  argument, or a terminator; reads of an older or ambiguous definition.
- A/X/Y overwritten before use, different live final A, different live N/Z,
  and register-value facts invalidated by memory writes.
- Logical homes sharing a physical byte, cross-lane source/destination overlap,
  exposed scratch aliases, out-of-bounds homes, and unknown final placement.
- Raw absolute and hardware readbacks, volatile accesses, indirect/indexed
  accesses, calls/helpers, machine blocks, and barriers.
- A proposed replacement still depending on a removed scratch definition,
  stale snapshot generation, overlapping candidates, convergence-budget
  exhaustion, and cost rejection.

Use an independent execution oracle for original/replacement sequences. Cover
all 65,536 low/high byte combinations for the small two-byte form when practical;
do not merely compare the matcher to its own reconstruction. VM regressions
should cover boundary bytes, repeated calls, RAM data/guards, pointer page
crossings, relocated addresses, and stack balance. Exercise cartridge and
standalone runtimes and the relevant raw/optimized NIR materialization paths.
Normalize checked-out fixture text before newline-sensitive instrumentation,
then compile both LF and CRLF through the actual parser.

## Validation and acceptance

During development run focused analysis, rewrite, emitter, and new VM cases.
For final qualification run the full MIR6502 unit filter, all 12 current
`mir6502_*` integration targets, and the emission sweep:

```sh
cargo test --locked --lib mir6502
cargo test --locked --lib codegen::tracked_emitter
cargo run --locked --bin actionc-mir6502-sweep -- --emit-load fixtures/mir6502
tools/check-mads-listings.sh
```

Run the new forwarding VM target plus the existing `absolute_readback`,
`tn_dispatch`, `tn_codegen_reproducers`, and relevant ABI/pointer-copy targets.
Validate the new fixture through both listing variants and the MADS oracle.
No NIR/printer contract or fixture change is intended. If implementation crosses
that boundary, also run all shared-contract checks required by
[AGENTS.md](../AGENTS.md), including `cargo test`, and explain fixture changes.

Completion requires:

- Generic one-byte and multi-byte cases improve, independent of names or fixed
  addresses, and the maintained TN `Strcpy` staging is eliminated.
- The local 12-byte/18-cycle target is met, or a stronger proved schedule is
  measured and explained; correctness must not be relaxed to meet that number.
- Emitted code/image size and occupied zero page do not grow for accepted
  rewrites. Report TN/TNDBG, per-routine bytes, scratch traffic, and execution
  cycles independently; do not add overlapping estimates.
- Absolute-readback, alias, liveness, ABI, LF/CRLF, emission, and MADS reassembly
  regressions stay green. The recently fixed readbacks remain present.

Stop at this scope. If TN requires a new cross-call, loop, or pointer-escape
proof, report the blocker and revise the plan explicitly before expanding it.

## Slice 0 results

Added neutral counted-pointer and direct-byte fixtures, a final-program
candidate/blocker census under the existing peephole report, and source-level
VM oracles. The census does not rewrite operations or change emission; a CLI
regression compares reporting-on output with reporting-off output byte for byte.
The fixture explicitly calls its observed routines so normal linking retains
them; no exported or routine-name exception is needed.

Frozen cartridge measurements are in
[baseline.json](benchmarks/6502-register-copy/baseline.json): TN 9,974 bytes,
TNDBG 13,079 bytes, and the neutral fixture 214 bytes. `Strcpy`, `CopyCounted`,
and its renamed counterpart `SendString` are each 53 bytes. Listings, maps,
materialized MIR and complete hashes are retained locally under
`build/register-copy/before/`.

Validation passed: the diagnostic/image-equivalence integration test and the
new VM target (two tests, including its symbol helper). Both runtimes and
LF/CRLF input execute byte stores, counted lengths 0/1/127/128/254/255, repeated
calls and page-crossing pointers with independent data and guard expectations.
No NIR contracts or existing fixture outputs changed.

## Slice 1 results

Generalized the existing transfer/store matcher to X/Y while retaining the
legacy X-only entry point. New Y behavior is optimized-only under
`enable_register_copy_forwarding`. It runs as a final transaction after the
previous materialization/placement choices; exact emission must be smaller,
affected routines/homes cannot grow, and frames/zero-page reservations remain
identical. Rejection restores the complete baseline. The transaction also
checks published callee A results because earlier caller rewrites may already
depend on them.

The small Y fold preserves the incoming high byte in A, allowing the emitter's
existing compiler-owned reload elimination to remove one additional ordinary
parameter load. No raw absolute read is removed.

| Cartridge workload | Baseline XEX | Slice 1 XEX | Saving |
| --- | ---: | ---: | ---: |
| TN | 9,974 | 9,970 | 4 |
| TNDBG | 13,079 | 13,075 | 4 |
| Neutral fixture | 214 | 205 | 9 |

`Strcpy` and both renamed neutral wrappers decrease from 53 to 49 bytes.
TN's `Copy` is unchanged at 676 bytes; its load/store case belongs to Slice 2.

Validation passed: two new X/Y move-store proof tests (including successor A
and live N/Z guards), all three legacy TXA tests, two integration tests with
feature-on/off comparisons over raw/optimized NIR, both runtimes and origins,
and the forwarding/readback VM targets (four tests). No fixture snapshots or
shared IR contracts changed.

## Slice 2 results

Added direct byte load/store forwarding from available A/X/Y, using the existing
machine-value and N/Z queries, the final emitted physical map, and structural
deadness checks. The driver has an optional map-building entry point; existing
callers retain their empty-map conservative policy. Private scratch cleanup
uses the existing reaching-definition proof plus exclusion of separately named
physical reads. Raw/absolute-backed reads remain ineligible.

Only one rewrite is applied per snapshot: disjoint operation ranges can still
have dependent register assumptions. Each rewrite strictly removes an operation,
so the initial routine operation count plus one bounds convergence. An exhaustive
two-lane execution regression also includes consecutive dependent load/store
pairs to detect stale batch assumptions.

The final transaction replays from the complete baseline against the proposed
emitted layout. A proof-only emission hook resolves baseline reference identities
in that layout, including scratch reads that the proposal removed. Reference
operations do not emit bytes or affect placement. Published callee A facts,
unchanged frames, verifier cleanliness and exact final cost remain mandatory.

| Cartridge workload | Baseline XEX | Slice 2 XEX | Saving |
| --- | ---: | ---: | ---: |
| TN | 9,974 | 9,959 | 15 |
| TNDBG | 13,079 | 13,055 | 24 |
| Neutral fixture | 214 | 186 | 28 |

`Strcpy` and both neutral wrappers reach 41 bytes, removing the full 12-byte
staging sequence. TN's `Copy` decreases from 676 to 673 bytes; TNDBG's `Copy`
decreases from 1,195 to 1,183. These are emitted sizes, not additive telemetry
estimates. Final backend qualification and cycle measurements follow in Slice 3.

Focused validation passed: ten register-copy proof/execution tests (including
all 65,536 low/high pairs, RAM spills, alias/successor preservation, live A/N/Z,
absolute reads, equal-cost rollback and published callee results), all 18 shared
rewrite-driver tests, both feature-control integration tests, and four tests
across the forwarding/absolute-readback VM targets. The integration cases cover
raw/optimized NIR, both runtimes and two origins; VM inputs also cover LF/CRLF.
No NIR contracts or existing fixture snapshots changed.

## Slice 3 results

Final emitted measurements and hashes are recorded in
[results.json](benchmarks/6502-register-copy/results.json), using compiler
`5aaf9d34` and the same sources as the frozen baseline. Regeneration confirms
TN at 9,959 bytes, TNDBG at 13,055 bytes and the neutral fixture at 186 bytes.
`Strcpy` and both renamed wrappers are 41 bytes. The additional direct-byte
case, `SaveBytes`, decreases from 23 to 19 bytes. The distinct occupied private
zero-page addresses and per-routine reservations are unchanged in all three
workloads; this slice removes traffic, not allocations.

The maintained TN and TNDBG XEX files were also executed directly in the VM,
calling their actual `Strcpy` and `MovePage` routines with source `$70FE` and
destination `$74FE`. Repeated boundary-length calls preserve copied data,
guards and stack balance. Both programs have identical measured totals:

| Counted length | Baseline cycles | Final cycles | Saving |
| --- | ---: | ---: | ---: |
| 0 | 129 | 111 | 18 |
| 1 | 147 | 129 | 18 |
| 127 | 2,415 | 2,397 | 18 |
| 128 | 2,433 | 2,415 | 18 |
| 254 | 4,701 | 4,683 | 18 |
| 255 | 4,719 | 4,701 | 18 |

These totals include the same test entry trampoline and the real copy loop.
Each call decreases private scratch traffic from two reads/two writes to zero.
They are whole-call measurements, not sums of candidate estimates. The artifact
probe and output are retained locally in `build/register-copy/artifact_probe.rs`
and `build/register-copy/final-tn-cycles.log`.

The maintained neutral VM regression now exercises feature-off/on configurations
over both runtimes, raw/optimized NIR, origins `$3000`/`$41C7`, and actual LF/CRLF
parsing. It verifies identical LF/CRLF images, unchanged private reservations,
boundary bytes, repeated calls, page-crossing pointers, source bytes, guards
and balanced stacks. Optimized-NIR counted wrappers save exactly 18 cycles and
four private accesses per call in every case; raw NIR already avoids some
staging and still improves. `SaveBytes` saves six measured cycles. This test
does not assume that every lowering path starts with the same staging pattern.

Final qualification passed:

- MIR6502 library filter: 1,147 passed, two existing ignored tests.
- Tracked emitter: all 15 tests.
- All 12 `mir6502_*` integration targets: 98 tests.
- MIR6502 emission/load sweep: all 169 fixtures.
- MADS oracle: 55 compiler cases and 212 assembled listings, including the
  new fixture in plain/source variants, both runtimes and edited origins.
- Eight VM targets: 15 tests across `register_copy_forwarding`,
  `absolute_readback`, `tn_dispatch`, `tn_codegen_reproducers`,
  `record_base_reuse`, `indirect_bitwise`, `static_table_pointer_copy` and
  `copy_values`. TN/TNDBG run with their existing complete dispatcher, panel,
  directory, menu and copy regressions.
- Full TN MADS output matches direct compilation at `$2C00` and `$3C00`;
  TNDBG and the neutral fixture also reassemble identically at their origins.
- Rust formatting for new implementation/test files, shell syntax and
  `git diff --check`.

The pseudo-machine contract documents admitted pairs, observable-read
exclusions, exact live-state proofs, private-definition cleanup and final-layout
rollback. No NIR/printer contract, existing snapshot or shared frontend changed;
validation stayed within MIR6502 and its emission/runtime consumers. The updated
requested TN assembly is available locally at `build/tn-mir6502/TN.asm`.
