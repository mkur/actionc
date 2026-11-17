# Fact-driven source-home admission qualification

Production revision `5b1d5799`, compared with `6167df8b` and its
[wide ABI receipt](../6502-wide-routine-abi/README.md). This qualifies the
[home-admission implementation plan](../../MIR6502_FACT_DRIVEN_HOME_ADMISSION_IMPLEMENTATION_PLAN.md).

The change connects source-home discovery to existing helper contracts and
qualifies global/static word addresses and bounded constant word shifts.
Discovery still proposes bindings; resumed expansion, physical residence,
preservation replay and strict whole-image cost decide whether to keep them.
Byte/word source-object eligibility, incoming argument extent, ZP pools and
one combined cross-call trial are unchanged. LONG source objects stay inline.

## Workload measurements

All comparisons use optimized MIR6502 and ActionCart at matching origins.
Sources are unchanged. Every retained cross-call proposal passes the final
whole-image cost guard; no subset retry or per-routine veto was introduced.

| Program | Origin | Packaged bytes before → after | New named ZP homes |
| --- | --- | ---: | --- |
| TN | `$3000` | 9,913 → 9,913 | None; image is byte-identical |
| Huffman portable | `$2000` | 3,851 → **3,846** | `ReadBits.n` at `$E0/$E1` |
| Huffman fixed-address driver | `$8000` | 3,781 → **3,756** | `ReadBits.n` at `$E0/$E1`, `i` at `$E2/$E3` |
| Huffman portable without leaf inlining | `$2000` | 3,851 → **3,846** | `ReadBits.n` at `$E0/$E1` |

Only `ReadBits` changes instruction size: 665 → 662 bytes in portable builds,
and 734 → 713 in the driver. Removing two/four bytes of inline source homes
accounts for the remaining packaged saving. Its four-byte `value` remains
inline. The source-home trials are accepted as complete batches: 5 bytes
smaller in each portable build and 25 smaller in the driver.

TN retains its ten named-ZP routines: `Ord`, `Items`, `FindItem`, `PopUp`,
`Strcpy`, `Instr`, `Fnamecmp`, `Sort`, `Tag` and `PrintNonzeroDigit`. Temporary
occupancy is unchanged: TN uses 32 bytes across 13 routines; Huffman uses
36 bytes across seven routines in portable builds and 44 in the driver,
with reuse across routines.

Entry certificate counts remain 30 for TN, nine for portable Huffman and eight
for the driver. `ReadBits`, `ReadHeader` and `Decode` retain current complete
entry facts. All 14 Huffman helper sites and all four TN sites remain certified.
TN plain/counting/detailed-trace images agree; each Huffman traced image agrees
with the ordinary public output path.

## Remaining blockers

- Portable `ReadBits.i` is live across `ReadByte`, which writes the ordinary
  global `inputPos` (`g5`). The source-fixed overlap query cannot resolve that
  global's address, so its exact write footprint excludes every candidate ZP
  byte. In the driver `inputPos` is fixed at `$0604/$0605`, allowing the query
  to prove separation. This is a conservative overlap limit, not an unknown
  helper contract or exhaustion of the ZP pool. `n` dies after initializing `i`
  and does not need to survive those calls. See
  [fixed storage](../../../src/mir6502/analysis/known_callees/fixed_storage.rs)
  and [residence selection](../../../src/mir6502/parameter_homes/cross_call.rs).
- `ReadHeader` still hits the pointer-read/variable-word-arithmetic guard at
  retained block 4, operation 3 (`Add.w`). Record-stride multiplication also
  remains outside the qualified expansion set. Admitting its `Rsh.w 3` alone
  cannot make the body eligible.
- Portable `Decode` now passes its global-address admission gate. Both of its
  local candidates are rejected at the call to `WriteByte`, block 9, operation
  2, because returning writes are unknown. Dependency events also report
  unknown writes for `ReadHeader` and `BuildTree`; complete entry inputs do not
  establish preservation. The driver reaches the same `WriteByte` blocker.
- `BuildTree` retains unqualified multiplication/variable shift paths. `ReadBit`
  has no parameters or source locals. `Main` also has no source objects; its
  deterministic first rejection is the program-entry ABI guard, which precedes
  the empty-object check. These are placement decisions, separate from entry
  certificate availability.

The existing conservative replay remains: TN declines `Pull`'s expanded writes
after ZP remapping in six attempts. Each Huffman build declines `WriteByte`'s
expanded reads twice, once before home selection and once in the new candidate
continuation. The baseline had one such attempt because it made no source-home
trial. No fallback is weakened or converted into a claimed proof.

## Independent execution controls

Each control uses automatic placement in both ActionCart and standalone, with
leaf inlining disabled. Sizes below are emitted payload bytes, compared with
the relevant home-selection gate disabled; workload sizes include packaging.

| Control | Bytes with gate off → on | Coverage |
| --- | ---: | --- |
| Narrow home live across a variable LONG shift | 163 → 155 | 50 invocations; high bits, both word lanes, result guards |
| Global and literal addresses, inline table | 95 → 77 | Relocation at `$3000` and `$5000`, numeric ZP guard |
| Global and literal addresses, fixed table | 89 → 71 | Fixed table and the same relocation/guard checks |
| Word LSH 1 | 60 → 40 | In-place source update and a live word sibling |
| Word LSH 4 | 79 → 61 | Nibble crossing and wraparound |
| Word RSH 1 | 53 → 38 | High bits and byte carries |
| Word RSH 4 | 75 → 60 | Nibble crossing and wraparound |

The address control executes 64 invocations; the shift control executes 120,
including count zero. Zero shifts are an existing boundary control, not a new
optimization claim. Both providers produce the same control sizes. The unit
static-address case uses a real static ID in `LeaAddr`; source string arguments
normally use `StaticAddr` directly.

Admission tests also cover unsupported/stale helper bindings, malformed helper
operands, repeated helper-site gating, exposed parameters, numeric reservations,
variable/count-five word shifts and carry-sensitive/pointer-cell exclusions.
Forced helper-result conflicts in either lane reject the trial and restore the
complete selected artifact. Existing cost and residence checks remain active.

## Validation and reproduction

The final MIR6502 unit filter has 1,536 passes and the same six failures recorded
in the baseline, with 15 ignored. No baseline expectation was relaxed. Focused
checks passed 73 parameter-home tests, two word-shift expansion tests and ten
runtime-contract tests. Existing wide divide-by-zero execution also passed.
The final checks additionally passed 11 call-ABI integration cases, 22
residence/library/wide-ABI/admission VM tests, the existing word-shift execution
control and three Huffman tests (370 vector executions plus LF/CRLF checks).
The combined fact audit passed both tests, including 14 independent fact
controls. TN execution was not repeated because its complete image is identical
to the previously qualified baseline. [checks.json](checks.json) records the
counts, log hashes, commands and six unchanged failure names.

The [fact receipt](facts.json) and [comparison](qualification.json) retain
images' hashes, source hashes, per-routine code sizes, selected homes, current
contracts, rejection events and fallback events. Large images/listings stay in
`build/`. The TN probe now retains home-admission events as well as contract
fallbacks; it adds no production tracing pass.

Reuse the existing audit runner with disabled incremental builds:

```sh
CARGO_INCREMENTAL=0 python3 docs/benchmarks/6502-runtime-contracts/run.py
PYTHONDONTWRITEBYTECODE=1 python3 docs/benchmarks/6502-home-admission/summarize.py
```

The runner installs and removes its test-only probe. Do not build or edit
`known_callees.rs` concurrently. `summarize.py --checks` additionally records
the focused/final logs named in `checks.json`; ordinary receipt regeneration
does not require those logs. Testing stayed within the affected MIR6502
backend and existing controls' modes/runtimes; no full repository or unrelated
backend suite was run.
