# ARRPAR.COM observations

Source: `surveys/probes/original-compiler/array_params.act`

Original load-file layout:

- Code/data segment: `$3000..$309A`
- RUNAD segment: `$02E2..$02E3 = $3083`

Storage layout inferred from generated code:

- `ba(4)`: inline byte array storage at `$3000..$3003`
- `ca(4)`: card-array descriptor at `$3004..$3007`
  - `$3004..$3005`: backing data pointer, initialized to `$309B`
  - `$3006..$3007`: byte size, initialized to `$0008`
- `x`: `$3008`
- `w`: `$3009..$300A`
- `Touch` parameter frame:
  - `bp`: `$300B..$300C`
  - `cp`: `$300D..$300E`
  - `i`: `$300F`
- `Touch` trampoline: `$3010`
- `Touch` body starts after `SArgs` metadata, around `$301A`
- `Main` trampoline/RUNAD: `$3083`
- `Main` body: `$3086`

Array parameter ABI:

- `PROC Touch(BYTE ARRAY bp, CARD ARRAY cp, BYTE i)` is accepted by the original compiler.
- Array parameters are passed as base addresses, not as full descriptors.
- `Touch(ba, ca, 1)` passes:
  - `bp = $3000`, the base address of inline byte array `ba`
  - `cp = [$3004]`, the backing data pointer stored in card-array descriptor `ca`
  - `i = 1`
- The first bytes follow the normal call ABI:
  - `A`: `bp` low byte
  - `X`: `bp` high byte
  - `Y`: `cp` low byte
  - `$A3`: `cp` high byte
  - `$A4`: `i`
- `Touch` uses `JSR $A0F5` (`SArgs`) plus inline frame metadata to copy those argument bytes into its parameter frame.

Array parameter lowering inside `Touch`:

- `bp(i)` computes `bp + i` into `$AE/$AF`, then uses `($AE),Y`.
- `cp(i)` computes `cp + i*2` into `$AE/$AF`, then uses `($AE),Y`.
- Card stores/loads use `Y=1` for the high byte and `Y=0` for the low byte, matching `ARRAYS.COM` and `ARRREF.COM`.

Current actionc comparison:

- actionc now supports array parameters as two-byte pointer-backed references.
- actionc now passes sized byte arrays as their inline base address and sized card arrays as the backing pointer from their descriptor.
- actionc now uses the original `SArgs` metadata sequence for this parameter frame.
- actionc now binds descriptor-backed array storage labels just past the saved
  code without emitting the zero backing bytes into the load file, matching the
  original descriptor pointer and segment length.
- The current `actionc` load file matches the VM-captured original output
  byte-for-byte.

Likely compatibility work:

- Use this probe as a guard for array-parameter ABI, `SArgs` frame metadata,
  descriptor-backed array storage, and byte/card array indexing.
