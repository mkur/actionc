# RETFLOW.COM observations

Source: `surveys/probes/original-compiler/retflow.act`

Original compiler output:

- `RETFLOW.COM`
- Atari binary load segment: `$3000..$30D9`
- `RUNAD` segment: `$02E2..$02E3 = $30A3`

Probe intent:

- Confirm code shape for early `RETURN` in `FUNC` and `PROC`.
- Confirm whether original emits immediate `RTS` at each return site or branches
  to a common epilogue.
- Confirm return value setup around nested `IF` and loop exits.
- Confirm multi-byte return setup in early-return branches.

Current actionc comparison:

- actionc generated `outputs/actionc/retflow.hex` and
  `outputs/actionc/retflow.lst`.
- actionc currently emits direct `RTS` at each return site.

Observed layout:

| Symbol | Address / range | Notes |
| --- | --- | --- |
| `g` | `$3000` | global `BYTE` |
| `h` | `$3001` | global `BYTE` |
| `w` | `$3002..$3003` | global `CARD` |
| `Pick.x` | `$3004` | param byte |
| `Pick` | trampoline `$3005`, body `$3008` | direct 1-byte prologue |
| `PickCard.x` | `$3034` | param byte |
| `PickCard` | trampoline `$3035`, body `$3038` | direct 1-byte prologue |
| `EarlyProc.x` | `$3055` | param byte |
| `EarlyProc` | trampoline `$3056`, body `$3059` | direct 1-byte prologue |
| `LoopReturn.limit` | `$3070` | param byte |
| `LoopReturn.i` | `$3071` | local byte |
| `LoopReturn` | trampoline `$3072`, body `$3075` | direct 1-byte prologue |
| `Main` | trampoline `$30A3`, body `$30A6` | `RUNAD=$30A3` |

Conclusions:

- Original Action! emits result setup immediately followed by `RTS` for each
  `RETURN(expr)` site.
- Original Action! emits immediate `RTS` for `PROC RETURN` inside an `IF`.
- `RETURN(expr)` inside a loop also emits direct result setup plus `RTS`; it
  does not branch to a shared function epilogue.
- Original preserves some unreachable control-flow structure after returns:
  `Pick` still contains a join jump after the `RETURN(2)` then-branch and still
  emits the final `RETURN(4)` after the exhaustive `IF`/`ELSE`.
- The saved segment ends with an extra trailing `RTS` byte after `Main`'s body
  `RTS`, matching the pattern seen in other original probes.

Return examples:

```text
Pick first return:
  LDA #$01
  STA $A0
  RTS

EarlyProc early return:
  LDA #$11
  STA g
  RTS

LoopReturn inside loop:
  LDA i
  STA $A0
  RTS
```

Multi-byte returns use high-byte-first setup, consistent with `RETURNS.COM`:

```text
PickCard true branch:
  LDA #$12
  STA $A1
  LDA #$34
  STA $A0
  RTS
```

Condition-shape observations:

- `IF x = 0` compiles as `LDA x` plus `BEQ`.
- `IF x = 1` / `IF i = 2` compiles as `LDA value`, `EOR #literal`, `BEQ`.
- `WHILE i < limit` uses `CMP limit` plus `BCC`.

Questions to answer from original output:

- Confirm whether `RETURN(expr)` in `DO`/`UNTIL`, `FOR`, and `ELSEIF` nests
  follow the same immediate-`RTS` shape. Current evidence says yes, but this
  probe covers `IF` and `WHILE` only.
