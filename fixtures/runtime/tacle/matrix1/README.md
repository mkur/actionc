# TACLeBench matrix1

`matrix1.act` ports Juan Martinez Velarde's integer matrix multiplication from
DSP-Stone, as collected by TACLeBench. The maintained source preserves the
original **10x10 matrices**, three nested loops, typed pointer traversal, and
initialization through array parameters. It uses LONGINT for matrix elements.

The VM harness derives BYTE, INT, and CARD variants and two rectangular shapes
from that source. All variants share the multiply body; no multidimensional
arrays, recursion, or new language features are needed. Both modern backends
(`--mode optimized` and `--mode mir6502`) run with cartridge and standalone
runtimes.

## Provenance

- [Pinned upstream source](https://github.com/tacle/tacle-bench/blob/c6a0d73e47bbd2bc86e34637156fb26dd4d5cf08/bench/kernel/matrix1/matrix1.c),
  revision `c6a0d73e47bbd2bc86e34637156fb26dd4d5cf08`, source header version 1.x.
- Author: **Juan Martinez Velarde**; originally from **DSP-Stone**.
- Upstream permits use, modification, and redistribution freely. `matrix1.c`
  is the unmodified reference, retaining its complete attribution and permission
  notice.
- LF-normalized SHA-256:
  `de07299ca342c626454bc655a4ba4425935571f36643383e51e5bfc27d46654d`.
- The Action! adaptation was added on 2026-09-13 under the repository's
  [GNU GPL license](../../../../LICENSE).

## Layout and Action! adaptations

For dimensions `Rows`, `Inner`, and `Columns`, the multiplication is
`A[Rows,Inner] * B[Inner,Columns] = C[Rows,Columns]`. All three arrays are flat,
with the upstream layout:

| Matrix | Storage | Element offset |
| --- | --- | --- |
| A | Row-major | `row*Inner + k` |
| B | Column-major | `column*Inner + k` |
| C | Column-major | `column*Rows + row` |

Each output column starts A's pointer at its beginning. Each output row starts
B's pointer at the selected column. The inner loop multiplies successive pairs
and accumulates through C's pointer, then advances that pointer to the next
output element. C post-increments become explicit Action! byte-address increments
by `ElementBytes`, after both source reads.

Loop indexes use CARD, sufficient for all tested dimensions. Matrix elements,
typed pointers, array parameters, and the volatile initialization value share
the selected element type. The checksum remains LONGINT and the status remains
INT in every variant.

Command zero retains the original initialization: A and B become all ones and
C is cleared. The original workload produces 100 values of 10, checksum 1000,
and status zero. Command one uses host-supplied A and B. Multiply initializes
each C element before accumulating, so prior C contents are irrelevant.

The checksum predicate stays **`checksum=1000`**, including for rectangular
variants: their initialized workloads return `-1` because their sums are 105
and 258. Status zero describes that predicate, not general matrix correctness.
Tests compare the complete product independently of status.

## Arithmetic and C reference

The faithful LONGINT port interprets C `int` as signed 32-bit. Narrow variants
use explicit `uint8_t`, `int16_t`, or `uint16_t` elements in C. Extended inputs
exercise wrapping multiplication and accumulation modulo the element width,
and a wrapping 32-bit checksum.

The reference generator preserves the original loops and advancing pointers.
For wrapping vectors it replaces the multiply-accumulate expression with an
int64_t calculation followed by explicit width/sign conversion, and defines
checksum addition through uint32_t. These operations avoid C signed overflow
and implementation-defined narrowing while matching Action!'s integer behavior.
They are documented extensions to the upstream arithmetic contract.

An independent Python implementation uses indexed dot products and validates
every C output element, checksum, and status. For **45 LONGINT cases** whose
products, partial sums, and checksum all fit int32_t, a second compiled C
reference uses the original arithmetic expressions and must produce identical
state. These include the original workload for each shape.

## Host memory contract

Tests compile at `$3000`. All multibyte fields are little-endian. For element
width `w` (1, 2, or 4), the array lengths below are in elements.

| Address | Meaning |
| --- | --- |
| `$0600` | Command: 0 initialize then multiply; 1 multiply host matrices |
| `$0601` | Status, INT: 0 if checksum is 1000, otherwise -1 |
| `$0605` | Checksum, LONGINT |
| `$06FF` | Completion marker, `$A5` |
| `$07FF` | A, `Rows*Inner` elements |
| `$0DFF` | B, `Inner*Columns` elements |
| `$13FF` | C, `Rows*Columns` elements |

Inputs and output occupy disjoint buffers. Each starts at page offset `$FF`,
and surrounding bytes remain guards. Command one must leave A and B unchanged.
Every VM execution compares the entire `$0600..$1FFF` region, including every
matrix element, checksum, status, completion byte, command, and guard.

## Coverage and checks

| Rows x Inner x Columns | Purpose |
| --- | --- |
| 10x10x10 | Original workload: 1,000 multiply-accumulates |
| 3x7x5 | Distinct dimensions and asymmetric layout: 105 multiply-accumulates |
| 2x129x1 | 258 A elements, including indexes 255–257 and traversal across pages |

Each shape has 22 cases for LONGINT and INT, and 20 for BYTE and CARD: **252
C-reference cases**, **1,008 VM executions**, and 48 compiled programs.

Cases include original initialization, zero matrices, zero on either side,
asymmetric matrices, rectangular identity matrices on either side, boundary
values, a lone nonzero product at the final element, product/accumulation wrap,
signed values, minimum-value negation, and deterministic small/full-range random
inputs. The initial C buffer contains arbitrary values to verify that every
output element is reset and written.

C instrumentation reaches **14/14 outcomes of seven for-loop conditions** for
each type and shape, and checks their exact iteration counts on every run. This
is loop-condition coverage, not complete path coverage. LF and CRLF pass through
source instrumentation, vector parsing, and actual compilation/execution.
Standalone tests load no ROMs.

The port exposed a classic arithmetic staging bug: RHS address preparation or
calls could overwrite a materialized left operand. Compound left values could
also overwrite their own source pointer while being staged. The shared helper
path now stages into ordinary result homes and preserves the left value across
RHS evaluation. A focused compiler regression covers indirect byte/word
multiplication, division, remainder, and compound operands with RHS calls.

```sh
cd tools/vm-runtime-tests
cargo test --locked --test matrix1
```

## Regenerating vectors

```sh
python3 tools/generate_matrix1_vectors.py
python3 tools/generate_matrix1_vectors.py --check
```

Regeneration requires Python 3 and a GCC-compatible C compiler (`--cc clang` is
supported). It verifies the LF-normalized source hash and serializes individual
elements independently of host endianness. Normal Rust tests consume committed
vectors without a C compiler or network access.

Each row contains the type, shape, label, command, three input arrays, three
expected arrays, checksum, and status. The command is decimal; memory fields
encode complete little-endian contents as hex.
