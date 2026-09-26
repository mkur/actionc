# Native 65816 CRC/sieve optimization series

Implementation of the [eight-slice plan](../../MIR65816_CRC_SIEVE_OPTIMIZATION_PLAN.md),
starting at `279a791f`. Original CRC and sieve reports remain immutable. Each
`slice-N.json` records source/binary hashes, matching benchmark measurements,
and the compile-only frozen Exec delta against the preceding slice.

| Slice | Change | CRC8 cycles | CRC16 cycles | CRC32 cycles | Exec bytes saved in slice |
| --- | --- | ---: | ---: | ---: | ---: |
| Baseline | Original code | 13,671,811 | 12,868,237 | 24,090,260 | — |
| 1 | Mixed-edge identities | 13,147,043 | 12,868,237 | 17,394,488 | 971 |
| 2 | Native staged words | 13,147,043 | 12,868,237 | 17,003,492 | 128 |
| 3 | Short word shifts in A | 13,147,043 | 12,016,269 | 17,003,492 | 46 |

CRC cycles use the same 8,192-byte input, with stack guards included. Slice 1
kernel bytes are 362 / 324 / 490 versus 376 / 324 / 584. All Action results,
ABI checks and access checks pass in debug and release hosts; foreign machine
results are unchanged. Calypsi CRC8 O2's ten pre-existing result errors remain
diagnostic controls, excluded from valid rankings. Debug and release profiles
are identical, including normal and cross-bank input placements.

Slice 1 validates complete mixed-edge homes and staging before emission. It
omits only isolated exact stack identities, retains every other source capture
before destination writes, and repairs the final A.low/N/Z when needed. Hidden B,
X/Y and the other status bits retain the original byte-copy behavior. Frames and
staging reservations are unchanged. Focused selector tests: 147 passed, one
existing ignored case. Native `mixed_edges`, `word_edges`, `acyclic_edges`,
`pointer_edges`, and `edge_coalescing`: 16 passed in each host profile, including
LF/CRLF, relocation and existing pointer-edge IRQ/NMI reentry checks.

Frozen Exec now contains 315,819 compiler-code bytes. Its guard-subtracted loaded
estimate is 254,174 bytes (including the unchanged 8,300 assembly and 2,307
initialized-data bytes). Frames, ABI metadata, data and all 72,252 guard-region
bytes are unchanged. This subtraction is an accounting estimate: guard regions
also contain required frame arithmetic. No guard-disabled image was built and
no full/final backend or hosted Exec qualification was run.

Reproduce a slice by rebuilding `actionc-65816 --release`, building the affected
kernels with `tools/compare65816/{crc,sieve}.py --output
target/crc-sieve-series/slice-N-{crc,sieve}`, and executing their `run_*.py`
wrappers in both host profiles. CRC controls require
`--allow-external-result-errors`. Then run
`python3 -B tools/compare65816/crc_sieve_series.py N --families crc` (or `sieve`,
or both). Frozen Exec inputs and the original compiler binary must be present
in the local ignored audit caches; hashes are checked before use.

Slice 2 saves another 390,996 CRC32 cycles and 10 kernel bytes. Its cost gate
includes all mode transitions and full-A save/restore through existing RESULT
scratch. Only complete stack groups of widths 1/2/4 qualify; other mixed groups
retain byte transfers. Frame and staging reservations stay fixed. Four focused
planner tests and 13 native edge tests pass; native tests and CRC measurements
pass in both host profiles, including new mixed-edge task-switch/IRQ/NMI reentry
coverage. Frozen Exec saves 128 more bytes, reaching a guard-subtracted
loaded estimate of 254,046 bytes.

Slice 3 saves 851,968 CRC16 cycles and 10 bytes. Bit sieve falls from 9,289,832
to 8,732,887 cycles at 8,191 slots and from 18,623,518 to 17,505,373 at 16,000;
its five-routine code shrinks from 634 to 613 bytes. Standard sieve is unchanged.
The replacement handles complete two-byte homes and counts 1–7; right shifts
remain logical for INT bit patterns. The typed LSR-A form includes effects,
tracking and independent decoding support. Validation: 287 emitter tests passed
(one existing ignored); 18 focused native shift/state/effect tests passed in each
host, including independent assembly and instruction-boundary IRQ/NMI reentry.
Both full benchmark vector sets pass with unchanged foreign controls. Frozen
Exec saves 46 bytes in this slice, for a loaded estimate of 254,000.

| Slice | Standard sieve, 8,191 slots | Bit sieve, 8,191 slots | Bit sieve, 16,000 slots | Standard / bit code bytes | Exec bytes saved in slice |
| --- | ---: | ---: | ---: | --- | ---: |
| Baseline | 3,400,699 | 9,289,832 | 18,623,518 | 264 / 634 | — |
| 3 | 3,400,699 | 8,732,887 | 17,505,373 | 264 / 613 | 46 |
| 4 | 3,220,497 | 7,802,521 | 15,638,079 | 250 / 561 | 0 |

Slice 4 introduces relocated `LDA long,X` only for static BYTE bases and
captured CARD indexes. Its dynamic indexed effects and X use are explicit;
volatile, displaced, scaled and loop-X cases retain existing selection. Tests
cover independent ca65 bytes, replay, exact read traces, indexes through 65,535,
bank carry, LF/CRLF, two o65 rebases and instruction-boundary IRQ/NMI reentry.
288 emitter tests passed (one ignored), followed by five focused preflight
checks; seven existing indexed-access tests passed, and seven load/consumer/X
runtime tests passed in both hosts. Nine disassembler tests passed. The full
sieve vectors agree in both hosts. The new test harness needed trace opt-in and
initialization of already-mapped data; neither correction changed emitted code.
Frozen Exec has no eligible size changes in this slice.
