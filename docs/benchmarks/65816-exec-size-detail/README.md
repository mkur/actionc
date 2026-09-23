# Exec816 code-size audit, current sources

Measured on 2026-09-23. This is an inventory and recommendation, **not an
implemented optimization or hosted Exec qualification**. The largest remaining
opportunities are repeated compiler sequences: guards, call argument setup,
wide comparisons and scalar returns. They do not require a new public ABI or
broader register allocation.

## Current workload and compiler integration

Exec has advanced to `c3500c813b76e4b1559960f13342c5bf5e015731`. Its current pin,
`2d73c03a0cb02e1f9d5b54d49679d6c2c4d8d78d`, is a side-branch child of
`ae1f555e`, not current actionc main. It lacks the completed BYTE/pointer
comparison improvements. Main measured here is
`9f16e08b39308a1b2e9d9d1981f7410f84dbacc1`.

Both compiler modes build the same eight-task shell, console and MyDOS
configuration, with **stack checks enabled**:

| Compiler / mode | Compiler routine bytes | All executable bytes | XEX file bytes |
| --- | ---: | ---: | ---: |
| Current Exec pin, raw | 609,790 | 617,355 | 636,175 |
| Current main, raw | 534,815 | 542,380 | 559,888 |
| Current Exec pin, optimized | 576,958 | 584,523 | 602,751 |
| Current main, optimized | 503,453 | 511,018 | 527,916 |

Main already saves **73,505 executable bytes (12.58%)** in the optimized build,
and 74,975 in raw. This is a measured compiler comparison, not a new saving
from this audit. All 551 routine contracts match within each mode: frames,
homes, incoming/outgoing arguments, results and call records. Only routine
addresses and sizes differ. Generated Action sources match after normalizing
their build-directory paths.

There is an integration prerequisite: Exec now passes `stack_checks` in its
layout, but main rejects that unknown field. An audit-only adapter asserts
`stack_checks:true` and removes just that field from the compiler input. Main
always emits the mandatory guards. It rejects unchecked input rather than
silently adapting it. Reconcile the stack-check configuration feature and run
Exec qualification before updating its pin; this report does not update it.

The historical frozen `8e1ff57` result remains 502,845 executable / 519,393 XEX
bytes. It is a different workload. The current workload adds routines and
guards; do not interpret its larger absolute total as a compiler regression.

## Where the bytes go

The optimized main build contains 503,453 compiler bytes, 7,565 executable
platform/assembly bytes and 2,315 initialized data bytes. Compiler routines
alone account for **95.4% of the XEX file**. Changing the container or trimming
strings is a much smaller opportunity than improving emission.

The following instruction categories partition the compiler routine bytes;
guards are removed from every other row:

| Category | Bytes | Share |
| --- | ---: | ---: |
| Stack guards, 2,320 instances | 104,400 | 20.7% |
| Stack-relative instructions | 142,922 | 28.4% |
| DP instructions | 58,546 | 11.6% |
| REP/SEP mode changes, 23,243 instructions | 46,486 | 9.2% |
| JML | 30,344 | 6.0% |
| Conditional branches | 11,088 | 2.2% |
| JSL | 7,072 | 1.4% |
| Indirect and absolute-long memory | 14,344 | 2.8% |
| Other instructions | 88,251 | 17.5% |

MIR attribution provides a different, overlapping view: direct call spans cost
179,419 bytes, including 79,560 guard bytes. Their **non-guard cost is 99,859
bytes**, plus 76 for the single indirect call. Return spans cost 37,894 bytes.
These figures must not be added to the instruction-category totals.

TASKPOLICY contributes 123,201 bytes, SHELLAPP 66,063, DOSCALLS 27,950,
MYDOSFILE 19,848 and MYDOS 18,655. No single routine dominates: the largest is
ShellParse at 5,931 bytes, followed by ShellFinish at 4,986 and
FSRELATIVE.Begin at 4,371. General improvements to repeated sequences are more
promising than special treatment of a large function. See the full
[per-routine comparison](routine-sizes.csv) and [inventory](inventory.json).

## Ranked implementation opportunities

### 1. Initialize outgoing padding once; write each argument once

The [call selector](../../../src/mir65816/emit/select.rs) reserves the outgoing
area, clears **every byte**, and then writes each argument over its payload
bytes. In optimized output, 1,769 call sites reserve 8,343 outgoing bytes in
total. Their 6,878 payload bytes overwrite earlier zero stores. Those redundant
`STA d,S` instructions occupy **13,756 code bytes**; raw has 13,846 bytes.
These are static totals over call sites, not runtime stack requirements.

This is the recommended first small slice: initialize only alignment gaps and
tail padding, then retain the existing argument writes. The explicit clear
sequence is present in final machine bytes at every counted call. The forecast
counts only the removed two-byte payload clear stores, not additional mode or
load improvements. Leave padding zero, outgoing extent, stack alignment,
guard placement, transfer peak, source capture and caller cleanup unchanged.
Prove that captured argument sources cannot overlap the fresh outgoing area;
retain conservative call/helper and preemption contracts.

The selected slice now has a proposed
[implementation plan](../../MIR65816_CALL_PADDING_PLAN.md); implementation is pending.

Native-width argument copies are a separate follow-up. There are 411 word,
1,315 three-byte and 464 four-byte arguments, plus 255 BYTE arguments. The
current argument writer handles all payloads bytewise even though ordinary
word/four-byte memory transfers already use native pairs. Also inventory result
capture: 171 word results still pass through byte stores and XBA. Neither change
requires register arguments or an ABI revision.

### 2. Compact existing stack checks

Every recognized guard is 45 bytes. Four internal conditional transfers each
use an inverse short branch over JML; all four targets fit short relative
branches. Converting just those transfers gives a **37,120-byte encoding
opportunity** across 2,320 guards (37,248 raw). The remaining local JML can
potentially save another 4,640 bytes with BRA, separately.

Retain every check and its amount, position before mutation/push, underflow and
domain-limit behavior, fault target, and A/X/flag postconditions. This should
extend typed branch/layout machinery, with offset, relocation and proof
metadata rebuilt; it must not patch arbitrary byte patterns. Guard semantics
and asynchronous stack transitions require focused execution qualification.
No guard removal, hoisting or unchecked-build comparison is proposed.

### 3. Select native 32-bit equality and consume branch-only results

There are **346 four-byte comparisons occupying 37,062 bytes**. Eq/Ne accounts
for **228 sites and 24,568 bytes**. Both signed and unsigned equality currently
use the general bytewise less/equal/greater algorithm, including three Boolean
arms. Signed equality also performs unnecessary sign-bias operations.

For example, HEAPCORE.Rounded's `LONGCARD == 0` compare alone is 106 bytes,
followed by a 20-byte Boolean branch span. Its captured value could be tested
as two native words, avoiding full ordering and Boolean materialization when
the result is consumed only by the branch. The inventory finds 215 adjacent
Eq/Ne branch candidates with no other syntactic temp occurrence; eligibility
still needs typed use-def and operand preflight, not a debug-text matcher.

Start with Eq/Ne, including zero tests, using only captured operands and proven
home extents. Preserve original volatile/aliased loads and materialize canonical
0/1 when a value is needed. Treat signed/unsigned ordering as a separate slice.
The current byte costs are measured; replacement savings are not yet measured.

### 4. Return BYTE constants directly in A16

The generic non-word return path clears four DP result bytes, fills the result,
reloads A/X and tears down the frame. **496 BYTE constant returns** use this
path, costing 15,594 bytes including teardown. A16 `LDA #$00xx` followed by the
existing teardown would save **19 bytes per site, or 9,424 bytes**. The return
span is currently 31 bytes, or 33 when it also restores entry A16; preserve that
entry restoration when required. BYTE results require zero-extended A16, while
X is unspecified by ABI v1.

Then consider the 160 captured BYTE returns and wider A/X returns separately.
Do not widen a one-byte home read without proof of ownership of the extra byte.
Keep the frame release, RTL, boundary mode and native return effects intact.

### 5. Broaden local branch relaxation

Outside guards, **3,382 long conditional transfers** can save 13,528 bytes in
the current layout; 3,163 separate local JMLs have another 6,326-byte BRA
opportunity. Raw counts are 3,510 / 3,270, or 14,040 / 6,540 bytes. These are
initial-layout encoding forecasts, not an implemented fixed-point result.

The inventory uses typed labels, fixups and dispatch metadata. It excludes
already-short dispatches that happen to be followed by JML, and does not count
the JML inside a long conditional again as an unconditional candidate. All
conditional candidates stay within the routine/bank and have no interior label.

**Do not add these forecasts to comparison-rewrite forecasts.** Of the 3,382
non-guard conditional sites, 2,768 belong to the generic four-byte comparisons;
native selection replaces many of them. Guards are accounted separately.
Preserve label/fixup rebasing, PER continuations, proof spans and o65 relocation.

### 6. Smaller address and constant-operation slices

- **Drop the final unused index-scale shift.** Every one of 394 indexed
  operations emits `ASL $14; ROL $15; ROL $16` after its last stride bit, even
  stride one (237 sites). Those final scratch updates have no subsequent use
  in address construction: **2,364 bytes** of candidate instructions, in both
  modes. Prove scratch/flag deadness and preserve full 24-bit address arithmetic,
  memory ordering and the execution-domain scratch contract.
- **Specialize constant shifts.** The 55 shift spans occupy 4,709 bytes; 52
  have constant counts. Byte-multiple shifts still use general counter loops,
  bounds handling and repeated memory shifts. Select copies/zero/sign fill for
  proven constant cases. The 4,709 bytes are current cost, not removable bytes.
- **Reduce repeated pointer construction and same-width casts.** This remains
  useful, but ordinary three-byte transfers already support overlapping native
  word copies. Avoid adding another general forwarding layer. Reuse existing
  ownership, liveness and checked rewrite facts, with calls, helper clobbers,
  aliases and volatile accesses as barriers.

## Work not justified by this inventory

A register-argument ABI or broad register allocator is not necessary for the
opportunities above. DP is execution-domain scratch and call-clobbered; do not
keep values there across calls or preemption without the existing contracts.

Thirty routines, totaling 24,828 bytes, have no incoming *compiler* code/data
relocation. That is **not a dead-code saving**: the set includes TASKPOLICY.Init,
Dispatch and assembly/platform entry points. Dead stripping needs an explicit
root/export manifest covering separately assembled calls and task entries.

This audit changes no bank-zero reservation. The compared builds have identical
runtime budget: 24,672 bytes excluding OS, 61,536 including OS. Code-bank savings
must not be presented as additional bank-zero task stack/DP capacity.

## Evidence and reproduction

[builds.json](builds.json) records revisions, input/artifact hashes, compatibility
adjustment and contract checks. [inventory.json](inventory.json) contains both
raw and optimized attribution. [examples.lst](examples.lst) retains actual
optimized instructions for a return, call, wide comparison and indexed load.

The isolated Exec checkout is recorded in `builds.json`. Its build commands use
`tools/native_program.py --source examples/shell.act --tasks --task-capacity 8
--console --stack-checks --dos-mounts config/shell-mydos.json`, adding `--no-opt`
for raw, the appropriate `--compiler-dir`, and separate `--output` directories.
The main build additionally uses `--allow-compiler-override` and the explicit
layout adapter described above. The original dirty version banner was retained
in the isolated source checkout; live source, pin and local edits were preserved.

Audit-only source and full JSON/listing artifacts remain under
`target/exec-code-size-detail`: `build_main.py`, `probe/src/main.rs` and
`analyze.py`. Their hashes are recorded. The probe compiles the pinned generated
sources through current main, exports typed MIR spans, labels, fixups, dispatch
metadata and routine references, and checks against the separately built image.
Instruction attribution uses the repository's 65816 disassembler and an exact
45-byte guard recognizer. Selected call clear sequences and final index shifts
were also verified against emitted bytes. Retained reports avoid checking large
executables and temporary audit tooling into the compiler.

Validation was four complete shell builds, raw/optimized static machine-code
inspection, accounting and source/ABI/frame/home comparisons. No optimization
was implemented; no new execution-cycle, VM, hosted boot or preemption claim is
made. A chosen implementation must run the affected 65816 tests, raw/optimized
machine-code checks and relevant ABI/guard/preemption and relocation controls.
