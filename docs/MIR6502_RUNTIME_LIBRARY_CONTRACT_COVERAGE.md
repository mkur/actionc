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

These assembly entries consume the public ABI directly, including fifth/sixth
argument bytes. They do not acquire a synthesized SArgs prologue. An unknown
buffer extent stays uncertain independently of the finite scratch set. A
consumer without a combined representation retains an unknown footprint.

## Remaining public-library coverage

The [implementation plan](6502_STANDARD_LIBRARY_CONTRACTS_IMPLEMENTATION_PLAN.md)
starts with the seven public graphics routines, in four slices on both providers.

| Interfaces | Missing returning proof components |
| --- | --- |
| SCopyS, SAssign | Subrange arithmetic, bounded string effects, jumps into SCopy interiors. |
| StrB/StrC/StrI, ValB/ValC/ValI | Conversion scratch, string effects and internal returning dependencies. |
| StrLC/StrLI, ValLC/ValLI and LONGCARD/LONGINT I/O | Public wide ABI metadata is available. String effects, conversion scratch and I/O dependencies still restrict entry certificates; ordinary bodies can use available helper facts. |
| StrR, ValR and REAL I/O | Atari floating-point service effects and pointer buffers. |
| Peek/PeekC, Poke/PokeC | Explicit numeric memory access semantics and complete selected implementation effects; identity is retained now. |
| Rand, Sound, SndRst, Paddle/PTrig/Stick/STrig | Device/state observations and hardware writes. |
| Position, SetColor | Fixed-address effects, `Pos1` fallthrough, and register-dependent color writes. |
| Plot, Locate, DrawTo, Fill, Graphics | Screen CIO and status handling, plus graphics state, pixels and `Locate`'s result. |
| Put/Print/Get families, Input families, Open/Close/XIO/Note/Point | CIO/OS returning dependencies, device state and input/output buffers. |
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
