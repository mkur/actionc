# Stress Suite Status

Last refreshed after adding toolkit-inspired stress programs.

## Summary

The stress suite is now a regression guardrail rather than an active source of
large compatibility gaps. Every original-compatible stress file compiles under
both the original Action! cartridge and `actionc`. The remaining byte-level
differences are either exact matches, same-size drift, or cases where `actionc`
is intentionally tighter than the cartridge output.

The newest files are intentionally harder than the old probe-like stress
programs. They were added after the Action! Toolkit showed that the important
bugs live in interactions: long expression chains, runtime helpers, record
pointers, pointer aliases, local initialized storage, and fixed-address aliases
appearing together.

## Current Results

| Stress source | Original Action! | actionc | Status |
| --- | ---: | ---: | --- |
| `records.act` | 539 bytes | 539 bytes | exact match |
| `zero_page_scalars.act` | 98 bytes | 98 bytes | exact match |
| `strings.act` | 546 bytes | 546 bytes | same size, byte-different |
| `real_expr_chains.act` | 712 bytes | 743 bytes | toolkit-style expression chains |
| `advanced_pointers.act` | 1138 bytes | 1265 bytes | toolkit-style pointer/record mix |
| `layout_integration.act` | 570 bytes | 623 bytes | toolkit-style storage layout mix |
| `arrays.act` | 1167 bytes | 1154 bytes | actionc smaller |
| `pointers.act` | 900 bytes | 884 bytes | actionc smaller |
| `arithmetic_control.act` | 674 bytes | 672 bytes | actionc smaller |
| `calls.act` | 549 bytes | 546 bytes | actionc smaller |
| `zero_page.act` | 332 bytes | 256 bytes | intentional policy difference |

## Exact Matches

`records.act` is a byte-for-byte match and should be treated as a strong
regression sentinel for record layout, record-pointer field access, and
record-value argument passing.

`zero_page_scalars.act` is a byte-for-byte match for the original-compatible
subset of zero-page declarations. It verifies scalar aliases like
`BYTE zp_b=$E0` and `CARD zp_sum=$E4`.

## Toolkit-Inspired Stress

`real_expr_chains.act` stresses long CARD/INT expression chains inspired by
PMG/Turtle-style code: indexed `CARD ARRAY` terms, runtime multiply/shift
helpers, left-to-right materialization pressure, and function calls surrounding
the expression chain without relying on original-invalid nested call arithmetic.

`advanced_pointers.act` stresses real data plumbing: record pointers linked
through `CARD` fields, byte/card/int pointer variables, pointer-indexed
load/store, record-pointer parameters, loops, and pointer aliases crossing
routine calls.

`layout_integration.act` stresses storage placement interactions:
fixed-address aliases, initialized scalars, initialized and unsized arrays,
large local arrays, local initialized arrays, and absolute hardware arrays in
one source file.

## Accepted Differences

`strings.act` is the same size but byte-different. The main known drift is that
`actionc` preserves indexed assignment targets across calls more defensively
than the original compiler in some shapes. That is intentional for now: the
original reloads the scalar index after the call, which can be unsafe if the
call mutates the index variable.

`arrays.act`, `pointers.act`, `arithmetic_control.act`, and `calls.act` are now
smaller than the original compiler output. These are mostly due to safe modern
peepholes: separate source/target pointer slots, direct byte-shaped shifts,
folded-zero arithmetic, deferred register argument loading, and in-place
constant shifts.

`zero_page.act` is not an original-compatible code-shape comparison. It keeps
the current `actionc` policy where pointer declarations initialized to
zero-page addresses can become real zero-page pointer storage. Original Action!
treats `BYTE POINTER p=$E4` as normal object storage initialized to pointer
value `$00E4`.

## Validation

Use these commands after codegen changes:

```sh
cargo test -q
scripts/check-stress-fixtures.sh
surveys/stress/compare-original.sh zero_page_scalars
surveys/stress/compare-original.sh real_expr_chains advanced_pointers layout_integration
```

`scripts/check-stress-fixtures.sh` reports `zero_page.act` as `XFAIL` because that
source intentionally covers actionc's zero-page pointer-storage policy, not the
original compiler's pointer-initializer semantics.

Use `surveys/stress/compare-original.sh all` when refreshing all captured original
outputs. The detailed byte and segment notes live in `COMPARISON.md`.

## Next Direction

The stress suite no longer shows an obvious large compiler gap. Prefer using
real toolkit sources for the next compatibility improvements, with the stress
suite acting as a guardrail. If new stress coverage is needed, add focused
programs for mixed records/arrays, deeper string calls, pointer aliases in
loops, or signed comparisons inside nested control flow.
