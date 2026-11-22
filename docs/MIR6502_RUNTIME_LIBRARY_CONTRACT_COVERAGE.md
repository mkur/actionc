# MIR6502 runtime and library contract coverage

The [design](MIR6502_RUNTIME_HELPER_CONTRACT_DESIGN.md) separates canonical
inputs, guaranteed results, possible observations and successful-return writes.
The resolver's selected implementation is the authority for the contracts below.
Contracts do not grant purity, instruction motion or flag provenance.

## Runtime helpers

| Group | Supported facts | Remaining boundary |
| --- | --- | --- |
| SArgs | AX Y and later ABI lanes, contiguous frame definitions, successful scratch writes, selected body/dependencies | Own-frame ZP scattering remains unsupported. Capture facts cover representable incoming ZP lanes, currently 5–96 bytes. |
| MulByte, Mul, Lsh, Rsh | Canonical inputs/results and successful writes | Source caller eligibility remains separate. |
| Mul32, Lsh32, Rsh32 | All input/result bytes, including `$C4–$C7` | Public LONG metadata is separately supported by the [wide ABI contract](MIR6502_WIDE_ROUTINE_ABI_DESIGN.md); private helper homes remain distinct. |
| Division/remainder, signed/unsigned, narrow/wide, shared divmod | Successful results and writes | Full effects retain Error observations and the zero-divisor stop. |
| Fault | Existing terminal full effects | No returning result or preservation certificate. |
| Unsupported/custom replacements | Existing conservative effects | No default-body certificate is borrowed. |

## Public SYS implementations

Both supported providers retain the original typed interface identity. Linked
implementations additionally retain body, relocation and fallthrough evidence.
Changed evidence invalidates the contract. Shared cartridge addresses do not
merge interface signatures: `Peek` and `PeekC` remain distinct.

| Interface | Inputs / result | Fixed writes | Buffer effects / dependencies |
| --- | --- | --- | --- |
| SCompare | Two string pointers / signed word at `$A0/$A1` | `$A0–$A2`, `$A4–$A6` | Reads both length-prefixed strings; internal assembly call retained in body evidence. |
| SCopy | Destination/source pointers / void | `$A0–$A2` | Reads source length/payload; writes destination length/payload, at most 256 bytes. Reverse payload order; no memmove claim. |
| Zero | Destination pointer, word count / void | `$A0–$A4` | Destination writes; falls through into SetBlock. |
| SetBlock | Destination pointer, word count, byte value / void | `$A0–$A3` | Destination writes, dynamic extent. |
| MoveBlock | Destination/source pointers, word count / void | `$A0–$A3`, `$A5` | Reads source and writes destination, dynamic extent; forward copy. |
| Position | Word column in A/X, byte row in Y / void | `$54–$56`, `$5A–$5C` | Includes the `Pos1` fallthrough. |
| SetColor | Register, hue, luminance in A/X/Y / void | For proven register 0–4, `$A0`, `$A2`, selected color shadow and hardware register | Y survives. Other register values retain conservative writes. |
| Plot | Word column in A/X, byte row in Y / void | Unknown across CIO | Reviewed coordinate setup and volatile COLOR read, then channel 6 `PutD`. |
| Locate | Word column in A/X, byte row in Y / BYTE at `$A0` on success | Unknown across CIO | Reviewed `Position` call and channel 6 `GetD`. An open error continuation prevents a guaranteed result definition. |
| DrawTo, Fill | Word column in A/X, byte row in Y / void | Unknown across CIO | Shared `GrIO` setup reads COLOR and writes `$02FB`. The reviewed entries reach XIO `$11` and `$12` respectively. |
| Graphics | Byte mode in A / void | Unknown across CIO | Reviewed editor and screen close/open sequence. For S:, mode supplies AUX2 and `(mode & $30) XOR $1C` supplies AUX1. |
| PutD, GetD | Channel in A, put character in X / GetD character at `$A0` on success | Unknown across CIO | Shared `CCIO` sets a zero IOCB length and commands `$0B` and `$07`. It stages the accumulator in `$A0` before status handling. |
| Close, Open | Channel in A / void. Open also takes a counted filename in X/Y and mode and auxiliary bytes in `$A3/$A4`. | Unknown across CIO | Commands `$0C` and `$03`. Open copies the filename through the pointer at `$9B/$9C` on cartridge and into `$0500` on standalone. Cartridge Graphics calls the internal open entry. |
| XIO | Command in Y, AUX bytes in `$A3/$A4`, counted string pointer in `$A5/$A6` / void | Unknown across CIO | A zero AUX1 keeps the IOCB auxiliary bytes. A zero string length skips CIOV. DrawTo and Fill reach this path with commands `$11` and `$12`. |
| InputD, InputMD, InputSD, InputS | Counted destination and channel. InputMD also takes a byte maximum. InputS reads the current channel. | Unknown across CIO | GET RECORD `$05` targets the payload at destination plus one. The maximum comes from the destination header, the `InputMD` argument, or 255 for `InputSD` and `InputS`. A zero maximum skips CIOV. `InS` saves the low returned count minus one, with zero kept as zero, before status handling. |
| Note, Point | Channel in A. Note takes pointers to a CARD sector and a BYTE offset. Point takes those values directly. | Unknown across CIO | `Note` (`$26`) copies AUX5 to the byte destination, then AUX3/AUX4 to the word destination after successful status handling. `Point` (`$25`) fills AUX3–AUX5 before dispatch. |
| Paddle, Stick, STrig | Port in A / byte in A and `$A0` | `$A0` | Reads paddle shadow bytes, PIA ports or GTIA triggers respectively. Hardware reads retain their ordering. |
| PTrig | Proven port 0–7 in A / byte in A and `$A0` | `$A0` | PIA read and indexed read of the in-body mask table. Unproven ports retain conservative effects because high bytes can index past the table. |
| Rand | Limit in A / byte at `$A0` | `$A0` for proven zero. A nonzero or unproven limit also allows the union of the reviewed `MultI` scratch ranges. | Reads POKEY RANDOM at `$D20A`. A zero limit bypasses multiplication. |
| Sound | Proven voice 0–3 in A, pitch in X, distortion in Y and volume at `$A3` / void | `$A2` and the voice's two POKEY registers | An unproven voice retains the error continuation and conservative effects. |
| SndRst | No arguments / void | `$0232`, `$D200–$D208`, `$D20F` | Reads the serial-control shadow at `$0232` and resets POKEY audio state. |
| Peek, PeekC | Numeric address in A/X / BYTE at `$A0` or CARD at `$A0/$A1` | `$A0–$A3` for a proven address outside cartridge bank controls | Both entries read two bytes, including the wrapped second address. A known observation protects both bytes against home relocation. |
| Poke, PokeC | Numeric address in A/X and low value in Y. PokeC also takes the high byte at `$A3`. | `$A0/$A1` and one or two proven destination bytes | Qualified writes stay below the image and outside the stack and hardware. PokeC at `$A0` or `$A1` can redirect its second store and retains conservative writes. |

These assembly entries consume the public ABI directly, including fifth/sixth
argument bytes. They do not acquire a synthesized SArgs prologue. An unknown
buffer extent stays uncertain independently of the finite scratch set. A
consumer without a combined representation retains an unknown footprint.

A focused MIR6502 caller keeps a live byte in ZP across `Sound` with a
proven voice. The dynamic and out-of-range controls use inline storage.
Before/after MIR6502 builds at origin `$3000` produced byte-identical
images for the three toolkit callers on cartridge and standalone.
[SORT.DM1](../samples/toolkit/modern/SORT.DM1) ·
[PMG.DM2](../samples/toolkit/modern/PMG.DM2) ·
[MUSIC.DEM](../samples/toolkit/modern/MUSIC.DEM)

In standalone MIR6502, the reviewed `Peek` label falls through directly to
`PeekC` with the address in A/X. Building Lipsum at `$3000` reduced the
standalone XEX from 902 to 894 bytes by removing two parameter stores and
their inline storage. The cartridge build was byte-identical at 453 bytes.
The computed addresses in the glyph loop retain conservative effects, and
both builds have the same source ZP homes.
[Lipsum caller](../samples/benchmarks/bench/lipsum.act#L19)

## Remaining public-library coverage

The [implementation plan](6502_STANDARD_LIBRARY_CONTRACTS_IMPLEMENTATION_PLAN.md)
starts with the seven public graphics routines, in four slices on both providers.
The [CIO phase](6502_SYS_CIO_CONTRACTS_IMPLEMENTATION_PLAN.md) follows their
shared `PutD`, `GetD`, `Close`, `Open` and `XIO` dependencies.

| Interfaces | Missing returning proof components |
| --- | --- |
| SCopyS, SAssign | Subrange arithmetic, bounded string effects, jumps into SCopy interiors. |
| StrB/StrC/StrI, ValB/ValC/ValI | Conversion scratch, string effects and internal returning dependencies. |
| StrLC/StrLI, ValLC/ValLI and LONGCARD/LONGINT I/O | Public wide ABI metadata is available. String effects, conversion scratch and I/O dependencies still restrict entry certificates; ordinary bodies can use available helper facts. |
| StrR, ValR and REAL I/O | Atari floating-point service effects and pointer buffers. |
| Peek, PeekC, Poke, PokeC with computed addresses | A numeric address proof at the call site. |
| Sound with an unproven voice | Effects of a returning error handler. |
| Plot, Locate, DrawTo, Fill, Graphics, PutD, GetD, Open, Close, XIO | Transitive device and returning error-handler effects. The reviews bind the provider's wrapper through the CIO setup and status path. |
| InputD, InputMD, InputSD, InputS, Note, Point | Returning CIO device and error-handler guarantees. Note's write-back also needs ordinary destination and pointer-scratch alias checks. |
| Other Put/Print/Get and numeric Input routines | CIO/OS returning dependencies, device state and input/output buffers. |
| Break, Error, Panic | Program-stop behavior is accepted where a supported helper includes it. No public returning preservation claim. |

Ordinary Action wrappers use the existing acyclic analysis when all their
boundaries are supported. The inventory grants no blanket contract to a name,
module, or native wrapper. Finite observations alone do not guarantee profitable
ZP placement; caller eligibility, exposed objects, scratch/setup interference,
layout checks and the final cost guard still apply.

Parameterized native wrappers with buffer writes retain unknown returning
footprints until their argument-relative effects can be substituted at the
caller. An explicit destination whose bound overlaps implementation scratch also
retains unknown writes: it could modify the helper's pointer/count. These are
preservation restrictions, not source errors. No new alias-analysis subsystem is
introduced.
