# TACLeBench jfdctint

`jfdctint.act` ports the integer JPEG forward DCT from TACLeBench. It transforms
an 8x8 block in place: eight row transforms followed by eight column transforms.
It uses LONGINT arithmetic, scaled integer coefficients, flat arrays and loops,
with no recursion, floating point, OS calls, or new language constructs.

This software is based in part on the work of the Independent JPEG Group.

## Provenance

- [Pinned upstream directory](https://github.com/tacle/tacle-bench/tree/c6a0d73e47bbd2bc86e34637156fb26dd4d5cf08/bench/kernel/jfdctint),
  revision `c6a0d73e47bbd2bc86e34637156fb26dd4d5cf08`, source header version 1.x.
- Original author: Thomas G. Lane, Independent JPEG Group, copyright 1991-1994;
  modified by Steven Li and collected by the SNU-RT benchmark suite.
- `jfdctint.c` is the unmodified TACLeBench reference. Its accompanying
  [README](README) is also retained unmodified, including the IJG copyright,
  permission and warranty notices. These terms also apply to derived code.
- LF-normalized SHA-256 of `jfdctint.c`:
  `916186c1fcac8f8bec8e4d523c84c055f2693eb87d13e4ea789c34e5911c1468`.
- LF-normalized SHA-256 of the upstream `README`:
  `fc0a9acfdabcddad7e5a9b156da2df8f0dd0295658332faa047d63b5c771f36e`.

## Action! adaptations

The Action! adaptation was added in 2026. It retains the original operation
order, constants, initialization sequence and checksum predicate. C's 32-bit
`int` values become LONGINT. The C pointer becomes an unsized LONGINT ARRAY
view, advanced explicitly in bytes: 32 per row and 4 per column. The loop counter
counts up through eight iterations; the arithmetic does not use its value.
The initialization modulus is explicitly CARD(65535), preserving positive
65535 when widened into the LONGINT calculation. Bare decimal 65535 is INT -1
in Action!.

`Descale(value,bits)` adds the rounding bias and performs a sign-preserving
right shift. Action!'s `RSH` is logical. For a negative rounded value, the helper
shifts its nonnegative complement and complements the result again. This also
handles LONGINT's minimum value without negating it. The used counts are 2, 11,
and 15; the bias addition wraps at 32 bits.

The final coefficients retain the original factor of eight. A constant input
block of value `v` in JPEG range produces DC coefficient `64*v` and 63 zero AC
coefficients. Quantization and complete JPEG encoding are outside this kernel.

`Main` initializes the original input, runs the transform and records the
checksum and status in ordinary globals. The original checksum is **1668124**;
status is zero for that checksum and -1 otherwise. Tests validate every
coefficient independently of this predicate.

## Target separation

The maintained source has no absolute addresses, host commands or completion
mailboxes. All benchmark globals, arrays and local views use compiler-allocated
storage. The root test verifies raw and optimized NIR and lowers both to
MIR6502, MIR68k and both MIR65816 layouts. This is a lowering check; executable
behavior is currently tested only with the 6502 VM.

The Rust VM adapter adds a driver that copies test input into the ordinary
`block` array, snapshots the initialized block and row-pass result, and copies
the final coefficients to host buffers. It supplies the origin, little-endian
serialization and the following 6502-only mailboxes:

| Address | Meaning |
| --- | --- |
| `$0600` | Command: 0 original initialization, 1 supplied block, 2 Descale only |
| `$0601` | Shift count for command 2 |
| `$0602` | INT checksum status |
| `$0604` | LONGINT checksum |
| `$06FF` | Completion marker `$A5` |
| `$07FF` | 64 LONGINT input values, preserved |
| `$09FF` | Initialized block |
| `$0BFF` | Row-pass snapshot; untouched for command 2 |
| `$0DFF` | Final output |

The object starts at `$3000`. Buffers cross page boundaries deliberately;
every byte of `$0600..$0FFF`, including guards and unchanged input, is compared.
These addresses belong to the adapter, not the benchmark source.

## Reference arithmetic and coverage

The generator compiles the pinned C operation sequence with unsigned 32-bit
arithmetic and an explicit arithmetic-shift helper. This defines wrapping for
the original large inputs and extended full-range vectors, avoiding signed C
overflow and signed-shift assumptions. The descending loop counter remains
signed. Coefficients are interpreted as signed 32-bit values at the boundary.

A second C build retains signed 32-bit arithmetic for 155 JPEG-range cases.
Its negative left shifts become multiplication and its rescaling uses signed
64-bit floor division. Those inputs keep intermediate arithmetic in range;
all initialized values, row results, final coefficients and checksums must
agree with the wrapping build. Python separately checks the initialization,
original checksum, constant-block identity, final checksum and rounding edges.

There are **181 C-reference cases and 724 VM executions**, across optimized
classic and MIR6502 with cartridge and standalone runtimes:

- Original initialization and zero/constant blocks.
- Positive and negative impulses at each of the 64 positions.
- Checkerboard, stripes, ramps and asymmetric inputs.
- Integer boundaries and deterministic JPEG-range/full-range random blocks.
- 192 direct Descale inputs across counts 2, 11 and 15, including rounding
  ties, LONGINT minimum/maximum and bias overflow.

The C instrumentation reaches 8/8 outcomes of four loop conditions and 6/6
sign outcomes of the three descaling counts. It asserts the exact loop counts
for each command. This is condition coverage, not complete path coverage.
LF and CRLF pass through source instrumentation, vector parsing, compilation
and execution. Standalone runs load no ROMs.

```sh
cargo test --test jfdctint
cd tools/vm-runtime-tests
cargo test --locked --test jfdctint
```

## Regenerating vectors

```sh
python3 tools/generate_jfdctint_vectors.py
python3 tools/generate_jfdctint_vectors.py --check
```

Regeneration requires Python 3 and a GCC-compatible C compiler (`--cc clang`
is supported). Tests consume committed vectors without a C compiler or network.
The generator verifies both upstream file hashes after newline normalization
and serializes values independently of host endianness. Each vector contains
the label, command, shift, input, initialized block, row-pass snapshot, output,
checksum and status. Numeric buffers use little-endian hex for the 6502 adapter.
