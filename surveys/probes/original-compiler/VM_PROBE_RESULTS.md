# VM Probe Results

Generated with `action-compiler-vm/scripts/run-probe` against the original
Action! cartridge and OS ROM.

## Captures

| Probe | VM output | Result |
| --- | --- | --- |
| `optional_args.act` | `outputs/vm/OPTARGS.COM` | Compiled, 63 bytes |
| `sargs.act` | `outputs/vm/SARGS.COM` | Compiled, 286 bytes |
| `argthr.act` | `outputs/vm/ARGTHR.COM` | Compiled, 209 bytes |
| `arith.act` | `outputs/vm/ARITH.COM` | Compiled, 309 bytes |
| `array_assign.act` | `outputs/vm/ARRASN.COM` | Compiled, 127 bytes |
| `array_fixed_origin.act` | `outputs/vm/ARRFIX.COM` | Compiled, 18 bytes |
| `array_inline_boundary.act` | `outputs/vm/ARRTHB.COM` | Compiled, 1209 bytes |
| `array_inline_global_threshold.act` | `outputs/vm/ARRTHG.COM` | Compiled, 699 bytes |
| `array_inline_local_threshold.act` | `outputs/vm/ARRTHL.COM` | Compiled, 875 bytes |
| `drawframe.act` | `outputs/vm/DRAWFRM.COM` | Compiled, 413 bytes |
| `empty_proc.act` | `outputs/vm/EMPTYPR.COM` | Compiled, 28 bytes |
| `external_call.act` | `outputs/vm/EXTCALL.COM` | Compiled, 62 bytes |
| `fnamecmp.act` | `outputs/vm/FNAMECMP.COM` | Compiled, 290 bytes |
| `index_scaling.act` | `outputs/vm/IDXSCALE.COM` | Compiled, 260 bytes |
| `bool_edges.act` | `outputs/vm/BOOLEDGE.COM` | Compiled, 117 bytes |
| `record_args.act` | `outputs/vm/RECARGS.COM` | Compiled, 80 bytes |
| `record_ptr_order.act` | none | Expected original compiler failure, error 7 |
| `nested_calls.act` | `outputs/vm/NESTED.COM` | Compiled, 91 bytes |
| `precode_zp.act` | `outputs/vm/PREZP.COM` | Compiled, 29 bytes |
| `array_params.act` | `outputs/vm/ARRPAR.COM` | Compiled, 167 bytes |
| `strpass.act` | `outputs/vm/STRPASS.COM` | Compiled, 169 bytes |
| `strnam.act` | `outputs/vm/STRNAM.COM` | Compiled, 267 bytes |
| `stridx.act` | `outputs/vm/STRIDX.COM` | Compiled, 52 bytes |
| `returns.act` | `outputs/vm/RETURNS.COM` | Compiled, 234 bytes; matches hand capture |
| `layout_order.act` | `outputs/vm/LAYOUT.COM` | Compiled, 114 bytes |
| `large_local_arrays.act` | `outputs/vm/LGLARR.COM` | Compiled, 185 bytes |

## Array Inline Threshold

`ARRTHG.COM`, `ARRTHL.COM`, and `ARRTHB.COM` classify the original compiler's
sized `BYTE ARRAY` storage threshold.

- Global and local `BYTE ARRAY(255)` and `BYTE ARRAY(256)` use inline vtype
  `$9A`.
- Global and local `BYTE ARRAY(257)` use descriptor/backing vtype `$92`.
- The broader 128-byte increment probes confirm the descriptor/backing shape
  for 320 bytes and larger.
- Current `actionc` comparison:
  - `ARRTHL.COM` is byte-exact.
  - `ARRTHB.COM` has the same load size and threshold layout but differs in
    two inline local metadata pairs for `BYTE ARRAY(255)` and
    `BYTE ARRAY(256)`. In the captured original output, bytes 0-1 of those
    local inline arrays contain `$01,$32`; `actionc` zero-fills them. Bytes 2-3
    match the declared length metadata.
  - `ARRTHG.COM` has the same load size and marker/routine addresses, but the
    descriptor pointer words for large global byte arrays differ because the
    original chooses different unsaved backing addresses. Treat this as a known
    load-file difference, not as evidence of a different inline threshold.
- The combined `array_inline_threshold.act` stress probe captures useful local
  symbol snapshots, but it is too large for reliable VM save-output use; keep
  the split probes as the stable evidence.

## Current `actionc` Comparison Status

| Probe | `actionc` status |
| --- | --- |
| `optional_args.act` | Compiles; outputs in `outputs/actionc/optional_args.*` |
| `sargs.act` | Compiles; outputs in `outputs/actionc/sargs.*` |
| `argthr.act` | Compiles; outputs in `outputs/actionc/argthr.*` |
| `arith.act` | Compiles; outputs in `outputs/actionc/arith.*` |
| `array_assign.act` | Compiles; outputs in `outputs/actionc/array_assign.*` |
| `external_call.act` | Compiles; outputs in `outputs/actionc/external_call.*` |
| `fnamecmp.act` | Compiles; outputs in `outputs/actionc/fnamecmp.*` |
| `index_scaling.act` | Compiles; outputs in `outputs/actionc/index_scaling.*` |
| `bool_edges.act` | Compiles; outputs in `outputs/actionc/bool_edges.*` |
| `record_args.act` | Compiles; outputs in `outputs/actionc/record_args.*` |
| `record_ptr_order.act` | Compiles in `actionc` as an intentional sane-default divergence from the cartridge error |
| `nested_calls.act` | Compiles; outputs in `outputs/actionc/nested_calls.*` |
| `array_params.act` | Compiles; outputs in `outputs/actionc/array_params.*` |
| `bools.act` | Compiles; outputs in `outputs/actionc/bools.*` |
| `strpass.act` | Compiles; outputs in `outputs/actionc/strpass.*` |
| `stridx.act` | Compiles; outputs in `outputs/actionc/stridx.*` |
| `returns.act` | Compiles; outputs in `outputs/actionc/returns.*` |
| `layout_order.act` | Compiles; outputs in `outputs/actionc/layout_order.*` |
| `large_local_arrays.act` | Compiles; current output is an accepted large-local-storage divergence |

## Full Sweep Snapshot

Regenerated current `actionc` outputs for every probe with a VM capture and
compared Atari load files byte-for-byte. This sweep uses the cleaned
`stridx.act` named-string-index probe.

The current exact code-segment matches are:

| Probe | Result |
| --- | --- |
| `argthr.act` | Exact |
| `arith.act` | Exact |
| `abi_calls.act` | Exact |
| `abi_system_call.act` | Exact |
| `array_assign.act` | Exact |
| `array_params.act` | Exact |
| `arrays.act` | Exact |
| `array_fixed_origin.act` | Exact |
| `locarr.act` | Exact |
| `array_refs.act` | Exact |
| `booleq.act` | Exact |
| `bools.act` | Exact |
| `bool_edges.act` | Exact |
| `boolword.act` | Exact |
| `boolthen.act` | Exact |
| `control_flow.act` | Exact |
| `external_call.act` | Exact |
| `empty_proc.act` | Exact |
| `fnamecmp.act` | Accepted divergence; semantic guard for pointer-deref indirect-indexed comparisons |
| `functions.act` | Exact |
| `index_scaling.act` | Exact |
| `layout_order.act` | Exact |
| `locals.act` | Exact |
| `nested_calls.act` | Exact |
| `optional_args.act` | Exact |
| `pointers.act` | Exact |
| `precode_zp.act` | Exact |
| `records.act` | Exact |
| `retflow.act` | Exact |
| `returns.act` | Exact |
| `sargs.act` | Exact |
| `signedge.act` | Exact |
| `strinit.act` | Exact |
| `stridx.act` | Exact |
| `strlit.act` | Exact |
| `strloc.act` | Exact |
| `strmut.act` | Exact |
| `strnam.act` | Exact |
| `strpass.act` | Exact |
| `tn_value_index.act` | Accepted divergence; isolates TN `Value(v(i))` / `Value(v(i)+1)` / `Instr` drift |
| `ywalk.act` | Exact |

Current divergences:

| Probe | VM bytes | `actionc` bytes | Byte diff count | Current classification |
| --- | ---: | ---: | ---: | --- |
| `record_args.act` | 80 | 81 | 16 | intentional sane divergence from apparent original bad output |
| `fnamecmp.act` | 290 | 287 | many | accepted semantic guard; `actionc` is smaller but now emits `EOR (zp),Y` / `CMP (zp),Y` for pointer deref comparisons |
| `large_local_arrays.act` | 185 | 185 | 2 | accepted minor storage-byte divergence; both outputs skip large backing storage and use the same array-to-array pointer code |
| `tn_value_index.act` | 319 | 318 | many | accepted TN drift probe; `actionc` is 1 byte smaller after direct function-return constant compare and retained Y reuse |

Known current blockers or intentional divergences:

| Probe | Status |
| --- | --- |
| `record_ptr_order.act` | Original error 7; current `actionc` compiles intentionally |
| `record_args.act` | Current `actionc` preserves sane call semantics instead of reproducing apparent original bad output |
| `fnamecmp.act` | Current `actionc` keeps a smaller branch/register shape while matching the required indirect-indexed pointer-deref comparison semantics |
| `large_local_arrays.act` | Current `actionc` now emits the same `$3000-$30AC` compact code segment and skips large backing storage; remaining differences are two bytes that look like original uninitialized small local array residue |
| `tn_value_index.act` | New focused TN drift probe. The extra implicit `RTS` after machine-block tail routines has been fixed; `Value(v(i)+1)` now stages through the original-style indexed word plus constant shape; `Instr` now uses cached indexed FOR bounds and indexed byte `EOR` equality; function-return constant equality now branches directly. Remaining byte-count difference is a deliberate `Y=0` reuse where original reloads `LDY #0`. |

## Focused Sweep Snapshot

Regenerated current `actionc` outputs for the active comparison set after the
Y-walk constant-store work. This snapshot compares against the current VM
captures where available.

| Probe | VM segment | `actionc` segment | Segment mismatch | Code mismatch | Classification |
| --- | --- | --- | ---: | ---: | --- |
| `arith.act` | `$3000-$3128` / 297 | `$3000-$3128` / 297 | 0 | 0 | Exact against VM capture |
| `control_flow.act` | `$3000-$30B2` / 179 | `$3000-$30B2` / 179 | 0 | 0 | Exact against VM capture |
| `drawframe.act` | `$3000-$3190` / 401 | `$3000-$3190` / 401 | 0 | 0 | Exact against VM capture |
| `bools.act` | `$3000-$3120` / 289 | `$3000-$3120` / 289 | 0 | 0 | Exact against VM capture |
| `signedge.act` | `$3000-$3139` / 314 | `$3000-$3139` / 314 | 0 | 0 | Exact against VM capture |
| `array_params.act` | `$3000-$309A` / 155 | `$3000-$309A` / 155 | 0 | 0 | Exact against VM capture |
| `array_fixed_origin.act` | `$3000-$3005` / 6 | `$3000-$3005` / 6 | 0 | 0 | Exact against VM capture |
| `arrays.act` | `$3000-$30D6` / 215 | `$3000-$30D6` / 215 | 0 | 0 | Exact against VM capture |
| `locarr.act` | `$3000-$30CE` / 207 | `$3000-$30CE` / 207 | 0 | 0 | Exact against VM capture |
| `locals.act` | `$3000-$30F0` / 241 | `$3000-$30F0` / 241 | 0 | 0 | Exact against VM capture |
| `sargs.act` | `$3000-$3111` / 274 | `$3000-$3111` / 274 | 0 | 0 | Exact against VM capture |
| `stridx.act` | `$3000-$3027` / 40 | `$3000-$3027` / 40 | 0 | 0 | Exact against VM capture |
| `strmut.act` | `$3000-$3033` / 52 | `$3000-$3033` / 52 | 0 | 0 | Exact against VM capture |
| `strnam.act` | `$3000-$30FE` / 255 | `$3000-$30FE` / 255 | 0 | 0 | Exact against VM capture |
| `precode_zp.act` | `$3000-$3010` / 17 | `$3000-$3010` / 17 | 0 | 0 | Exact against VM capture |
| `empty_proc.act` | `$3000-$300F` / 16 | `$3000-$300F` / 16 | 0 | 0 | Exact against VM capture |

The `0 -> 1` Y-walk work moved the first `signedge.act` divergence past the
initial byte/word constant stores while keeping `arith.act` exact. Preserving
the `Y` hint across unrelated `LDA`/`STA` constant stores made `bools.act`
exact, and carrying the immediate true-label `Y=1` hint into compatible signed
branches made `signedge.act` exact.
Adding the original sized-byte-array length word at inline storage offsets 2-3
made `strmut.act` exact.
Binding descriptor-backed array labels without emitting zero backing bytes into
the load file made `array_params.act` exact.
Applying the sized-byte-array length-word convention to routine-local inline
byte arrays made `locals.act` exact; the same storage convention also made
`sargs.act` exact after regeneration.
Emitting local sized non-byte arrays as descriptors with reverse-ordered
post-segment backing storage, plus direct `absolute,X` indexing for local inline
byte arrays, made `locarr.act` exact.
Matching original byte-array dynamic store ordering, card-array scaled-index
address generation, and `Y` reuse across a word constant store made
`arrays.act` exact.
Rewriting `stridx.act` away from unsupported direct literal indexing and toward
named string storage made the named constant/dynamic string-index probe exact.
Matching original high-before-low pointer staging for pointer-like ABI argument
bytes and pointer constant stores made `abi_calls.act`, `abi_system_call.act`,
`external_call.act`, and `array_assign.act` exact.
Matching the original word increment-by-one peephole and compact word-zero
branch shape made `bool_edges.act` exact.
Branching directly on byte zero comparisons plus using `Y` for compatible
absolute byte zero stores made `retflow.act` exact; the same word
increment-by-one peephole also made `pointers.act` exact after regeneration.
Using the original `$AE` byte-index temporary plus `$AC/$AD` element pointer
for descriptor-backed array expression indexes made `index_scaling.act` exact.
Omitting the implicit compatible-mode `RTS` for a bodyless `PROC` made
`empty_proc.act` exact and matches TN's `PROC Error()` fallthrough pattern.
Computing non-zero pointer-backed record field offsets directly from the
pointer slot into `$AE/$AF` made `records.act` exact.

## Boolean Equality Focus Probes

Generated with the compiler VM and compared against current `actionc` output.

| Probe | VM segment | `actionc` segment | Segment mismatch | Code mismatch | Classification |
| --- | --- | --- | ---: | ---: | --- |
| `boolthen.act` | `$3000-$302E` / 47 | `$3000-$302E` / 47 | 0 | 0 | Exact |
| `booleq.act` | `$3000-$3078` / 121 | `$3000-$3078` / 121 | 0 | 0 | Exact |
| `boolword.act` | `$3000-$30B4` / 181 | `$3000-$30B4` / 181 | 0 | 0 | Exact |
| `ywalk.act` | `$3000-$3019` / 26 | `$3000-$3019` / 26 | 0 | 0 | Exact |

Findings:

- `boolthen.act`: the original carries `Y=#1` into the `IF a=b THEN` body and
  emits `STY eq_out` without reloading. In the following `IF a#b THEN` body it
  reloads `LDY #$01` before `STY ne_out`, so the carry is not a blanket
  "all labels preserve Y" rule. Current `actionc` now matches this probe
  exactly by applying an explicit byte-equality label hint.
- `booleq.act`: the original emits one `LDY #$01` and then multiple `STY`
  stores while `Y` remains known. Example: after `a=1`, it stores `b=1` with
  `STY $3001` instead of reloading `LDY #$01`. Current `actionc` now matches
  the straight-line adjacent-store case, byte-equality true arms, and
  byte-inequality true arms when the immediately preceding straight-line store
  kept `Y=#1` live.
- `boolword.act`: the original word equality/inequality test branches to the
  final `BEQ`/`BNE` when the low-byte `EOR` is non-zero. Current `actionc`
  now matches that shape exactly.
- `ywalk.act`: for `a=1 b=2 c=1 d=0`, original emits `LDY #1; STY a; LDA #2;
  STA b; STY c; DEY; STY d`. It does not walk upward with `INY` for `2`, but it
  keeps `Y=#1` live across the intervening `A` store and walks downward to zero.
  Current `actionc` now matches this probe exactly.

## Notes

- `optional_args.act` confirms the cartridge accepts omitted trailing
  arguments. The first call emits only `A=#1`; the second emits `X=#3`,
  `A=#2`, and leaves `Y` untouched.
- `optional_args.act` diff classification:
  - VM output is `$3000-$3032`; current `actionc` output is `$3000-$3032`.
  - The code segment now matches exactly after the vector-slot/SArgs update and
    compatible trailing `RTS` emission.
  - Caller-side omitted-argument behavior matches the current model: no bytes
    are synthesized for omitted trailing parameters.
- `sargs.act` diff classification:
  - VM output is `$3000-$3111`; current `actionc` output is `$3000-$3111`.
  - `SArgs` prologue targets and metadata now line up at the tested routine
    entries.
  - After matching original-style high-byte-first two-byte copies, direct
    immediate register staging, and trailing `RTS` emission, the current
    `actionc` output is also `$3000-$3111`.
  - The remaining difference is one storage/filler byte: the VM/original has
    `$04` at `$300C`, while current deterministic storage zero-fills it.
  - The VM/original hand-captured files differ in storage bytes only for this
    probe; the VM zero-fills regions where the hand capture preserved live
    memory.
- `argthr.act` diff classification:
  - VM output is `$3000-$30C4`; current `actionc` output is `$3000-$30C4`.
  - The direct-versus-`SArgs` threshold is now matched: two argument bytes use
    direct stores; three or more use `SArgs`.
  - The code segment now matches exactly after compatible trailing `RTS`
    emission.
- `arith.act` diff classification:
  - VM output is `$3000-$3128`; current `actionc` output is `$3000-$3128`.
  - Runtime helper targets for signed `*`, `/`, and `MOD` now use cartridge
    helper addresses instead of the `$04E8..$04EC` vector-slot addresses.
  - Compatible `CARD LSH` and `CARD RSH` now use the cartridge runtime helpers
    even for constant shift counts, matching the original ARITH probe.
  - Two-byte constant/copy order now follows the original high-byte-first shape
    in compatible mode.
  - Compatible immediate constant stores use `Y` for stable byte/value-1 and
    high-byte 0/1 cases. The executable code now matches the original ARITH
    probe; remaining differences are storage bytes.
- `bools.act` diff classification:
  - VM output is `$3000-$3120`; current `actionc` output is `$3000-$3120`.
  - `actionc` now accepts the bitwise `IF a AND b` and `IF a OR b` conditions.
  - The `AND`/`OR` condition fragments now match the original local shape:
    compute the bitwise result, store it in `$AE`, reload `$AE`, then branch on
    non-zero.
  - Byte equality, unsigned `CARD` comparisons, and signed `<`/`>=` condition
    fragments now use the original `EOR`, subtract/carry, and
    subtract/sign-flag branch shapes. The generated load file is exact against
    the VM capture.
- `control_flow.act` diff classification:
  - VM output is `$3000-$30B2`; current `actionc` output is `$3000-$30B2`.
  - Byte `=` / `#` conditions now use the original `EOR` plus branch shape,
    improving the `ELSEIF x = 1` and `UNTIL x = 6` control-flow fragments.
  - Byte `<=` loop bounds and compatible negative byte `FOR` steps now use the
    original reversed-carry and `ADC #$FF` shapes.
  - Compatible immediate constant stores use `Y` for stable byte/value-1 cases.
    The executable code now matches the original CONTROL probe; remaining
    differences are storage bytes.
- `signedge.act` diff classification:
  - VM output is `$3000-$3139`; current `actionc` output is `$3000-$313C`.
  - Compatible signed `<`/`>=` now use the original low-byte `CMP`, high-byte
    `SBC`, then `BMI`/`BPL` shape, including original overflow behavior.
  - Compatible 16-bit equality/inequality now uses the original low-byte `EOR`
    plus high-byte `ORA`/`EOR` branch shape.
- `array_assign.act` required the VM to implement `ORA (zp),Y` while compiling;
  after that it compiled and saved normally. This is a good comparison target
  for unsized array/string pointer assignment.
- `external_call.act` must declare global storage before the fixed-address
  `PROC` declaration. Placing variables after the external `PROC` caused an
  original compiler error before any object was saved.
- `external_call.act` currently avoids the `[]` empty-body marker because this
  cartridge rejected that form in the initial probe variant.
- `index_scaling.act` uses an uninitialized `INT ARRAY` plus assignments in
  `Main`; the cartridge rejected the original negative `INT ARRAY` initializer
  form with error 9. `actionc` now compiles the complex dynamic indexes in this
  probe; its load file still differs from the VM original and remains useful for
  byte-level compatibility work.
- `index_scaling.act` diff classification:
  - The original load file is `$3000-$30F7` while the current `actionc` load
    file is `$3000-$318D`.
  - The initialized array storage prefix now matches the original shape:
    `ba` bytes, `ba` pointer, `ca` bytes, `ca` pointer, then the uninitialized
    `ia` descriptor.
  - The original uses compact address sequences around `$AC/$AD` and `$AE/$AF`
    and reuses `Y` aggressively for low/high byte accesses.
  - Current `actionc` computes complex indexes through a 16-bit temp and then
    uses the existing indirect-indexed path. This is semantically plausible but
    not byte-shape compatible.
  - Recommended next compatibility slice from this probe is compact dynamic
    index/address lowering, especially matching the original `$AC/$AD` result
    pointer sequence and `Y` reuse.
- `bool_edges.act` needs the usual trailing `OD` after `DO ... UNTIL ...`.
- `record_args.act` compiles, so this cartridge accepts direct record fields as
  call arguments for this small case despite the manual errata warning.
- `record_args.act` diff classification:
  - VM output is `$3000-$3043`; current `actionc` output is `$3000-$3044`.
  - Current `actionc` now uses compact absolute register loads for direct
    record-field call arguments (`LDY abs`, `LDX abs`, `LDA abs`) instead of
    `LDA abs; TAY/TAX` transfer sequences.
  - The remaining mismatch appears to be an original compiler bug rather than a
    useful ABI rule: the cartridge output loads the second argument low byte
    from `$A1`, leaves the first `BYTE` record field out of `A`, and emits an
    unresolved `JSR $0000`. `actionc` intentionally keeps the sane/correct call
    semantics here.
- `record_ptr_order.act` reproduces the manual errata behavior:
  `PROC Touch(BYTE x, Pair POINTER p)` fails with error 7 when the record
  pointer parameter is not first.
- Step 4 call/return edge sweep:
  - `nested_calls.act` now matches exactly: both VM and current `actionc`
    output `$3000-$304E`.
  - `optional_args.act` now matches exactly: both VM and current `actionc`
    output `$3000-$3032`.
  - `argthr.act` now matches exactly: both VM and current `actionc` output
    `$3000-$30C4`.
  - `returns.act` VM output is `$3000-$30DD`; current `actionc` output is
    also `$3000-$30DD`, and the code segment now matches exactly after matching
    the cartridge's subtraction setup order (`SEC; LDA #0; SBC ...`).
  - `array_params.act` VM output is `$3000-$309A`; current `actionc` output is
    `$3000-$30A2`.
  - The executable code now matches the cartridge shape for byte and card array
    parameters: byte arrays use direct pointer-plus-index setup, card arrays
    use the original `ASL/PHP/PLP` scaled-index sequence, and two-byte
    indirect loads/stores reuse `Y` with `INY/DEY`.
  - The remaining differences are data/storage shape: the VM/original leaves
    `$04` in the `ba` storage prefix and does not include the eight zero bytes
    of `ca` backing storage in the saved load segment, while current `actionc`
    emits deterministic zero-filled backing storage.
  - `strpass.act` VM output is `$3000-$309C`; current `actionc` output is
    `$3000-$309C`, and the code segment now matches exactly. Matching this
    required direct pointer-plus-constant and pointer-plus-byte-index address
    setup, plus preserving known `Y=#0` across adjacent indirect loads.
  - `sargs.act` now uses direct immediate-to-register staging near the final
    SArgs call (`LDY/LDX #imm`). The only remaining mismatch is the known
    deterministic storage/filler byte at `$300C`.
  - `record_ptr_order.act` is intentionally excluded from byte comparison:
    the cartridge reports error 7 and writes no object, while current `actionc`
    compiles it under the sane-default record-pointer policy.
