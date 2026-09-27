# Native 65816 CRC speed comparison

Measured on 2026-09-26 with actionc `7936f5d6` and Calypsi 5.18. Calypsi produces
faster, smaller kernels for all three CRCs. Its `-O2 --speed` CRC8 output is
incorrect on several inputs, so the valid CRC8 comparison uses `-O1 --speed`.

The kernels come from `c-bench-64/benchmarks/src`. Equivalent Action! ports keep
the same bit-at-a-time algorithm, unsigned widths, pointer traversal, loop
bounds, polynomials and initial/final XOR values. No lookup tables, hand assembly
or algorithm substitutions were introduced.

## Results

Cycles cover one call on the same deterministic 8,192-byte buffer. Lower is
better. The ratio is Action cycles divided by Calypsi cycles.

| Kernel | Action cycles | Calypsi cycles | Calypsi faster | Action bytes, excluding guard | Calypsi bytes |
| --- | ---: | ---: | ---: | ---: | ---: |
| CRC8 | 13,671,811 | 6,479,751 (`-O1`) | 2.11x | 349 | 151 |
| CRC16 | 12,868,237 | 5,045,999 (`-O2`) | 2.55x | 297 | 113 |
| CRC32 | 24,090,260 | 5,092,163 (`-O2`) | 4.73x | 557 | 158 |

Action cycles include its entry guard: **25 cycles per call**, negligible for
this workload. Each guard occupies **27 bytes**, excluded from the table's code
size. Full measured sizes are 376, 324 and 584 bytes. Guard costs are identified
from emitted instructions; this was not a guard-disabled build. Sizes count
the callable CRC routine, excluding the unused Action `Main`, caller and startup.

All Action kernels and the Calypsi configurations in this table returned the
expected results on all 16 test vectors. Debug and release builds of the VM
harness produced identical measurements.

| Input | CRC8/GSM-A | CRC16/XMODEM | CRC32/CKSUM |
| --- | ---: | ---: | ---: |
| `123456789` | `$37` | `$31C3` | `$765E7680` |
| 8,192-byte benchmark | `$BA` | `$E173` | `$6399681C` |

## Calypsi CRC8 correctness failure

`-O2 --speed` produces a 115-byte CRC8 routine, but it fails **10 of 16 vectors**.
For example, `123456789` returns `$E8` instead of `$37`. The 8 KiB benchmark
happens to return the correct value; its 4,317,262-cycle timing is therefore
retained as diagnostic data and excluded from the valid performance comparison.

In the [O2 listing](crc8-calypsi-O2-speed.lst), the CRC byte is at `3,S`:

```asm
lda 2,s          ; A16: CRC occupies the high byte, for the sign test
bpl ?L12
asl a            ; shifts that word, rather than loading CRC at 3,S
eor ##29
sep #32
sta 3,s
```

The [O1 listing](crc8-calypsi-O1-speed.lst) reloads `3,S` after the branch,
before shifting. That distinction explains the wrong result on the true arm;
O1 passes every vector. Both builds pass ABI, protected-memory and input-read
checks. The failure is in the returned CRC, not invocation of the C routine.

The default runner rejects incorrect results. For this diagnostic comparison,
`--allow-external-result-errors` retains foreign-compiler CRC mismatches while
still rejecting any Action result error or any compiler's ABI/access violation.
Report generation requires exactly the ten observed O2 CRC8 failures and
retains them in [failures.json](failures.json). A completed diagnostic run does
not mean every compiler output was correct.

## Where Action spends the time

The dynamic instruction profiles identify stack traffic as the main cost.
These columns count the full cycles of stack-addressed instructions and
`REP`/`SEP` respectively; they are disjoint instruction groups, not bus-byte
counts or independent forecasts of removable work.

| Kernel | Stack-addressed instruction cycles | Share | REP/SEP cycles | Share |
| --- | ---: | ---: | ---: | ---: |
| CRC8 | 7,388,948 | 54.05% | 2,924,442 | 21.39% |
| CRC16 | 8,043,568 | 62.51% | 1,695,543 | 13.18% |
| CRC32 | 17,256,198 | 71.63% | 2,138,679 | 8.88% |

The stack-addressed group is opcodes `A3/83/63/E3/C3/23/03/43`.
The [profiles](profiles.json) retain execution counts and cycles by opcode and
instruction address, allowing these totals to be reproduced.

**CRC32 gives the clearest first optimization slice.** Its
[Action listing](crc32-actionc-optimized.lst) contains the following copies
inside the eight-iteration bit loop:

| Copy path | Stack slots | Measured cycles | Share of whole call |
| --- | --- | ---: | ---: |
| True arm, `$010131..$01014F` | `$06..$09` to `$14..$17` and back | 2,108,992 | 8.75% |
| False arm, `$01016B..$010189` | `$0A..$0D` to `$14..$17` to `$06..$09` | 2,085,312 | 8.66% |
| Backedge, `$01019E..$0101AC` and `$0101B2..$0101C0` | `$06..$09` to `$14..$17` and back | 4,194,304 | 17.41% |

Each row's executed sequence moves four bytes through a staging area using
16 byte loads/stores, costing 64 cycles per occurrence. The true and false arms
are alternatives; the backedge executes on every bit iteration. Together these
copies cost **8,388,608 cycles (34.82%)**, excluding mode changes and jumps.

Only the true-arm and backedge CRC copies return the value to its existing
home: **6,303,296 cycles (26.17%)**. The false arm transfers a new result and
must retain that transfer. Eliminating proven identity copies is therefore a
substantial, bounded opportunity; it is not valid to forecast removal of the
entire 34.82%. The compiler must check actual storage overlap and the complete
parallel-copy group before removing any transfer. No optimization or resulting
speedup has been implemented or measured in this snapshot.

Calypsi keeps CRC16 in Y and shifts in A (`TYA; ASL A; EOR #$1021` on the true
arm), whereas Action repeatedly captures and reloads intermediate values.
Calypsi keeps CRC32 in two direct-page words and shifts with `ASL`/`ROL` in
place. Action already uses native word operations for CRC32, but surrounds
them with stack-to-scratch and scratch-to-stack transfers.

Recommended compiler slices, in order:

1. **Remove proven identity edge copies.** Start with private stack homes and
   explicit overlap checks. Then use native word transfers within mixed-width
   copy groups where the transfer remains necessary.
2. **Fuse sign-mask tests into branches.** CRC32's `(crc AND $80000000)#0`
   currently materializes both words of the AND result, including an always-zero
   low word, and tests the result. The sequence at `$0100F3..$010109` costs
   2,752,142 cycles (11.42%). Loading the high word and branching on its sign
   avoids most of this work. CRC8 and CRC16 have corresponding top-bit tests.
3. **Reduce shift and result staging.** Keep a 16-bit shift result in A through
   a following XOR and store; support direct immediate BYTE operations; reduce
   redundant mode changes. Longer-term register/DP residency can reduce loop
   traffic further, but needs broader lifetime and clobber analysis.

These are general generated-code patterns, not CRC-specific rewrites. Their
whole-image effect on Exec has not been measured; a hot-loop speed benefit
cannot be extrapolated directly to Exec's size or overall runtime.

## Method and reproduction

- Calypsi: `-O2 --speed --data-model huge --code-model large`, plus the CRC8
  `-O1 --speed` control. Action uses its default optimized pipeline; its CLI
  currently has no separate speed/space switch.
- Code starts at `$010000`, direct page at `$2000`, entry stack pointer at
  `$5FE0`, native A16/X16. Each compiler uses its normal argument/result ABI.
  Action receives a three-byte pointer and CARD length on the stack; Calypsi
  receives a huge pointer in `_Dp[0..3]` and length in A16. The huge pointer
  occupies four bytes while addressing the 24-bit address space.
- Timing runs from function entry through `RTL`, including function setup and
  teardown, excluding caller argument packing and call/startup instructions.
  Counts come from the pinned native VM with its timing patch, not physical
  hardware. No wait states, DMA or interrupts are injected.
- Eight payloads: empty, zero, `$80`, `$FF`, `123456789`, ascending 256 bytes,
  deterministic random 257 bytes, and deterministic random 8,192 bytes. The
  random stream is xorshift32 (13/17/5), seed `$08162026`, low byte per step.
  This is not the original benchmark's machine ROM contents.
- Each payload runs at `$12E000` and `$12FFF9`; longer inputs in the latter
  placement cross a bank. An independent byte-table oracle checks results.
  Read-only input and unmapped neighbors detect writes or out-of-range reads;
  every input byte must be read exactly once. ABI restoration, preserved DP
  state and stack canaries are checked.
- Each case runs with I clear and set. Both masks must give identical records.
  Seven artifacts times sixteen vectors gives 112 paired records, or 224
  executions per host build. Both host builds ran: **448 executions** total.
- Actual LF and CRLF versions of every C/Action fixture were compiled and
  required to produce identical images. Only the focused CRC runtime target
  ran; no full backend or Exec qualification was run.

The source suite was at `58036235788d9c9741c67f82e1fb9bb6e2f0e3f6`, with an
existing local modification to `crc16.c`. The frozen
[C and Action fixtures](../../../tools/native65816-runtime-tests/tests/fixtures/crc_bench)
and [provenance](provenance.json) record the exact tested inputs, including that
local file's hash. C extraction removes benchmark/UI wrappers, exports CRC8,
and defines `__data` as empty; the selected algorithm bodies are retained.

From the repository root, using the recorded suite contents and Calypsi 5.18
tools installed under `/usr/local/bin`:

```sh
CARGO_INCREMENTAL=0 CARGO_PROFILE_RELEASE_DEBUG=0 \
  cargo build --release --bin actionc-65816
python3 -B tools/compare65816/crc.py --suite /Users/michalkurcewicz/atari/c-bench-64
python3 -B tools/compare65816/run_crc.py target/crc65816-comparison/manifest.json \
  --allow-external-result-errors
python3 -B tools/compare65816/run_crc.py target/crc65816-comparison/manifest.json \
  --debug --allow-external-result-errors
python3 -B tools/compare65816/report_crc.py
```

The build checks extraction against the frozen C fixtures and records compiler,
source and artifact hashes. The runner authenticates inputs before and after
execution; the report requires matching debug/release results and qualification
evidence. Rerunning with a newer compiler records a new comparison, not a
reproduction of this historical result.

Raw evidence: [summary](summary.csv), [all measurements](measurements.csv),
[profiles](profiles.json), [failures](failures.json), [provenance](provenance.json).
Final linked listings: [Action CRC8](crc8-actionc-optimized.lst),
[CRC16](crc16-actionc-optimized.lst), [CRC32](crc32-actionc-optimized.lst);
Calypsi [CRC8 O1](crc8-calypsi-O1-speed.lst), [CRC8 O2](crc8-calypsi-O2-speed.lst),
[CRC16 O2](crc16-calypsi-O2-speed.lst), [CRC32 O2](crc32-calypsi-O2-speed.lst).
