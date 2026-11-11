# Immediate bitwise argument X forwarding results

Implemented through the [three-slice plan](../../MIR6502_ARGUMENT_X_BITWISE_PLAN.md).
`enable_argument_x_bitwise_forwarding` is enabled in optimized MIR6502 and
disabled in the conservative default. Measurement date: 2026-11-11.
[baseline.json](baseline.json) freezes eight controls at `9a43c963`;
[results.json](results.json) records the corpus, hashes, storage and checks.

The selector can retain a stored byte in X across an immediate BYTE AND/OR/XOR
in A. It keeps the bitwise instruction, every store, home and source access
in place, inserts TAX beside the existing store and removes the later X
reload. The extension uses the existing costed batch and typed physical/flag
proof. Earlier restricted staging retains its basic gap policy.

## Sizes and cycles

Only the new control changes within each pair. Precise callee entry proofs
and all preceding optimized strategies remain enabled, with modern MIR6502,
optimized NIR and entry-reachable linking. Sizes include XEX framing.

| Input | Runtime | Before | After | Saved |
| --- | --- | ---: | ---: | ---: |
| Neutral, either origin | cartridge | 205 | 202 | 3 |
| Neutral, either origin | standalone | 205 | 202 | 3 |
| TN | cartridge | 9,858 | 9,857 | 1 |
| TN | standalone | 9,983 | 9,982 | 1 |
| TNDBG | cartridge | 12,953 | 12,952 | 1 |
| TNDBG | standalone | 13,078 | 13,077 | 1 |

Each neutral forwarding routine shrinks 40→39 bytes. Calls through the
status-setting harness with pointer `$7000`, word `$1234` and tag 1 take
142→141 cycles for all three operations. Both runtimes/origins agree. The
fixture uses no embedded runtime, so cartridge and standalone sizes coincide.
Neutral origins are `$3000`/`$41C7`; TN variants use `$2C00`.

`Tag` shrinks 91→90 bytes in both variants. Its `XOR #$7F` setup now permits
forwarding the captured high byte. A sample toggling the first of two untagged
rows takes 241→240 cycles in both runtimes/variants. Sixty-four exact-image
calls cover both row indices and all combinations of tagged/untagged rows.
They check row data, tagged count/mask, Store's arguments and full entry
flags/stack; every case saves one cycle. `Strcat` remains 113 bytes and its
lengths-3/8 sample remains 355 cycles.

Eight of 36 runtime/origin pairs shrink; 28 are byte-identical. Sixteen
additional applications save sixteen bytes across those images: three sites
in each of four neutral images and one in each of four TN images. Every new
site uses a zero-page reload, saving one byte/one cycle. All eight frozen
controls and the previous 32 feature-on corpus images match the new feature-off
outputs. Frames, storage capacities, reservation lengths and occupied zero
page agree. Code-relative data can relocate; mapped symbol totals can count
aliases and the zero-page census excludes implicit runtime-only scratch.

## Scope and proof

The new family requires BYTE width, A as destination and left operand,
`ConstU8` as right operand, AND/OR/XOR, no carry input and ignored carry output.
These immediate instructions define N/Z and preserve C/V and X. Identity
masks still emit the actual instruction. The policy extends only G between
the retained store and X reload; the existing tail vocabulary is unchanged.
Variable/memory/WORD operands, other registers, shifts, address builders,
intermediate calls and unknown effects keep their existing paths.

Both N/Z obligations remain: TAX must not overwrite a live input, and removing
LDX must not remove an observed definition. Existing tail restoration and
callee-entry contracts retain their gates. Complete physical writes must
leave H unchanged, incoming X must be dead and G must not read/clobber X,
including implicit address inputs. The driver reconstructs the typed gap
policy, physical map and exact replacement. Final replay checks proposed
callee input facts and published outputs, unchanged frames and strict XEX
savings. Missing or altered witnesses reject.

TN/TNDBG each gain the `Tag` site. `Sort` retains its unsupported indexed setup;
the nine unavailable callee-entry cases retain their previous eligibility.
All other arithmetic, pointer, predicate, string-loop, resident-string and
graphics controls keep their sizes. The selector has no source-program or
callee-name special case.

The same accepted batch uses seven top-level emissions for maps, code/home
costs and XEX costs. No full materializations or per-site trials are added.
The neutral feature-off case uses one map; the accepted feature-on case uses
seven emissions. TN variants already have accepted baseline batches and use
seven in both settings. Precise entry-query/CFG counts stay unchanged within
each measured pair. Single release-profile samples under concurrent testing
are 22.2→21.2 ms for neutral cartridge, 8.61→9.08 s for TN and 25.51→25.08 s
for TNDBG; these observations do not establish a compiler-speed change.

## Qualification and reproduction

Six focused tests include 73,728 emitted executions over all byte inputs and
forwarded byte values, all three operations, six immediate boundaries,
both origins and all C/V combinations. An independent host oracle checks
A/X/Y/N/Z/C/V/SP and the actual immediate opcode, including identity masks.
Tests retain X use/clobber, H alias, unsupported-tail and wrong-policy/witness
fallbacks, store/frame equality, reporting and idempotence.

Two VM tests cover 8,304 matrix calls across raw/optimized NIR, both runtimes/
origins and actual LF/CRLF parsing, plus 192 alias/feature-composition calls.
They compare normalized writes and returned state, guards and ordered word
reads. Alias cases model parameter/pointer setup and the case where the low
capture store changes the next source byte. Both settings reproduce exact
measured neutral images. Eight measured TN images pass 80 string and 16
post-call parameter-home alias cases, plus the 64 real Tag executions.

Final checks pass 1,234 MIR6502 unit tests, 107 integration tests in 20 targets,
169 emission/load fixtures and 37 tests in 12 affected VM targets, with two
TN dispatch tests separately. Coverage also includes
eight LF/CRLF MADS assemblies/relocations, scoped arithmetic/indexed/indirect
runtime and selection checks, and frozen Strcat static/VM audits. Exact counts
are in `results.json`. The existing indexed-byte selection-count discrepancy
remains separate from runtime correctness. No shared NIR or semantic contract
changed.

The exporter writes XEX, assembly, MIR, entry/parameter maps and complete
facts to ignored `build/argument-x-bitwise/measured/`:

```sh
CARGO_INCREMENTAL=0 cargo test --release --locked --lib argument_x_bitwise_measure_corpus -- --ignored --nocapture
CARGO_INCREMENTAL=0 cargo test --locked --manifest-path tools/vm-runtime-tests/Cargo.toml \
  --test argument_x_bitwise_measurements -- --ignored --nocapture
CARGO_INCREMENTAL=0 cargo test --locked --test mir6502_argument_x_bitwise -- --ignored --nocapture
```

MADS must be on PATH or selected with `ACTIONC_MADS`. Set
`ACTIONC_ARGUMENT_X_BITWISE_MEASURED_IMAGES` to the measured directory's
absolute path for fresh neutral and `tn_dispatch` image comparisons.
`ACTIONC_ARGUMENT_X_BITWISE_BASELINE` selects frozen neutral controls, and
`ACTIONC_ARGUMENT_X_BITWISE_METRICS=1` prints neutral cycle samples. Historical
exporters/helpers explicitly disable this later control to preserve their
artifacts.
