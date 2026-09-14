# TACLeBench binary search

`kernel.inc` ports Sung-Soo Lim's binary search from the **SNU-RT Benchmark
Suite for Worst Case Timing Analysis**, as collected by TACLeBench. It retains
the original 15 key/value records, iterative search, and pseudorandom
initialization. No new language constructs are needed.

The maintained Action! source uses LONGINT. The VM harness derives BYTE, INT,
and CARD variants by changing the record fields, query, result, and function
types, and explicitly narrowing initialization values. All four variants share
the same search body. Both modern backends (`--mode optimized` and
`--mode mir6502`) run with cartridge and standalone runtimes.

`binarysearch.act` supplies portable state and an entry around `kernel.inc`.
The fixed host memory map and completion marker now live in
[`tools/vm-runtime-tests/fixtures/binarysearch_driver.act`](../../../../tools/vm-runtime-tests/fixtures/binarysearch_driver.act).
The native adapter uses emitted symbol addresses and verifies record count,
element extent and stride before serializing individual fields in target order.

## Provenance

- [Pinned upstream source](https://github.com/tacle/tacle-bench/blob/c6a0d73e47bbd2bc86e34637156fb26dd4d5cf08/bench/kernel/binarysearch/binarysearch.c),
  revision `c6a0d73e47bbd2bc86e34637156fb26dd4d5cf08`, source header version 2.0.
- Author: **Sung-Soo Lim**; originally named `bs`, derived from the SNU-RT
  Benchmark Suite and collected by MRTC before TACLeBench.
- Upstream permits use, modification, and redistribution provided the SNU-RT
  Benchmark Suite is acknowledged. `binarysearch.c` is the unmodified reference,
  retaining its attribution and permission notice.
- LF-normalized SHA-256:
  `6be708e5870717a7a62b7b26cb9589838eb9ddcb40c64f15e3b883fab90c9bce`.
- The Action! adaptation was added on 2026-09-13 under the repository's
  [GNU GPL license](../../../../LICENSE), retaining the required acknowledgement.

## Types and source behavior

| Record fields and query | Search result | Record size | C-reference cases |
| --- | --- | --- | --- |
| LONGINT | LONGINT | 8 bytes | 233 |
| BYTE | INT | 2 bytes | 460 |
| INT | INT | 4 bytes | 231 |
| CARD | LONGINT | 4 bytes | 229 |

The original C `int` and `long` use explicit signed 32-bit types in the reference.
The narrower C references change the same field, query, and result types as the
Action! variants. BYTE results widen to INT and CARD results widen to LONGINT,
so all unsigned stored values remain representable alongside the missing-key
result, `-1`. Signed variants retain the upstream ambiguity when a found record
itself stores `-1`; the API returns a value, not a separate presence flag.

The search keeps signed INT indexes, including the terminating upper bound of
`-1`. While the bounds overlap, it selects `(low+up) RSH 1`, compares the key,
and narrows one bound. The sum is nonnegative and at most 28, making Action!'s
logical shift equivalent to the original C right shift. A match assigns the
stored value and sets `up=low-1`, preserving the original loop exit and its
duplicate-key selection.

The volatile generator seed remains LONGINT in every variant. Initialization
uses `(seed*133+81) MOD 8095`, with the original zero seed and call order. Its
intermediate product needs more than 16 bits. BYTE table fields take the low
eight bits of generated values; INT and CARD hold the full generated range.

The upstream initializer produces **unsorted keys** and searches for key 8,
returning `-1`. Command zero preserves this workload, including the original
record order; it does not silently sort the table. Command one accepts a host
table whose keys must be nondecreasing. Additional sorted-table vectors exercise
successful searches and meaningful missing-key searches throughout the range.

## 6502 host memory contract

Tests compile at `$3000`. All multibyte fields are little-endian, with no record
padding. Let `w` be the record-field width: 1 for BYTE, 2 for INT/CARD, 4 for
LONGINT.

| Address | Meaning |
| --- | --- |
| `$0600` | Command: 0 initialize and search for 8; 1 search host table |
| `$0601` | Query, `w` bytes; unchanged and ignored by command zero |
| `$0609` | Search result, 2 or 4 bytes as listed above |
| `$0611` | Volatile generator seed, LONGINT; unchanged by command one |
| `$06FF` | Completion marker, `$A5` |
| `$07FF` | Fifteen key/value records, `30*w` bytes |

Command zero overwrites the seed and every record. Command one preserves the
complete table. Unused bytes between control fields and around the table remain
guard bytes, including the bytes beyond a narrower result. The table starts
at page offset `$FF`, exercising field accesses across a page boundary.

## Coverage and checks

The **1,153 C-reference cases** run in all four backend/runtime combinations,
for **4,612 6502 VM executions** and 16 compiled programs. The native adapter
adds **2,306 executions** across eight raw/optimized programs, checking complete
records, result, seed, unchanged query/command, and stack/register preservation. Each type covers:

- Original initialization and search, starting from a poisoned table and seed.
- Every position in a sorted table, plus queries below, between, and above its
  keys. BYTE additionally tests **all 256 query values** against a boundary table.
- Signed/unsigned extrema and adjacent values, including `$7FFF/$8000`,
  `$7FFFFFFF/$80000000`, and return values 255, 65535, -32768, and -2147483648.
- Duplicate groups and all-equal keys with distinct stored values, checking the
  exact record selected by the C search.
- Eight deterministic random sorted tables per type, querying all 15 present
  keys and five absent keys in each.

C instrumentation reaches **6/6 outcomes of the three if/while conditions for
each type**. This is branch coverage, not complete path coverage. Expected
lookup results, table contents, and seed values come from C. An independent
Python linear lookup checks unique-key results and membership of duplicate-key
results before accepting vectors; the exact duplicate choice comes from C.

VM tests compare every byte in `$0600..$09FF`, including the result, seed,
complete table, unchanged query, and guards. Standalone runs load no ROMs. LF
and CRLF pass through source instrumentation, vector parsing, and compilation
before actual execution. The narrow Action! variants are generated in temporary
directories, so there are no duplicated maintained search implementations.

```sh
cd tools/vm-runtime-tests
cargo test --locked --test binarysearch
```

```sh
cargo test --locked --manifest-path tools/vm68k-runtime-tests/Cargo.toml --test binarysearch
```

## Regenerating vectors

```sh
python3 tools/generate_binarysearch_vectors.py
python3 tools/generate_binarysearch_vectors.py --check
```

Regeneration requires Python 3 and a GCC-compatible C compiler (`--cc clang` is
supported). It verifies the normalized source hash, compiles four typed C
references, and serializes individual fields independently of host padding and
endianness. Normal Rust tests use the committed vectors without a C compiler or
network access.

Each row contains the type, label, command, query, seed, table, expected seed,
expected table, and result. The command is decimal; all six memory fields encode
their complete little-endian contents as hex. The table consists of interleaved
keys and values at the selected type's width.
