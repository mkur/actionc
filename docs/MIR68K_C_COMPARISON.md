# MIR68K compared with MC68000 GCC

The [reference command](../tools/mir68k-c-reference/README.md) compiles equivalent
C and executes it in the same r68k VM as Action!. The alignment, control-flow and
register-retention work reduces default insertion-sort instructions by 47.5%
and matrix1 instructions by 55.6%. All reference states still match. This closes
the bounded code-quality milestone; construct and platform coverage can resume.

## Recorded results

The initial compiler baseline is `545d8e3`; the current compiler is `8ecf73a`.
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

## Wider native corpus

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

## Evidence for each change

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

## Reproduction and validation

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
Cross-platform validation is running on compiler revision `8ecf73a` in
[the Linux, Windows and macOS CI run](https://github.com/mkur/actionc/actions/runs/34837619732).
Its result was pending when this report was committed.

## Remaining opportunities

GCC still has smaller output and fewer executed instructions. Its multiply
helper uses SWAP, while ours rearranges words with shifts and reloads. GCC also
uses postincrement addressing, direct memory arithmetic, byte branches and
broader loop/register optimization. Those remain focused follow-ups, along
with extending the paired C corpus to SHA and DCT with explicit wrapping and
rounding contracts. A general allocator, new language constructs and platform
startup were outside this milestone; the next implementation work can return
to construct and platform coverage.
