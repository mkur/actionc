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
