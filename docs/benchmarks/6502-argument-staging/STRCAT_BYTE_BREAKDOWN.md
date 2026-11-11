# Strcat byte and cycle breakdown

The current 125-byte `Strcat` spends 37 bytes preparing pointers, 20 preparing
outgoing arguments, 14 homing incoming parameters, 50 on computation and
memory accesses, and four on call/return instructions. Sixteen of the outgoing
argument bytes are temporary staging; four write required ABI homes.

This accounts for the entire 74-byte difference from the frozen 51-byte
Oscar64 callable control. Only ten bytes of that difference belong to outgoing
argument transfer. The other 64 belong to parameter homing, pointer preparation
and computation. These are observed costs, not proven removable bytes.

## Scope and identities

The Action body is the modern MIR6502, optimized-NIR `Strcat` from
[TN.ACT](../../../samples/tn/modern/TN.ACT), with origin `$2C00`. Its canonical
range is `$3875..$38F2`, containing 57 instructions. The accounting agrees
across TN/TNDBG, cartridge/standalone and argument-staging feature-off/on:
eight measured images match the [frozen hashes](baseline.json). Compiler code
is `6311c110`; the unchanged baseline code is `476cfecf`.

The C controls are the frozen diagnostic callable/no-inline `-Os` and `-O3`
images in the [kernel scorecard](../../../surveys/tn/c-oscar64/kernels.json).
Both have a 51-byte `Strcat`. Their incoming pointers already occupy zero-page
pairs `P5/P6` and `P7/P8`, which remain usable across the helper call. Their ABI
and C automatic-storage assumptions contribute to this result.

Body sizes exclude caller argument placement, parameter storage and the
separate `MovePage` body. The [instruction manifest](strcat-breakdown.json)
contains every instruction, byte assignment, address and verified image hash.

## Complete byte accounting

| Cost | Action bytes | C bytes | Difference | Action sample cycles |
| --- | ---: | ---: | ---: | ---: |
| Incoming parameter homing | 14 | 0 | 14 | 19 |
| Pointer preparation and reconstruction | 37 | 0 | 37 | 52 |
| Computation and semantic memory accesses | 50 | 37 | 13 | 77 |
| Outgoing argument capture and transfer | 20 | 10 | 10 | 30 |
| `JSR MovePage` and `RTS` | 4 | 4 | 0 | 12 |
| **Total** | **125** | **51** | **74** | **190** |

The cycle sample uses `s=$7000`, `s^=3`, `t=$7200`, `t^=8`. The Action column
counts only instructions inside `Strcat`, including its JSR/RTS. Executing
`MovePage` adds 163 cycles and the common wrapper adds 17, giving the previously
measured 370. With `s=$70FF`, an empty destination and one source byte, the
conditional high-byte increment raises `Strcat` to 194 cycles; the helper
takes 37 and the wrapper 17, for 248. All eight images agree.

The 50-byte computation category includes pointer scratch traffic inside
arithmetic lowering. It is the emitted implementation cost of that computation,
rather than a minimum instruction count for the Action expression.

## Execution order

The same 125 bytes divide into these contiguous phases in the canonical TN
cartridge listing. Address ranges are half-open.

| Range | Bytes | Work |
| --- | ---: | --- |
| `$3875..$3883` | 14 | Save A/X/Y and `$A3` to ordinary `s`/`t` parameter homes |
| `$3883..$388A` | 7 | Establish original `s` in `$AE/$AF`, reusing incoming X |
| `$388A..$38A4` | 26 | Compute `s+s^`, then add one through `$AC/$AD` |
| `$38A4..$38AC` | 8 | Copy computed destination to `$A0/$A1` |
| `$38AC..$38BB` | 15 | Compute `t+1` and place its bytes in `$A2/$A3` |
| `$38BB..$38CB` | 16 | Reload `t` into `$AC/$AD`, read `t^`, write `$A4` |
| `$38CB..$38D1` | 6 | Reload destination A/X and source-low Y |
| `$38D1..$38D4` | 3 | Call `MovePage` |
| `$38D4..$38E8` | 20 | Rebuild `s` in `$AC/$AD` and `t` in `$AE/$AF` |
| `$38E8..$38F1` | 9 | Execute the indirect byte update `s^==+t^` |
| `$38F1..$38F2` | 1 | Return |
| **Total** | **125** | |

Outgoing argument transfer has a useful finer split:

| Transfer | Bytes | Sample cycles |
| --- | ---: | ---: |
| Copy destination `$AC/$AD` to `$A0/$A1` | 8 | 12 |
| Capture source-low in `$A2` | 2 | 3 |
| Reload A/X/Y from `$A0/$A1/$A2` | 6 | 9 |
| Write required source-high `$A3` and length `$A4` | 4 | 6 |
| **Total** | **20** | **30** |

The 16-byte staging portion requires preservation and observation proofs before
deletion. Retaining the high byte in X through the existing setup could replace
one reload with `TAX`, a one-byte opportunity while keeping its store. The
current selector excludes this indirect-producer/machine-callee shape.

## Why the post call reloads need alias proofs

At the call, `$AE/$AF` still contain the original `s` and `$AC/$AD` the original
`t`. `MovePage` is annotated as preserving those pairs. Reusing them for the
final update would still require proof that its copy leaves the parameter homes
unchanged. The current report records two
`pointer-retention-unknown-call-footprint` blockers in `Strcat`; the shared
[callee write summary](../../../src/mir6502/analysis/known_callees.rs) treats
unsupported indirect writes as unknown.

The VM confirms a concrete counterexample. Let the destination append address
equal `t`'s parameter home, and copy two bytes `$00,$73` there. `MovePage`
changes the parameter value from `$7200` to `$7300`. With source lengths two
at `$7200` and five at `$7300`, the subsequent `s^==+t^` must use five. Starting
from destination length 255, the correct result is four; retaining the earlier
`t` would produce one. A second probe overwrites `s`'s home and confirms that
the update must follow its new pointer. Both probes pass on all eight images.

These 20 bytes therefore preserve observable behavior for admitted pointer
aliases. Pair preservation alone does not establish source-home equality.

## Implications for the next optimization

The next bounded investigation should examine computation within the first
argument and sharing pointer setup before the call. The destination expression
uses 26 bytes, including 14 bytes of instructions accessing pointer scratch
cells (with a conditional `INC`). The C control uses a register increment and
carry repair. The two pre-call setups of original pointers cost 17 bytes; `t`
is also read from its home during `t+1` calculation. Eliminating duplicate
home reads needs proof that intervening argument writes cannot alias that home.

Extending high-byte forwarding to this setup is a smaller candidate, with a
one-byte opportunity if its store remains. The 20-byte post-call reconstruction
requires stronger callee write-range and parameter-alias facts before it can
be reduced for any qualified case. Its byte count cannot be treated as a
generic saving available from `MovePage`'s preservation annotation.

## Reproduction and validation

The audit uses the frozen listings under `build/argument-staging/before/`,
the eight images from `argument_staging_measure_corpus`, and the two frozen
Oscar64 kernel controls. The generator verifies every classified instruction
against its XEX bytes, checks full-image hashes, compares `-Os`/`-O3` totals,
and exercises actual LF/CRLF listing parsing.

```sh
python3 tools/compare6502/tn/strcat_breakdown.py --check \
  --roles-output build/argument-staging/strcat-byte-roles.tsv
cargo test --locked --manifest-path tools/vm-runtime-tests/Cargo.toml \
  --test strcat_cost_breakdown -- --ignored --nocapture
```

The opt-in VM test attributes actual cycles to those instruction roles and
executes 16 normal/carry cases plus 16 parameter-alias cases. Entry tables and
generated role tables also pass through LF/CRLF parsing. The compiler and its
selection policy are unchanged by this audit.
