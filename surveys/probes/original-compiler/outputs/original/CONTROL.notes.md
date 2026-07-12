# CONTROL.COM observations

Source: `surveys/probes/original-compiler/control_flow.act`

Original load-file layout:

- Code/data segment: `$3000..$30B2`
- RUNAD segment: `$02E2..$02E3 = $3006`

The first six bytes of the segment are global storage:

- `i`: `$3000`
- `x`: `$3001`
- `w`: `$3002..$3003`
- `n`: `$3004..$3005`

The original compiler does not zero-fill this storage in the load file. It emits whatever bytes are already present in that region, then points RUNAD at the first executable byte after storage.

Control-flow lowering notes:

- The original emits the same broad branch shape as actionc: conditional branch to the then/body path, otherwise an absolute `JMP` to the alternate/exit path.
- `EXIT` lowers to a direct `JMP` to the active loop exit label.
- `FOR i = 1 TO 3` uses an inclusive unsigned compare, then increments `i` with `INC $3000`.
- `FOR i = 3 TO 1 STEP -1` compares the lower bound against the loop variable and exits when the lower bound is greater/equal according to the generated branch pattern, then decrements by adding `$FF`.
- `x ==+ 1` is optimized to `INC $3001` in several loop bodies.
- `w = 12 * 34` calls the runtime multiply helper at `$A000`; result low byte is in A and high byte is in X.
- `n = -5` is emitted as immediate two-byte storage of `$FFFB`.

Current actionc comparison:

- The current compatible output places storage in the segment and uses `$3000`-based absolute references.
- It deliberately zero-fills storage, so storage bytes differ from the original's ambient memory bytes.
- It now emits original-style `INC abs`/`DEC abs` forms for byte `==+ 1`, `==- 1`, and byte `FOR` steps.
- It now calls the original multiply helper at `$A000` for `w = 12 * 34` instead of folding the constant product.
- Byte equality conditions now use the original `EOR` plus `BEQ/BNE` branch
  shape, which tightens the `ELSEIF x = 1` and `UNTIL x = 6` fragments.
- Byte `<=` in positive `FOR` bounds now uses the original reversed carry
  shape (`LDA bound`, `CMP target`, `BCS body`).
- Compatible negative byte `FOR` steps now use the original `CLC`, `ADC #$FF`
  decrement shape instead of the ordinary `DEC` peephole.
- Runtime helper setup for constant left high bytes now uses direct `LDX #imm`
  rather than `LDA #imm` plus `TAX`.
- Compatible byte constant `1` stores now use `LDY #1` / `STY`, matching the
  stable original pattern in this probe.
- Current comparison after the loop-shape update:
  - VM output is `$3000-$30B2`; current `actionc` output is also
    `$3000-$30B2`.
  - The executable code now matches the original probe. Remaining differences
    are the initial storage bytes.
