# Record base reuse measurements

Stage 0 establishes generic record sequences, typed MIR before/after target
materialization, final listings and emitted execution checks. The
[implementation plan](../record-base-reuse-plan.md) defines the proof and cost
boundaries. [stage0.json](stage0.json) pins the fixture, compiler, reporter and
image hashes; full artifacts are retained under `target/record-bases/stage0`.

The generic fixture is 699 load-file bytes in cartridge and standalone modes.
`WriteFields` is 86 instruction bytes, `KnownFields` 69, `RebindParameter` 64
and `ReadCurrent` 46. Actual LF/CRLF source compilations produce identical
images, typed MIR and payloads. The VM target checks ordinary and page-crossing
record accesses, retained neighbor bytes and a pointer referring to its own
parameter cell. The latter changes the pointer through its first field write;
the subsequent field write must follow the changed pointer.

The [TN audit](../size-audit-2026-10-13/README.md) supplies 28 repeated setups:
27 after indirect stores and one after direct local captures. Typed MIR retains
the source cells and complete field displacements, but temp expansion turns
most preparations into byte loads/stores. The final post-materialization
boundary retains typed byte operands and provides the actual emitted storage
placement. This is the selection boundary for the new proofs. A provisional
materialization layout is insufficient when home elimination, coloring or
deferred storage changes final addresses.

Record types and ordinary Action ABI parameter classification supply no
universal disjointness proof. The self-cell execution case demonstrates why
unknown parameter pointers must retain fallback. Caller-only bounds also need
an explicit complete entry/caller contract; private parameter homes do not
establish that contract.

Reproduce the generic census with a supplied compiler and reporter:

```sh
CARGO_TARGET_DIR=target/record-bases/build CARGO_INCREMENTAL=0 \
  CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_DEV_OPT_LEVEL=1 \
  cargo build --bin actionc --example record_base_reuse_report -j2
python3 surveys/tn/measure-record-bases.py \
  --compiler target/record-bases/build/debug/actionc \
  --reporter target/record-bases/build/debug/examples/record_base_reuse_report \
  --out-dir target/record-bases/recheck
```

The focused execution target is `record_base_reuse` in
`tools/vm-runtime-tests`. Compiler production output is unchanged in Stage 0.

Stage 1 adds complete base proofs, byte dependencies and wrapping write
footprints. The optional emission proof map resolves stable storage IDs through
the converged final layout; ordinary output does not build the map. Unit cases
cover physical aliases, partial pair clobbers, unknown indirect destinations,
proven literal destinations, stale generations and forged proof fields.
Production selection remains unchanged in this slice.
