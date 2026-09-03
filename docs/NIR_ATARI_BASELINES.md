# NIR Atari Object Baselines

Snapshot date: 2026-09-03.

These deterministic object hashes guard the NIR target-independence migration
against accidental changes to established Atari 6502 output. They were produced
from commit `8db760895484fe545a3e665b0072b567f68bd511` with `actionc 0.1.0` and
embedded VFS hash
`699fde62625c3fb9e563e8961eb27071a2dcaaca96bfed6f0f05f08c0a887162`.

An intentional output change may update a row only when the implementing slice
records why the byte contract changed. A target-plumbing or representation-only
slice should leave every row unchanged.

## Source Coverage

| Source | Coverage | Source SHA-256 |
| --- | --- | --- |
| `fixtures/runtime/record_assignment.act` | packed and nested records, record arrays, record pointers, overlap-safe copies | `118e4a1671c38c12f84a645a3cec4ba410d42a9952ccc5be846081e41747ec35` |
| `fixtures/mir6502/routine_address_assignment.act` | callable pointer storage and routine-address relocation | `9bc7962c0cf68ed207d1eff7d67d7e96849f916a13738585749fefe900f4c370` |
| `fixtures/runtime/initialized_arrays.act` | byte/card arrays, descriptors, local/global initialized backing | `61ddff792433f56b4259923e965ab432f61d9f7290164b44504d5e1c1db87b5b` |
| `fixtures/nir/inline_asm_fixed_array.act` | fixed-address array and inline-assembly relocation/effects | `9fd22c2983ed75e54c780917514e6f528350173dadddf6d51325192ff6dd4673` |
| `fixtures/runtime/standalone_minimal.act` | standalone runtime packaging without cartridge dependencies | `b56da58b7800ec6eb2ccd1744f44dba1d0b882c9315fb167f0da711a0085af03` |
| `fixtures/runtime/native_routine_abi_baseline.act` | fixed parameter/local storage, initialized local, local alias, local byte/card arrays, record, escaped local address, indirect call, and deliberately non-reentrant recursion | `2994a56fddc7a737368705dfc097253fb25fb9fae42cef8f5dafd5960782a8cc` |

## Object Matrix

All rows use `--profile modern`. `classic` and `mir6502` name the selected
backend; `cart` and `standalone` name the runtime.

| Source | Backend | Runtime | Bytes | Object SHA-256 |
| --- | --- | --- | ---: | --- |
| `record_assignment.act` | classic | cart | 915 | `c2b4da8e4f7a0ea53656f4901162fdf7b34ccfd88b663394d579d0b097e0dab7` |
| `record_assignment.act` | mir6502 | cart | 689 | `8b2903acc57a4d8a081e2235d089fdaf8fb567089d6f859f580640fb6fd038cd` |
| `routine_address_assignment.act` | classic | cart | 26 | `f74c3681ef5c61ee093212de9d7bf883e9a01ed747db15d92cdce7dbb32f71f0` |
| `routine_address_assignment.act` | mir6502 | cart | 26 | `8035435cbbaffe57de27ab7f0b7840e6f4e711811dbe874965e9244c0826a621` |
| `initialized_arrays.act` | classic | cart | 102 | `663dd7fdc7b1c3def1acbedff4eb0ba9d31ac5d452ce95785ed1ac7c9c9bb00d` |
| `initialized_arrays.act` | mir6502 | cart | 107 | `c5ba5f8fc266823a56d97d910c756fd554c66d684e2b39e9ccf211c9b094cb52` |
| `inline_asm_fixed_array.act` | classic | cart | 25 | `31ed6095ad35b52447a1133815714b6b078aeb1978dbadd6c86c0a7f72740671` |
| `inline_asm_fixed_array.act` | mir6502 | cart | 25 | `31ed6095ad35b52447a1133815714b6b078aeb1978dbadd6c86c0a7f72740671` |
| `standalone_minimal.act` | classic | standalone | 13 | `c4b9e2ad6afdafa91b90d1ac0e4e2002295f42029aff605bec1a020b03850eab` |
| `standalone_minimal.act` | mir6502 | standalone | 13 | `c4b9e2ad6afdafa91b90d1ac0e4e2002295f42029aff605bec1a020b03850eab` |
| `native_routine_abi_baseline.act` | classic | cart | 179 | `a0eb8aaceb56346ded51589db1df15eb4e677ff5319bc46bcfd2ee38c34d29d0` |
| `native_routine_abi_baseline.act` | mir6502 | cart | 154 | `74074ada4ca03643b4eb50798d95b4055edeaf6b4cf31d7b95f8a675d6d75171` |
| `native_routine_abi_baseline.act` | classic | standalone | 179 | `a0eb8aaceb56346ded51589db1df15eb4e677ff5319bc46bcfd2ee38c34d29d0` |
| `native_routine_abi_baseline.act` | mir6502 | standalone | 154 | `74074ada4ca03643b4eb50798d95b4055edeaf6b4cf31d7b95f8a675d6d75171` |

## Native Routine ABI Migration Inspection Baseline

The native-routine fixture is intentionally compile-only. Its recursive
routine reuses fixed parameter and local cells under the classic ABI, so
executing it would not be a valid recursion test. These textual-output hashes
guard the IR and map shape. Slice 2 intentionally updated the NIR rows to print
structured activation, storage duration, size, and alignment facts. MIR6502,
map, and object rows remain unchanged.

Slice 6 reproduced every object and inspection row on 2026-09-03 after adding
MIR6502's explicit `ClassicStatic` activation guard. All byte counts and
SHA-256 hashes remained unchanged, and no snapshot refresh was required.

| Inspection | Bytes | SHA-256 |
| --- | ---: | --- |
| unoptimized NIR | 3040 | `fe8546d6854ff6a93812a270e35b9b62efb92319f785660405d2fa95c63cefcc` |
| optimized NIR | 2935 | `db9c08c70c66e3775da6e53954a09a79c8832fbb3960793a7dc2466a4ec45e42` |
| pre-materialization MIR6502 | 2050 | `a2b7abc8ca89f05c9966be2557cd30a0caa227efbcdfbcd4b419037719b2e079` |
| classic/cart map | 1057 | `5314a371aa671b6971448b633bf3727957e147d30984c5db01f7a9104b5a0701` |
| MIR6502/cart map | 206 | `f20f17a251c88e73be32621125b90747074e3a656b83507ea655d6f459a70270` |

## Reproduction

For each source, compile the indicated backend/runtime pair and hash the
result. For example:

```sh
mkdir -p target/nir-target-baselines
cargo run --bin actionc -- --profile modern --backend mir6502 --runtime cart \
  -o target/nir-target-baselines/record-assignment-mir6502-cart.xex \
  fixtures/runtime/record_assignment.act
wc -c target/nir-target-baselines/record-assignment-mir6502-cart.xex
shasum -a 256 target/nir-target-baselines/record-assignment-mir6502-cart.xex
```

The routine-activation inspection hashes can be reproduced without retaining
generated files, for example:

```sh
cargo run --quiet --bin actionc-emit -- --profile modern --emit-nir \
  fixtures/runtime/native_routine_abi_baseline.act | shasum -a 256
cargo run --quiet --bin actionc-emit -- --profile modern \
  --backend mir6502 --emit-mir6502 \
  fixtures/runtime/native_routine_abi_baseline.act | shasum -a 256
```

The baseline concerns the complete load-format object, including segment and
run-address records. Runtime execution tests remain authoritative for behavior;
these hashes additionally catch layout, relocation, and packaging drift.
