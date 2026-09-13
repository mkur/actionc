# TACLeBench insertion sort

`kernel.inc` ports Sung-Soo Lim's insertion sort from the **SNU-RT Benchmark
Suite for Worst Case Timing Analysis**, as collected by TACLeBench. It retains
the original ten unsigned values, preceding sentinel, adjacent swaps, and all
six iteration statistics. The nested loops are iterative and need no new
language constructs.

This is a host-driven compiler fixture with one shared algorithm in
`kernel.inc`. `insertsort.act` supplies ordinary globals and a portable entry;
the MC68000 harness finds those objects through compiler-emitted symbols.
The 6502 driver lives in
[`tools/vm-runtime-tests/fixtures/insertsort_driver.act`](../../../../tools/vm-runtime-tests/fixtures/insertsort_driver.act)
and supplies the original fixed host buffers and completion marker. Both
6502 backends (`--mode optimized` and `--mode mir6502`) still run with cartridge
and standalone runtimes at `$3000`.

## Provenance

- [Pinned upstream source](https://github.com/tacle/tacle-bench/blob/c6a0d73e47bbd2bc86e34637156fb26dd4d5cf08/bench/kernel/insertsort/insertsort.c),
  revision `c6a0d73e47bbd2bc86e34637156fb26dd4d5cf08`, source header version 1.x.
- Author: **Sung-Soo Lim**; derived from the SNU-RT Benchmark Suite and collected
  by MRTC before TACLeBench.
- Upstream permits use, modification, and redistribution provided the SNU-RT
  Benchmark Suite is acknowledged. `insertsort.c` is the unmodified reference,
  including its attribution and permission notice.
- LF-normalized SHA-256:
  `2c35c25aff3fcfa23caa5fbe3c1d94195e1a336cb3043faf1db2419c10d3368f`.
- The Action! adaptation was added on 2026-09-13 under the repository's
  [GNU GPL license](../../../../LICENSE), retaining the required acknowledgement.

## Action! adaptations

Array elements use **LONGCARD**, preserving a 32-bit interpretation of C's
`unsigned int`. The statistics use **LONGINT**, including the initial minimum
of 100,000. Loop indexes fit INT; the initialization loop retains VOLATILE.
The swap temporary uses the same unsigned type as the array. This avoids
implementation-defined unsigned-to-signed conversions for extended high-bit
test inputs. The C oracle makes the same explicit type choice.

The original input is `[0, 11, 10, 9, 8, 7, 6, 5, 4, 3, 2]`. Sorting produces
`[0, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11]`, with checksum result zero. The original
return check is retained as `sum # 65`; extended-input sums are explicitly
unsigned modulo 2^32 in both the port and C oracle.

Index zero is a **sentinel no greater than any of the ten values**, and remains
unchanged. The first value is already a sorted prefix, so the outer loop starts
at index two. The inner loop moves each smaller value left by swapping adjacent
elements until the prefix is sorted. The sentinel makes an extra index-bound
condition unnecessary. Host inputs must satisfy this precondition.

`Main` adds the host command and result byte; the 6502 adapter also adds its
completion marker. The MC68000 harness detects return through its trampoline. Command
zero runs the original initialization and workload. Command one copies the
host input before sorting and uses host-supplied statistics; Sort resets the
current counters while retaining and updating the recorded minima/maxima.

## 6502 host memory contract

The fixed addresses belong to the 6502 adapter. Its 32-bit values are little-endian.

| Address | Meaning |
| --- | --- |
| `$0600` | Command: 0 original workload, 1 host input |
| `$0601` | Checksum result: 0 if the wrapping sum is 65, otherwise 1 |
| `$06FF` | Completion marker, `$A5` |
| `$0701`, `$0705`, `$0709` | Outer iteration count, minimum, maximum; LONGINT |
| `$070D`, `$0711`, `$0715` | Inner iteration count, minimum, maximum; LONGINT |
| `$07FF..$082A` | Working/output array: sentinel plus ten LONGCARD values |
| `$08FF..$092A` | Read-only host input: sentinel plus ten LONGCARD values |

The original workload ends with statistics `[9, 9, 9, 9, 1, 9]`. The current
inner counter describes the last insertion; it is not the total number of
swaps. Both arrays begin at page offset `$FF`, exercising accesses across pages.

## Coverage and checks

`vectors.txt` contains **209 C-reference cases**, run in all four 6502
backend/runtime combinations (**836 VM executions**) and in raw/optimized
MC68000 modes (**418 additional VM executions**):

- The original initialization/workload, ascending and descending arrays,
  duplicates, all equal values, and first/last misplaced elements.
- Byte, word, and signed/unsigned 32-bit boundaries, checksum wrap, and four
  nonzero sentinels, including sentinels with their high bit set.
- All 120 permutations of a five-element prefix, with the remaining five
  elements in place.
- 32 deterministic random arrays and their repeated sorted inputs with retained
  statistics, plus nine statistics seeds covering signed limits and thresholds.

Instrumentation reaches **12/12 outcomes of the six C if/while conditions**.
This is branch coverage, not complete path coverage. Expected array contents,
statistics, and checksum results come from C. The generator independently
checks C's sorted output and wrapping checksum using Python.

Every VM execution checks the complete 44-byte output array, 24-byte statistics,
result, completion marker, unchanged input, and all guard bytes in
`$0600..$09FF`. Standalone runs load no ROMs. LF and CRLF source/vector text use
the actual compiler, vector parser, and VM execution paths.

```sh
cd tools/vm-runtime-tests
cargo test --locked --test insertsort
```

The MC68000 runs decode the reference's little-endian fields into numbers,
then store them in target byte order. They compare all array elements, statistics,
checksum, unchanged input and command, and require balanced stack/register
return with protected code and unmapped guards. No benchmark object uses a
fixed address. The raw configuration compiles LF files and the optimized one
compiles CRLF files, including the shared include.

```sh
cargo test --locked --manifest-path tools/vm68k-runtime-tests/Cargo.toml --test insertsort
cargo run --locked --manifest-path tools/vm68k-runtime-tests/Cargo.toml -- fixtures/runtime/tacle/insertsort/insertsort.act
```

## Regenerating vectors

```sh
python3 tools/generate_insertsort_vectors.py
python3 tools/generate_insertsort_vectors.py --check
```

Regeneration requires Python 3 and a GCC-compatible C compiler (`--cc clang` is
supported). It verifies the normalized source hash, uses explicit 32-bit C
types, and serializes fields independently of host endianness. Normal Rust
tests use the committed vectors without a C compiler or network access.

Each row contains a label, command, input statistics, input array, expected
statistics, expected array, and result. The four memory fields encode their
complete contents as hex; commands and results are decimal.
