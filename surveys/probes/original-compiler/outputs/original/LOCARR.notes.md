# LOCARR.COM observations

Source: `surveys/probes/original-compiler/locarr.act`

Original compiler output:

- `LOCARR.COM`
- Atari binary load segment: `$3000..$30CE`
- `RUNAD` segment: `$02E2..$02E3 = $30C7`

Probe intent:

- Confirm local sized `CARD ARRAY` and `INT ARRAY` storage layout.
- Compare local non-byte arrays against global sized non-byte arrays, which use
  descriptors plus backing storage.
- Confirm whether local non-byte array backing storage is inline in the routine
  frame, after the routine body, or descriptor-backed elsewhere.
- Confirm local dynamic indexing code shape for byte and non-byte arrays.

Current actionc comparison:

- actionc now emits local sized `CARD ARRAY` and `INT ARRAY` declarations as
  descriptors with post-segment backing storage, matching the original layout.
- actionc now uses absolute indexed addressing for compatible dynamic indexing
  of inline byte arrays.
- actionc generated `outputs/actionc/locarr.hex`,
  `outputs/actionc/locarr.lst`, and `outputs/actionc/locarr.com`.
- The current `actionc` load file matches the VM-captured original output
  byte-for-byte.

Observed layout:

| Symbol | Address / range | Notes |
| --- | --- | --- |
| `gb` | `$3000` | global `BYTE` |
| `gw` | `$3001..$3002` | global `CARD` |
| `gi` | `$3003..$3004` | global `INT` |
| `LocalArrays.i` | `$3005` | local byte |
| `LocalArrays.bytes` | `$3006..$3009` | local inline `BYTE ARRAY(4)` |
| `LocalArrays.words` | `$300A..$300D` | local `CARD ARRAY(3)` descriptor |
| `LocalArrays.words.data` | `$30D3..$30D8` | pointer target, 6 backing bytes |
| `LocalArrays.nums` | `$300E..$3011` | local `INT ARRAY(2)` descriptor |
| `LocalArrays.nums.data` | `$30CF..$30D2` | pointer target, 4 backing bytes |
| `LocalArrays` | trampoline `$3012`, body `$3015` | no params |
| `Main` | trampoline `$30C7`, body `$30CA` | `RUNAD=$30C7` |

Descriptor contents:

```text
words:
  $300A..$300B = $30D3  ; backing data pointer
  $300C..$300D = $0006  ; byte size

nums:
  $300E..$300F = $30CF  ; backing data pointer
  $3010..$3011 = $0004  ; byte size
```

Conclusions:

- Local sized `BYTE ARRAY(n)` is inline in the routine storage block.
- Local sized `CARD ARRAY(n)` and `INT ARRAY(n)` use the same 4-byte descriptor
  shape as global sized non-byte arrays.
- Local non-byte array descriptors live in the routine storage block, but their
  backing storage is assigned after the code segment.
- Local non-byte backing storage is assigned in reverse local declaration order;
  in this probe, `nums` backing precedes `words` backing even though the
  descriptors remain in declaration order.
- In this saved file, descriptor backing pointers target bytes immediately after
  the saved load segment:
  - segment ends at `$30CE`
  - `nums` data starts at `$30CF`
  - `words` data starts at `$30D3`
- Dynamic local byte-array indexing uses absolute indexed addressing:

```text
LDX i
LDA bytes,X
STA gb
```

- Dynamic local non-byte-array indexing uses `$AE/$AF` and `($AE),Y`, matching
  global descriptor-backed array access.
- Original stores and loads non-byte array elements high-byte-first through
  `Y=1`, then low-byte through `Y=0`, while preserving little-endian storage.

Questions to answer from original output:

- Confirm whether original saved load files normally omit descriptor backing
  bytes when those backing bytes are placed immediately after the code segment,
  or whether this is an artifact of the original save workflow.
