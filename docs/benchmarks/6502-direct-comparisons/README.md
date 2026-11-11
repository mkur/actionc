# Direct comparisons and carry reuse: measured results

Measured 2026-11-11 against the compiler frozen in [baseline.json](baseline.json).
[results.json](results.json) records source/image hashes, configurations,
code/storage extents, zero-page ranges, site counts and cycle measurements.
The six feature-off control images match the frozen images byte for byte.

The selector handles unsigned BYTE `captured <= indirect` and
`captured > indirect`. It retains the capture and ordered reads, puts the
indirect result in A, reverses the predicate and uses the existing carry
branch selector. SemIR and NIR contracts are unchanged. The implementation
includes a final code/home/image cost check, with rollback per routine and a
combined layout check. General reuse across independent comparisons or CFG
joins remains outside this implementation.

## Final sizes

Both sides use modern MIR6502, optimized NIR, identical sources and runtime,
and the source origin ($2C00 for TN/TNDBG). Only
`enable_captured_byte_comparisons` changes. Sizes include XEX framing.

| Program | Runtime | Before | After | Saved |
| --- | --- | ---: | ---: | ---: |
| TN | cartridge | 9,891 | 9,873 | 18 |
| TN | standalone | 10,016 | 9,998 | 18 |
| TNDBG | cartridge | 12,987 | 12,969 | 18 |
| TNDBG | standalone | 13,112 | 13,094 | 18 |
| Neutral three-routine fixture | either | 131 | 113 | 18 |
| Array-parameter string loop | either | 78 | 75 | 3 |

In both TN variants and runtimes, `Instr` changes from 66 to 60 bytes;
`Print` and `PrintFname` each change from 58 to 52. `Fnamecmp` remains 185.
The code totals fall by exactly 18 bytes. Initialized data and ordinary
storage sizes are unchanged, as are the 2,794 reserved BSS bytes. The BSS
range follows the shorter code by 18 bytes. The declared/allocated zero-page
union is unchanged; the three improved routines drop their individual length
temporary. The final header consumes carry directly, with the indirect load
followed by `CMP index_home` and the carry branch.

The first implementation grew `Fnamecmp` by 26 bytes by replacing its existing
dual-indirect selector with another pointer preparation and capture home. The
final cost check restores that routine and retains the three improvements.
Its `captured-byte-compare-declined-final-cost` report distinguishes a legal
candidate from a profitable final selection.

## Applicability

Counts below apply separately to each runtime. They count this new suffix
family, rather than every comparison in a program.

| Input | Candidates | Applied | Declined |
| --- | ---: | ---: | ---: |
| Neutral fixture | 3 | 3 | 0 |
| TN | 4 | 3 | 1: final cost |
| TNDBG | 4 | 3 | 1: final cost |
| `array_param_string_length_loop.act` | 1 | 1 | 0 |
| `comparison_values.act` | 0 | 0 | 0 |
| `pointer_invariant_loop.act` | 0 | 0 | 0 |
| `samples/graphics/fedora.act` | 0 | 0 | 0 |
| `standalone_sys_strings_runtime.act` | 0 | 0 | 0 |

All 16 program/runtime pairs have non-increasing final images. The extension
helps this captured-byte/pointer shape; these results do not establish a
general size improvement for arbitrary graphics, library or numeric code.
The proof tests separately exercise capture, extra-use, machine-state and
effect blockers that do not occur among these corpus candidates.

## Executed cycles

These measurements execute the exact TN/TNDBG feature-off/on images from the
size run. Both variants and runtimes agree at pointers $7000 and $70FF.
Cycles include the common call wrapper.

| Routine and workload | Before | After |
| --- | ---: | ---: |
| `Instr`: absent byte in a 200-byte counted string | 12,477 | 11,068 |
| `Print`: eight characters, `Putchar` replaced by RTS | 650 | 585 |
| `PrintFname`: eight-byte string, same output stub | 506 | 455 |

The search improves by 1,409 cycles (11.3%). Print timings cover the actual
loops and calls with deterministic output stubs, without screen-I/O time.
The search oracle covers empty strings, first/middle/last matches, misses,
lengths through 255, returned values, the final index and unchanged input.
The print oracle checks every emitted byte through length 255.

Neutral offset-one comparisons cover both outcomes and equality at two code
origins, in both runtimes, including a pointer crossing from $70FF to $7100.
All 80 measured cases improve by 6–8 cycles. Representative `<=` calls take
55→47 cycles for equality, 54→47 for less-than and 56→48 for greater-than.

## Validation and reproduction

The completed checks are 1,171 MIR6502 unit tests, 101 integration tests across
all 14 `tests/mir6502*.rs` targets, and 169 MIR6502 fixtures through emission
and load-file generation. The five affected VM targets pass 15 tests covering
comparison values/loops, TN reproducers, dispatch and the neutral regressions.
Focused emitted-instruction tests execute 262,144 calls covering every BYTE
pair for both predicates, with the feature off/on. VM tests also cover actual
LF/CRLF sources, aliases, live captures and a changing hardware input.

Run the optional image measurement before the optional TN image execution:

```sh
cargo test --release --locked --lib captured_compare_measure_corpus -- --ignored --nocapture
cargo test --locked --manifest-path tools/vm-runtime-tests/Cargo.toml --test direct_comparison_measurements -- --ignored --nocapture
```

Large images, MIR, entry tables and raw metrics stay under ignored
`build/direct-comparisons/`. The [implementation plan](../../MIR6502_DIRECT_COMPARISONS_AND_FLAG_REUSE_PLAN.md)
records the slices, proof boundaries and remaining scope.
