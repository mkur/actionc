# Address-value implementation qualification

The [implementation plan](../address-values-implementation-plan.md) owns the
stages; the [audit](../output-audit-2026-10-12/README.md) supplies the frozen
full-root measurements and cycle profiles.

## Stage 0

[stage0.json](stage0.json) pins the qualified compiler SHA, both generic
cartridge listings' instruction/data and home census, and the four root images.
The source layout and compiler are unchanged. New
[known-address cases](../reproducers/known-record-address.act) retain complete
pointer assignments while reading/writing fixed and compiler-placed records.

Current MIR instruction bytes are 11 for direct CARD field access, 32 through
a known-address local, 45 for a local-pointer field store, and 9 for the returned
constant field address. The dynamic stride-20 local read remains 104 bytes;
the returned address remains 76. These are baseline observations, not CI size
assertions.

The `address_values` and `tn_codegen_reproducers` VM targets passed (2 and 3
tests respectively). They execute classic/MIR, cartridge/standalone and actual
LF/CRLF sources. The new target checks both return lanes, full source-local
pointer contents, genuinely changed word stores, and surrounding guard bytes.
Existing `record_consumer_boundaries` owns effectful indexes and two record
layouts; proof/alias rejection coverage is extended with its selector stages.

Development uses a single isolated, debug-info-free target directory:

```sh
CARGO_TARGET_DIR="$PWD/target/tn-address-values/build" CARGO_INCREMENTAL=0 \
  CARGO_PROFILE_TEST_DEBUG=0 CARGO_PROFILE_TEST_OPT_LEVEL=1 \
  cargo test --locked --manifest-path tools/vm-runtime-tests/Cargo.toml \
  --test address_values --test tn_codegen_reproducers -j2 -- --nocapture
```

Listings and logs are retained under `target/tn-address-values/stage0`. The
historical audit collector remains pinned to the pre-implementation compiler;
later stages write separate reports rather than replacing its measurements.

## Stage 1

The complete word `ComputeAddress` operation and its conservative full-width
fallback are implemented. Same-block provenance follows exclusive word copies,
constant displacements and constant-stride scaling; it retains originating
loads/calls and records input evaluation sites. Reaching-pointer-store queries
reject nonlocal cells, mutations and call/unknown-write barriers. Selection is
still disabled.

Six focused MIR tests pass: complete-word/phase verification, operation-family
effects, three provenance checks and actual fallback execution. The execution
check covers both materialization profiles/runtimes, strides 1/2/20/255/256/65535,
offsets 0/38/255/256, unaligned bases and full CARD wrap.
[stage1.json](stage1.json) confirms all four full-root images are byte-identical
to Stage 0. SemIR/NIR contracts and snapshots are unchanged.

## Stage 2

Optimized MIR forwards a known storage address from an exact complete local
pointer store to its same-block field consumer. The store and source evaluation
remain. Typed direct operands resolve at final layout; descriptor addresses and
fixed pointer cells are excluded. The transaction driver independently checks
the unchanged prefix, retained word store and exact resulting field identity.
Calls, any intervening write and one-lane mutation invalidate the fact.

| Generic cartridge routine | Stage 0 instruction bytes | Stage 2 | Local bytes |
| --- | ---: | ---: | ---: |
| ReadLocal / ReadOrdinary | 32 | 21 | 2 |
| WriteLocal | 45 | 26 | 2 |
| AddressZero | 63 | 15 | 0 |
| AddressField | 18 | 18 | 0 |

Both known local reads now have direct field operands and no indirect-Y access;
the source local still receives both pointer bytes. Direct fixed field reads
remain 6/11 bytes. `AddressZero` no longer calls multiply and has no spills.
The algebraic `AddressExpanded` comparison remains 222 bytes; the selector
currently requires an exclusive address flow to a complete word store.

[stage2.json](stage2.json) retains cartridge/standalone measurements and actual
LF/CRLF image equality. Three selector proof tests, the five generic VM tests,
and the four affected integrations pass (call ABI, MIR fixtures, materialization
gaps and rewrite inventory). No NIR or MIR-lowering snapshots changed.

Reproduce generic measurements with
`python3 surveys/tn/measure-address-values.py --compiler <actionc> --out-dir <target-directory> --crlf`.
The collector checks both runtimes and can add full TN/TNDBG roots with `--roots`.
