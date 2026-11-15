# MIR6502 scoped storage observation

Focused qualification, 2026-11-15. Implements the
[design](../../MIR6502_STORAGE_OBSERVATION_DESIGN.md) and
[sliced plan](../../MIR6502_STORAGE_OBSERVATION_IMPLEMENTATION_PLAN.md).
Broad backend qualification is deferred under the user's fast-iteration direction.

## Independent control

`fixtures/runtime/storage_observation.act` combines a record search with an
initialized counter and a separate routine that stages a computed byte argument.
Leaf, handoff and cross-call placement remain enabled in both arms; only
`enable_scoped_storage_observation` changes. ActionCart, origin `$3000`:

| Metric | Off | On |
| --- | ---: | ---: |
| Packaged image | 261 | 227 |
| FindRecord code | 80 | 51 |
| FindRecord ordinary homes | 5 | 0 |
| FindRecord ZP homes | 0 | 5 |
| Walk code | 116 | 116 |
| AdvanceRecord code | 30 | 30 |
| Core materializations | 2 | 2 |
| Emission invocations | 28 | 28 |
| Inner emission probes | 112 | 112 |
| Preservation replays | 2 | 2 |
| Invocation-proof snapshots | 0 | 1 |

The off arm rejects a 229-byte placement trial because Walk grows two bytes.
The on arm deletes its extra staging store and accepts 227 bytes: **34 bytes
saved**, with no routine growth. The actual argument sequence has `SBC #$01;
TAX; LDA #$80`, without `STA $E3`. FindRecord still captures and updates its own
parameter at `$E2/$E3` and initializes its local counter at `$E4`.

Results match with small-leaf inlining enabled and disabled. The source-owned
`$E0–$EF` window control retains identical output across the gate. Traced and
untraced executable, map and relocation outputs match. Receipts include source,
configuration, library, probe and artifact hashes:
[off](control-off.json), [on](control-on.json).
Control timings were collected while a separate test build was active; they
are not performance measurements.

## Local validation

- `cargo test --lib storage_observation`: five grouped tests covering both vetoes,
  inventory aliases, complete and entry-dependent objects, physical versus
  logical initialization, read provenance, live reads, historical exposure,
  stale snapshots, conflicting placement and the complete cleanup transaction.
- `cargo test --test mir6502_cross_call_homes storage_observation`: independent
  final savings, no routine growth, unchanged materialization count, initialized
  local placement, trace equality and a source-owned window negative control.
- `cargo test --manifest-path tools/vm-runtime-tests/Cargo.toml --test cross_call_homes storage_observation`:
  repeated parameter/local invocations, canonical arguments, return values,
  stack balance, page crossing, byte wrap and source-memory sentinels. Cart off/on,
  actual LF/CRLF loading, one standalone execution and the protected-window case.

The initial VM fixture compared LF/CRLF source spans as well as executable
artifacts. Their debug offsets correctly differ. Its corrected comparison checks
bytes, relocations, storage symbols and routine ranges while executing both
source forms. No compiler correction was needed.

Check receipts: [slice 2](slice2-checks.json), [slice 3](slice3-checks.json),
[integration](slice4-integration-checks.json), [VM](slice4-vm-checks.json).
The full backend suites, MADS reconstruction, raw-NIR and shifted-origin matrices
were not run locally. Existing CI coverage is unchanged.
