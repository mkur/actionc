# TACLeBench Statemate

The separate [SHA-0 port](sha/README.md) exercises 32-bit arithmetic, rotations,
wide arrays, and block padding against a fixed-width C reference.
The [Dijkstra port](dijkstra/README.md) adds graph searches, self-referencing
record pointers, queue exhaustion, and complete node/queue state comparisons.
The [Huffman decoder port](huff_dec/README.md) adds variable-length bit streams,
record tables, and iterative tree construction/traversal against C state vectors.
The [insertion-sort port](insertsort/README.md) adds unsigned 32-bit array sorting,
data-dependent nested loops, and signed iteration statistics against C vectors.
The [binary-search port](binarysearch/README.md) adds iterative searches over
BYTE, INT, CARD, and LONGINT records, with shared source and typed C references.
The [matrix1 port](matrix1/README.md) adds multiply-accumulate loops, typed pointer
traversal, rectangular matrices, and explicit wrapping arithmetic at each width.
The [jfdctint port](jfdctint/README.md) adds an integer JPEG forward DCT, signed
rounding, long arithmetic sequences and complete row/column-pass comparisons.

`statemate.act` ports the experimental car-window controller from TACLeBench.
It retains all **16 nested switches as CASE statements**, with 34 explicit arms
and 16 defaults, the four interacting control routines, and the original
100-microstep driver. It exercises control flow, shared state, array flags,
signed measurements, and unsigned timer arithmetic without OS or floating-point
dependencies.

This is a host-driven compiler test fixture. It requires actionc's modern
profile (`--mode optimized` or `--mode mir6502`); CASE is not part of the original
Action! cartridge language. Both the cartridge runtime and standalone runtime
are tested. The entry point reads its command and initial state from RAM and
signals completion with `$A5` at `$06FF`.

## Provenance

- Author: Friedhelm Stappert, C-LAB, Paderborn, Germany.
- Generated originally by the STARC statechart code generator; collected by
  MRTC and subsequently TACLeBench.
- [Pinned upstream source](https://github.com/tacle/tacle-bench/blob/c6a0d73e47bbd2bc86e34637156fb26dd4d5cf08/bench/sequential/statemate/statemate.c),
  revision `c6a0d73e47bbd2bc86e34637156fb26dd4d5cf08`, source header version 2.0.
- Upstream permission: “may be used, modified, and re-distributed freely”.
- `statemate.c` is the unmodified reference, including its attribution and
  permission notice. Its LF-normalized SHA-256 is
  `e740e35e70bd4c91e5f6f970ae2cb874b78200dba2bfd19a3daddd9f47d50dca`.

## Action! adaptations

The port is maintained as Action! source. Shortened names retain the controller
and state names; `state.tsv` maps every original global to its Action! name,
type, and byte offset. Early C `break` paths become IF/ELSE continuations inside
the same CASE arm, preserving transition priority and nested dispatch.

C logical operands are explicitly compared with zero. This preserves C's
truth-value behavior even for input flags such as `$80` and `$FF`, since bare
Action! AND/OR also serve as bitwise operators. C `char`, `int`, and `unsigned
long` are given explicit unsigned 8-bit, signed 16-bit, and unsigned 32-bit
representations in both the port and the host reference. Test counters and
measurements avoid signed overflow; timestamp subtraction wraps at 32 bits.

The driver deliberately retains the upstream WCET modifications: `time=1` and
exactly 100 microsteps instead of waiting for convergence. The interface routine
is called by initialization, as upstream; individual test commands also exercise
its timer behavior. Additional tests seed internal substates because this fixed
clock cannot naturally reach every timer-dependent state. Those seeds are
compiler coverage inputs, not a claim that every combination is reachable from
normal window operation.

The original checksum shifts an `unsigned long` by indices up to 63, which is
not a valid portable checksum for a 32-bit `long`. Tests instead compare all
**201 state bytes**, including all 64 array flags and every scalar global.

## VM contract and coverage

| Location | Meaning |
| --- | --- |
| `$0600` | Command supplied by the host |
| `$06FF` | Completion marker, `$A5` |
| `$07A1..$0869` | Input/output state in `state.tsv` order, little-endian |

| Command | Operation |
| --- | --- |
| 0 | Original benchmark: Init, then Controller; supply zeroed state |
| 1 | Interface/timer update |
| 2 | Child-lock controller |
| 3 | Door controller |
| 4 | Anti-pinch controller |
| 5 | Motor-current/block detector |
| 6 | Controller again, retaining the supplied state |

`statemate-vectors.txt` contains **157 input/output vectors**: original startup,
eight successive controller calls with button/sensor changes, 28 timer cases
around expiry and 32-bit wrap, and 120 seeded dispatch cases. Coverage measured
in the C reference reaches **50/50 switch arms** and **157/164 IF outcomes**.
It does not claim complete path coverage.

The `statemate` VM target runs all vectors in both modern backends and both
runtimes: **628 executions**. It checks every state byte, the unchanged command,
completion, and every surrounding guard byte in `$0600..$08FF`. The unaligned
state block includes a LONGCARD crossing a page boundary. LF and CRLF vector
text pass through the same parser and VM path. Standalone executions load no ROMs.

```sh
cd tools/vm-runtime-tests
cargo test --locked --test statemate
```

The port exposed two classic-generator bugs, both fixed with the port:
indirect RHS address preparation clobbered A in bitwise operations, and
short-circuit joins retained a Y constant that was not valid on every incoming
path. The small `indirect_bitwise` VM target additionally checks byte/word
AND/OR/XOR, conditional OR, input preservation, and guards in all six
mode/runtime combinations (1,536 executions).

## Regenerating reference vectors

From the repository root:

```sh
python3 tools/generate_statemate_vectors.py
python3 tools/generate_statemate_vectors.py --check
```

Regeneration needs Python 3 and a GCC-compatible C compiler (`--cc clang` is
also supported). It uses the vendored C source without network access, fixes its
integer widths, excludes the original checksum/main wrapper, and instruments
switch arms and IF outcomes to select additional deterministic inputs. It
serializes the original C globals explicitly, independent of host structure
padding or endianness. It never translates or executes the Action! port to
produce expected results. Normal Rust/VM tests need no C compiler.
