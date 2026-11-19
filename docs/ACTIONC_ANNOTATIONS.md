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

A full review starts with `reviewed-6502 <16 hex digits>`. Bare
`abi-inputs-only` declares that supplying the ordinary arguments suffices.
Full reviews admit forward conditional branches to local instruction boundaries
and a single terminal `RTS`. References to an ordinary global array can include
a constant offset within the array. The fingerprint includes that offset and the
array extent.
`reads-only` and `writes-only` bound all program-data accesses in their
respective direction. Use space-separated absolute addresses, absolute ranges,
or global array slices such as `asciiMap[0..3]` with inclusive byte offsets.
Use `none` for an empty set. Omitting a direction leaves it unknown.
`preserves` names registers and numeric zero-page ranges that survive a normal
return. `returns A=$A0` describes the returned byte. Both backends match the review to the selected implementation, ABI and referenced
tables before using it.

For a single `ASM OPAQUE` body, `reviewed-6502` uses a separate fingerprint
of the argument and result ABI, payload bytes and typed address relocations.
The relocation binds the target, byte selector, offset and operand use. Both
backends use the declared contract at this boundary. MIR uses those relocations
to identify explicit addresses even when a memory footprint is open.
This format accepts numeric addresses and local labels. A symbolic external
dependency requires a different review.
An opaque call may declare `preserves` with its general write footprint open.

A composed assembly review can call other reviewed routines by symbolic
`JSR`. Its fingerprint includes each target's name, validated implementation
fingerprint and declared guarantees. A stale child disables its callers'
reviews transitively. The reviewer checks each internal calling convention,
all return paths and stack balance. These bodies can use local loops,
multiple `RTS` exits and balanced `PHA`/`PLA`. Numeric calls, jumps out of
the routine and recursive review dependencies cannot supply a contract.

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

TN's `CalcAdr` sets its own carry and loads the screen base and coordinates
from program data. It writes only the screen pointer at `$E6/$E7` and defines both bytes on every normal return.
Its review omits `reads-only` because an unchecked row byte can index beyond
the 48-byte table. The compiler retains the indexed addresses in its storage
inventory while callers see an unknown read footprint.

## Current Use

The annotations feed `RoutineEffects` and `RoutineFacts`, so modern codegen can
reuse stable register facts, zero-page facts, prepared pointers, and known return
placement across annotated calls. This is intentionally metadata, not source
semantics; it should describe the routine accurately rather than request an
optimization.

The proposed shared contract covers how classic codegen and MIR6502 can
consume the same reviewed assembly facts. [Shared 6502 callee facts](6502_SHARED_CALLEE_FACTS_DESIGN.md)

### Reviewing an argument-relative buffer

A review may add `reads-buffer P N zero=256` or `writes-buffer P N zero=0`.
Both clauses accept either zero convention. The one-based formal parameter
`P` supplies a 16-bit address and `N` supplies an 8-bit byte count. The span
begins at that address and includes exactly the specified number of bytes,
including the stated interpretation of zero. List all additional fixed
accesses in the matching `reads-only` or `writes-only` clause.

For example, a reverse copy with arguments `(CARD dst, src BYTE count)`
declares `reads-buffer 2 3 zero=256` and `writes-buffer 1 3 zero=256`.
The buffer review accepts conditional branches to instruction boundaries
inside the routine and requires a single terminal RTS. The compiler checks
the identity of the body and instantiates its declared spans from the actual
ABI values or proven pointer ranges. Unresolved addresses, wraparound and
unsafe memory keep the calling optimization conservative.

A reviewed `defines-on-return $E6-$E7` guarantees a physical write to
each named byte on every normal return. It accepts numeric zero-page bytes
and inclusive ranges. If the review gives a `writes-only` footprint, list
these bytes among its fixed writes. A callee can still read a byte before
writing it. Entry obligations and read footprints describe those reads.
Native wrappers inherit the definition only along paths that establish it.

The optional `entry-reads abi` certifies that the implementation physically
observes only incoming argument lanes as entry requirements. A review can
certify ordinary argument sufficiency with `abi-inputs-only` even if an
instruction observes an incoming flag whose value cannot affect the outcome.

### Reviewed indirect destinations

`requires indirect-writes $0100-$FFFF` is a caller precondition. Every byte
written indirectly on a normal call must fall in the inclusive range,
including indexed, multi-row and wrapped addresses. The caller can establish
it as an API invariant. No runtime guard is emitted. The accompanying
`writes-only` clause supplies the complete additional direct and scratch
writes, including those of reviewed callees. Omitting or changing the
precondition invalidates the fingerprint. The range also participates in
transitive review identity. Code that calls the routine outside this domain
cannot rely on the contractual guarantees.
