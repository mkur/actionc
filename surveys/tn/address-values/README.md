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
