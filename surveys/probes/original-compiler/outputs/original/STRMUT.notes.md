# STRMUT.COM observations

Source: `surveys/probes/original-compiler/strmut.act`

Original compiler output:

- `STRMUT.COM`
- Atari binary load segment: `$3000..$3033`
- `RUNAD` segment: `$02E2..$02E3 = $300C`

Probe intent:

- Confirm mutable indexed writes into initialized `STRING` storage.
- Confirm character literal assignment to string elements.
- Confirm copying string element bytes into a byte array.

Current actionc comparison:

- actionc now generates `outputs/actionc/strmut.hex`,
  `outputs/actionc/strmut.lst`, and `outputs/actionc/strmut.com`.
- The current `actionc` load file matches the VM-captured original output
  byte-for-byte.

Questions to answer from original output:

- Is initialized `STRING` storage mutable in-place? Yes.
- Does `text(1)='Z` lower like normal byte-array element assignment? Yes.
- Does copying from `STRING` to `BYTE ARRAY` use byte-array addressing only?
  Yes, for constant indexes it uses direct absolute byte loads/stores.

Observed layout:

| Symbol | Address / range | Notes |
| --- | --- | --- |
| `text` | `$3000..$3004` | `$04 "ABCD"` |
| `bytes` | `$3005..$3008` | 4-byte inline `BYTE ARRAY` storage; initial bytes 2-3 contain `$0004` |
| `a` | `$3009` | result byte |
| `b` | `$300A` | result byte |
| `c` | `$300B` | result byte |
| `Main` | trampoline `$300C`, body `$300F` | `RUNAD=$300C` |

Lowering:

```text
; text(1) = 'Z
LDA #$5A
STA $3001

; a = text(1)
LDA $3001
STA $3009

; bytes(0) = text(0)
LDA $3000
STA $3005

; bytes(1) = text(1)
LDA $3001
STA $3006

; b = bytes(0)
LDA $3005
STA $300A

; c = bytes(1)
LDA $3006
STA $300B
```

Conclusions:

- Initialized `STRING` storage is mutable in-place. `text(1)='Z` overwrites
  the first character byte at `$3001`; it does not allocate or copy storage.
- `STRING` element access with a constant index lowers the same way as
  `BYTE ARRAY` element access: direct absolute byte load/store at
  `base + index`.
- Index 0 remains the length byte. `bytes(0) = text(0)` copies `$04`, not the
  first character.
- Uninitialized sized global `BYTE ARRAY` storage carries the declared length
  as a little-endian word at offsets 2-3 when the array is large enough.
