# Native 16-bit ADD/SUB: implementation and qualification

Completed on 2026-09-21 against actionc main. Native word selection reduces both
ADD and SUB from **116 bytes / 162 VM cycles to 98 bytes / 135 cycles**, including
the unchanged entry guard and return. This meets the approved 100-byte/140-cycle
ceilings. The [implementation plan](MIR65816_WORD_ARITHMETIC_PLAN.md) is complete:
`7907b77` captures the baseline and semantic regressions; `7803040` implements
selection; the commit containing this report records final qualification.

## Implemented boundary

For two-byte ADD/SUB with a two-byte stack destination, the emitter accepts
U16 immediates, zero-extended U8 immediates, and exact-width stack temporaries
or parameters. It checks all word extents, including current stack displacement,
before emitting `LDA; CLC/SEC; ADC/SBC; STA` in A16. Immediate-left subtraction
preserves operand order. Signed and unsigned words retain modulo-65536 behavior;
signed widening still uses explicit casts. Other legal operand forms retain the
bytewise path, and malformed homes remain errors.

The result is written to its allocated home within the operation. Selection
uses no DP scratch, helper, push, or value surviving into the next operation.
Existing mode management handles calls, labels, edges, and returns. Existing
loads still own all volatile/absolute/pointer access widths and ordering;
arithmetic operates on their captured private values. See the
[emission contract](MIR65816_EMISSION_CONTRACT.md) for the precise invariants.

Physical ABI v1, image v3, the experimental o65 profile, temporary allocation,
stack guards, and Exec816's compiler pin are unchanged. This slice changes
instruction selection only; stack traffic and frame pressure remain unchanged.

## Measured effects

The 14-kernel paired corpus contains 66 independent input vectors. Each snapshot
has 264 records, each covering both incoming interrupt-mask states: **528 machine
executions per host build**. Raw and optimized target output is measured in both
debug and release VM builds. All 112 LF/CRLF compilations in each snapshot
produce equivalent binaries. Inputs arrive after compilation.

Representative optimized Action results, before → after:

| Kernel | Code bytes | VM cycles | DP byte reads + writes | Observed stack bytes |
| --- | ---: | ---: | ---: | ---: |
| add(13,41) | 116 → 98 | 162 → 135 | 14 → 10 | 8 → 8 |
| subtract(13,41) | 116 → 98 | 162 → 135 | 14 → 10 | 8 → 8 |
| constant chain | 112 → 95 | 148 → 123 | 14 → 10 | 6 → 6 |
| sum loop(13) | 311 → 276 | 3,542 → 2,866 | 170 → 66 | 16 → 16 |
| loop rotation | 401 → 338 | 2,624 → 2,223 | 118 → 46 | 26 → 26 |
| recursive sum(13) | 368 → 333 | 5,363 → 4,687 | 300 → 196 | 190 → 190 |
| byte sum(16) | 405 → 374 | 6,747 → 6,011 | 414 → 286 | 22 → 22 |
| forward copy(8) | 410 → 393 | 4,023 → 3,823 | 276 → 244 | 18 → 18 |

ADD/SUB save 18 bytes and 27 cycles each; the optimized sum loop saves 676
cycles (19.1%). Raw constant-chain code improves from 427 to 215 bytes and
658 to 348 cycles. Identity, maximum, wide shifts, record fields, and unlink
are unchanged. Guard and return costs still account for substantial fixed
overhead; the remaining ten DP accesses in add/subtract come from existing
result marshalling, outside this arithmetic sequence.

The [full delta table](benchmarks/65816-word-arithmetic/delta.md) includes raw
output and all kernels. Its [machine-readable checks](benchmarks/65816-word-arithmetic/delta.json)
confirm no Action code-size or cycle regression on any vector and identical
complete routine storage contracts, observed stack depths, stack byte traffic,
argument layouts, and guard costs. See the
[baseline](benchmarks/65816-word-arithmetic/before/tables.md) and
[post-change](benchmarks/65816-word-arithmetic/after/tables.md) snapshots for CSV
measurements, provenance, and disassembled final bytes. The
[comparison runner](../tools/compare65816/README.md) documents reproduction.

All 264 Action executions per host build pass. The external comparison still
exits with a failure for the known optimized vbcc unlink corruption in both
interrupt-mask states; no new C failure occurs, and all vbcc measurements are
identical to baseline. Incorrect C output remains marked invalid in the report
and is excluded from performance conclusions.

## Qualification

The [qualification record](abi/action65816-word-arithmetic-qualification.json)
binds the following results to source, VM, fixture, tool, and artifact hashes:

- 14 focused compiler unit tests and 56 native ABI/emission/o65/CLI integration
  tests pass. Three disassembler decoding/truncation tests pass.
- All 48 ordinary native tests pass in debug and release, covering raw and
  optimized serialized machine code. The external comparison is ignored by
  default and was run separately in both builds as described above.
- Four new arithmetic tests cover 420 semantic executions and 16 independent
  ca65 encoding/execution probes per host build: signed/unsigned boundaries,
  carry/borrow chains, operand orders, byte/word mode transitions, branch joins,
  volatile traces, aliasing and bank crossing, and live words across direct and
  indirect assembly calls clobbering A/X/Y and all 64 DP scratch bytes.
- Exhaustive IRQ injection covers 2,416 raw and 2,264 optimized enabled
  instruction addresses. Both modes reach seven ADC/SBC windows; carry setup,
  arithmetic, and result-store addresses are individually interrupted. Seeded
  IRQ/NMI, shared helpers, context restoration, pointer preemption, and stack
  guard failures also pass.
- All seven o65 execution tests pass at their independent placements, including
  preempted tasks. All 118 saved artifacts are identical across host builds.

Native execution used the qualification runner's pinned VM and CPU timing
patch, Rust 1.95.0 and ca65/ld65 2.18 on macOS ARM64. These are emulator cycle
measurements, not board timings. No SemIR/NIR contract changed; root validation
was scoped to the affected native compiler consumers, without a full root test
run or NIR sweep.
