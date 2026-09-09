# Bounded private aggregate forwarding — slice 3

Implemented 2026-09-09, relative to `8bd6e5e`. The earlier contract, analysis,
fault synchronization and fresh-initialization changes are separate commits.
Historical [slice 1](PRIVATE_AGGREGATE_FORWARDING_BASELINE.md) and
[slice 2](PRIVATE_AGGREGATE_FRESH_INITIALIZATION.md) measurements are unchanged.

## Proof and rewrite

`src/nir/aggregate_forwarding.rs` runs on verified NIR, after ordinary value/CFG
cleanup and before scalar storage propagation/promotion. It replaces reads of a
redundant private snapshot with reads of its stable source, then removes the
initializing copy. It does **not** redirect producer writes into later homes.

Eligibility requires all of the following:

- The destination is an ordinary, uninitialized `AggregateCapture` whole home.
  Its sole complete initializing `CopyBytes` has matching nominal definition ID
  and target extent, and a known ordinary whole-home source. Names and equal
  byte sizes alone are not sufficient.
- Capture identity is not exposed: bounded internal address consumers are
  allowed, but pointer stores/observations, unknown address flows, aliases,
  data relocations and machine visibility prevent forwarding.
- Every capture reference is inventoried, including direct places, resolved
  indirect reads/writes, aggregate ABI values, return operands and effects.
  All uses must follow this copy in the same block. No other capture write is
  allowed. An initializing copy therefore executes before each use on every
  execution of the block, including repeated loop entries.
- Both the source and captured image remain unchanged until the last redirected
  read. Existing byte-region queries reject overlapping or unresolved writes,
  volatile operations, calls, machine/REAL effects and possible runtime faults.
  A call or source mutation **after** the final read does not invalidate reuse.
- Copy consumers have known disjoint destinations. Partial overlap, self-copy
  consumers and unknown destinations remain conservatively staged.

Direct field reads and constant indexed accesses through bounded SSA `AddrOf`
origins are supported. Source identity may already be observable: it is the
removed capture's identity that must stay private. Writes through unknown
aliases and calls during the read interval still reject reuse. This does not
assume all ordinary sources are unaliased or immutable.

The initializing copy supplies the complete snapshot definition; this is not a
claim that every byte of an arbitrary source has a source-language initializer.
Reads preserve the existing ordinary byte-image semantics, including padding,
inactive union bytes and copied pointer values. Volatile reads cannot disappear.

After each removed copy, temporary definition locations and immutable analyses
are refreshed. Further chain links can then be considered. Existing home elision
removes captures only after a full reference census, additionally checking
program-wide data relocations and opaque machine visibility. Existing pure-temp
cleanup now also handles unused `AddrOf` computations; calls and volatile loads
used to evaluate their inputs remain separate operations. Scalar promotion
blockers and the complete caller-owned aggregate ABI verifier are unchanged.

## Measurements

Full reproducible data: [four-target NIR](PRIVATE_AGGREGATE_BOUNDED_NIR.csv) and
[Atari VM/initial MIR](PRIVATE_AGGREGATE_BOUNDED_VM.csv), using the unchanged
12-case shared corpus and four byte seeds. The 288 VM runs compare the entire
watched memory region, not just a reported checksum.

| Cartridge optimized-MIR capture chain | Slice 2 → slice 3 XEX bytes | CPU cycles | Logical capture bytes |
| --- | --- | --- | --- |
| Record | 311 → 290 | 400 → 376 | 12 → 9 |
| Union | 311 → 290 | 400 → 376 | 12 → 9 |
| Variant | 452 → 424 | 523 → 491 | 20 → 16 |

Each loses one logical copy and one physical capture home; ABI-expanded initial
MIR copy sites drop from five to four. Native capture-chain probes likewise
lose one nominal-sized home/copy, respecting their alignment/extents. These are
logical allocated extents, not claims about peak native frame or zero-page use.
Native NIR/record-union MIR checks do not imply native execution coverage or a
new native Error adapter for variants.

The native union acceptance test intentionally now requires a full overlap-safe
copy in raw MIR and no copy/capture home in optimized MIR for its unchanged-source
probe. Pointer width, endianness and field-displacement checks are retained.
No NIR snapshots change in this slice.

The shared fresh-call, mutation-after-snapshot and ordered-argument probes retain
their slice 2 costs. Required variant validation/fault paths are preserved.
Classic code is unchanged: this slice operates on NIR, not shared SemIR.

`tests/support/fresh_maybe_byte.act` still has `item` plus its CASE snapshot.
Fresh construction remains four instructions / 10 bytes / 12 cycles; full
optimized MIR remains 61 Main instruction bytes / 56 cycles to PrintBE entry.
Cross-block CASE reads deliberately fail this slice's bounded proof.

## Coverage and reproduction

The compiler tests cover all four layouts, direct/indirect internal reads,
nominal mismatch, volatility, faults, dynamic indexes, mutation between reads,
unknown writes, partial overlap, identity exposure, data/effect-only references,
use before initialization and aggregate ABI/CASE retention. New VM oracles cover
record/union full-image tails, mutation via direct stores/calls/pointers,
repeated/skipped initialization and guarded variant selection after mutation.

```sh
cargo test --test aggregate_forwarding
cargo test --test aggregate_forwarding_baseline -- --nocapture
cargo test --manifest-path tools/vm-runtime-tests/Cargo.toml --locked --test aggregate_bounded_forwarding
cargo test --manifest-path tools/vm-runtime-tests/Cargo.toml --locked --test aggregate_forwarding_audit -- --nocapture
cargo test nir_fixtures_match_snapshots
cargo run --bin actionc-nir-sweep -- fixtures/nir
cargo run --bin actionc-mir6502-sweep -- fixtures/mir6502
cargo check --all-targets
cargo test
cargo test --manifest-path tools/vm-runtime-tests/Cargo.toml --locked
```

Final verification passed: 2,992 compiler tests (22 existing ignored tests),
44 NIR and 167 MIR6502 sweep cases, all-target checking and unchanged NIR
snapshots. Final pinned VM coverage is 212 tests: 67 library tests plus 145
integration tests across all 44 binaries, with the integration binaries run in
four parallel batches. The focused nine-test compiler, four-test bounded VM,
nine-test fresh-initialization VM and 288-execution shared corpus audits passed.
Both saved measurement CSVs were rechecked after the dead-address cleanup and
remain identical. `git diff --check` is clean.

## Next boundary

Slice 4 adds path-sensitive source stability, initialization and lifetime checks
across branches/merges, then CASE capture reuse and exact subobject forwarding.
Guards that can change a later-read selector still require a snapshot. Calls
after the last relevant read should remain harmless to the proof. Slice 5 owns
argument/return-buffer and physical ABI improvements; producer redirection needs
its own publication proof, not a relaxation of snapshot read forwarding.
