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

## TN and optimized default

With the same sources and optimized gates, ActionCart `$3000` drops from
**9,847 to 9,791 bytes: 56 bytes saved**. The off image, map and relocations
exactly match the previously qualified baseline. Traced and untraced artifacts
are identical in each arm. [Off receipt](tn-off.json), [on receipt](tn-on.json),
[comparison and artifact hashes](comparison.json).

| Routine | Off: code + ordinary homes | On: code + ordinary homes |
| --- | ---: | ---: |
| Ord | 80 + 5 | 51 + 0 |
| Items | 56 + 3 | 37 + 0 |
| MoveMenuBar | 115 + 4 | 115 + 4 |
| Draw | 145 + 1 | 145 + 1 |
| Tag | 88 + 2 | 88 + 2 |
| TagAll | 87 + 0 | 87 + 0 |
| Next | 30 + 0 | 30 + 0 |
| Strcpy | 20 + 0 | 20 + 0 |
| Fnamecmp | 105 + 0 | 105 + 0 |

The old 9,799-byte trial saved 48 bytes but grew four routines by two bytes each.
Scoped observation permits four more dead stores to be deleted (10 instead of 6
in the cross-call trial), producing 9,791 bytes with no routine growth. Both
vetoes now accept their certified foreign ownership. Ord has five resident bytes
and Items three. Source stores and ABI captures remain intact. The conservative
queries for pruning and movement remain unchanged.

The earlier 119/146/90/87 figures for MoveMenuBar/Draw/Tag/TagAll included their
ordinary homes. The table above separates instruction and storage bytes.

| Compiler work | Off | On |
| --- | ---: | ---: |
| Core materializations | 6 | 6 |
| Emission invocations | 163 | 163 |
| Inner emission probes | 978 | 978 |
| Preservation replays | 24 | 24 |
| Physical source inventories | 60 | 60 |
| Invocation-proof snapshots | 0 | 3 |
| Rewrite analyses | 50,792 | 50,800 |

There are three certificate snapshots across existing terminal transactions;
none is rebuilt per store. The eight additional rewrite analyses are consistent
with the four newly deleted stores and final-placement replay. No materialization,
emission, inventory or preservation pass was added.

Single-run traced times were 20.38 / 20.38 seconds (off/on); untraced times were
19.41 / 24.04 seconds. Peak process RSS, including retained traces and both
compilations, was 722,042,880 / 737,427,456 bytes. These observations do not establish
a timing regression or speedup. No repeated timing benchmark was run.

The larger trace reached about 36.6 MB. Initial probe attempts stopped at the
32 MiB diagnostic limit before saving results; the focused measurement suite now
uses 64 MiB and preserves probe error output. The compiler's default trace limit
and executable behavior are unchanged. The reported A/B uses complete traces.

`Mir6502Config::optimized()` now enables the feature; `Default` keeps it off.
The [final public-API check](slice5-default-checks.json) confirms explicit-on
output and both parent gates. Its initial fixture had no calls from Main and
was correctly pruned by the public frontend; the corrected test makes the
control routines reachable. No compiler correction was required.
TN itself was measured for code and selection; runtime validation used the
independent VM control, not a full TN scenario run.

Reproduce the TN A/B after building the library with the plan's shared-target
settings (add `--scoped-observation` for the on arm):

```sh
python3 tools/compare6502/parameter_homes.py \
  --api-rlib build/checked-arena/shared-target/debug/deps/libactionc-bf811c1ef8b2df31.rlib \
  --output build/storage-observation/recheck-off \
  --enabled --handoff --cross-call --suite storage-observation --tn-only
```

Use the freshly built rlib for the current checkout; its filename can vary.
Use `--skip-tn` instead of `--tn-only` for the independent controls.
