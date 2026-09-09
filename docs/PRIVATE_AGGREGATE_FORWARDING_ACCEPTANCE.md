# Shared aggregate forwarding: final acceptance

Implementation: `5a6ba3b` (slice 4 CFG/subobject reuse) and `0853f8b`
(slice 5 complete ABI captures), on top of the separately committed slices 1–3.
This final slice adds cost assertions and measurements, not a new compiler pass.

## Delivered behavior and limits

Records, unions and variants share the same nominal extent, byte-region,
address-use and must-availability proofs. Stable CASE selectors and aggregate
binders can reuse their source. A guard that mutates a later-read selector keeps
its independent snapshot. Fixed nested fields use byte overlap, not different
field IDs, to distinguish independent storage.

Defined direct callees may consume the same stable whole caller capture more
than once, while the physical ABI retains independent mutable parameters.
Capture-to-capture return staging can disappear without removing the physical
return transfer. Single-use native result producers may target a fresh final
whole home under a separate publication proof. Every rewrite retains verified
NIR and refreshes analyses; no MIR-to-SemIR lookback, ABI redesign or relaxed
complete-value verifier is involved.

Remaining deliberately conservative extensions are:

- Indirect/external call capture sharing and narrower interprocedural effects.
- Atari result routing through mutable destination-pointer relays, with a
  genuine routine-static reentry/publication proof.
- True hidden-result-slot forwarding inside callees and subobject result ABI
  buffers; entry/return transfers are not all redundant.
- Dynamic source-address lifetimes and general alias/escape analysis. Unknown
  pointer writes, hardware/volatile access, faults and machine effects still
  block availability before later consumers.
- Producer coalescing with extra source consumers, including validation reads.
  Existing fresh semantic lowering and safe snapshot read forwarding still
  apply where eligible; required validation/failure-before-publication remains.

These are follow-ups beyond the delivered bounded slices, not unsupported
language cases: rejected optimizations retain ordinary staging and semantics.

## Reproducible measurements

Full data: [four-target logical NIR](PRIVATE_AGGREGATE_FORWARDING_FINAL_NIR.csv)
and [Atari physical/code/VM costs](PRIVATE_AGGREGATE_FORWARDING_FINAL_VM.csv).
The original twelve shape/case pairs are preserved; six additional ABI probes
cover two shared arguments with mutable callee values, and return capture chains.
The corpus has 144 NIR rows, 108 VM cost rows and 432 executions (four seeds,
three backend lanes, two runtimes). All executions check the entire guarded
$0600–$06FF memory window and bounded completion at code origin $3000.

Cycle counts are CPU cycles from entry to the completion store, not PAL frames
or isolated transfer costs. The raw lane is raw NIR with optimized MIR selection;
the optimized lane additionally runs NIR optimization. Classic uses modern
SemIR lowering. Native tests verify layouts/ABI/lowering, not native execution;
the existing variant Error-adapter capability diagnostic remains explicit.

Logical copy sites/byte sums and capture extents are separate from initial-MIR
ABI-expanded copy sites/byte sums. Both are static counts, not dynamic traffic.
The final VM CSV also reports distinct physical byte addresses covered by
routine-scoped storage symbols and by mapped routine code ranges. Aliases are
deduplicated; these mapped extents exclude globals/unmapped scratch, may include
mapped runtime routines, and are not peak live memory, total RAM or native frame
sizes. XEX length includes image/loader overhead and separately stored data.

All 48 historical classic/raw-MIR rows retain their slice 3 XEX sizes and cycle
counts. Historical slice 1, 2 and 3 measurement files are unchanged.

### Original corpus: optimized MIR, cartridge runtime

| Variant case | Slice 3 → final XEX bytes | CPU cycles | Initial-MIR copy sites |
| --- | --- | --- | --- |
| Fresh call | 378 → 350 | 441 → 409 | 3 → 2 |
| Capture chain | 424 → 368 | 491 → 427 | 4 → 2 |
| Snapshot then mutation | 575 → 519 | 791 → 727 | 6 → 4 |
| Ordered arguments | 846 → 790 | 1321 → 1257 | 10 → 8 |

The mutation/ordered-argument savings remove downstream redundant CASE copies,
not the required earlier snapshots. Record/union original-corpus costs retain
the slice 3 results: their remaining copies encounter ABI, observable write or
unknown-pointer boundaries. The pre-existing record/union fresh-call optimized
lane remains 272 bytes / 352 cycles versus raw 261 / 340; these passes do not
claim to fix that separate scalar/target-selection overhead.

### Added ABI probes: raw → optimized NIR, cartridge runtime

| Probe | XEX bytes | CPU cycles | Initial-MIR copies | Mapped routine storage bytes |
| --- | --- | --- | --- | --- |
| Record/union shared arguments | 572 → 532 | 922 → 878 | 10 → 8 | 50 → 44 |
| Variant shared arguments | 925 → 838 | 1466 → 1368 | 11 → 8 | 71 → 59 |
| Record/union return chain | 282 → 272 | 364 → 352 | 4 → 3 | 21 → 19 |
| Variant return chain | 417 → 366 | 480 → 424 | 4 → 2 | 29 → 21 |

The return-chain record's physical storage delta need not equal its logical
capture size: later storage placement/home elision is measured independently.
Every added probe asserts lower ABI-expanded copy counts, lower emitted XEX
size and fewer execution cycles in both runtimes. The full corpus asserts no
growth in logical copies/copied bytes/capture bytes or ABI-expanded copy sites.

The minimal `LET item=MaybeByte.SOME(42)` plus CASE stays at 10 bytes / 12 cycles
for construction, now followed by direct validation/read of item. Optimized
MIR reaches PrintBE in 46 cycles instead of 56, with XEX sizes 94 cartridge /
377 standalone instead of 103 / 386. A dedicated cost assertion prevents
regaining that snapshot; it does not execute printing or compare those numbers
with whole-corpus program cycles.

## Safety and acceptance coverage

| Boundary | Coverage |
| --- | --- |
| All-path initialization, diamonds, backedges, repeated/skipped initialization | `aggregate_cfg_forwarding`, `aggregate_bounded_forwarding`, `aggregate_fresh_initialization` |
| Overlapping union views, source mutation, alias/pointer effects and exposed identity | `aggregate_regions`, `aggregate_forwarding`, `unions_access` |
| Record/union self-copy, both partial-overlap directions, page crossings and large extents | `aggregate_values`, `unions_values` VM tests |
| Variant identical/adjacent storage, forbidden partial overlap in both directions | `variants` VM tests |
| Invalid outer/nested tags, unchanged destination on failure, fault timing | `variants`, `aggregate_calls`, `aggregate_fresh_initialization` VM tests |
| CASE guard effects and aggregate/nested binders | `aggregate_cfg_forwarding`, `case_guards`, `nested_patterns` |
| Argument order, indirect callee capture, mutable parameter independence and returns | `aggregate_call_forwarding`, `aggregate_calls`, `aggregate_indirect` |
| Union tails, pointer pointee sharing, native padding and activation | Four-target corpus, aggregate/union/native lowering tests, fresh static-reentry VM test |
| Idempotence and malformed input/replacement rejection | CFG/call/region/forwarding compiler tests; unchanged complete ABI verifier tests |

Reproduction:

```sh
cargo test --test aggregate_forwarding_baseline -- --nocapture
cargo test --test aggregate_forwarding --test aggregate_cfg_forwarding --test aggregate_call_forwarding
cargo test nir_fixtures_match_snapshots
cargo run --bin actionc-nir-sweep -- fixtures/nir
cargo run --bin actionc-mir6502-sweep -- fixtures/mir6502
cargo check --all-targets
cargo test
cargo test --manifest-path tools/vm-runtime-tests/Cargo.toml --locked
cargo test --manifest-path tools/vm-runtime-tests/Cargo.toml --locked --test aggregate_forwarding_audit --test aggregate_fresh_initialization -- --nocapture
```

Final verification passed:

- Full compiler run: 3,001 passed, zero failures, 22 existing ignored tests.
  The subsequently added slice 6 negative input/result-alias and intervening-call
  regression also passed on all four targets in the final six-test call-boundary
  suite. The expanded four-target corpus and forwarding/CFG/fixture suites pass.
- Full pinned VM suite: 217 passed, zero failures/ignored tests — 67 library
  tests and 150 integration tests across all 46 binaries. Integrations ran in
  four parallel batches (54, 33, 18 and 45 tests); no binaries were omitted.
- The final ten-test cost/fresh-initialization rerun passes after adding mapped
  storage/code metrics and MaybeByte cost assertions, including all 432 corpus
  executions. Saved CSVs contain exactly 144 NIR and 108 VM measurement rows.
- NIR snapshots, 44 NIR sweep cases, 167 MIR6502 sweep cases, all-target checking
  and `git diff --check` pass. The four optimized fixture changes are explained
  in the slice 4/5 notes; no raw fixture or historical CSV changed.
