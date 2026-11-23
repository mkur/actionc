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
| SCopyS | Destination/source pointers in A/X and Y/`$A3`, inclusive byte bounds at `$A4/$A5` / void | `$A0–$A5` | Reads the source length and clamps the end to it. Writes the destination length, including for an empty slice, and copies the selected payload in reverse through `SCopy+10`. |
| SAssign | Same ABI as `SCopyS` / void | `$A0–$A2`, `$A4–$A6` | Reads the source length, takes at most that many source bytes, and copies in reverse into the selected destination slice through `SCopy+14`. A nonempty copy can update the destination length. |
| ValB, ValC, ValI | String pointer in A/X / BYTE at `$A0` or CARD/INT at `$A0/$A1` | `$A0–$A5` and a temporary stack byte for a proven source span outside cartridge bank controls | The shared parser reads an indexed 256-byte window. The standalone `ValB` and `ValC` labels pass A/X directly to `ValI`. |
| StrB, StrC, StrI | BYTE value and destination in A and X/Y, or word value and destination in A/X and Y/`$A3` / void | Wrapper scratch, hardware stack, FPP workspace, and up to 4, 6, or 7 bytes at a proven destination | `CToStr` calls IFP (`$D9AA`) and FASC (`$D8E6`) and stages a counted magnitude at `$0550`. The destination bound includes its length byte and the possible `StrI` sign. [FPP workspace audit](Action_2027/ATARI_FPP_ORACLE.md#sys-integer-string-formatting) |
| StrLC, StrLI | LONGCARD or LONGINT followed by a string pointer / void | Selected native scratch and arithmetic helper writes, plus up to 11 or 12 bytes at a proven destination | Both providers use the same Action! formatter. The destination bound includes the length byte and the possible sign. |
| ValLC, ValLI | String pointer in A/X / four result bytes at `$A0–$A3` on return | Selected native scratch, private storage and result lanes | Reads at most 256 bytes including the source length. Invalid input and overflow enter provider-specific error shims that stop even if the handler returns. |
| ValR | Counted string pointer in A/X, REAL destination pointer in Y/`$A3` / void | Selected native scratch, hardware stack, FPP workspace and six bytes at the proven destination | Reads at most 256 bytes including the length. Both providers select `SysValR`, which copies the payload into a private NUL-terminated buffer, calls AFP at `$D800`, and copies FR0 to the destination. |
| StrR | REAL source pointer in A/X, counted destination pointer in Y/`$A3` / void | Selected native scratch, hardware stack, FPP workspace and up to 17 destination bytes for a proven normalized REAL | Both providers select `SysStrR`. It copies six bytes to FR0, calls FASC at `$D8E6`, clears the final character's high bit and writes the length. |
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
| Put, PutE, PutDE | Put takes the character in A. Put and PutE use the current channel. PutDE takes the channel in A. | Unknown across CIO | Shared `CCIO` sends a single byte with command `$0B`. PutE and PutDE supply `$9B`. |
| Print, PrintD | Counted string. Print uses the current channel. PrintD takes an explicit channel. | Unknown across CIO | `Output` sends the payload with command `$0B` and skips dispatch for an empty string. |
| PrintE, PrintDE | Same arguments as Print and PrintD, respectively. | Unknown across CIO | `Prt` uses command `$09` for the counted payload. If `XIOstr` returns with Z set, including when the payload is empty, it dispatches `$0B` with `$9B` in A. |
| PrintB, PrintC, PrintI and their E, D, DE variants | Value on the current channel, or channel in A followed by the value / void | Unknown across CIO | `CToStr` stages a counted unsigned magnitude at `$0550`. Negative INT values send `-` first on a separate character transfer. The E variants send a separate `$9B` after the digits return. The character and counted payload transfers use `$0B`. |
| Close, Open | Channel in A / void. Open also takes a counted filename in X/Y and mode and auxiliary bytes in `$A3/$A4`. | Unknown across CIO | Commands `$0C` and `$03`. Open copies the filename through the pointer at `$9B/$9C` on cartridge and into `$0500` on standalone. Cartridge Graphics calls the internal open entry. |
| XIO | Command in Y, AUX bytes in `$A3/$A4`, counted string pointer in `$A5/$A6` / void | Unknown across CIO | A zero AUX1 keeps the IOCB auxiliary bytes. A zero string length skips CIOV. DrawTo and Fill reach this path with commands `$11` and `$12`. |
| InputD, InputMD, InputSD, InputS | Counted destination and channel. InputMD also takes a byte maximum. InputS reads the current channel. | Unknown across CIO | GET RECORD `$05` targets the payload at destination plus one. The maximum comes from the destination header, the `InputMD` argument, or 255 for `InputSD` and `InputS`. A zero maximum skips CIOV. `InS` saves the low returned count minus one, with zero kept as zero, before status handling. |
| InputB, InputC, InputI, InputBD, InputCD, InputID | Current channel or explicit channel in A. BYTE result at `$A0`, CARD and INT results at `$A0/$A1` on success. | Unknown across CIO | A 19-byte GET RECORD through `InputD` starts at `$0551`. The wrapper passes the counted buffer at `$0550` to the shared decimal parser after `InputD` returns. The linked aliases pass the original argument registers through without inline parameter stores. |
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

`SCopyS` and `SAssign` use finite footprints when both buffer addresses and
slice bounds are proven at the call. The bounds account for the length byte
and the original reverse-copy behavior on overlapping buffers. Possible
destination bytes must be below the loaded image and outside the stack,
hardware and pointer scratch. Possible source reads must avoid cartridge
bank controls.

With a proven destination, the focused callers keep a live byte in ZP across
`StrB` and `StrC`. `StrI` has a finite footprint too. Its outgoing pointer
uses `$A3`, its signed path can write `$A0–$A2`, and the documented FPP
workspace covers the ordinary `$E0–$EF` pool. A byte live across that call
therefore still falls back to inline storage with the current pool.
The standalone link manifest retains the preceding `StrC` entry for the
positive `StrI` branch, including when the program only calls `StrI`.

The wide decimal reviews bind the compiler-owned source separately from the
generated link manifest. MIR6502 uses the reviewed buffer extent at a proven
RAM address from `$0600` to below the loaded image and the hardware range. It
collects physical writes from the selected native body, SArgs capture, and
selected arithmetic helper contracts. The two formatters admit a live caller
byte in ZP on both providers. The parser's returning proof checks its four
physical result stores and retains the exact opaque error shim. The focused
parser callers still keep the live byte inline. The result uses `$A0–$A3`,
and the selected parser's scratch and homes cover `$E0–$EF`. Full effects still
include the error handler and its observations. [Provider sources](../embedded/runtime/SYSLONG.ACT) · [Cartridge error shim](../embedded/runtime/SYSLONGC.ACT#L13)

`InputLC` and `InputLI` read from `DEVICE`. Their explicit-channel forms,
`InputLCD` and `InputLID`, pass the supplied channel to `InputMD`. On both
providers each wrapper requests a 255-byte record. After a returning CIO
error, the wrapper checks the last byte transferred. An ATASCII EOL (`$9B`)
lets it parse the counted digits. A partial record without EOL raises error
104, and the caller's result stays untouched even when that handler returns.
[Wide input error fixture](../fixtures/runtime/long_integer_input_error.act)

`PrintLCE`, `PrintLIE`, `PrintLCDE` and `PrintLIDE` write the decimal
text and the ATASCII EOL (`$9B`) in separate CIO requests. On both providers,
if the digit write fails and the error handler changes `DEVICE` before
returning, `PrintLCE` and `PrintLIE` send EOL to the new current channel.
`PrintLCDE` and `PrintLIDE` send EOL to the original explicit channel. The VM
checks the failed transfer's channel and digits and the single EOL byte
delivered after the handler returns.
[Wide output error fixture](../fixtures/runtime/long_integer_output_error.act)

For `ValR`, MIR6502 requires both complete buffer spans in ordinary RAM from
`$0600` to below the loaded image and hardware. It checks the selected AFP call
and reserves the wrapper's physical writes, the hardware stack, and the FPP
windows at `$00D4–$00FF` and `$057E–$05FF`. With these proofs, a live caller
byte can stay in zero page. The VM fixture exercises a 255-character payload,
both bytes bordering the six-byte result, and a caller byte live across the call.
[Provider source](../embedded/runtime/SYSREAL.ACT#L31) ·
[REAL VM fixture](../fixtures/runtime/native_real_library.act)

For `StrR`, MIR6502 recognizes a complete copy or negation of an immutable,
normalized six-byte literal in the same block. Only register argument setup
and disjoint outgoing zero-page stores may intervene. Both spans must fit
in unbanked RAM at `$0600–$3FFF`, below the loaded image and disjoint from
the selected implementation's scratch. A changed FASC target invalidates
the call contract. An unknown value behind an otherwise known source
pointer keeps the formatter's effects open. [FASC bounds and OS-B exception](Action_2027/ATARI_FPP_ORACLE.md#fasc-and-transcendental-vectors)

The four REAL output wrappers format into a local `STRING text(20)`.
On ActionCart, `PrintR` and `PrintRE` use the current channel through
`Print` and `PrintE`. The latter sends PUT RECORD (`$09`), with the
reviewed conditional EOL continuation. Standalone sends the digits on
channel zero through `PrintD` (`$0B`) and `PrintRE` then sends a separate
`$9B` via `PutDE`. On both providers `PrintRD` uses the explicit channel
and `PrintRDE` follows that transfer with `PutDE`. The VM case distinguishes
channel zero, current channel two and explicit channel three. The device
and returning error-handler effects remain open for all four wrappers.
[Cartridge wrappers](../embedded/runtime/SYSREALC.ACT#L15) · [Standalone wrappers](../embedded/runtime/SYSREAL.ACT#L49) · [CIO fixture](../fixtures/runtime/resident_numeric_output.act)

When a nonempty ActionCart `PrintRE` record write returns CIO status `$82`,
the zero flag is clear. `Prt` skips its fallback EOL before `ChkErr`
invokes the error handler. If the handler changes `DEVICE` and returns,
there is no subsequent write. With the same error and handler, standalone
`PrintRE` sends one `$9B` byte to channel zero, and `PrintRDE` on both
providers sends it to the original explicit channel.
[REAL output error fixture](../fixtures/runtime/real_output_error.act)

Both REAL input wrappers issue GET RECORD (`$05`) before calling `SysValR`.
On ActionCart, `InputR` reads `DEVICE` through `InputS`. Its 256-byte local
string holds the length byte and a transfer of up to 255 bytes. Standalone
`InputR` reads channel zero through `InputMD`. Both `InputRD` implementations
pass the explicit channel to `InputMD`. Each `InputMD` call requests 126 bytes
into a 127-byte local string.

The input routine sets the string length to the low returned count minus one
when the count is positive. It does this before invoking the error handler.
If the handler returns after a partial read, `SysValR` parses that counted
payload and writes the six-byte REAL. For example, when CIO returns the six
bytes `-7.258` without EOL, both wrappers parse `-7.25`. The VM checks the
destination and both guard bytes for short records, full transfers and
returning errors on the default and explicit channels. Device and returning
error-handler effects remain unknown.
[Cartridge input](../embedded/runtime/SYSREALC.ACT#L43) · [Standalone input](../embedded/runtime/SYSREAL.ACT#L79) · [REAL input fixture](../fixtures/runtime/resident_real_input.act)

## Remaining public-library coverage

The [implementation plan](6502_STANDARD_LIBRARY_CONTRACTS_IMPLEMENTATION_PLAN.md)
starts with the seven public graphics routines, in four slices on both providers.
The [CIO phase](6502_SYS_CIO_CONTRACTS_IMPLEMENTATION_PLAN.md) follows their
shared `PutD`, `GetD`, `Close`, `Open` and `XIO` dependencies.

| Interfaces | Missing returning proof components |
| --- | --- |
| SCopyS, SAssign with unproven operands | Numeric buffer addresses and bounds at the call site. |
| ValB, ValC, ValI with an unproven source | An address proof that excludes cartridge bank controls from the indexed read window. |
| StrB, StrC, StrI with an unproven destination | A numeric span below the loaded image and the hardware range, disjoint from the pointer, stack, staging, and FPP workspace. |
| StrLC, StrLI, ValLC, ValLI with unproven buffers | A proven ordinary RAM span disjoint from the selected implementation and its scratch. |
| LONGCARD/LONGINT I/O | Both providers have coverage for input record termination and output EOL routing after a returning error. CIO device and error-handler effects remain unknown. |
| ValR with an unproven buffer | Both the 256-byte source span and the six-byte destination span must be proven disjoint from implementation scratch. |
| StrR with unproven operands | A normalized packed source and both ordinary RAM spans. A malformed OS-B result can leave the wrapper's copy unterminated. |
| PrintR, PrintRE, PrintRD, PrintRDE | The wrapper paths are checked on both providers, including EOL handling by PrintRE and PrintRDE after a returning error. CIO device and error-handler effects stay unknown. |
| InputR, InputRD | The channel, record length and subsequent REAL conversion are checked on both providers. Returning CIO device and error-handler effects stay unknown. |
| Peek, PeekC, Poke, PokeC with computed addresses | A numeric address proof at the call site. |
| Sound with an unproven voice | Effects of a returning error handler. |
| Plot, Locate, DrawTo, Fill, Graphics, PutD, GetD, Open, Close, XIO | Transitive device and returning error-handler effects. The reviews bind the provider's wrapper through the CIO setup and status path. |
| InputD, InputMD, InputSD, InputS, Note, Point | Returning CIO device and error-handler guarantees. Note's write-back also needs ordinary destination and pointer-scratch alias checks. |
| Put, PutE, PutDE, Print, PrintE, PrintD, PrintDE | Returning CIO device and error-handler guarantees. |
| InputB, InputC, InputI, InputBD, InputCD, InputID | Returning CIO device and error-handler guarantees. The parser runs after a returning error handler. |
| PrintB, PrintC, PrintI and their E, D, DE variants | Returning CIO device and error-handler guarantees. Subsequent transfers can depend on scratch and device state left by an earlier transfer. |
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
