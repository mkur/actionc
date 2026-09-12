# INLINE Q4.12 validation

At the 2026-09-11 rollout, the standalone MIR6502 Mandelbrot recurrence expanded
its two `SqrWide` calls and one `MulFloor` call. Both original Q4.12 bodies
remained emitted. Only these
two library functions carry `PUBLIC INLINE`; their arithmetic and ABI are
unchanged. Selection uses typed computation, effects and emitted costs, with
no module-name, routine-name or sample-constant predicates.

These are the rollout measurements. The subsequent
[comparison-branch optimization](MIR6502_COMPARE_BRANCH_FUSION_PLAN.md#delivered-validation-and-measurements)
changes layout and can cause the single `MulFloor` request to decline under
the unchanged cost proof; both square sites still expand in the tested
standalone layouts. `INLINE` remains a preference.

## Matched measurements

Measured 2026-09-11 from this implementation, at origin `$2000`, standalone
runtime. Both builds use the same annotated source, optimized NIR and compiler;
only `Mir6502Config::enable_small_leaf_inlining` differs. Listings were checked
byte by byte against their executable before profiling. Every coordinate and
escape count was checked against an independent integer oracle.

| Measure | Inliner disabled | Inliner enabled |
| --- | ---: | ---: |
| Emitted code and data | 2,138 bytes | 2,414 bytes |
| XEX file, including headers | 2,150 bytes | 2,426 bytes |
| 640-point grid, 6,167 updates | 20,430,780 cycles | 20,276,676 cycles |
| Original 16,000 points, 151,649 updates | 524,207,635 cycles | 520,414,063 cycles |
| 640-point `SqrWide` / `MulFloor` calls | 13,350 / 6,167 | 0 / 0 |
| Original-grid `SqrWide` / `MulFloor` calls | 328,964 / 151,649 | 0 / 0 |
| 640-point multiply helper calls | 19,517 | 19,517 |
| Original-grid multiply helper calls | 480,613 | 480,613 |
| 640-point multiply helper cycles | 14,357,820 | 14,357,820 |
| Original-grid multiply helper cycles | 374,745,561 | 374,745,561 |

Inlining adds 276 bytes and removes 154,104 cycles (0.75%) on the small grid,
or 3,793,572 cycles (0.72%) on the original grid. Helper algorithms, helper
invocations and their measured work are unchanged. `ViewportX` also retains
651,301 cycles for 160 columns and 1,228,136 cycles for 320 columns.

The older 20,769,965 / 532,548,330 totals predate these IR and scratch-promotion
changes. They are historical context, not the disabled-inliner control. Keeping
the annotations in both current builds includes their NIR promotion effects
on both sides of the comparison.

Five alternating builds produced median build times of 1.00 seconds disabled
and 4.52 seconds enabled (4.52×). Ranges were 0.98–1.45 and 4.43–6.48 seconds;
other verification jobs were running concurrently. The audit measures the warm
compiler API from module loading through emission, excluding process startup
and artifact writes, using the debug compiler profile. This is a measured
compilation-time cost, not a release-compiler throughput claim. All five builds
of each configuration produced byte-identical XEX files.

## General cost constraints resolved by the rollout

Materialization can insert comparison and continuation blocks. Region costing
now includes them until the next original caller boundary. It can recognize
bijective renumbering of private block-local virtual zero-page bytes when no
value enters/leaves the block and their addresses remain unexposed. Other
storage identities and control stay exact.

Retained helper relocation can change branch page penalties. Bounded placement
trials can move a compiler-owned wide helper to an existing routine boundary
after the caller within the wholly structured prefix, before the first
machine-block routine, preserving its instructions, ABI, effects and ID references.
Each alternative must pass the normal emitted-cost and growth checks. Opaque
callees that reach wide helpers cannot hide a positive relocation penalty.
The measured standalone build uses two placement trials and one accepted
placement; its two expansion groups prove at least 11 and 12 saved cycles per
execution of their respective regions.

The requested site ceiling is 192 bytes, increased from the initial 128 after
a profitable 148-byte site exceeded that initial ceiling. The other limits
remain 512 bytes/caller, 1,024/requested-program and 1,280/combined-program.
The sixteen requested trials include placement alternatives; automatic policy
limits remain unchanged. No retained-body deletion credit is taken.

`INLINE` remains a preference. The shorter cartridge layout may retain Q4.12
calls when it cannot prove a saving. Other backends retain ordinary calls.
Branched helper wrappers, ordinary nested calls, recursive expansion and
pointer/aggregate inlining remain unsupported. SArgs costs can also be unknown.

## Correctness and reproduction

The exhaustive arithmetic audit covers all 65,536 signed inputs for squaring,
65,536 deterministic product pairs and 289 boundary pairs, including INT minimum
and negative fractional products. It compares the disabled and costed modes
and an explicitly expanded control, so cost-based fallback cannot hide an
untested arithmetic clone. Smaller VM tests cover full LONG results, repeated
calls, multiple helpers and nonreturning division faults in both runtimes.

The standalone recurrence selection regression covers both Atari and VBXE
programs. Disabled-inliner controls passed the complete 160×192 Atari and
320×192 VBXE framebuffer oracles. They used an isolated copy of the same
production source with only the optimized configuration's inliner flag disabled.
Enabled builds pass the same complete framebuffer oracles. The complete VM
suite passes (293 tests), as do the full root tests, NIR snapshots, 51-fixture
NIR sweep and 167-fixture MIR6502 sweep. No existing NIR or MIR snapshot
changes in this library rollout. Earlier new snapshots
record the typed INLINE metadata and intentional private-scratch promotion.

```sh
cargo test --lib q4_exhaustive -- --ignored
cargo test --lib q4_inline_matched_artifacts -- --ignored --nocapture
cargo test nir_fixtures_match_snapshots
cargo run --bin actionc-nir-sweep -- fixtures/nir
cargo run --bin actionc-mir6502-sweep -- fixtures/mir6502
cargo test
cargo test --manifest-path tools/vm-runtime-tests/Cargo.toml
```

The final complete runs also use `CARGO_PROFILE_TEST_OPT_LEVEL=1` to speed
host-side compiler and VM execution while retaining test-profile assertions.
This changes Rust test execution, not Action compilation settings.

The ignored artifact test writes deterministic matched XEX/listing pairs and
`build-times.csv` under `build/inline-validation/`. The existing
`build/oscar64-mandelbrot/action-profile.rs` profiles either pair, with its
`original` argument selecting the 16,000-point grid. Generated binaries,
listings and profile output remain under ignored `build/`.
