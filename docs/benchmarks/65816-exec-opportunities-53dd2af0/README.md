# Exec compiler opportunities after immediate argument pushes

Reanalyzed compiler **53dd2af0** on 2026-09-26. The strongest remaining
opportunity is to let a captured private pointer's **final store or direct
call** consume its original home. The current forwarding pass deliberately
stops before either operation. Extending that bounded window models **15,024
bytes (14.7 KiB)** of savings, enough to close the estimated 7.9 KiB release
gap without cross-call lifetimes or general alias analysis.

These are offline candidate models, not implemented or executed optimizations.
Budget **11–14 KiB** for the pointer work until actual compiler deltas replace
the estimate. That allowance would put this workload at roughly **250–253
KiB**, including package assembly and initialized data, without debug guards.

## Baseline and scope

The inventory probe rebuilt the frozen Exec `622b139-dirty` workload with the
current compiler. All **120 input hashes** match. The rebuilt image, including
routine metadata and every segment byte, equals the final image from the
[previous five slices](../65816-constant-index-byte/README.md).

| Measurement | Bytes |
|---|---:|
| Compiler code, 631 routines, guards included | 331,914 |
| Compiler stack guards, 2,676 sites | 72,252 |
| Compiler code with guard ranges subtracted | 259,662 |
| Package assembly, carried forward | 8,300 |
| All initialized data, carried forward | 2,307 |
| **Estimated loaded code + initialized data without guards** | **270,269** |
| Release cap, 256 KiB | 262,144 |
| **Remaining gap** | **8,125** |

The baseline is **263.9 KiB**. This is guard-range subtraction from the frozen
guarded build, not a separately linked guard-disabled release. Compiler data
still measures 951 bytes and is already included in the 2,307-byte allowance.
The audit does not measure subsequent changes in live Exec sources.

No compiler or Exec source was changed. No full backend or hosted Exec
qualification was run.

## Ranked candidates

Footprint is the current non-guard code occupied by a candidate, not the amount
that can disappear. Savings retain mode changes unless the modeled replacement
explicitly accounts for them. No frame reduction, new branch relaxation or
guard removal is credited. Site counts denote captures/operations, not unique
source lines or unique calls.

| Candidate | Sites | Current footprint | Modeled saving |
|---|---:|---:|---:|
| Private pointer capture ending at a store | 1,126 | 9,682 B | **9,008 B** |
| Private pointer capture ending at a direct call | 752 | 6,140 B | **6,016 B** |
| Adjacent private BYTE/CARD/LONG capture and consumer | 657 | 4,102 B | **3,496 B** |
| Bounded BYTE/constant indexed address formation | 19 | 1,793 B | **1,288 B** |
| Native pieces in remaining mixed/LONG edge copies | 46 | 2,155 B | **873 B** |
| Direct operands for BYTE Add/Sub/AND/OR/XOR | 217 | 2,458 B | **868 B** |

The models total **21,549 bytes (21.0 KiB)**. They address disjoint original
instruction regions, but their implemented eligibility and interactions still
need measurement. The pointer pair is sufficient to justify the next work;
the total is not a release-size promise.

### 1. Admit the final store or call in pointer forwarding

Current captures typically contain this eight-byte private copy in A16:

```asm
LDA source,S
STA capture,S
LDA source+1,S
STA capture+1,S
```

The overlapping words touch exactly the three owned private bytes. At a later
store or call, another sequence reads `capture`. If every use is in a stable
window ending at that operation, the consumer can read `source` and the four
copy instructions disappear. Required REP/SEP instructions stay in the model.

The candidate windows require:

- A complete immutable, non-escaping incoming parameter, or a complete
  non-addressable local frame object with canonical accesses.
- Every captured-temp use in the same block, with no intervening calls,
  stores, copies or volatile operations before the final consumer.
- No cast consumer, edge consumer or subsequent use; earlier pointer uses
  must fit the existing supported selectors.
- A complete stack source disjoint from temporary homes. For calls, its full
  extent must also fit stack-relative addressing at the outgoing stack delta.
- Binding expiry at the terminal operation. Stores and calls remain barriers
  for all later operations.

The store group contains **1,069 address-base uses** and **57 stored-pointer
values**. Address-base uses are the simpler first slice: copy the original
private pointer into address scratch, then perform the unchanged external
store. A stored-pointer value needs its own complete transfer/overlap preflight.
The call group reads the source while packing outgoing arguments, before JSL;
it does not borrow storage through execution of the callee.

| Source and terminal use | Captures | Saving |
|---|---:|---:|
| Incoming parameter → store address base | 574 | 4,592 B |
| Local frame pointer → store address base | 495 | 3,960 B |
| Incoming parameter → stored pointer value | 37 | 296 B |
| Local frame pointer → stored pointer value | 20 | 160 B |
| Incoming parameter → direct-call argument | 501 | 4,008 B |
| Local frame pointer → direct-call argument | 251 | 2,008 B |
| **Total** | **1,878** | **15,024 B** |

This is broadly distributed: the largest routine contribution is 176 bytes
within either terminal category. The opportunity is repeated private copying,
not one unusually large procedure.

The old whole-routine immutable-parameter idea has a surviving transfer ceiling
of 9,160 bytes across 1,145 captures. **1,112 of those are already in the
terminal windows above.** The incremental ceiling is only **33 captures / 264
bytes**. Do not add the whole-routine figure to this plan or expand lifetime
analysis merely to pursue it.

### 2. Adjacent private scalar captures

There are 657 surviving complete private scalar captures whose sole use is the
immediately following compare, arithmetic operation, store or direct call.
No other source operation intervenes. The same source-identity restrictions
apply as above; external/indirect loads and casts are excluded.

| Width | Sites | Modeled saving |
|---|---:|---:|
| BYTE | 106 | 416 B |
| CARD / INT | 261 | 772 B |
| LONGCARD / LONGINT | 290 | 2,308 B |
| **Total** | **657** | **3,496 B** |

Some word consumers already reuse A. In **143 sites**, the model retains the
source load and credits only the redundant capture stores; it does not count
an already-eliminated reload again. Other sites redirect the consumer's private
read to the original home and remove the capture. A practical initial budget
is **2.5–3 KiB**, with 32-bit operands offering most of the benefit.

This needs typed read-home resolution in scalar selectors and clear handling
of existing A/NZ facts. Keep it separate from pointer forwarding.

### 3. Finish bounded indexed address formation

The previous indexed-access work improved loads/stores. Indexed `AddressOf`
still uses the generic byte-wise address computation. Of 27 indexed address
operations occupying 2,737 bytes, **16 unsigned BYTE indexes and three constant
indexes** form a bounded subset occupying 1,793 bytes.

Reuse the native bounded scale, then add it to the captured pointer's low word
and propagate carry into the bank byte. A BYTE index must still be read as one
byte and zero-extended explicitly. The model includes the outgoing A8 mode.
For stride 44, an example drops from 123/125 bytes to **35 bytes**; power-of-two
scales also improve substantially.

The 19 replacements model **1,288 bytes** saved, so budget about **1 KiB**.
All 19 templates assembled to the modeled lengths. This is address arithmetic,
not permission to widen, reorder or repeat a pointee memory access.

The other eight indexed address operations and 46 unbounded CARD-indexed
loads/stores need full carry-aware scaled address construction. Their current
footprints are 944 and 2,837 bytes respectively; no saving is assigned here.

### 4. Native pieces for remaining control-flow copies

Word-only and pointer-only edge copies already have specialized handling.
The remaining fallback saves all edge sources into staging slots, then assigns
the destination block parameters byte by byte. **46 unconditional edges** with
LONG or mixed-width values admit smaller native pieces.

Keep that two-phase schedule and all staging storage. Copy complete words;
three-byte private values can use checked overlapping words. A local mode-cost
choice handles mixed BYTE/word pieces and retains the existing A16 boundary.
The model saves **873 bytes** from 2,155 bytes. Conditional edges and further
copy scheduling are excluded. This is a modest isolated slice, not a new
register allocator.

### 5. Direct BYTE arithmetic operands

All 217 BYTE Add/Sub/AND/OR/XOR sites still contain a right-operand scratch
round trip. For example, ignoring unchanged carry setup and destination store:

```asm
; Current: 8 bytes
LDA right,S
STA scratch
LDA left,S
ADC scratch

; Proposed: 4 bytes
LDA left,S
ADC right,S
```

Use the corresponding immediate, stack-relative or DP operand directly.
Each site saves **four bytes**, totaling **868 bytes**. Carry setup, A8 mode,
the result store and operand order remain unchanged. This is a small selector
slice with relatively little proof machinery. The 15 operation/operand-form
templates assembled with the expected lengths.

## What the large totals do and do not imply

The complete non-guard partition is in [footprint.csv](footprint.csv). Loads
still occupy 82,545 bytes, calls 59,154, stores 37,648, and comparisons with
associated control flow 33,537. Stack-relative instructions occupy 91,538
bytes and mode changes 39,694 bytes; these overlap the MIR partition.

Those totals include necessary work. In particular, mode changes and pointer
scratch setup are not independent pools of removable bytes. This audit does
not recommend general register allocation, memory-access relaxation, tail
calls, code outlining or source-specific exceptions to reach the current cap.

## Suggested small slices

1. Forward immutable incoming pointers into their final store address setup
   (model 4,592 bytes).
2. Forward immutable incoming pointers into their final direct-call packing
   (4,008 bytes), with changing-S and interruption coverage.
3. Extend the same terminal windows to non-addressable local pointers
   (5,968 bytes across address-base stores and calls).
4. Add stored-pointer-value consumers separately (456 bytes), then measure
   the same frozen image before deciding how much more work the cap needs.
5. Direct BYTE arithmetic operands are the smallest independent follow-up.
   Scalar captures, bounded indexed addresses and mixed-width copies provide
   further options.

The first two slices have an **8,600-byte model**, only 475 bytes more than the
current gap. Do not treat that small modeled margin as sufficient release
headroom; the local-pointer extension supplies a more useful buffer.

## Evidence and limitations

[ranking.csv](ranking.csv) records totals; [candidates.csv](candidates.csv)
records routine/block/operation locations; [examples.lst](examples.lst) keeps
current instruction sequences; [provenance.json](provenance.json) records
hashes and scope. [audit.py](audit.py) reproduces the offline counts from the
enriched probe image/inventory. [probe.rs](probe.rs) is the read-only Rust
inventory probe. [check_encodings.py](check_encodings.py) assembles the small
replacement templates; it does not execute them.

Checks performed: frozen-input hashes, exact rebuilt-image equality, complete
instruction decoding and non-overlapping span accounting, actual capture
instruction matching, private source/temporary extent separation for terminal
pointers, call stack-displacement bounds, cross-cohort overlap checks, and ca65
length assertions. Candidate selection uses printed typed MIR only in this
offline report; implementation must use the compiler's structured types and
checked emission APIs. Runtime equivalence remains implementation work.

Large generated inventories stay under `target/exec-ranking-53dd2af0/`.
No full compiler test suite, full backend qualification, or hosted Exec
qualification was run for this analysis.
