# LOCALS.COM observations

Source: `surveys/probes/original-compiler/locals.act`

Original compiler output:

- `LOCALS.COM`
- Atari binary load segment: `$3000..$30F0`
- `RUNAD` segment: `$02E2..$02E3 = $30CA`

Probe intent:

- Confirm routine-local storage layout relative to parameter storage and the
  public routine trampoline.
- Confirm local scalar byte order for `CARD` locals.
- Confirm whether local `BYTE ARRAY` storage is inline in the routine frame.
- Confirm whether local pointer variables use the same two-byte low/high layout
  as globals.
- Confirm whether `SArgs` metadata points at the first parameter byte only, or
  at a frame that also includes locals.

Cases:

| Routine | Interesting storage |
| --- | --- |
| `LocalOnly()` | byte local, card local, local byte array, pointer local |
| `ParamLocal(BYTE a,CARD w)` | 3 parameter bytes plus byte/card locals |
| `CardLocal(CARD w)` | 2 parameter bytes plus card/byte locals |

Observed layout:

| Symbol | Address / range | Notes |
| --- | --- | --- |
| `gb` | `$3000` | global `BYTE` |
| `gr` | `$3001` | global `BYTE` |
| `gw` | `$3002..$3003` | global `CARD` |
| `gc` | `$3004..$3005` | global `CARD` |
| `gp` | `$3006..$3007` | global `CHAR POINTER` |
| `LocalOnly.lb` | `$3008` | local byte |
| `LocalOnly.lw` | `$3009..$300A` | local card |
| `LocalOnly.buf` | `$300B..$300D` | local inline `BYTE ARRAY(3)` |
| `LocalOnly.p` | `$300E..$300F` | local pointer |
| `LocalOnly` | trampoline `$3010`, body `$3013` | no params |
| `ParamLocal.a` | `$3061` | param byte |
| `ParamLocal.w` | `$3062..$3063` | param card |
| `ParamLocal.lb` | `$3064` | local byte |
| `ParamLocal.lw` | `$3065..$3066` | local card |
| `ParamLocal` | trampoline `$3067`, body `$306A` | `SArgs`, metadata `$61 $30 $02` |
| `CardLocal.w` | `$309A..$309B` | param card |
| `CardLocal.tmp` | `$309C..$309D` | local card |
| `CardLocal.flag` | `$309E` | local byte |
| `CardLocal` | trampoline `$309F`, body `$30A2` | direct 2-byte prologue |
| `Main` | trampoline `$30CA`, body `$30CD` | `RUNAD=$30CA` |

Conclusions:

- Routine locals are allocated immediately after parameters, before the routine
  trampoline.
- `SArgs` metadata points at the first parameter byte, not at a separate
  locals-only area. Locals follow the parameter bytes inside the same routine
  storage block.
- Local sized `BYTE ARRAY` storage is inline in the routine storage block.
- Local pointer variables use the same two-byte low/high storage shape as
  globals.
- The 3-byte `ParamLocal` frame still uses `SArgs` when locals are present.
- The 2-byte `CardLocal` frame still uses a direct prologue when locals are
  present.

Instruction-order observations:

- Original stores `CARD` constants and copies high byte first in many places,
  while preserving little-endian storage:

```text
lw = $2233:
  LDA #$22
  STA $300A
  LDA #$33
  STA $3009

gw = lw:
  LDA $300A
  STA $3003
  LDA $3009
  STA $3002
```

- Pointer assignment to the local byte array also stores high byte first:

```text
p = buf:
  LDA #$30
  STA $300F
  LDA #$0B
  STA $300E
```

Current actionc comparison:

- actionc emits globals first.
- actionc emits each routine's parameter/local storage immediately before that
  routine's trampoline.
- actionc now emits the same `SArgs` metadata sequence for the 3-byte
  `ParamLocal` frame.
- actionc now initializes local sized `BYTE ARRAY` storage with the original
  length-word convention at offsets 2-3 when those bytes fit.
- The current `actionc` load file matches the VM-captured original output
  byte-for-byte.

Questions to answer from original output:

- Does original use the same layout for local `CARD ARRAY` as global sized
  `CARD ARRAY`, or a different routine-local descriptor/backing placement?
