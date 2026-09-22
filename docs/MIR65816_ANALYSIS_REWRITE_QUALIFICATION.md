# Native 65816 analysis and checked-rewrite qualification

Slices 0–9 of the [implementation plan](MIR65816_ANALYSIS_REWRITE_IMPLEMENTATION_PLAN.md)
are complete. The foundation preserves the frozen `1ce9624` output and migrates
one existing optimization: adjacent temporary A16 load forwarding. The
[final record](abi/action65816-analysis-rewrite-qualification.json) binds the
results to compiler, fixture, tool and evidence hashes.

Slice 7 is `883b5f51` ([atomic transactions](MIR65816_CHECKED_REWRITES.md)).
Slice 8 was split into shadow qualification `4130d858` and authoritative
application `e4fd88b5` ([candidate and proof contract](MIR65816_ADJACENT_CHECKED_FORWARDING.md)).
This report and its measurement tool complete slice 9. The compiler tested before
the authoritative commit matches that commit's recorded input hashes exactly.

## Completed interfaces and scope

Typed selected actions own physical effects, symbolic control flow and compiler
obligations. Immutable snapshots provide physical byte-home liveness, attribution
to particular stored definitions, register-lane liveness and independent flag
liveness. Queries validate owner, generation and reachability. The checked driver
validates sealed plans, freshly replays and lays out scratch output, rebuilds
facts, and publishes atomically. Failed transactions preserve the original output.

The production consumer captures the actual LDA before omission, restores all
projected-away loads, and removes each only through the driver. Rejection retains
the actual load and its verified continuation. Full A/N/Z equality remains
required; flag deadness does not broaden eligibility. Calls, helpers, aliases,
protected compiler events, stack equations and preemption contracts retain their
existing barriers. Public ABI v1, image v3, o65 profile v1, stack guards and
interrupt reserves are unchanged. No stores or additional homes are eliminated.

## Qualification

| Check | Result |
| --- | --- |
| Native emitter/proof library | 175 passed, including 13 checked-rewrite tests |
| Affected root integration targets | 61 passed: state boundary, ABI, contract, emission, o65 and both CLIs |
| Full native execution | 131 passed in each of debug, release and isolated CRLF debug |
| Native artifacts | All 658 identical across those three runs; all 657 earlier artifacts preserved |
| Isolated CRLF | 22 converted text fixtures, a rebuilt root state-boundary test and the full native suite |
| Frozen comparison corpus | 56 builds, 224 artifact files equal under the existing gate and 264 unchanged complete records; 528 executions per host profile |
| Action corpus | All 132 records correct; all 28 raw/optimized builds also checked through CRLF source compilation |
| Comparison tooling | 82 tests, including deliberate equality-gate mutations |
| Qualification provenance | Five real-file mutations reject changed source, fixture, added/deleted source and qualifier code |

The external comparison retains exactly its known optimized vbcc `unlink`
vector-0 failure. Those two comparison test commands therefore return failure;
the strict gate verifies that all records agree with the frozen baseline and
that no additional failure appears. Native compiler qualification passes.
The existing artifact checker canonicalizes only the listing's source-directory
prefix; emitted executable bytes and Action images are exact matches.

The [new observation file](benchmarks/65816-analysis-rewrite/adjacent-checked-summary.json)
is separate from historical measurements. It classifies all 102 candidate
requests: 75 accepted, 27 blocked, exactly matching shadow decisions. Blockers
are missing adjacent capture (19), unavailable A16/home/NZ identity (6), and
non-temporary homes (2). Ten additional raw/optimized fixture builds cover
forwarding, frame/parameter behavior, calls/aliases and preemption.

Mutation tests cover forged/stale plans, wrong effects, protected events,
generation invalidation, replay failure and rollback. Withholding final temporary
ownership restores the actual load; altered addressing, duplicate candidates and
corrupted bytes are rejected. The qualifier now hashes inputs before invoking
Cargo and refuses to publish a manifest if they differ afterward. This closes
the provenance limitation explicitly recorded in the earlier shadow debug run.

Representative optimized target results remain unchanged:

| Kernel / input | Bytes | VM cycles | Additional stack bytes |
| --- | ---: | ---: | ---: |
| Rotation / 13 | 126 | 735 | 8 |
| Sum loop / 13 | 120 | 1,092 | 6 |

No NIR, semantic, allocator or shared-runtime code changed. Validation therefore
used the affected root targets and full native workspace; it did not run a full
root `cargo test` or NIR sweep. The owned isolated CRLF worktree and build cache
were removed after preserving their manifest and artifacts.

## Host compilation cost

The [measurement tool](../tools/compare65816/measure_host.py) compares release
CLIs built with Rust 1.95.0, default release settings, no optional features and
incremental compilation disabled. The baseline is compiler `1ce9624`; the after
binary contains the qualified `e4fd88b5` compiler source. These measurements
cover the whole foundation relative to the frozen baseline, not just slice 8.

On macOS ARM64, each compiler first compiles all 28 Action corpus inputs once
to warm caches. Seven measured rounds alternate compiler and build order:
392 separate measured processes plus 56 warm-ups. Wall time includes startup,
parsing, compilation and JSON writing. `wait4` supplies each child's CPU time and
peak RSS. Every output image must match the frozen hash on every invocation.
No build or native qualification ran concurrently. All samples, per-build
medians, binary/tool hashes and platform details are in
[host-compilation.json](benchmarks/65816-analysis-rewrite/host-compilation.json).

| Measure | Before | After |
| --- | ---: | ---: |
| Median total wall time for 28 builds | 0.1468 s | 0.3609 s |
| Median per-process peak RSS | 5.70 MiB | 6.92 MiB |
| Maximum observed per-process peak RSS | 6.17 MiB | 9.56 MiB |

The corpus time increases 2.46×; median peak RSS increases 21.4%. Per-build median
wall ratios range from 1.10× to 8.12×. Raw constant-chain compilation rises to
41.7 ms, and raw rotation to 40.2 ms. The small corpus includes process-startup
cost and does not establish scaling on large programs or cold-cache behavior.

Home analyses already share immutable states within each solver run and copy
only when a transfer/join changes facts. Every accepted edit still rebuilds all
analyses and freshly replays output. The next efficiency slice should profile
that repeated work and growing routines while preserving generation invalidation
and proof obligations. Cross-generation reuse needs its own invalidation design.
Subsequent optimization plans must consume the checked API and declare their own
measured code/traffic deltas; this migration grants no broader rewrite eligibility.
