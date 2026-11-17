# MIR6502 complete integer routine ABI metadata

Status: implemented and qualified, 2026-11-17.

The [implementation plan](MIR6502_WIDE_ROUTINE_ABI_IMPLEMENTATION_PLAN.md)
delivers the metadata, proof consumers, runtime controls and measured qualification
in separate commits.
The [qualification](benchmarks/6502-wide-routine-abi/README.md) records the
three new Huffman entry certificates and unchanged TN/Huffman images and homes.

## Decision

Extend the existing direct routine ABI metadata to describe `LONGINT` and
`LONGCARD` as single four-byte integer parameters and results. Separate ABI
value width from MIR instruction width. Retain the current calling convention,
byte/word operations, parameter identities and memory layout.

Parameter metadata and result metadata are independent facts. Failure to
describe one must not erase the other. Use one shared projection of the direct
ABI description for byte counts, physical argument homes and result layout.

This extends the [typed direct result contract](MIR6502_CALL_FACT_CONTRACT_DESIGN.md#typed-direct-result-contract)
and completes the public integer ABI boundary left separate by the
[runtime-helper contract work](MIR6502_RUNTIME_HELPER_CONTRACT_DESIGN.md).
It grants no new memory-effect, preservation or flag guarantees merely from a
type declaration.

## Problem and affected routines

At the [Huffman baseline](benchmarks/6502-runtime-contracts/README.md), every
arithmetic-helper site was certified, but `ReadBits(CARD n)` lost its direct
parameter and result metadata because it returns LONGCARD. Its callers could
not obtain complete composed entry facts.

The limitation is general:

| Signature or public API | Missing facts at the baseline |
| --- | --- |
| `LONGCARD FUNC ReadBits(CARD n)` | Wide result, plus the otherwise supported word parameter |
| `SYS.ValLC`, `InputLC`, `InputLCD` and signed `LI` equivalents | Four-byte result; parameter discovery also fails |
| `SYS.StrLC`/`StrLI` and long-integer print families | Four-byte parameters; void result remains known |
| `MATH.Q4_12.SqrWide(INT)` | Four-byte result also suppresses the narrow parameter |
| `MATH.INTEGER.AsrLI(LONGINT, BYTE)` | Four-byte parameter and result |

The math routines may be inlined; the call-boundary limitation matters when
their calls survive. Pointer-based REAL APIs do not have this wide-integer
signature problem. Their floating-point services and memory effects remain
separate coverage questions. `CSTRING.u32toa`, previously included in this
inventory, belongs to the native 65816 library and is outside this backend's
scope. A 6502 control covers the analogous LONG/pointer/SIZE signature.

The emitted ABI already handles wide calls. [Call lowering](../src/mir6502/lower.rs)
and [call planning](../src/mir6502/call_plan.rs) split a wide argument into two
word operands and describe a wide result with a primary word and an additional
word. The gap is between this executable representation and its retained facts:

- `MirDirectParamAbi::Scalar` and `MirDirectResultAbi::Scalar` used
  `MirWidth`, which contains only Byte and Word.
- `direct_parameter_abi` additionally required the result to fit that same
  narrow model before retaining any parameters.
- Entry completeness checks required `slot.scalar_width == Some(width)`;
  a valid four-byte scalar slot has `storage_size == 4` and no single
  `MirWidth`.
- `validate_direct_call_result` rejected additional result words.
- Materialization dropped both primary and additional result declarations
  after emitting the captures. Wide calls must retain their complete declaration
  while the capture operations become explicit loads/stores.
- `MirScalarSignature` already records integer LONGs as two words, but excludes
  pointer signatures. It is not a substitute for the direct ABI contract.

## Ownership and representation

SemIR retains source typing and legality. Verified NIR already supplies the
integer bit width, storage extent, pointer address space and callable signature.
MIR6502 derives the physical ABI description from that structured information.
No new NIR form, source syntax or backend lookup into SemIR is required.

Introduce an ABI integer width independent of `MirWidth`. The intended shape is:

```rust
enum MirAbiIntegerWidth { Byte, Word, Long } // 1, 2, 4 bytes

enum MirDirectParamAbi {
    Scalar(MirAbiIntegerWidth),
    Pointer,
}

enum MirDirectResultAbi {
    Void,
    Scalar(MirAbiIntegerWidth),
    Pointer,
}
```

Retain the existing `MirFrame.direct_parameter_abi` and
`MirRoutine.direct_result_abi` fields. Extend their value types and consumers;
do not introduce a parallel wide-only signature or another callee database.
One LONG remains one logical parameter, one `ParamId` and one four-byte slot.
The two emitted words do not become two formal parameters.

Signed and unsigned integers of equal width share their physical ABI. LONGINT
and LONGCARD therefore use the same `Long` representation. Signed conversion
and arithmetic remain owned by the existing typed computation. Pointers retain
their distinct ABI kind even though Atari data pointers occupy two bytes.

`MirWidth` remains Byte/Word. An ABI width exposes its byte extent and its
lowered transfer layout explicitly; it must not silently convert Long to Word.
Keep `MirScalarSignature` for its existing integer-only consumers. Generate
its integer transfer groups using the same width projection and verify
consistency where both descriptions are present. Do not use its absence to
invalidate pointer ABI knowledge or its presence to fabricate missing facts.

## Independent metadata production

Derive parameters solely from the parameter list and its supported fixed-arity
calling convention. Derive the result solely from the result type:

| Fact | Meaning |
| --- | --- |
| Parameter `None` | Unknown/unsupported complete parameter contract |
| Parameter `Some([])` | Known zero-argument routine |
| Result `None` | Unknown/unsupported result contract |
| Result `Some(Void)` | Known procedure without result bytes |
| Result `Some(Scalar(Long))` | Declared four-byte integer result |

Keep the parameter list all-or-nothing for this extension; no per-parameter
unknown lattice is needed. Variadic parameter contracts remain outside the
existing complete-entry proof. A known result may still be retained.

Validate integer kind, declared bit width and byte extent together. A four-byte
aggregate is not a LONG, and a LONG with inconsistent two-byte metadata must
not be treated as a word. Retain existing data-pointer/address-space checks.
REAL by-value, aggregates and other unsupported forms remain unsupported.

For `ReadBits`, the result is `Scalar(Long)` and parameters are
`[Scalar(Word)]`. For a LONG/pointer/SIZE routine, parameters include a single `Scalar(Long)`, a
`Pointer` and the target's SIZE integer representation. Neither requires a
library-name special case.

## Physical projection and call validation

Use the existing [ABI module](../src/mir6502/abi.rs) for the shared mapping:

| Logical value | Bytes | Lowered transfers | Public result bytes |
| --- | ---: | --- | --- |
| Byte integer | 1 | Byte | `$A0` |
| Word integer | 2 | Word | `$A0/$A1` |
| Long integer | 4 | Word at offset 0, Word at offset 2 | `$A0–$A3` |
| Data pointer | 2 | Word | `$A0/$A1` |

Argument placement is the existing byte stream: offsets 0, 1 and 2 use A, X
and Y; offset 3 onward uses `$A3` onward. Values are little-endian, without
new alignment or padding. A first LONG uses A/X/Y/`$A3`; after a leading BYTE,
it uses X/Y/`$A3/$A4`. In `ReadBits`, the CARD argument uses A/X while the
LONGCARD result occupies `$A0–$A3`. Incoming registers have no `$A0–$A2` mirrors.

Preserve the currently supported argument extent. Projections must check bounds
and reject an unavailable proof on overflow, rather than saturating multiple
bytes onto one ZP address. This design does not extend the argument area.

Before argument expansion, check complete logical parameter transfer groups.
After expansion, check each byte's canonical physical home. A missing high
word, duplicate lane, misplaced later argument or wrong group width is not a
complete invocation. Reuse these projections in call planning and verification
instead of adding another independent `bits == 32` ABI table.

Preserve existing source rules for omitted arguments. Where a complete logical
prefix is legal, it must not end halfway through a LONG. Residence and complete
entry proofs still require their existing full input-supply obligations; a
legal abbreviated call does not automatically receive such a certificate.

For a wide result, validate the existing physical declaration: primary word
at return offset 0 and additional word at offset 2. Check the homes and widths
of captured results against that layout. A missing/overlapping high result
declaration or an undeclared additional result must invalidate the proof.
Capturing or using only part of a result, or discarding it entirely, does not
change the callee's declared result layout. Do not require dead result copies
to survive merely to retain ABI knowledge.

The result validator must handle both the pre-materialization representation
and the retained declaration after results become explicit stores/loads.
Typed indirect calls retain their existing encoding and structural checks;
this extension does not infer a particular callee's effects through a pointer.

## Capture and entry facts

The [direct capture builder](../src/mir6502/abi/param_capture.rs) already handles
four-byte scalar extents. Preserve its order and emitted stores. Entry
completeness must instead check each logical ABI value against its frame slot:

- One-/two-byte integers retain their matching scalar width and extent.
- A Long requires scalar storage, four bytes, and the existing absence of a
  single machine scalar width. Its typed ABI description supplies authority;
  storage size alone cannot establish a Long contract.
- Pointers retain their current storage and data-address-space restrictions.
- Parameter IDs, ordering, base offsets and capture coverage remain consistent.

Count total bytes, not logical values or word operands, when selecting and
validating capture. A single LONG occupies four capture bytes. LONG plus BYTE
occupies five and uses the existing SArgs strategy. Reuse its selected-binding
evidence and contiguous-frame contract; retain the current representable
5–96-byte capture range. Do not scatter a SArgs callee's frame or change its
descriptor, scratch footprint, Break handling or terminal behavior.

## Guaranteed results and proof freshness

Extend [result definition analysis](../src/mir6502/analysis/known_callees/entry_inputs/acyclic.rs)
to ask about all four canonical result bytes. A declaration identifies candidate
outputs; it does not prove that the routine writes them.

Retain the current per-byte, every-return-path proof. If only `$A0/$A1` are
defined, do not certify `$A2/$A3`. Reads precede definitions at a call boundary;
reading an old high result byte before defining it still creates an entry
requirement. Conditional writes, possible clobbers and paths with no normal
return cannot fabricate output definitions. A complete wide-result guarantee
requires all four bytes to pass the existing proof.

Use these facts through native wrappers and supported linked implementations.
Library interface identity and selected-body evidence remain required.
Private helpers retain their explicit result locations, including `$C4–$C7`;
the public `$A0–$A3` projection must not replace their private ABI.

Routine ABI metadata, frame/capture, selected binding and descendant facts
remain dependencies of cached entry proofs and transformation checks. Changing
only a result from Word to Long must invalidate prior facts. Rebuild analyses
through the existing scopes and preserve replay, final-cost acceptance and
rollback. Extend current verifier checks so known inconsistent metadata is
diagnosed; missing optional evidence continues to give a conservative answer.

## ZP placement and observations

Update canonical argument validation, entry analysis and ABI-byte observation
accounting to consume the complete description. In particular, later fixed
argument bytes and all four result bytes must remain observable to home
preservation and overlap checks. No consumer may truncate Long to Word to
reuse a narrow helper.

This enables existing narrow caller homes to be considered across a certified
wide-signature callee. It does not automatically make the callee's own LONG
parameters or locals eligible for ZP placement. Keep the current source-home
eligibility, pool, arithmetic/body restrictions, exposure rules and cost guard.
Broader placement of four-byte source objects is separate work.

Complete ABI knowledge is also independent of unknown pointee writes or OS/I/O
effects. `ReadBits` should lose the ABI-specific blocker; its callers can still
fail another proof. Public conversion and I/O routines need their remaining
implementation/effect contracts. No TN or Huffman size reduction is promised.

## Validation and acceptance

Use focused MIR6502 checks, grouped around distinct boundaries:

- Metadata: LONGINT/LONGCARD parameters and results; narrow parameters with
  wide results; wide parameters with narrow/void results; pointer mixtures;
  independent unknown parameter/result facts; invalid width/type combinations.
- Layout/capture: a first LONG and one following a BYTE, a following argument,
  direct four-byte capture and five-byte SArgs capture. Reject incomplete
  groups, wrong homes and extent overflow; preserve emitted ABI bytes.
- Results: all four bytes defined, missing/conditional high bytes, nested
  forwarding, partial/unused capture, and stale result/binding metadata.
- Public controls: `ReadBits`, `SqrWide`, `AsrLI`, `ValLC`/`ValLI`, plus a
  LONG/pointer/SIZE signature control,
  using existing sources and disabling inlining where a call boundary matters.
  Keep conversion/I/O effect failures distinct from ABI failures.
- Execution: reuse the existing wide nested direct/typed-indirect call control
  in [wide integers](../tools/vm-runtime-tests/tests/wide_integers.rs), the parser
  function-pointer control in [long integer I/O](../tools/vm-runtime-tests/tests/long_integer_io.rs),
  and SArgs/caller-preservation coverage in both runtimes. Include a caller home
  outside scratch and a collision with a high result/argument byte.
- Qualification: repeat the existing Huffman facts audit and its cartridge/
  standalone vectors, retaining LF/CRLF loader coverage; compare TN size/homes
  as a narrow-ABI control. Report remaining blockers and any layout fallback.

Readable MIR output must distinguish a single 32-bit ABI value from its
lowered word transfers. Record unavailable entry proofs separately from
supported signatures. Implementation validation follows the repository's backend scope.
If shared NIR contracts change, the mandatory shared-contract checks apply.

Acceptance requires accurate independent ABI descriptions, consistent physical
projection, complete byte coverage in proofs, unchanged call/capture execution,
and conservative behavior where effects or implementation evidence remain
unavailable. Keep the work in MIR6502; unrelated allocator, alias-analysis and
calling-convention redesigns are outside this note.
