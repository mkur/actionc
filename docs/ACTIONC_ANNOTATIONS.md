# actionc Routine Annotations

`actionc` recognizes a small set of compiler-only comments immediately before a
routine declaration. They are ignored by original Action! source tooling because
they are comments, but `actionc` uses them to describe call ABI/effects for
machine blocks and hand-written runtime shims.

## Syntax

```action
;@actionc returns A=$A0
;@actionc preserves A X Y $AC/$AD $AE-$AF
;@actionc clobbers $A0/$A1 kx/ky
;@actionc writes Ioerr $0340-$03BF
PROC Foo=*()
[$60]
```

Supported forms:

| Annotation | Meaning |
| --- | --- |
| `returns A=$A0` | after the call, accumulator `A` contains return-slot byte `$A0` |
| `preserves A X Y` | listed CPU registers survive the call |
| `preserves $AE-$AF` | listed zero-page bytes/ranges survive the call |
| `preserves $AE/$AF` | slash form for a two-byte zero-page pair |
| `clobbers $A0/$A1` | listed zero-page bytes/ranges are written by the call |
| `clobbers kx/ky` | listed symbols are resolved to their current zero-page storage |
| `writes $0340-$03BF` | listed absolute address ranges are written by the call |
| `writes Ioerr` | listed symbols are resolved to their current storage range |

Registers are conservative by default: unless a routine explicitly says it
preserves `A`, `X`, or `Y`, the state tracker invalidates those registers across
the call. Zero-page and absolute-memory facts are preserved only for known calls
whose effects do not write the referenced bytes.

Use symbolic names for source-owned locations whose addresses may move, for
example `kx/ky` or `Ioerr`. Symbolic `preserves`/`clobbers` items must resolve
to zero-page storage at codegen time. Symbolic `writes` items may resolve to
zero-page or absolute storage. Fixed ABI/runtime locations such as `$A0`,
`$AE/$AF`, or IOCB ranges should remain literal for clarity.

Machine-block byte scanning is advisory only. `actionc` may report observed
writes in maps or comparison output to help author annotations, but optimization
decisions trust source-level effects, explicit annotations, and built-in system
profiles rather than inferred machine-code effects. Unannotated machine blocks
act as conservative barriers.

## Current Use

The annotations feed `RoutineEffects` and `RoutineFacts`, so modern codegen can
reuse stable register facts, zero-page facts, prepared pointers, and known return
placement across annotated calls. This is intentionally metadata, not source
semantics; it should describe the routine accurately rather than request an
optimization.
