# Wide routine ABI qualification

Qualified 2026-11-17 at production revision `3e26c489`, compared with `7c8110aa`
and its [runtime-contract receipt](../6502-runtime-contracts/README.md).
This completes the [implementation plan](../../MIR6502_WIDE_ROUTINE_ABI_IMPLEMENTATION_PLAN.md).

## Measured outcome

All four packaged images, routine code sizes, named ZP homes and temporary ZP
allocations are unchanged. Sources and image hashes are recorded in
[qualification.json](qualification.json). Comparisons use optimized MIR6502
and ActionCart at the same origins as the baseline.

| Program | Origin | Packaged bytes before → after | Routines with named ZP homes | Entry certificates before → after |
| --- | --- | ---: | ---: | ---: |
| TN | `$3000` | 9,913 → 9,913 | 10 → 10 | 30 → 30 |
| Huffman portable | `$2000` | 3,851 → 3,851 | 0 → 0 | 6 → **9** |
| Huffman fixed-address driver | `$8000` | 3,781 → 3,781 | 0 → 0 | 5 → **8** |
| Huffman portable, leaf inlining disabled | `$2000` | 3,851 → 3,851 | 0 → 0 | 6 → **9** |

`ReadBits`, `ReadHeader` and `Decode` gain complete composed entry certificates
in every Huffman configuration. No routine loses a certificate. `ReadBits`
retains one CARD parameter and one LONGCARD result:

```text
parameters: [Scalar(Word)]
result: Scalar(Long)
entry inputs: A, X; no flags or fixed-byte requirements
guaranteed result bytes: $A0, $A1, $A2, $A3
```

The fact repair propagates through both callers. It does not expand the
source-object placement policy. All 14 Huffman helper sites and TN's four
helper sites remain certified. Public result slots remain distinct from the
private wide-helper result slots at `$C4–$C7`.

TN retains named homes in `Ord`, `Items`, `FindItem`, `PopUp`, `Strcpy`, `Instr`,
`Fnamecmp`, `Sort`, `Tag` and `PrintNonzeroDigit`. Temporary allocation remains
32 bytes across 13 routines. Huffman retains 36 temporary bytes across seven
routines in portable configurations and 44 in the driver, with reuse across
routines. Packaged bytes exclude uninitialized storage.

## Remaining blockers and fallback

- The generic body/signature rejection covers distinct causes: `ReadBit` and
  `Main` have no parameters or source locals to place; `ReadBits` encounters the
  explicit-helper exclusion; `ReadHeader` encounters arithmetic/pointer guards;
  portable `Decode` encounters global-address formation. These admission gates
  and eligibility for four-byte source objects were deliberately unchanged.
- Driver `Decode` now has complete descendant entry inputs. Its two local-home
  candidates still fail across `WriteByte` because that callee's returning
  writes are unknown. `ReadHeader` and `BuildTree` also retain unknown writes;
  entry completeness does not grant preservation.
- Local-only leaf discovery and competition with temporary ZP remain separate
  limits. `ReadHeader` already uses all 16 temporary pool bytes.
- Public long conversion/I/O declarations now have complete ABI metadata;
  buffer effects, conversion scratch and OS dependencies still need their own
  proofs. No blanket library contract was added.

The existing layout fallback remains: TN declines `Pull`'s expanded writes
after ZP remapping and selects conservative replay (six recorded attempts).
Each Huffman configuration declines `WriteByte`'s expanded reads and selects
conservative replay. No fallback was converted into an unsupported success.

TN's plain, counted and detailed-trace images agree. Each Huffman traced image
matches the ordinary public output path. All selected images also match the
baseline byte for byte.

## Validation

| Check | Result |
| --- | --- |
| Full MIR6502 library-unit filter | 1,531 passed; six unchanged baseline failures; 15 ignored |
| `mir6502_call_abi` integration | 11 passed |
| New `wide_abi_homes` VM target | Two passed; both runtimes |
| Existing wide nested direct/typed-indirect call filter | One passed |
| Existing parser function-pointer filter | One passed |
| `cross_call_homes` VM target | 14 passed |
| `runtime_library_contracts` VM target | Three passed |
| Huffman MIR6502/provider and newline controls | Three passed; 370 vector executions |
| Combined TN/Huffman fact audit | Two passed, including 14 independent fact controls |

The new public test covers twelve real SYS/math APIs and a LONG/pointer/SIZE
signature control. `CSTRING.u32toa` was removed from the original MIR6502
inventory after confirming that CSTRING is native-65816-only. No target
restriction was bypassed. The execution control keeps a narrow caller home in
ZP across first/shifted LONG arguments, following arguments and SArgs capture;
it checks signed bit patterns, wraparound, complete results and memory guards.

Huffman uses 185 vectors per provider, cartridge LF and standalone CRLF through
the actual compiler/VM loader, plus the vector-normalization control. TN VM
tests were not repeated because its full image is unchanged from the previous
qualification. Existing control targets retain their original mode coverage.

The six unit failures match the previous qualification's independently
reproduced baseline failures exactly. No expectations were relaxed.
[checks.json](checks.json) records their names, counts and log hashes. This was
MIR6502 qualification; no shared frontend/NIR contract changed and no full
repository or other-backend suite was run.

## Receipt and reproduction

- [facts.json](facts.json): selected per-routine ABI, entry/result facts,
  effects, named/temporary homes and helper/library contracts.
- [qualification.json](qualification.json): baseline/current comparison,
  source/image hashes, configurations, trace equality and fallback events.
- [checks.json](checks.json): validation results and baseline failure reference.

Run from the repository root:

```sh
env CARGO_INCREMENTAL=0 python3 docs/benchmarks/6502-runtime-contracts/run.py
python3 docs/benchmarks/6502-wide-routine-abi/summarize.py
```

The reused runner temporarily includes the existing probes and removes the
include in `finally`. Do not build or edit `known_callees.rs` concurrently.
The new summarizer preserves the previous receipt and checks `ReadBits`'s
complete metadata and four defined result bytes. Large raw images, listings,
MIR and traces remain under ignored `build/`. No diagnostic hook remains in
production.
