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
