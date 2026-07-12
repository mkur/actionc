# MIR6502 CFG Temp Liveness Plan

Snapshot date: 2026-06-10.

The lane-safe byte-temp cleanup exposed a block-local liveness bug. MIR temps
and the spill homes they lower to are routine-scoped, but the cleanup only
looked at uses later in the same block plus the block terminator. TN has
generated compare blocks where a predecessor writes `VTempByte` lanes and
successor blocks read those lanes. Removing the predecessor definitions corrupts
control flow and screen output.

## Goal

Add a small CFG liveness layer for MIR temp byte lanes so dead-temp cleanup can
distinguish values that are dead locally from values live into successors.

## Scope

- Track `VTempByte { id, byte }` lanes.
- Track whole `VTemp(id)` uses as full-temp liveness.
- Compute per-block `use`, `def`, `live_in`, and `live_out` sets.
- Iterate to fixed point over routine successors.
- Keep the first implementation analysis-only, then wire it into byte-temp
  producer cleanup in a separate slice.

## Non-Goals

- No full SSA conversion.
- No phi construction.
- No memory alias analysis.
- No register or scratch-memory liveness in this slice.
- No behavioral expansion of byte-temp deletion until CFG live-out seeding is
  tested against TN.

## Implementation Slices

1. Add CFG temp-byte liveness analysis and tests.
2. Seed existing byte-temp producer cleanup with `live_out[block]`.
3. Re-enable/remodel direct-load and constant deletion through the CFG-aware
   path.
4. Retry binary deletion only after the `GetAnyKey`, `Draw`, and
   `DrawWinFrame` successor-use patterns are proven blocked by live-out.

## Required Regression Shape

```text
b0:
  VTempByte(t, 0) = ...
  VTempByte(t, 1) = ...
  jump b1

b1:
  compare VTempByte(t, 0)
  branch ...
```

`b0.live_out` must contain the lane used by `b1`, so the definition in `b0`
cannot be removed by a block-local reverse scan.

## Observability

When wired into cleanup, report at least:

- `mir-copy-prop-dead-temp-byte-def-blocked-successor-live`
- existing exact/full/sibling lane blockers remain intact

Run TN gates after each behavioral slice:

```sh
cargo test -q --lib mir6502
cargo run --quiet --bin actionc-emit -- --backend mir6502 --emit-load samples/tn/modern/TN.ACT \
  > target/tn-mir.xex
cargo run --quiet --manifest-path ../action-compiler-vm/Cargo.toml -- run \
  --cart roms/action.rom --os roms/rev02.rom \
  --load-object target/tn-mir.xex --dump-screen-on-stop --max-steps 1000000
```
