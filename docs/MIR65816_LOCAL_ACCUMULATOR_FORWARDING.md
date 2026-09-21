# Native 65816 local accumulator forwarding

Completed on 2026-09-21. Baseline counts and machine-code probes are committed
as `5c8fe62`; checked selection and focused execution as `d946c33`. The
[plan](MIR65816_LOCAL_ACCUMULATOR_FORWARDING_PLAN.md) retains the forecasts and
scope. The [qualification record](abi/action65816-accumulator-forwarding-qualification.json)
binds these results to source, tools and saved artifacts.

## Selection and contracts

Adjacent eligible word operations now reuse A16 instead of reloading the same
private stack temporary. A completed native ADD/SUB or direct, nonvolatile word
load can supply arithmetic's actual left operand, the selected comparison
operand, an ordinary direct word store, or an A16 return. Arithmetic chains may
publish each new result. All producer stores and allocated homes remain.

The private fact identifies the exact TempId, stack slot, zero transient frame
displacement and emission cursor. It certifies the full word and its N/Z flags.
Every intervening instruction or label invalidates that cursor, even an
instruction that preserves A but changes flags. Other MIR operations clear the
fact by default. Calls, helpers, edges, joins, mode transitions, stores and stack
movement cannot carry it forward. Existing complete operand/address preflight
runs even when the load is omitted; malformed extents retain their diagnostics.

Source memory is never cached. Volatile, indirect/indexed, DP and non-word
transfers remain outside selection. Mutable incoming parameters and addressable
locals retain their original homes and accesses; only their captured private
temporary can qualify. Physical ABI v1, image v3, the o65 profile, DP allocation,
frame/staging reservations, stack guards and Exec816's pin are unchanged.

## Measurements

The immutable baseline is the
[direct-edge snapshot](benchmarks/65816-single-word-edges/after/tables.md),
qualified at `0e8248c`. Before changing emission, all 224 artifact hashes across
56 builds, 264 matching host records and the previous qualification were checked.
See the [new snapshot](benchmarks/65816-local-accumulator-forwarding/after/tables.md)
and [complete delta](benchmarks/65816-local-accumulator-forwarding/delta.md).

| Kernel / input | Mode | Bytes before / after | Cycles before / after | Stack reads before / after | Unchanged stack peak |
| --- | --- | ---: | ---: | ---: | ---: |
| identity(13) | raw / optimized | 63 / 61 | 71 / 66 | 7 / 5 | 4 |
| add(13,41) | raw / optimized | 74 / 72 | 98 / 93 | 13 / 11 | 8 |
| maximum(13,41) | raw / optimized | 110 / 104 | 111 / 101 | 15 / 11 | 6 |
| sum_loop(13) | raw | 174 / 164 | 2,032 / 1,767 | 303 / 197 | 14 |
| sum_loop(13) | optimized | 146 / 140 | 1,595 / 1,395 | 245 / 165 | 16 |
| byte_sum($12FFFC,16) | raw | 272 / 262 | 5,003 / 4,678 | 660 / 530 | 16 |
| byte_sum($12FFFC,16) | optimized | 244 / 238 | 4,476 / 4,231 | 590 / 492 | 22 |

All forecasts match exactly. The predeclared inventory covers **76 static sites
in 22 Action builds**, with **1,422 executions per incoming I state** across
112 positive vector records. Each removal saves two static bytes, one executed
instruction, five qualified VM cycles and two private stack-byte reads. All
stack writes, DP traffic, frame maps, guard costs and stack peaks are unchanged.
Fusion and word-edge counts retain their semantic coverage. The six unselected
Action builds remain byte-identical. LF/CRLF corpus builds match.

The [instruction-stream checker](../tools/compare65816/check_accumulator_forwarding.py)
proves that only declared `LDA d,S` instructions disappear, with required JSL/JML
target relocation. Producer stores remain; an independent entry into an omitted
reload is rejected. It maps each old load to the first surviving consumer and
requires every declared site to execute. The separate delta mode enforces the
exact dynamic savings while leaving previous accounting modes strict.

Both external host commands save all 264 records (528 executions per host),
retaining the known optimized vbcc unlink vector-0 failure in both I states.
All vbcc records and code remain identical. Its raw vasm listing differs only
in the exact source-header build path. The failing output receives no exemption.

## Execution and qualification

Nonserialized `Code.mir_spans` supplies typed operation boundaries for test-only
evidence. An independent index checks producer/consumer identities, private
homes, instruction boundaries, retained stores and actual consumer encodings.
A bare CMP or TAY is insufficient evidence. Comparison and return decoders now
accept authenticated resident operands without losing their existing tail,
edge or target checks. The evidence never participates in CPU execution.
Saved corpus images must exactly match independent recompilation before indexing.

Independent ca65 original/forwarded snippets compare full registers, flags,
memory and exact traffic at the consumer and after execution. Compiler tests
reject changed flags, stores, labels, widths, identities, reused homes, invalid
displacements and transient stack state. Generated-code probes exercise all
four consumer kinds with boundary words and unchanged volatile traces. Existing
aliasing, bank-crossing memory, clobbering calls, indirect/recursive/helper paths,
mixed widths, cyclic edges and stack fault tests remain in the full suite.

The new IRQ probe checks **98 raw / 76 optimized task/PC sites**, covering the
producer store, surviving consumer boundaries, carry/compare flags and complete
return teardown in both task domains. Restoration is compared with an
uninterrupted CPU step and invocation frame. Both compare carry outcomes and
zero/negative/nonzero words are exercised. Both existing seeded IRQ/NMI schedules
pass. General coverage reaches 2,237 raw / 2,077 optimized enabled addresses;
targeted fused coverage retains 300 raw / 296 optimized sites, all 222 cyclic-copy
sites and all 24 flag outcomes. Direct-edge and materialized comparison probes
retain 34 and 96 sites per mode respectively.

The new o65 probe executes serialized bytes at $100000 and $600000 with moved
data and fault imports. Forty executions cover both compiler modes, both I
states and five boundary pairs; every forwarding site is reached and checked
against its private word and flags. Volatile traces remain exact. Compilation
objects are discarded before relocation and execution.

Full native qualification passes **84 tests in each host build**, with **302
identical artifacts**. Compiler checks pass 40 native library tests and 58
ABI/emission/o65/CLI integration tests. Python comparison tests pass 16 cases;
disassembler tests pass five. The source generator and LF/CRLF checks pass.
An isolated checkout with CRLF in both new/shared fixtures rebuilds and passes
the 23 forwarding, preemption and o65 tests, producing identical saved artifacts.
No NIR, semantic or allocator contract changed, so the full root suite and NIR
sweep were outside this slice. Evidence is corrected pinned VM execution;
board and Exec816 loader qualification remain separate.
