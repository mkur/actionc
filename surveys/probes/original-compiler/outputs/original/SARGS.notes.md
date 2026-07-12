# SARGS.COM observations

Source: `surveys/probes/original-compiler/sargs.act`

Original load-file layout:

- Code/data segment: `$3000..$3111`
- RUNAD segment: `$02E2..$02E3 = $310A`

Storage and routine layout inferred from generated code:

- globals:
  - `ga..gf`: `$3000..$3005`
  - `gw`: `$3006..$3007`
  - `gz`: `$3008..$3009`
  - `data(4)`: `$300A..$300D`
  - `cp`: `$300E..$300F`
  - `wp`: `$3010..$3011`
- `B3`:
  - params: `$3012..$3014`
  - trampoline: `$3015`
  - body starts at `$3018`
- `B4`:
  - params: `$3031..$3034`
  - trampoline: `$3035`
  - body starts at `$3038`
- `C2`:
  - params: `$3057..$305A`
  - trampoline: `$305B`
  - body starts at `$305E`
- `Mix`:
  - params: `$307D..$3080`
  - trampoline: `$3081`
  - body starts at `$3084`
- `Ptrs`:
  - params: `$30A3..$30A6`
  - trampoline: `$30A7`
  - body starts at `$30AA`
- `Caller`:
  - trampoline: `$30C9`
  - body starts at `$30CC`
- `Main`:
  - trampoline/RUNAD: `$310A`
  - body starts at `$310D`

Probe intent:

- Identify exactly when original Action! emits a direct register-to-frame
  prologue versus an `SArgs` helper prologue.
- Confirm frame metadata shape for byte-only, card, mixed, and pointer
  argument lists.
- Confirm caller-side ABI remains the same regardless of callee prologue style.
- Confirm nested call shape through `Caller()` -> helper routines -> `Main()`.

Cases:

- `B3(BYTE a,b,c)`: three argument bytes, should clarify whether `A`/`X`/`Y`
  can be direct-stored without `SArgs`.
- `B4(BYTE a,b,c,d)`: four byte arguments, first spill to `$A3`.
- `C2(CARD a,b)`: four bytes but two parameters, all scalar cards.
- `Mix(BYTE a,CARD b,BYTE c)`: mixed byte/card layout across `A`/`X`/`Y`/`$A3`.
- `Ptrs(CHAR POINTER p,CARD POINTER q)`: two pointer arguments, four bytes,
  adjacent to the `POINTERS.COM` result where original used `SArgs`.

Expected broad ABI shape from current findings:

- Argument bytes flatten left-to-right, low byte before high byte.
- Incoming byte offsets:
  - 0: `A`
  - 1: `X`
  - 2: `Y`
  - 3+: `$A3+`
- Calls target routine trampolines.
- Function/procedure parameter frames live before routine trampolines.

Current actionc comparison:

- actionc now emits original-style `SArgs` metadata for the probed
  parameterized routines.
- The caller-side byte placement matches original Action!.
- The current `actionc` load file matches the VM-captured original output
  byte-for-byte.

Observed original lowering:

- All probed parameterized `PROC`s use `JSR $A0F5`; no direct-store prologue
  appeared in this probe.
- The inline metadata after `JSR $A0F5` is:
  - low byte of frame base
  - high byte of frame base
  - argument byte count minus one
- Metadata by routine:
  - `B3`: `$12 $30 $02` for 3 bytes at `$3012`
  - `B4`: `$31 $30 $03` for 4 bytes at `$3031`
  - `C2`: `$57 $30 $03` for 4 bytes at `$3057`
  - `Mix`: `$7D $30 $03` for 4 bytes at `$307D`
  - `Ptrs`: `$A3 $30 $03` for 4 bytes at `$30A3`
- Caller-side ABI matches the existing model:
  - byte offset 0 in `A`
  - byte offset 1 in `X`
  - byte offset 2 in `Y`
  - byte offset 3 in `$A3`
- `C2`, `Mix`, and `Ptrs` copy multi-byte values from the parameter frame to
  destination globals high-byte-first.

Answers from this probe:

- `B3` uses `SArgs`; three byte parameters are not direct-stored here.
- `B4` also uses `SArgs`; it is not merely the first spilling case.
- `C2` uses `SArgs`.
- `Mix` uses `SArgs`.
- `Ptrs` uses `SArgs`, matching `POINTERS.COM`.
- The observed helper entry is consistently `$A0F5`.

Resolved compatibility work:

- Compatible parameter prologues use original-style `SArgs` metadata for the
  probed normal parameterized routines.
- Caller-side ABI remains unchanged and matches the original byte placement.
