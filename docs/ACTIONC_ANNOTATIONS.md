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
| `abi-inputs-only <16 hex digits>` | reviewed 6502 assembly accepts the ordinary Action! arguments with arbitrary incoming scratch registers, flags and scratch bytes |

Registers are conservative by default: unless a routine explicitly says it
preserves `A`, `X`, or `Y`, the state tracker invalidates those registers across
the call. Zero-page and absolute-memory facts are preserved only for known calls
whose effects do not write the referenced bytes.

Use symbolic names for source-owned locations whose addresses may move, for
example `kx/ky` or `Ioerr`. Symbolic `preserves`/`clobbers` items must resolve
to zero-page storage at codegen time. Symbolic `writes` items may resolve to
zero-page or absolute storage. Fixed ABI/runtime locations such as `$A0`,
`$AE/$AF`, or IOCB ranges should remain literal for clarity.

Classic codegen uses machine-block byte scanning for diagnostics. It may report observed
writes in maps or comparison output to help author annotations, but optimization
decisions trust source-level effects, explicit annotations, and built-in system
profiles rather than inferred machine-code effects. Unannotated machine blocks
act as conservative barriers.

### Reviewing an assembly entry for zero-page homes

Record this annotation after reviewing a straight-line leaf that returns
normally with `RTS`. Check that results, control flow and memory effects
depend only on its declared arguments and the program data it legitimately
accesses. Input lanes come from the signature. MIR6502 checks the actual call
lanes, result ABI and the callee's returning writes independently.

The hexadecimal fingerprint binds the review to the direct ABI, instruction
bytes and typed absolute relocations to ordinary globals, including each
referenced object's identity and byte extent. Addresses assigned during final
layout can move. A changed fingerprint disables that review. To obtain the
current value, put a placeholder of sixteen zeroes on the routine and inspect
the caller's rejected entry in a MIR6502 home-selection trace. Review the
implementation before recording the value. Subsequent builds check that value.

In TN, `Ascii` rotates the incoming carry into bit 3 and then masks it out
before indexing `asciiMap`. Its only data write is the byte result at `$A0`.
This lets `Convert` keep its pointer in a disjoint ZP pair across both calls.

## Current Use

The annotations feed `RoutineEffects` and `RoutineFacts`, so modern codegen can
reuse stable register facts, zero-page facts, prepared pointers, and known return
placement across annotated calls. This is intentionally metadata, not source
semantics; it should describe the routine accurately rather than request an
optimization.

The proposed shared contract covers how classic codegen and MIR6502 can
consume the same reviewed assembly facts. [Shared 6502 callee facts](6502_SHARED_CALLEE_FACTS_DESIGN.md)
