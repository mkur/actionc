# Scalar DP inventory

The read-only first slice of the [scalar DP plan](MIR65816_SCALAR_DP_PLAN.md)
checks 58 emitted routines across all 28 Action builds, including the uncounted
wrappers. Fourteen routines meet the initial typed whitelist. Their 84 private
word-memory sites reproduce all preliminary representative forecasts, including
optimized sum-loop at 120 bytes / 1,092 cycles / six stack bytes and rotation
at 130 bytes / 793 cycles / eight stack bytes. These remain forecasts: this
slice emits and executes no changed allocation.

[Typed facts](benchmarks/65816-scalar-dp-inventory/facts.json) retain exact temp
types, storage identities, addressability, operation kinds, word operands,
conservative effects, CFG, instruction spans and allocated homes. The separate
[inventory](benchmarks/65816-scalar-dp-inventory/inventory.json) records explicit
rejections for every other routine, independently reconstructed closed live
points, proposed home classes, staging/frame accounting, exact operand patches
and teardown removals, complete expected-image hashes and all-vector forecasts.
Calls, casts, unsupported widths and unsafe memory remain outside this slice.

Byte immediates are admitted as zero-extended native word operands, matching
existing word selection; this is distinct from promoting a byte temporary.
The preserved movement export remains unchanged in historical mode.

The image transform is constructed solely from old bytes and typed facts. It
remaps subsequent routines and references, changes only declared locations and
frame operands, and keeps checked zero-frame entries. All existing logical
forwarding and edge counters retain their values. Forecast hashes include
per-PC execution counters; those larger maps are reconstructed rather than
duplicated in the inventory file. Unchanged vbcc correctness failures remain
in the original comparison records.

Reproduce before enabling scalar allocation:

```sh
A816_COMPARISON_MANIFEST="$PWD/target/edge-coalescing-after/manifest.json" \
A816_MOVEMENT_FACTS="$PWD/target/scalar-dp-inventory-facts.json" \
CARGO_INCREMENTAL=0 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 \
  cargo test --test mir65816_scalar_dp_inventory -- --ignored
python3 -B tools/compare65816/inventory_scalar_dp.py target/edge-coalescing-after \
  --facts target/scalar-dp-inventory-facts.json \
  --output target/scalar-dp-inventory.json
python3 -B -m unittest discover -s tools/compare65816 -p 'test_scalar_dp_inventory.py'
```

The export requires complete equality with the saved images and compiles both
LF and CRLF source through the real compiler path. After enabling allocation,
use the committed facts to check the frozen transform, not a regenerated
inventory from changed output. Historical snapshots remain immutable.
