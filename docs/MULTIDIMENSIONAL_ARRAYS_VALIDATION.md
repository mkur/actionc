# Fixed multidimensional arrays: validation and cost

The modern feature supports fixed rank two and higher on classic 6502, MIR6502
and MIR68K. Semantic/NIR layout tests also cover both 65816 layouts. Shaped
parameters, dynamic bounds, partial indexing and row views remain deferred.

## Execution coverage

Focused tests exercise rectangular and rank-three indexing, widening BYTE
coordinates, partial zero filling, static element addresses, record elements,
inline fields, unaligned storage, pointer decay and rebinding, destination/base
capture around calls, and exact volatile-coordinate order. Native tests include
recursive local backing, offsets above 65535, ABI/stack guards, HUNK relocation
at multiple bases, and source-free version-2 artifact loading.

The shaped benchmark sources retain the original flat/pointer baselines and
pinned C oracles. Matrix1 uses all 252 vectors for LONGINT/BYTE/INT/CARD and
10x10x10, 3x7x5 and 2x129x1 dimensions. Its B/C coordinates retain the C
column-major byte order. DCT uses all 181 vectors, comparing initialized input,
row-pass snapshot, final block, checksum/status and sign-preserving rounding.
Both corpora run under classic/MIR6502 with cart/standalone runtimes and
raw/optimized NIR on 68K. LF and CRLF reach actual source instrumentation.

## Native measurements

Measured on 2026-09-14 with compiler `1f12a56`, r68k 0.2.2, origin `$10000`, native loop promotion
and default machine optimizations. The NIR column controls only shared NIR
optimization. These are uninstrumented upstream inputs: LONGINT 10x10x10 for
matrix1 and the benchmark-generated 8x8 block for DCT. Stack traffic counts
bytes observed by the VM, including calls and saved registers. Frame size is
the largest routine frame, not peak total stack use.

| Benchmark | Variant | NIR | Code bytes | Instructions | Max frame | Stack read bytes | Stack write bytes |
| --- | --- | --- | ---: | ---: | ---: | ---: | ---: |
| matrix1 | flat/pointer | raw | 1712 | 114612 | 182 | 76131 | 46179 |
| matrix1 | flat/pointer | optimized | 990 | 72336 | 68 | 24753 | 14349 |
| matrix1 | shaped | raw | 2332 | 272801 | 210 | 147327 | 100103 |
| matrix1 | shaped | optimized | 1550 | 223833 | 124 | 100091 | 69307 |
| jfdctint | flat/pointer | raw | 8642 | 50750 | 1504 | 26794 | 20342 |
| jfdctint | flat/pointer | optimized | 5768 | 38767 | 742 | 13560 | 10432 |
| jfdctint | shaped | raw | 12884 | 68030 | 2316 | 35418 | 27918 |
| jfdctint | shaped | optimized | 8044 | 50499 | 1170 | 18622 | 15438 |

At this milestone, the shaped forms cost more. Optimized matrix1 takes about 3.09 times
as many instructions; DCT takes about 1.30 times as many. Inspection of optimized
matrix1 MIR shows three extra 32-bit constant-stride products in the inner loop
for the C, A and B coordinates, alongside the data multiplication. The pointer
baseline advances cursors instead. Repeated address computation and additional
temporaries also increase frame/stack traffic. This milestone establishes
correct indexing; later loop-invariant offset hoisting and induction-variable
strength reduction must respect mutable descriptors and alias/effect rules.

These are the initial feature measurements. See
[the index arithmetic optimization report](INDEX_ARITHMETIC_OPTIMIZATION.md)
for subsequent reductions in instructions, code size and stack traffic.

Reproduce correctness and measurements:

```sh
python3 tools/generate_matrix1_vectors.py --check
python3 tools/generate_jfdctint_vectors.py --check
cargo test --locked --manifest-path tools/vm-runtime-tests/Cargo.toml --test matrix1 --test jfdctint
cargo test --locked --manifest-path tools/vm68k-runtime-tests/Cargo.toml --test matrix1 --test jfdctint
cargo run --locked --manifest-path tools/vm68k-runtime-tests/Cargo.toml --example code_quality -- matrix1 matrix1-multidimensional jfdctint jfdctint-multidimensional
```

The measurement tool prints compiler revision, compiler-file status, input
hashes and options to stderr. Its CSV uses the same settings for both variants.
The 6502 tests report object sizes and aggregate instruction counts separately;
their host-capture adapters are included in those totals.
