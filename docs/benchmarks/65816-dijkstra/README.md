# Native 65816 Dijkstra: actionc versus vbcc

Measured on 2026-09-23, from actionc main `16c2d7fb` (compiler source unchanged
since `29bfda5f`). **Both compilers pass every reference vector.** On the original
20-search workload, optimized Action emits **4.20×** the code and takes **1.85×**
the cycles of optimized vbcc. Peak stack consumption is 86 versus 56 bytes below
entry, but stack traffic differs much more: **6.34×**.

This larger workload points to address generation as the next area to improve.
The present frame sizes are modest; more temporary-slot packing or an ABI change
would not directly address the largest observed cost.

## Workload and necessary adaptation

The [existing fixture](../../../fixtures/runtime/tacle/dijkstra/README.md) ports
TACLeBench/MiBench's FIFO graph relaxation algorithm: 100 nodes, a 100×100 byte
weight matrix, signed 16-bit distances/indexes and a linked 1,000-record queue.
It exercises records, 24-bit pointers, pointer output parameters, calls and
aliasing. It is not a priority-queue implementation. Both ports retain its
increment-before-limit queue allocation and other documented upstream behavior.

The unmodified Action driver does **not** compile on current native 65816:
raw emission rejects integer `Div`, and optimized emission rejects integer `Mod`.
For **both languages**, the comparison changes only these driver operations:

- Initialize `j` to 50 instead of `100 / 2`.
- Omit `j MOD 100`: `j` is always 50..69 over the original 20 iterations.

The search and queue routines remain unchanged. The C source is generated from
the pinned upstream C file, with pragmas removed and a host protocol matching
Action's existing driver. Graphs are supplied through RAM. Every original
reference vector, including the full original and exhausting benchmarks, remains
unchanged and is checked against the independent host C oracle.

The generator retains the generated parallel sources under
`target/dijkstra-65816/source`. This is a compiler comparison of the adapted
workload, not a claim that the stock driver now compiles.

## Whole-module and original-benchmark results

| Compiler | Mode | Code bytes | Cycles | Instructions | Peak stack below entry |
| --- | --- | ---: | ---: | ---: | ---: |
| actionc | raw | 6,787 | 2,038,730,702 | 546,155,502 | 96 |
| actionc | optimized | 6,197 | 1,908,568,205 | 524,911,956 | 86 |
| vbcc | raw | 1,697 | 1,198,440,309 | 302,475,886 | 44 |
| vbcc | optimized | 1,477 | 1,029,379,326 | 257,272,716 | 56 |

Code includes all emitted routines, host dispatch/layout queries, Action stack
checks and vbcc's linked 24-byte multiplication helper. No code is stripped to
favor either compiler. Excluding Init/Main/QueryLayout leaves 4,375/3,959 bytes
for raw/optimized Action and 1,118/1,007 bytes for raw/optimized vbcc, including
that helper. Thus the size gap is also present in the actual search/queue code.

Action reserves 20,482 global bytes and vbcc 19,482. The difference is record
layout: Action queue records occupy 10 bytes, vbcc records 9. Both pointers are
24-bit. The harness queries each compiled layout and translates reference links;
it does not force matching layouts or change either ABI.

Peak stack excludes the incoming three-byte return address and includes nested
frames, outgoing arguments, return addresses and saved registers. Action's
guards remain enabled. Its 22 guard sequences occupy 990 static bytes and consume
2,877,984 cycles, **0.151%** of optimized benchmark execution. Subtracting just
that classified check cost would leave Action about 1.85× slower. This is not an
experiment with guard removal or a complete estimate of all ABI overhead.

### Dynamic traffic on the original benchmark

| Compiler | Mode | Stack byte reads+writes | Scratch DP byte reads+writes | REP/SEP instructions |
| --- | --- | ---: | ---: | ---: |
| actionc | raw | 243,671,581 | 351,381,506 | 53,050,091 |
| actionc | optimized | 178,702,872 | 351,321,586 | 56,585,113 |
| vbcc | raw | 61,515,570 | 332,236,427 | 7,530,466 |
| vbcc | optimized | 28,192,593 | 323,100,937 | 3,835,080 |

These are bus-byte accesses, including call/return and saved-register traffic,
not counts of source variables. Action DP accounting covers its 64-byte scratch;
vbcc covers its 80-byte virtual-register/temporary region. Action's ABI metadata
reads are recorded separately. Similar aggregate DP traffic does not imply
similar allocation: vbcc also spends heavily in its multiplication helper.

Action optimization reduces total cycles by 6.38% and stack traffic by 26.66%,
but does not reduce the dominant DP address work. Mode switches actually rise;
there are 14.75× as many as in optimized vbcc. The current state tracker and
native word operations help, but substantial address construction remains in
byte mode.

## Where the time goes

Exclusive routine cycles on the original benchmark, optimized builds:

| Routine | actionc | vbcc |
| --- | ---: | ---: |
| Find | 1,497,140,022 | 293,553,330 |
| Enqueue | 395,198,515 | 112,081,026 |
| Dequeue | 8,820,275 | 3,773,700 |
| Init | 6,439,023 | 555,081 |
| QueueLength | 959,680 | 179,940 |
| multiplication helper | 0 | 619,233,926 |

The remaining dispatch/checksum/benchmark-driver routines contribute 10,690
Action cycles and 2,323 vbcc cycles. QueryLayout executes only during setup and
is excluded from benchmark execution.

**Find consumes 78.44% of Action cycles.** Final machine code repeatedly
materializes a 24-bit base through DP and stack homes, scales an index with
three-byte shift/add sequences, and reloads addresses/values. For example,
`$010DB0..$010DD3` in the [optimized Action listing](actionc.optimized.lst)
scales the node index using byte ASL/ROL and ADC operations. These instructions
execute 1,497,500 times in the original benchmark. Another equivalent address
is constructed immediately afterward for the next node-field test. Word loads
and comparisons are already present; this is not simply missing native `CMP`.

The [optimized vbcc listing](vbcc.optimized.lst) instead retains several pointers
in DP register pairs, uses word loads/stores and advances node pointers by four
bytes in the inner loop. Its initialization loop also uses running pointers.
The smaller stack traffic comes with explicit callee-preserved DP saves; its
optimized peak stack is larger than its own raw peak, while execution is faster.

**Vbcc is not an ideal target:** its standard multiplication helper consumes
60.16% of total optimized cycles. It recalculates the matrix row offset inside
the neighbor loop, calling `___mulint16` for multiplication by 100. The helper
is included in all code/cycle totals. Comparing Find alone without attributing
its helper calls would exaggerate the speed advantage. Both compilers leave
opportunities in address calculation and loop-invariant work.

These are explanations from the final listings and dynamic profiles, not
measurements of hypothetical replacements. The measurements do not isolate a
register-passing ABI benefit or establish that a global register allocator is
needed next.

### Other representative vectors

Optimized cycles, including each vector's host dispatch and initialization:

| Case | actionc | vbcc |
| --- | ---: | ---: |
| original-0-50 | 119,784,282 | 62,446,761 |
| benchmark-exhaust | 110,610,223 | 31,730,488 |
| zero | 17,653,133 | 6,989,460 |
| enqueue-chain-3-256 | 2,050 | 999 |
| dequeue-31-32-255 | 1,300 | 440 |

Use [all 132 measurements](results.csv) rather than treating one speed ratio as
universal. [Routine profiles](routine-profile.csv) and
[instruction frequencies](instruction-profile.json) retain both the original
benchmark and one original-graph search in all four compiler/mode combinations.

## Recommended next slice

Improve **native-width scaled-index address formation** in the existing
[`prepare_address`](../../../src/mir65816/emit/select.rs) path. Replace the
bytewise implementation with a canonical 16-bit low-word plus 8-bit bank
calculation where it is safe, keeping the full 24-bit result and a conservative
fallback. This should be an improvement to ordinary lowering, using the existing
state/effect tracking, rather than another late peephole layer.

Measure the replacement before extending it. Cover representative strides
1/2/4/9/10/100, index boundaries, bank carries, supported signed-index semantics,
volatile/alias-sensitive accesses, helper/call clobbers and interrupt restoration.
Keep the public ABI, home allocation and stack guards unchanged. Require raw and
optimized final-byte validation, the existing small corpus, and these complete
Dijkstra vectors. Do not narrow arithmetic merely because this benchmark's
indices happen to fit.

Only then consider address reuse and loop induction/row-base hoisting. Hoisting
across calls needs actual effect/alias proofs: `node` is passed by address to
Dequeue, and Enqueue runs inside the neighbor loop. The source-level appearance
of an invariant is not sufficient evidence for a general compiler rewrite.

## Validation and reproduction

The release VM run passes **264 full-state executions**: 33 vectors × two
compilers × two compiler modes × I clear/set. It verifies every scalar global,
all 100 node records and all 1,000 queue entries, including native pointers,
unchanged graphs, protected record padding, stack canaries and ABI restoration.
The tests include negative scalar payloads, queue exhaustion, index/low-byte
boundaries, ties and random graphs. A separate QueryLayout execution discovers
native offsets and widths from each artifact.

Focused debug runs cover 88 enqueue executions and 32 dequeue executions; the
latter parse actual CRLF graphs/vectors. Their counters match the corresponding
release records; [focused-check provenance](focused-checks.json) retains the hashes. All four compiler builds also consume actual CRLF Action/C
sources and includes and produce identical image/binary bytes. The independent
vector generator's `--check` passes, along with three Python adaptation/hash
controls and the Rust packet/pointer controls. Only the affected native 65816
target was run; no 6502/68k suites or shared compiler checks were needed.

No IRQ/NMI is injected by this measurement harness; existing preemption tests
remain the authority for that behavior. Cycles are from the pinned qualified
CPU model with no device wait states, not physical-machine wall time. The
benchmark checks high-bank pointers but does not independently qualify every
24-bit address boundary. Compiler implementation and public ABI are unchanged.

```sh
CARGO_INCREMENTAL=0 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 \
  cargo build --release --bin actionc-65816
python3 -B tools/generate_dijkstra_vectors.py --check
python3 -B -m unittest discover -s tools/compare65816 -p test_dijkstra.py
python3 -B tools/compare65816/dijkstra.py --verify-crlf
python3 -B tools/compare65816/run_dijkstra.py target/dijkstra-65816/manifest.json
python3 -B tools/compare65816/run_dijkstra.py target/dijkstra-65816/manifest.json \
  --debug --filter enqueue
python3 -B tools/compare65816/run_dijkstra.py target/dijkstra-65816/manifest.json \
  --debug --filter dequeue --crlf-vectors
python3 -B tools/compare65816/report_dijkstra.py \
  target/dijkstra-65816/manifest.json target/dijkstra-65816/release-results.json \
  --qualification target/dijkstra-65816/release-qualification.json
```

Tools default to `~/atari/vbcc/bin`; override with `--vbcc-bin`. Flags match the
small corpus: vbcc `-O=0`/`-O=1023`, `-mhuge -ptr24 -near-threshold=0
-no-near-const`; vasm `-816 -opt-branch`; vlink `-brawbin1`. Action uses
`--no-opt` or the CLI's normal optimization setting. Each compiler retains
ordinary lowering that is present even in its raw mode.

The builder downloads the author's
[vbcc65816 release-2 package](http://www.ibaug.de/vbcc/vbcc65816_r2.zip) to ignored
build storage, or accepts `--runtime-archive`. It verifies SHA-256
`e8950454d55327e50f6a5f98dc22ac020ee3d6084a17c92d37e088d3c63a922c`
and extracts only the simulator target's `libvch.a`; it does not install over
local tools. Vbcc uses its own ABI and standard far-call helper, with symbols
linked into the same code/data address regions as Action.

[Provenance](provenance.json) retains exact tool/input/artifact hashes, commands,
layouts and CPU qualification (VM base `56ddc5c5`, timing patch
`afd5efebc9f0d153…`). The recorded release run used `qualify.py` directly; the
provided wrapper additionally binds future runs to the external artifact hashes.
The vasm/vlink listings contain relocated machine bytes, with library bytes
explicitly marked as a raw byte dump rather than invented assembler source.
The VM executes those helper bytes as part of correctness and timing checks.
No vendor runtime binaries are committed.
