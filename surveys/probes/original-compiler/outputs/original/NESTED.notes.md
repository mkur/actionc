# NESTED.COM observations

Source: `surveys/probes/original-compiler/nested_calls.act`

Original load-file layout:

- Code/data segment: `$3000..$304E`
- RUNAD segment: `$02E2..$02E3 = $3033`

Storage and routine entry layout:

- Global storage:
  - `g`: `$3000`
  - `r`: `$3001`
- `Inner`:
  - parameter `x`: `$3002`
  - public entry/trampoline: `$3003`
  - body starts at `$3006`
- `Outer`:
  - parameter `a`: `$3012`
  - parameter `b`: `$3013`
  - local `t`: `$3014`
  - public entry/trampoline: `$3015`
  - body starts at `$3018`
- `Main`:
  - public entry/trampoline/RUNAD: `$3033`
  - body starts at `$3036`

The original compiler emits each user routine as:

1. storage bytes for that routine's parameters and locals,
2. `JMP body`,
3. body code.

Calls target the trampoline, not the first body instruction. For example, `Outer` calls `Inner` with `JSR $3003`, and `Main` calls `Outer` with `JSR $3015`.

Argument passing:

- First byte argument is passed in `A`.
- Second byte argument is passed in `X`.
- The callee stores those registers into its own parameter storage at body entry.
- Function return value is still delivered through `$A0` for byte return values.

Current actionc comparison:

- actionc already uses the same register convention for byte arguments and `$A0` for byte returns.
- actionc now emits routine parameter/local storage immediately before each routine trampoline.
- Calls now target the trampoline label, so `Inner`, `Outer`, and `Main` line up with the original addresses for this probe when using zero-filled storage.
- Remaining byte differences include storage contents, small prologue/instruction ordering differences, and the original's extra trailing `RTS` before the RUNAD segment. Original Action! preserves ambient memory bytes in storage slots, while actionc emits deterministic zero bytes.
