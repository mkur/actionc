# MIR68K compared with MC68000 GCC

The [reference command](../tools/mir68k-c-reference/README.md) compiles equivalent
C and executes it in the same r68k VM as Action!. The latest comparison includes
flat and shaped matrix1 and DCT alongside the insertion-sort control.

## Descriptor alignment milestone

Compiler `bc9fafd` was measured with clean compiler sources, GCC 16.2.0,
GNU binutils 2.47.20260726 and the original-MC68000 `m68000/libgcc.a` multilib.
The target flags, `-O2`/`-Os`, no-LTO policy and existing C controls are unchanged.
The [new CSV](mir68k-descriptor-c-comparison.csv) retains all 15 rows, including
stack traffic. Earlier reports and CSVs below remain historical records.

These headline measurements use uninstrumented default entries and include
initialization, result checking, linked helpers and trampoline completion:

| Workload | Action! bytes / instructions | GCC `-O2` bytes / instructions | GCC `-Os` bytes / instructions |
| --- | ---: | ---: | ---: |
| insertsort | 1,622 / 5,349 | 574 / 1,426 | 440 / 1,499 |
| matrix1 | 1,044 / 69,106 | 306 / 20,230 | 298 / 22,032 |
| matrix1-multidimensional | 1,494 / 106,651 | 470 / 34,831 | 450 / 36,022 |
| jfdctint | 6,632 / 35,567 | 1,404 / 7,614 | 1,244 / 9,109 |
| jfdctint-multidimensional | 7,558 / 38,991 | 1,556 / 8,395 | 1,372 / 9,845 |

Each compiler passes 209 insertion-sort cases, 22 LONGINT/10×10×10 cases for
each matrix variant, and all 181 cases for each DCT variant: 1845 complete
reference executions plus 15 default runs. Separate DCT capture builds check
input, initialized state, row-pass state, final state, rounding, checksum and
status. Their reference instruction totals are marked `reference_instrumented`
in the CSV; capture calls do not affect headline bytes or instructions.
Uninstrumented default runs also check complete final arrays and scalar results
against the pinned upstream case.

The shaped C variants use mutable pointer-to-row descriptors and separate
backing arrays. The adapter reads the current pointer slot for array access,
for both languages. C uses valid aligned row objects; odd Action! addresses are
covered separately by native differential tests. The shaped matrix initializer
retains the flat 16-bit counter and volatile-read order using row division and
remainder, keeping C accesses within individual rows. Consequently its added
initialization cost is included in this separate shaped comparison. DCT uses
unsigned bit-pattern arithmetic and explicit sign extension to define wrapping
and shifts over the complete input corpus. Its IJG attribution and original
permission/no-warranty README are retained.

[The alignment report](MIR68K_DESCRIPTOR_ALIGNMENT.md) isolates the improvement:
shaped matrix1 falls from 144051 to 106651 instructions, and shaped DCT from
43279 to 38991. Guards add code and odd-path overhead, while frames and stack
traffic stay unchanged. Against GCC `-O2`, the resulting instruction ratios are
3.06× for shaped matrix and 4.64× for shaped DCT. These are instruction counts,
not cycle or elapsed-time ratios.

The remaining cost is visible in stack traffic and address formation. Shaped
matrix reads/writes 53741/31777 stack bytes in Action!, versus 16872/12076 with
GCC `-O2`. GCC's listing uses address registers, indexed effective addresses and
memory-destination arithmetic; these remain useful next targets for MIR68K.
Runtime guards are still needed for unknown Action! descriptors. This milestone
does not assume that a global initializer remains the descriptor's value.

Reproduce the latest comparison:

```sh
python3 tools/compare_mir68k_c.py --build-dir build/mir68k-descriptor-alignment/c-comparison insertsort matrix1 matrix1-multidimensional jfdctint jfdctint-multidimensional
```

That directory retains toolchain versions, selected libgcc, exact commands,
source hashes, resolved Action! options, assembly, linked helpers and listings.
The C runner's six adapter/instrumentation tests pass for actual LF/CRLF
handling; all four native flat/shaped DCT corpus configurations pass with the
shared instrumentation. Cross-compilation remains a developer-only dependency.

## Earlier recorded results

The earlier compiler baseline is `545d8e3`; that milestone completed at `8ecf73a`.
Both were measured with clean compiler sources, GCC 16.2.0 and GNU binutils
2.47.20260726, targeting the original MC68000. C sources, flags, selected
libgcc, fixtures and reference vectors are unchanged. Both GCC modes and
optimized Action! pass 209 insertion-sort and 22 matrix1 vectors, for 693
reference executions, plus the six default-entry executions. Matrix1 uses the
LONGINT/10×10×10 variant.

The [current CSV](mir68k-c-comparison.csv) and
[initial CSV](mir68k-c-comparison-baseline.csv) include exact stack traffic and
instruction totals across the complete selected corpus. Default-entry results:

| Benchmark | Compiler | Executable bytes | Instructions | Stack bytes read / written |
| --- | --- | ---: | ---: | ---: |
| Insertion sort | Action! initial | 2,242 | 10,617 | 3,506 / 2,586 |
| Insertion sort | Action! current | 1,610 | 5,572 | 2,125 / 1,843 |
| Insertion sort | GCC `-O2` | 574 | 1,426 | 186 / 120 |
| Insertion sort | GCC `-Os` | 440 | 1,499 | 382 / 140 |
| Matrix1 | Action! initial | 1,790 | 162,876 | 72,256 / 42,304 |
| Matrix1 | Action! current | 990 | 72,336 | 24,753 / 14,349 |
| Matrix1 | GCC `-O2` | 306 | 20,230 | 16,860 / 12,064 |
| Matrix1 | GCC `-Os` | 298 | 22,032 | 16,864 / 12,068 |

Against GCC `-O2`, current Action! executes 3.91× as many instructions for
default insertion sort and 3.58× for matrix1, down from 7.45× and 8.05×.
Across the reference corpus, the ratios fall from 7.75× and 7.62× to 4.26× and
3.24×. These are instruction ratios, not cycle or wall-clock timings. Both
sides include initialization, result checking and required linked helpers.
GCC remains free to inline and transform loops; its optimizations are not
disabled to match Action!'s current capabilities.

## Earlier native corpus

The [initial](mir68k-optimization-baseline.csv) and
[current](mir68k-optimization-current.csv) files retain raw and optimized NIR
measurements for all seven benchmarks. The table uses optimized NIR and counts
stack reads plus writes, including ABI traffic.

| Benchmark | Instructions, initial → current | Stack bytes, initial → current | Largest frame, initial → current |
| --- | ---: | ---: | ---: |
| Insertion sort | 10,617 → 5,572 | 6,092 → 3,968 | 182 → 140 |
| Matrix1 | 162,876 → 72,336 | 114,560 → 39,102 | 166 → 68 |
| Binary search | 9,156 → 8,302 | 2,488 → 1,622 | 126 → 68 |
| SHA | 28,606 → 14,924 | 28,211 → 15,543 | 786 → 452 |
| Integer DCT | 47,129 → 38,767 | 36,922 → 23,992 | 1,304 → 742 |
| ADPCM decoder | 22,924 → 16,361 | 18,713 → 15,160 | 794 → 704 |
| ADPCM encoder | 2,304,120 → 1,990,889 | 1,153,735 → 686,339 | 802 → 736 |

Every optimized benchmark reduces executable bytes, instructions, stack traffic
and frame size against both the initial baseline and slice 4's defaults.
Raw NIR does not benefit from source-local promotion. Allocation adds 4–26
instructions to four raw benchmarks relative to slice 4, through preserved-
register saves/restores, while their stack traffic and frames decrease. The
other raw instruction counts are unchanged. This small overhead is accepted;
optimized NIR is the default and improves throughout the corpus. Largest frame
means one routine's reservation, not peak stack use including nested calls.

## Earlier feature comparisons

The [feature comparison](mir68k-optimization-features.csv) uses the same current
compiler revision throughout. All configurations check their default result.
Raw rows are retained in the CSV; these are the optimized rows:

| Configuration | Insertion-sort instructions / stack bytes | Matrix1 instructions / stack bytes |
| --- | ---: | ---: |
| Current defaults | 5,572 / 3,968 | 72,336 / 39,102 |
| No indirect alignment selection | 9,393 / 3,968 | 123,836 / 39,102 |
| No control-flow selection | 6,359 / 4,186 | 83,841 / 42,356 |
| No register allocation | 5,957 / 6,026 | 89,476 / 140,162 |
| Conservative promotion | 5,686 / 4,236 | 99,363 / 109,294 |
| Conservative promotion and all target options disabled | 12,700 / 12,308 | 185,880 / 189,448 |

**Alignment:** matrix1's `Multiply` now uses native longword loads/stores where
verified NIR proves even addresses through pointer assignments and loop
updates. Odd and unknown pointers still use bytewise accesses. Unknown pointer
parameters in `PinDown` retain byte stores; a pointer type alone proves nothing.
Mutable descriptor initializers also remain insufficient evidence.

**Control flow:** branch-only comparisons emit CMP followed by the appropriate
conditional branch. Physical fallthrough removes avoidable jumps. Numeric
boolean results still normalize to 0/1, and selected edges preserve parallel
assignment semantics.

**Register retention:** matrix1's `Multiply` falls from 786 to 360 executable
bytes. Its three loop pointers occupy D4–D6, and its inner counter occupies D7.
The routine saves those registers once and restores them on return. Insertion
sort retains its promoted loop values across backedges. Values under register
pressure still spill. Wider promotion alone increases edge-staging traffic;
allocation resolves transfers using their final locations and breaks cycles
with one normalized spill slot. This is why promotion and allocation became
defaults together.

## Earlier reproduction and validation

The [implementation plan](MIR68K_OPTIMIZATION_IMPLEMENTATION_PLAN.md) records
the six slices and validation gates. Baseline files retain the old compiler's
results; use the recorded revision when reproducing historical NIR policy.
On `8ecf73a`, run:

```sh
cargo run --locked --manifest-path tools/vm68k-runtime-tests/Cargo.toml --example code_quality
python3 tools/compare_mir68k_c.py --build-dir build/mir68k-optimization/slice6/c-reference
```

For feature rows, select `insertsort matrix1` in the measurement example and
add `--no-pointer-alignment`, `--no-control-flow`, `--no-register-allocation`,
`--conservative-promotion`, or `--no-codegen-opt --conservative-promotion`,
respectively. Configuration names in the feature CSV follow that order;
`current` has no extra flags. The runners record resolved options, compiler
revision/status, input hashes, C toolchain and commands. Local CSVs, metadata,
machine listings and disassemblies are retained under
`build/mir68k-optimization/slice6/`. C compilation uses `-mcpu=68000`, explicit
integer widths and `-fwrapv`; see the reference command for the complete flags.

Local validation passes all 65 native tests, including complete reference
states for seven benchmark families, ABI preservation, recursion, pressure,
mixed widths, cyclic copies, odd pointers, volatile accesses and fault exits.
Backend/contract tests pass. Shared NIR changes pass the full compiler suite,
unchanged snapshots and all 51 sweep fixtures in an isolated checkout.
Cross-platform validation was started on compiler revision `8ecf73a` in
[the Linux, Windows and macOS CI run](https://github.com/mkur/actionc/actions/runs/34837619732).
Its result was pending when that earlier report was committed.

## Remaining opportunities

Address-register allocation, indexed/postincrement addressing, memory-destination
arithmetic and cheaper wide multiply helpers remain useful follow-ups. The
paired corpus now includes DCT; SHA is a possible later addition. Optimization
must continue to preserve mutable descriptors, captured addresses, volatile
accesses and odd-address behavior.
