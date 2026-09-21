# Native 16-bit comparisons: implementation and qualification

Completed on 2026-09-21 against actionc main. Native word comparisons reduce
maximum(13,41) from **186 bytes / 173 VM cycles to 146 / 149**, and optimized
sum loop(13) from **252 / 2,829 to 213 / 2,465**. Both now use zero DP scratch
traffic. The measurements match the plan's estimates and preserve stack depth.

The [plan](MIR65816_WORD_COMPARISONS_PLAN.md) is complete: `9dff815` records the
verified baseline and semantic coverage, `51e5bc7` implements selection, and
the commit containing this report records qualification. Existing local changes
were preserved.

## Implemented boundary

Operand width two enables native CMP for all unsigned relations and signed
equality/inequality. Signed ordering stays on the existing bytewise path: CMP
does not set V, so its flags cannot directly implement signed subtraction tests.
Gt/Le swap already captured operands and use BCC/BCS respectively.

The selector reuses checked U8/U16 immediates and exact two-byte stack
temps/parameters, including authoritative mutable parameter homes. It validates
both inputs and the exact one-byte destination before emitting bytes, labels or
mode changes. Legal unsupported forms retain generic emission; malformed homes
remain errors. Source word extents and the destination byte include transient
stack movement in their displacement checks.

The comparison consumes C/Z before loading 0 or 1, establishes A8 on both arms
and at the join, and writes one byte to the allocated Boolean home. Both source
words are read before that store. It uses no DP scratch, helper, push, X/Y, or
register/flag value carried across MIR operations. Calls, volatile accesses,
aliased loads and explicit casts keep their existing ordering and semantics.
Branch terminators still reload the Boolean; no compare-to-branch fusion or
general mode/branch cleanup was included.

See the [emission contract](MIR65816_EMISSION_CONTRACT.md). Physical ABI v1,
image v3, the o65 profile, allocation, stack guards and Exec816's pin are unchanged.
This is an emitter change with no SemIR/NIR contract change.

## Measurements

The immutable [post-return snapshot](benchmarks/65816-word-returns/after/tables.md)
is the baseline. [Baseline verification](benchmarks/65816-word-comparisons/baseline.json)
records its revision, tool and artifact hashes, and the new semantic tests passing
before selection changed. Representative optimized Action results include guards
and RTL:

| Kernel / input | Code bytes before → after | VM cycles before → after | DP byte reads + writes before → after | Stack bytes below entry S |
| --- | ---: | ---: | ---: | ---: |
| maximum(13,41) | 186 → 146 | 173 → 149 | 4 → 0 | 6 |
| loop rotation(13) | 314 → 277 | 2,186 → 2,000 | 36 → 0 | 26 |
| sum loop(13) | 252 → 213 | 2,829 → 2,465 | 56 → 0 | 16 |
| recursive sum(13) | 286 → 247 | 4,171 → 3,819 | 56 → 0 | 190 |
| byte sum(16 bytes) | 350 → 311 | 5,974 → 5,532 | 276 → 208 | 22 |
| forward copy(8 bytes) | 393 → 354 | 3,823 → 3,589 | 244 → 208 | 18 |

Raw maximum also measures 146 bytes / 149 cycles; raw sum loop drops from
254 / 2,904 to 218 / 2,596. Different incoming accumulator modes account for
the smaller raw loop reduction. Identity, add/subtract, constant chain, direct
calls, wide shift, record field and unlink retain their measurements.

The [full delta](benchmarks/65816-word-comparisons/delta.md) and its
[checked invariants](benchmarks/65816-word-comparisons/delta.json) cover every
vector in both target modes. There are no Action code-size/cycle regressions;
complete routine storage maps, argument/result metadata, observed stack peaks,
stack writes and guard costs are unchanged. Maximum and sum-loop guards remain
45 bytes / 32 cycles per entry. Regression tests enforce maximum at most
155 bytes / 160 cycles, optimized sum loop at most 220 / 2,500, and raw sum loop
at most 220 / 2,600, alongside their exact stack depths.

Stack reads increase only in maximum vectors 3 and 4, in each target mode:
14 → 16 bytes overall. Their unequal high bytes let the old comparison stop
early; native CMP reads both complete private words. These four +2 exceptions
were [declared before implementation](benchmarks/65816-word-comparisons/stack-read-deltas.json).
The delta checker accepts only those exact differences; its default remains
strict equality. Executed comparison intervals verify complete source reads,
one Boolean write and zero DP traffic. Original volatile memory traces remain
unchanged.

The [new snapshot](benchmarks/65816-word-comparisons/after/tables.md) includes
all CSV measurements, provenance, and raw/optimized final maximum listings
alongside identity/add/subtract/sum-loop/unlink. Its 66 vectors produce 264
paired-mask records: 528 executions per host build, with all 264 Action
executions passing. All 112 LF/CRLF compilations agree, and debug/release
measurements match exactly.

The external comparison still fails for optimized vbcc unlink in both I states.
All vbcc measurements are identical to baseline; the invalid output remains
reported and is excluded from valid-code performance claims.

## Qualification

The [machine-readable record](abi/action65816-word-comparisons-qualification.json)
binds the results to compiler, fixture, tool, VM and artifact hashes:

- 22 native compiler unit tests and 58 ABI/emission/o65/CLI integration tests
  pass. New selector tests cover predicates, signed fallback, operand homes,
  nonmutating errors/fallback, byte versus word boundaries and transient S changes.
- All **59 native tests pass in debug and release**. The external comparison
  is ignored by default and was run separately with the failure described above.
  All **166 saved artifacts** match across host builds.
- Five word-comparison tests perform 504 machine runs per build. They cover all
  six signed/unsigned relations across boundary pairs, exact Boolean values,
  byte canaries, operand orders, casts, mutable parameters, clobbering direct and
  indirect calls, volatile traces and bank-crossing aliases. Four LF/CRLF source
  pairs compile identically. Independent ca65 probes confirm CMP encodings and
  execution with varied incoming C/V.
- Exhaustive IRQ injection covers 2,311 raw / 2,153 optimized enabled instruction
  addresses, including three selected comparison windows. Existing seven
  arithmetic windows, seven word-return tails and zero-frame/pointer tests remain
  covered. A targeted comparison probe checks all 96 task/PC boundaries per mode,
  plus all 16 predicate/truth/task combinations with IRQ immediately after CMP
  while C/Z are live. Both seeded IRQ/NMI schedules also pass.
- All eight o65 tests pass, including a new comparison probe with 32 executions
  across both placements, target modes and I states. Its branches and Boolean
  materialization execute from relocated serialized bytes.
- Five disassembler tests and three delta-accounting tests pass. The strict
  default also validates the historical arithmetic-to-return measurements.

Execution uses the pinned VM with the existing CPU timing correction, Rust
1.95.0 and ca65/ld65 2.18 on macOS ARM64. These are VM cycles, not board timings.
Root checks were scoped to native consumers; no full root suite or NIR sweep
was required. Signed ordering and compare-to-branch fusion remain separate
follow-up candidates.
