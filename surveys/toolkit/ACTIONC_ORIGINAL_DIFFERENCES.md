# Original ACTION! vs actionc

This note tracks source and code-shape differences that matter when comparing
the original Atari ACTION! compiler with actionc. It is intentionally separate
from the source files so modernized examples can stay readable while preserving
the rationale for each rewrite.

## Modes

Use three mental buckets:

| Bucket | Meaning | Expected source style |
| --- | --- | --- |
| Original compiler | Cartridge/DOS-era ACTION! compiler behavior captured through VM probes or extracted objects. | Accepts historical shorthand and loose address idioms. |
| actionc compat | actionc trying to preserve old layout/codegen behavior closely enough for compatibility comparisons. | Should accept original source where practical, but may differ in harmless code shape. |
| actionc modern | actionc using modern layout/MIR6502 and stricter typed lowering. | Prefer explicit pointer/address spelling and avoid old ambiguous idioms. |

Compat is the place to preserve old behavior. Modern is the place to make
address intent explicit rather than widening the language until every old
compiler convenience becomes a general rule.

## Source-Level Differences

### Routine Labels as Data Addresses

Original ACTION! accepts a bare routine/data label in some numeric-address
contexts:

```action
DLptr=DL15
```

That means "store the address of `DL15`" in old code. In actionc modern, prefer:

```action
DLptr=@DL15
```

Rationale:

- In modern actionc, a bare routine name is primarily callable.
- Accepting `CARD = PROC LABEL` globally would blur callable values and integer
  addresses.
- `@DL15` is explicit and matches the existing address-of syntax.

Compat may still need to accept the original shorthand for byte-for-byte source
coverage, but modernized sources should not rely on it.

### Machine-Block Address Bytes

Original sources often use `label^` inside machine blocks to emit address bytes:

```action
[78 screen^ ... 66 text^ ... 65 DL15]
```

Modernized actionc sources should spell byte payloads explicitly:

```action
[78 <screen >screen ... 66 <text >text ... 65 <DL15 >DL15]
```

Rationale:

- `<label` and `>label` make the emitted low/high bytes unambiguous.
- Listings are easier to read, and the MIR6502 raw machine-block path does not
  need to treat `^` as an extra historical address-byte shorthand.

### Pointer Arguments from Absolute Byte Aliases

Original Toolkit code sometimes declares an absolute byte alias and then passes
it as a pointer:

```action
BYTE GraphP0=$D00D
Zero(GraphP0,5)
```

Modern actionc should use an explicit address:

```action
Zero(@GraphP0,5)
```

or, when a literal address is clearer:

```action
Zero(BYTE POINTER($D00D),5)
```

Rationale:

- `GraphP0` as a value is the byte at `$D00D`.
- `@GraphP0` is the address `$D00D`.
- Allowing arbitrary byte scalar variables to pass as pointers would hide real
  type mistakes.

### Local DEFINE Before Local Declarations

Original ACTION! allows local `DEFINE` declarations before routine locals:

```action
PROC PutFinger(BYTE chr)
  DEFINE hbase="46",
         vbase="38"

  BYTE ARRAY Hpos(0)=[...]
```

Modernized actionc sources should hoist such constants to module/global scope,
or place them where they do not split the routine declaration prelude:

```action
DEFINE hbase="46",
       vbase="38"

PROC PutFinger(BYTE chr)
  BYTE ARRAY Hpos(0)=[...]
```

Rationale:

- The current parser treats routine declarations as a prelude followed by
  executable statements.
- A local `DEFINE` before arrays can make following declarations look like a new
  top-level construct.
- Even when represented in SemIR, `DEFINE` is metadata rather than executable
  NIR.

Compat may eventually grow first-class support for this exact original shape,
but modern source should avoid depending on it.

## Code-Shape Differences

### Compat

Compat output may still differ from the original compiler even when semantics
match. Known acceptable differences include:

- different zero-page scratch choices,
- branch inversion when target distances or canonical lowering differ,
- slightly shorter actionc sequences when an identity operation is folded,
- equivalent store ordering when byte-for-byte parity is not required.

When strict compat parity matters, compare against VM-captured original objects
and document the exact routine-level delta before adding a shape-preserving
special case.

### Modern

Modern output is allowed to differ more substantially:

- modern layout may remove entry trampolines and duplicate final `RTS`
  sequences,
- NIR optimization may remove identities like `x+0`,
- MIR6502 may use different temporary slots or direct materialization paths,
- modernized source copies may replace ambiguous old idioms with explicit
  pointer/address expressions.

Modern comparisons should focus on correctness and reusable quality mechanisms,
not byte-for-byte original compiler parity.

## MUSIC.DEM Modernization

The current modernized copy is:

```text
samples/toolkit/modern/MUSIC.DEM
```

It differs from the extracted original in these ways:

- `DLptr=DL15` became `DLptr=@DL15`.
- Display-list machine-block operands use `<screen >screen`, `<text >text`,
  and `<DL15 >DL15` instead of `screen^`, `text^`, and bare `DL15`.
- `hbase` and `vbase` moved from a local `DEFINE` in `PutFinger` to a global
  `DEFINE`.
- `PMG.ACT` was replaced with the existing `PMG.ACT` shim, which makes
  pointer arguments explicit.
- `IO.ACT` is included through an explicit relative path from the new survey
  location.

The modernized source currently compiles with:

```sh
cargo run --bin actionc-emit -- --backend mir6502 --emit-source-listing \
  samples/toolkit/modern/MUSIC.DEM
```

## KALSCOPE.DEM Modernization

The current modernized copy is:

```text
samples/toolkit/modern/KALSCOPE.DEM
```

It differs from the extracted original in these ways:

- The display-list cursor changed from `CARD ARRAY dl` to `BYTE POINTER dl`.
- Mixed-width `dl^` stores were rewritten as explicit byte writes. For example,
  `dl^=ystart` became `dl^=<ystart` followed by `dl^=>ystart`, with explicit
  cursor increments between bytes.
- Optional `CARD POINTER` arguments use `CARD POINTER(0)` instead of untyped
  literal `0`.
- `Open(7,"K:",4)` became `Open(7,keyboardDevice,4,0)`, with the device string
  named globally.
- Hardware and ABI aliases such as `memCtl`, `plotArgX`, `trig`, and `CH` were
  moved to global absolute aliases instead of local absolute declarations.
- The input buffer alias changed from local `STRING numBuf(0)=$550` to global
  `CHAR POINTER numBuf=$550`.

These rewrites preserve the original intent while avoiding old untyped cursor
stores, omitted arguments, local absolute storage ambiguity, and zero-length
absolute local string aliases.

The modernized source currently compiles with:

```sh
cargo run --bin actionc-emit -- --emit-source-listing \
  samples/toolkit/modern/KALSCOPE.DEM
```
