# Native 65816 state-tracker results

The first two stages of the [design](MIR65816_STATE_TRACKER_DESIGN.md) are
implemented. [Qualification](abi/action65816-state-tracker-qualification.json)
proves unchanged native output against the accumulator-forwarding baseline.
The [implementation plan](MIR65816_STATE_TRACKER_IMPLEMENTATION_PLAN.md) is complete.

## Implementation boundary

[TrackedEmitter65816](../src/mir65816/emit/tracked.rs) privately owns the encoder
and [State65816](../src/mir65816/emit/state.rs). All production instructions use
closed typed forms that select encoding and effects together. The former width
cache, stack delta and resident-word shadow have been removed. Selection keeps
its existing preflight, addressing choices and producer/consumer policy.

Facts use immutable value identities, explicit widths, independent N/Z and C/V,
private stack ranges and contents generations. Overlap, indirect/DP writes,
calls and joins invalidate relations conservatively. A byte constant cannot
establish the hidden high accumulator lane. There is no source-memory cache.

Checked execution widths remain separate from permission to omit REP/SEP; every
label revokes that permission. Incoming edges and local backedges must agree
with execution contracts. The guard and S-transfer sequences retain checked
stack-address equations, a body anchor, outgoing displacement and transfer phase.
The indirect six-byte peak and normal return are separate events. Import IRQ
effects are unavailable until linking, so I preservation becomes unknown after
calls and joins. No additional optimization is enabled.

The default-off `native65816-state-proof` feature exposes immutable observations
and fixed probes. Ordinary compilation records no snapshots. Trace-on/off code,
labels, fixups, PER fixups, MIR spans and linked images agree. Independent ca65
programs check instruction bytes, while the VM checks constants and simultaneous
register/home/NZ relations. Opaque identities are never compared across dynamic
loop iterations or invocations. Relocated bytes are checked after explicit
rebasing before observations are tested.

## Unchanged measurements

The [strict equality report](benchmarks/65816-state-tracker/equality.json) covers
56 builds, 224 artifact files and every field of all 264 measurement records in
both host configurations. Only the exact vasm listing source-path header is
normalized. Artifact path provenance is checked relative to each build directory.
The [saved report](benchmarks/65816-state-tracker/after/tables.md) retains the
known optimized vbcc `unlink` vector-0 error in both incoming I states; both
external corpus commands still fail that case after saving all measurements.

| Kernel / input | Mode | Bytes | Cycles | Stack reads / writes | Peak | Forwarded loads |
| --- | --- | ---: | ---: | ---: | ---: | ---: |
| identity(13) | raw / optimized | 61 | 66 | 5 / 2 | 4 | 1 |
| sum_loop(13) | raw | 164 | 1,767 | 197 / 246 | 14 | 53 |
| sum_loop(13) | optimized | 140 | 1,395 | 165 / 188 | 16 | 40 |
| byte_sum($12FFFC,16) | raw | 262 | 4,678 | 530 / 559 | 16 | 65 |
| byte_sum($12FFFC,16) | optimized | 238 | 4,231 | 492 / 489 | 22 | 49 |

All 76 forwarding sites and 1,422 executions per incoming I state remain, with
identical fusion/copy PCs, homes, frame layouts and stack guards.

## Validation and limits

- 49 MIR65816 unit tests and 59 integration/boundary tests pass.
- Both complete native hosts pass 88 tests, with one optional corpus test ignored.
  All 302 prior artifacts remain identical; all 374 current artifacts match
  between hosts and bind to the same 412 compiler/fixture inputs.
- The existing raw/optimized IRQ coverage remains 2,237/2,077 enabled instruction
  addresses. Forwarding retains 98/76 task/PC sites; fused branches retain
  300/296 sites and 24 flag outcomes. Existing seeded IRQ/NMI, fault, helper,
  alias/volatile, recursion and o65 tests remain intact.
- The comparison tool tests pass 18 cases and the disassembler tests pass five.
  Corpus compilation checks LF/CRLF equivalence. An isolated CRLF checkout also
  rebuilds the frozen boundary test and four state-proof VM tests.

Physical ABI v1, image v3, experimental o65, allocation and Exec816's pin are
unchanged. Wider forwarding, block-entry width omission and X/Y/DP allocation
remain future measured slices. The first slice supplies proofs without changing
code quality or increasing register lifetimes.

## Scalar DP word prerequisite

Private word identities now include the address space: S+$20 and D+$20 are
distinct homes. Explicitly registered scalar DP words in D+$20..D+$3F use the
same contents generations and adjacent producer permissions as stack words.
Partial writes invalidate overlapping homes; selector scratch writes outside
registered residents, unknown writes, calls and joins retain their conservative
barriers. DP identities require the current fixed domain. Incoming reads and
frame witnesses remain stack based, while their captures may use either space.

Native word selection and parallel copies accept checked DP sources and
destinations, retaining stack staging and final A/N/Z repair. Proof snapshots
identify their address space and the native oracle reads them relative to D or
the frame anchor. Allocation remains disabled in this prerequisite: all 28
saved corpus images, maps and historical movement facts are byte-identical,
including LF/CRLF compilation. Validation covers 97 compiler unit tests, 60
integration tests and seven native state tests, including independent ca65
encodings, boundary arithmetic and same-offset stack/DP generation checks.

The [scalar allocation slice](MIR65816_SCALAR_DP.md) now enables these checked
homes for a bounded whole-routine whitelist. Final qualification includes 100
emitter/proof unit tests and 111 native tests per host profile. Loop residency
comes from CFG liveness; tracker permissions still stop at labels and calls.

## Indexed symbolic BYTE accesses

The CRC/sieve series admits typed `LdaLongX` for an allocated symbol plus a
captured unsigned CARD index. Selection requires an exact nonvolatile BYTE,
stride one, zero displacement/addend, complete checked private homes, and no
loop-X reservation. X is temporary addressing state and is not retained across
MIR operations. The source BYTE is neither widened nor cached.

Effects describe `SymbolIndexedX`, not an unindexed `Symbol`: the runtime X
value participates in the full 24-bit effective address. The form consumes X
and M-dependent memory width, conservatively blocks memory forwarding, and
creates a fresh A/N/Z value. Its relocation describes only the symbolic base;
the CPU supplies the index and bank carry. Replay derives these effects from
the instruction form, as it does for every other admitted instruction.

`StaLongX` uses the same checked address subset for constant or captured BYTE
payloads. X is established before loading the payload, including overlapping
private index/payload homes. Its effects consume A and X, preserve flags, and
record a dynamic `MayWrite`; no fixed private home is inferred. Unknown-write
invalidation applies to tracked memory facts. Read/modify/write expressions
retain their separate original load and store operations.
