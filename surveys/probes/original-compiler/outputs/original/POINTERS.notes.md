# POINTERS.COM observations

Source: `surveys/probes/original-compiler/pointers.act`

Original load-file layout:

- Code/data segment: `$3000..$30F8`
- RUNAD segment: `$02E2..$02E3 = $3065`

Storage layout inferred from generated code:

- `data(4)`: inline byte array storage at `$3000..$3003`
- `cp`: char pointer at `$3004..$3005`
- `wp`: card pointer at `$3006..$3007`
- `x`: `$3008`
- `y`: `$3009`
- `w`: `$300A..$300B`
- `z`: `$300C..$300D`
- `Touch` parameter frame:
  - `p`: `$300E..$300F`
  - `q`: `$3010..$3011`
- `Touch` trampoline: `$3012`
- `Touch` body starts after `SArgs` metadata at `$301B`
- `Main` trampoline/RUNAD: `$3065`
- `Main` body: `$3068`

Probe intent:

- Confirm global pointer storage layout for `CHAR POINTER` and `CARD POINTER`.
- Confirm array-name pointer value lowering for `data` and address-of lowering
  for `@z`.
- Confirm pointer dereference load/store lowering for:
  - char pointer store/load: `cp^ = value`, `x = cp^`
  - card pointer store/load: `wp^ = value`, `w = wp^`
- Confirm pointer arithmetic lowering for `cp ==+ 1`.
- Confirm pointer argument ABI for `PROC Touch(CHAR POINTER p, CARD POINTER q)`.

Expected broad ABI shape from current findings:

- Pointer values are two-byte addresses.
- Pointer arguments should flatten as low byte then high byte.
- For `Touch(data, @z)`:
  - `p` low should arrive in `A`
  - `p` high should arrive in `X`
  - `q` low should arrive in `Y`
  - `q` high should arrive in `$A3`

Observed original lowering:

- Pointer values are stored as two-byte addresses.
- Pointer assignments store high byte first, then low byte:
  - `cp = data` stores `$30` to `$3005`, then `$00` to `$3004`
  - `wp = @z` stores `$30` to `$3007`, then `$0C` to `$3006`
- Pointer dereference uses `$AE/$AF` as the temporary pointer pair, then
  accesses `($AE),Y`.
- `CHAR POINTER` dereference uses `Y=0`.
- `CARD POINTER` stores high byte first:
  - high byte through `Y=1`
  - low byte through `Y=0`
- `CARD POINTER` loads high byte first into `w+1`, then low byte into `w`.
- `cp ==+ 1` uses a pointer-specific increment peephole:
  - `INC cp`
  - branch if low byte did not wrap
  - `INC cp+1`
- `Touch(data, @z)` follows the expected ABI:
  - `A=$00`
  - `X=$30`
  - `Y=$0C`
  - `$A3=$30`
- `Touch` uses the original `SArgs` path (`JSR $A0F5`) plus inline metadata
  before the body, even though the argument list is only four bytes.

Current actionc comparison:

- actionc compiles this probe and emits deterministic global/routine storage.
- actionc treats a bare array name in pointer-value position as the array base
  address, matching common original Action! source idioms.
- actionc now dereferences pointers through `$AE/$AF` in compatible mode.
- actionc passes pointer arguments with the existing Action-compatible argument
  byte ABI.

Likely compatibility questions:

- Does original use direct `SArgs` metadata for pointer parameters when argument
  bytes spill past `A`/`X`/`Y`?
- Implement original-style high-byte-first card pointer dereference ordering.
- Implement pointer increment/decrement peepholes for `==+ 1` / `==- 1`.
- Consider high-byte-first pointer address assignment where byte-for-byte
  comparison matters.
