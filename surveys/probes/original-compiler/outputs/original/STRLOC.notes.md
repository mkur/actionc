# STRLOC.COM observations

Source: `surveys/probes/original-compiler/strloc.act`

Original compiler output needed:

- `STRLOC.COM`
- Atari binary load segment: `$3000..$3031`
- `RUNAD` segment: `$02E2..$02E3 = $302A`

Probe intent:

- Confirm local `STRING` initializer storage.
- Confirm local `CHAR ARRAY` string initializer storage.
- Compare local initialized string arrays with global initialized string arrays.
- Confirm local string storage relative to routine trampoline/body.

Current actionc comparison:

- Original-first probe for now.
- actionc parses/analyzes the source, but codegen does not yet support local
  initialized string/named `CHAR ARRAY` layout or indexed string reads.
- No actionc hex/listing was generated.

Questions to answer from original output:

- Does local `STRING local(0)="LOCAL"` auto-size in the routine storage block?
  Yes.
- Is local initialized string storage inline? Yes, immediately before the
  routine trampoline.
- Are local string initializer bytes included in the saved load segment? Yes.

Observed layout:

| Symbol | Address / range | Notes |
| --- | --- | --- |
| `g0` | `$3000` | result byte |
| `g1` | `$3001` | result byte |
| `g2` | `$3002` | result byte |
| `g3` | `$3003` | result byte |
| `LocalStrings.local` | `$3004..$3009` | `$05 "LOCAL"` |
| `LocalStrings.raw` | `$300A..$300D` | `$03 "XYZ"` |
| `LocalStrings` | trampoline `$300E`, body `$3011` | local storage precedes trampoline |
| `Main` | trampoline `$302A`, body `$302D` | `RUNAD=$302A` |

Lowering:

```text
; g0 = local(0)
LDA $3004
STA $3000

; g1 = local(1)
LDA $3005
STA $3001

; g2 = raw(0)
LDA $300A
STA $3002

; g3 = raw(2)
LDA $300C
STA $3003
```

Conclusions:

- Local initialized `STRING` and initialized `CHAR ARRAY` values are emitted as
  fixed storage in the load segment before the owning routine trampoline.
- `STRING local(0)="LOCAL"` auto-sizes to length byte plus characters:
  `$05 "LOCAL"`.
- `CHAR ARRAY raw(3)="XYZ"` stores `$03 "XYZ"`: the declared size is the
  string character count/capacity, while storage includes the extra length
  byte.
- Constant indexed reads from local initialized strings lower to direct
  absolute loads from `base + index`.
