# Private aggregate forwarding baseline

Measured 2026-09-09 at `191aa2a` plus the pending variant-storage-contract working
tree changes. The new region analysis is read-only and is not in the optimizer
pipeline; no copy forwarding or code-generation change is enabled in this slice.

## Reproduction and coverage

```sh
cargo test --test aggregate_regions
cargo test --test aggregate_forwarding_baseline -- --nocapture
cargo test --manifest-path tools/vm-runtime-tests/Cargo.toml --locked --test aggregate_forwarding_audit -- --nocapture
```

Source generators and the host oracle are shared in
`tests/support/aggregate_forwarding_cases.rs`. Four cases run for each of record,
union and variant: fresh aggregate-returning call, two-binding capture chain,
snapshot followed by original mutation, and a by-value argument followed by a
later argument that mutates its original source.

Records and unions both contain three inline bytes on Atari; the union also has
a two-byte CARD view, so the third byte checks full-image copying beyond that
view. Variants contain a tag and three active BYTE fields. Their sizes and checks
are not artificially equalized. Native union padding follows the target layout.

The NIR test covers raw/optimized programs on all four targets (96 measurement
rows) and proves the analysis does not mutate them. Record/union native lowering
canaries pass. Variant canaries accept only the existing missing-native-Error-
adapter capability diagnostic, when needed; they do not claim native execution.

The VM audit runs four byte inputs (0, 1, 127, 255), three backend paths (classic,
raw NIR to optimized MIR selection, optimized NIR to optimized MIR selection),
and both cartridge/standalone runtimes: 288 executions and 72 measurement rows.
Each execution has bounded completion and checks the entire $0600-$06FF guarded
window, not just a checksum. Code origin is $3000. Reported cycles run from VM
execution entry until the completion byte is stored; they are CPU cycles, not
PAL frame timings or isolated copy costs.

## Results

Full data: [logical NIR measurements](PRIVATE_AGGREGATE_FORWARDING_BASELINE_NIR.csv)
and [VM/initial MIR measurements](PRIVATE_AGGREGATE_FORWARDING_BASELINE_VM.csv).

Selected cartridge-runtime optimized-MIR baselines:

| Case | Record/union XEX bytes / cycles | Variant XEX bytes / cycles |
| --- | --- | --- |
| Fresh call | 272 / 352 | 378 / 441 |
| Capture chain | 311 / 400 | 616 / 793 |
| Snapshot then mutation | 380 / 622 | 657 / 926 |
| Ordered arguments | 586 / 1,069 | 846 / 1,321 |

These are complete microprogram costs, including helper/image overhead. No
performance improvement is attributed to this analysis-only slice. In several
cases existing optimized NIR produces worse final costs than raw NIR; retain
both baselines rather than silently selecting the smaller result. Resolving that
pre-existing selection/pressure difference is not part of slice 1.

Logical copy counts and capture bytes currently do not decrease under NIR
optimization. Physical ABI expansion adds transfers: the record/union ordered-
argument case has six logical CopyBytes sites but nine initial-MIR CopyBytes
sites. The corresponding variant case has seven and ten. Initial MIR counts are
still static sites, before target copy selection, not dynamic byte traffic.
Classic has no initial-MIR count (CSV `n/a`). Logical capture-byte sums are not
physical zero-page allocation, native frame size, or peak live storage.

The existing `adt_codegen_audit` and `unions_codegen_audit` remain separate
controls for guards, nested payloads, direct union views and larger copies. Their
historical documents are not overwritten by this baseline.

## Proof foundation and remaining work

`analyze_aggregate_regions` provides verified immutable per-routine facts:

- checked exact storage ranges, target-sized extents and activation identity;
- byte-range equality/overlap for nested fields and union views;
- bounded SSA AddrOf origins and constant indexing (not descriptor points-to);
- explicit address-use classification, including data-relocation exposure; and
- same-block unchanged-range queries with write/effect rejection reasons.

Tests cover every target, source mutation, unknown pointer writes, aliases,
absolute storage, volatile copies/accesses, calls, machine code, arithmetic fault
barriers, initializer address exposure, dynamic indexing, 257-byte extents,
offset overflow, invalid points and unverified input.

This is the first slice-1 increment. An exact range or internal address use is
not an initialization/liveness proof or permission to merge storage. Remaining
work includes complete initialization/version/lifetime eligibility as demanded
by forwarding, broader snapshot/payload/guard cases, fresh-destination lowering
and then actual copy/home elimination. Complete-capture ABI checks, scalar
promotion blockers and observable snapshot semantics remain unchanged.

## Verification of this increment

Passed: 2,976 compiler tests (including ten focused region-proof tests and the
four-target baseline test), unchanged NIR snapshots, all 44 NIR and 167 MIR6502
fixtures, `cargo check --all-targets`, the new 288-execution VM audit and the
existing ADT/union VM audit tests. The entire pinned VM suite was not rerun:
this increment does not change executable lowering or optimization behavior.
