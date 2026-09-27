# Native address selection measurements

The [implementation plan](../../MIR65816_ADDRESS_SELECTION_PLAN.md) records scope
and the frozen compiler/Exec baseline. JSON snapshots include source and artifact
hashes, raw/optimized code/file/descriptor sizes, relocations and routine frames.
Code bytes include unchanged debug guards; o65 files also include loader metadata.

| Checkpoint | HELLO optimized code / file | CAT optimized code / file |
| --- | ---: | ---: |
| Baseline | 929 / 2,359 | 3,749 / 7,403 |
| Slice 1: direct symbolic AddressOf | 881 / 2,311 | 3,689 / 7,343 |
| Slice 2: constant address chains | 826 / 2,256 | 3,634 / 7,288 |
| Slice 3: direct constant-index BYTE access | 714 / 2,066 | 3,634 / 7,288 |
| Slice 4: CARD-indexed BYTE loads | 714 / 2,066 | 3,513 / 7,167 |
| Slice 5: CARD-indexed BYTE stores | 714 / 2,066 | 3,449 / 7,103 |

Slice 1 saves 48/60 bytes in HELLO/CAT in both raw and optimized modes. Frames,
guard bytes, relocation counts, initialized data, BSS and DP reservations are
unchanged. Reserved bank-zero delta: **zero**, including per-task reservations.

Development validation: two selector unit tests; 17 integration/CLI tests across
`mir65816_address_selection`, `mir65816_o65`, `actionc_65816_o65_cli`; nine native
VM tests across `address_selection`, `pointer_values`, `replay`. Runtime cases
cover raw/optimized compilation, fixed/o65 placements, exact address lanes and
canaries; the existing pointer cases include IRQ/NMI. CLI checks include LF/CRLF.
The native runner uses the pinned CPU and enables state-proof/replay checking.

Command size measurements use real command sources and interfaces. Diagnostic
fixed links used to inspect code have placeholder provider addresses and are
not executed. Full backend/Exec qualification was reserved for the final slice
and subsequently stopped at the user's request.

Slice 2 saves another 55 bytes in each command in both modes. Three selector
tests, 13 integration/o65 tests and 16 native VM tests pass. The focused address
target additionally executes one-past fallback with the object ending exactly
at the 24-bit bus boundary. Shared/repeated uses, returned bases, calls, loaded
pointers and cross-block uses preserve producers or reject folding. The native
o65 suite covers independent ABI providers, banked data, tasks and IRQ/NMI;
fresh replay passes. Frames, guards, relocation counts and bank-zero reservations
remain unchanged.

Slice 3 saves another 112 HELLO code bytes in each mode. Its file saves 190:
112 code, 64 relocation-proof bytes and 14 ordinary relocation bytes. CAT is
unchanged. Three selector tests, 14 integration/o65 tests and 13 native VM tests
pass. New traces check every target byte read/write around a mutating call in
both ordinary and volatile cases, including untouched array neighbors. One-past
accesses retain the existing path. Frames, guards and bank-zero reservations
remain unchanged.

Slice 4 saves 121 CAT code/file bytes in both modes, with HELLO unchanged.
Four focused integration tests cover admission and signed/wide/scaled fallback.
Fourteen native VM tests across `address_selection`, `loop_x`,
`checked_rewrites` and `instruction_effects` pass. Full CARD boundaries,
bank carries, 24-bit wrap and relocated symbols preserve exact BYTE read traces.
Frames, guards, relocation counts and bank-zero reservations remain unchanged.

Slice 5 saves 64 optimized CAT code/file bytes. Raw CAT saves 69 code bytes and
70 file bytes; the extra file byte comes from relocation-stream delta encoding
after layout changes. HELLO is unchanged. Five focused integration tests and
eight address-selection VM tests pass, including all BYTE values, exact
read-before-write traces, relocated symbol stores, and IRQ/NMI re-entry at
reached load/store preparation instructions. Frames, guards and bank-zero
reservations remain unchanged.

Across the series, optimized HELLO code shrinks by 215 bytes (23.1%) and its file
by 293 bytes (12.4%). CAT code and file each shrink by 300 bytes (8.0% and 4.1%).
These figures retain the existing debug guards and ABI.

The [cycle probe](cycles.json) uses the same serialized fixed image layout,
caller, inputs and exact memory-access oracle at every checkpoint. It exercises
a symbol-plus-offset address, constant BYTE read/write, and CARD-indexed pointer
read/write across a bank boundary. Both compiler modes and incoming I states
produce the following results; cycles include the unchanged caller and guards.

| Checkpoint | Probe code bytes | VM cycles |
| --- | ---: | ---: |
| Baseline | 442 | 761 |
| Slice 1 | 406 | 707 |
| Slice 2 | 351 | 618 |
| Slice 3 | 245 | 444 |
| Slice 4 | 221 | 408 |
| Slice 5 | 197 | 372 |

Every slice reduces both metrics on this probe. These are compiler-probe cycles,
not end-to-end HELLO/CAT timings.

Final qualification exposed an existing preemption-test assumption: it required
CLC/SEC immediately before every stack-relative ADC/SBC, including LONGCARD's
upper word, which must retain the lower word's carry/borrow. The baseline compiler
reproduced the same assertion failure. The test now uses MIR source spans solely
to identify standalone word arithmetic for that shape assertion; it still injects
IRQ at every reached enabled instruction and checks independent final results.
The corrected test passes on both baseline and candidate compilers.

The [check record](qualification.json) separates completed checks from the
interrupted final matrix. Completed compiler checks: 247 unit tests and 74
integration/CLI tests (five existing opt-in inventory tests ignored). Completed
Exec groups: core raw/optimized, boundary raw/optimized, and 128-byte-sector raw
concurrency with trace/replay. The full native VM rerun and DOS raw group were
stopped; release VM and remaining hosted/command groups were not run. This is
not a full backend or Exec qualification result. Exec's compiler pin is unchanged.
