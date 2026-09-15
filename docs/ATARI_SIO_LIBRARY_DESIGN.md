# Atari SIO library design for FujiNet

Status: transport foundation implemented, 2026-09-15; FujiNet convenience layers
and optional disk helpers remain proposed. The library uses existing modern
Action! constructs. The [68K checkpoint](MIR68K_CHECKPOINT.md) records the previous
work.

## Purpose and scope

**FujiNet support is the north star.** An Action! program should be able to open
a network resource, exchange data in small caller-owned buffers, handle errors
with CASE, and close the channel. The first useful milestone is a complete HTTP
fetch through FujiNet; JSON queries and adapter services follow it.

Provide synchronous Atari serial-device requests without requiring applications
to manipulate the OS Device Control Block (DCB). `ATARI.SIO` is the transport
foundation for that goal. Use UNION for overlapping byte representations and
VARIANT for transfer direction, data/wait/EOF outcomes and distinct failures.

The first delivery consists of the embedded `ATARI.SIO` transport, the
device and command constants modules and FujiNet network primitives with short
convenience calls, named defaults and Action! string helpers.
A small `ATARI.SIO.DISK` module remains useful for transport
diagnostics but does not gate network support. Support modern classic and MIR6502
with both cartridge and standalone runtimes, using a compatible Atari OS.
Standalone removes the Action! cartridge dependency, not the OS dependency.
The compatibility profile rejects the new constructs. This is an Atari 8-bit
library, not an Atari ST or portable 68K API.

SIO framing, checksums, timing and existing retries remain with the OS. Use
`SIOV` at `$E459`, with the DCB at `$0300`; do not route through CIO or the disk
handler `DSKINV`, which imposes its own command defaults. The original
[Atari OS source, SIO and disk-interface routines](https://atariwiki.org/wiki/Wiki.jsp?page=Atari+800+ROM+OS+Source+Listing)
defines those boundaries. We are implementing the caller side of that interface.

Initial exclusions: a serial-device server, cassette transport, asynchronous
SIO requests, a DOS filesystem driver, automatic drive/density discovery,
formatting, custom baud-rate engines, OS patch installation and SIO2SD-specific
commands. FujiNet support is delivered incrementally rather than requiring its
entire command set in the first release. A patched OS may service the same
vector; that does not establish its compatibility without testing.

## FujiNet layers and milestones

| Module | Responsibility | Priority |
| --- | --- | --- |
| ATARI.SIO.DEVICES | Documented SIO device base IDs as public BYTE constants | Foundation |
| ATARI.SIO.DISK.COMMANDS | Common disk command bytes, independent of disk helpers | Foundation |
| ATARI.FUJINET.NET.COMMANDS | FujiNet network command bytes, independent of network helpers | Foundation |
| ATARI.SIO | DCB marshalling, OS call and raw transport result | Foundation |
| ATARI.FUJINET.NET | Channel defaults, string/span open and write helpers, status, bounded reads and close | First application milestone |
| ATARI.FUJINET.JSON | Channel mode, parse and query operations over an owned network channel | Next network milestone |
| ATARI.FUJINET.BUFFERED | Optional buffered byte/block reads, with optional PROCEED notification | Later interactive-client work |
| ATARI.FUJINET.CONTROL | Adapter information, Wi-Fi status, then host/device slots and mount operations | Subsequent adapter work |
| ATARI.FUJINET.APPKEY | Persistent application settings through AppKey commands | Subsequent application support |
| ATARI.SIO.DISK | Small sector/status helpers for ordinary drives and mounted images | Optional diagnostic companion |

The Atari network endpoints use SIO IDs `$71..$78`; adapter commands use `$70`.
Keep these protocol families in their own modules. The transport knows only
device, unit, command, auxiliary bytes and transfer shape. Network calls use
the SIO protocol directly, without requiring an installed CIO `N:` handler.
The upstream [network command table](https://github.com/FujiNetWIFI/fujinet-firmware/wiki/SIO-Commands-for-Device-IDs-%2471-to-%2478)
and [adapter command table](https://github.com/FujiNetWIFI/fujinet-firmware/wiki/SIO-Commands-for-Device-ID-%2470)
define these families.

The first application assumes FujiNet is already configured and connected.
It fetches a controlled HTTP resource larger than its receive buffer, handles
waiting and EOF, and closes the channel on success and failure. This gives us
an observable networking result before adding a configuration utility. TCP
write/readback against a controlled echo service proves the send path. HTTPS
is a later acceptance case against the selected firmware, not a host-side TLS
implementation in Action!.

Pin a FujiNet firmware revision and matching command fixtures when implementing
each module. Record exact payload sizes, termination, auxiliary meanings and
supported units; the command tables evolve, and some older per-command pages
list fewer units than the network table. Do not turn a timeout or NAK into a
definitive firmware-version or capability diagnosis. Optional protocol commands
need the upstream capability query where applicable.

## Lessons from Mad Pascal

The comparison baseline is the `blibs` bundled with Mad Pascal at revision
`8dd0541fa36037bfe20831d9a7c77fa80fc22ffc`. These are existing implementations;
the Action! interfaces here remain proposals.

| Reference API | Design decision |
| --- | --- |
| [sio.pas](https://github.com/tebe6502/Mad-Pascal/blob/8dd0541fa36037bfe20831d9a7c77fa80fc22ffc/blibs/sio.pas) exposes DCB, direction constants and ExecSIO, with shared status variables. | Keep the raw OS interface private and return request/result values. |
| [fn_sio.pas](https://github.com/tebe6502/Mad-Pascal/blob/8dd0541fa36037bfe20831d9a7c77fa80fc22ffc/blibs/fn_sio.pas) provides short network calls and adapter/directory/mount operations; its network wrappers use unit 1 and a shared timeout. | Match the short calls while retaining explicit channels, per-channel settings and returned errors. |
| [fn_tcp.pas](https://github.com/tebe6502/Mad-Pascal/blob/8dd0541fa36037bfe20831d9a7c77fa80fc22ffc/blibs/fn_tcp.pas) adds a 256-byte circular buffer, string/byte helpers and PROCEED notification. | Include string convenience now; schedule buffering and notification as an optional later layer. |
| [fn_cookies.pas](https://github.com/tebe6502/Mad-Pascal/blob/8dd0541fa36037bfe20831d9a7c77fa80fc22ffc/blibs/fn_cookies.pas) provides AppKey get/set helpers. | Keep AppKeys and adapter operations as concrete follow-up deliverables after network/JSON acceptance. |

Unions provide named views of protocol bytes; variants make error and stream
outcomes explicit. Neither justifies making common application calls cumbersome.
The first HTTP and TCP examples must exercise the convenience API rather than
constructing DCB-like requests. Record code-size and copy costs before claiming
an efficiency advantage over these small Pascal wrappers.

## Device constants

Add `ATARI.SIO.DEVICES` at `embedded/modules/atari/sio/devices.act` as an
independent constants module. Export each ID with `PUBLIC CONST BYTE`; importing
the catalogue requires no transport code, runtime initialization or device
discovery. Start with these agreed names:

| Constant | DDEVIC base | Meaning |
| --- | ---: | --- |
| DISK | `$31` | Disk drive family; select the drive with DUNIT. |
| PRINTER | `$40` | Printer family. |
| FUJINET | `$70` | FujiNet adapter/control endpoint; use unit 1. |
| FUJINET_NETWORK | `$71` | FujiNet network family; select the channel with DUNIT. |

These values come from the [Atari OS equates](https://raw.githubusercontent.com/cc65/cc65/master/asminc/atari.inc)
and the FujiNet command tables linked above. Expand the catalogue with other
documented device families as their IDs are verified; record a source and unit
convention for each entry. A named ID does not imply an implemented driver or
support for that device's protocol in SIO.Execute.

Callers use `USE ATARI.SIO.DEVICES AS DEV`. For example,
`device=DEV.DISK` with `unit=2` selects D2; `device=DEV.FUJINET_NETWORK` with
`unit=2` selects network endpoint `$72`. Keep unit numbers separate from base
IDs. Protocol wrappers document any unit restrictions they impose; the simple
disk helpers below pass the unit through unchanged.

Keep the IDs as BYTE constants and `Request.device` as BYTE. The catalogue is
not a transport allowlist: callers can use any device byte. Commands and status
codes remain with their owning protocol namespaces. Network, control and disk wrappers import
these constants instead of repeating device-address literals.

## Command constants

Publish command bytes as `PUBLIC CONST BYTE` in separate constants modules
under each protocol. Start with:

| Module | Constants |
| --- | --- |
| ATARI.SIO.DISK.COMMANDS | `READ=$52`, `WRITE=$50`, `WRITE_VERIFY=$57`, `STATUS=$53` |
| ATARI.FUJINET.NET.COMMANDS | `OPEN=$4F`, `CLOSE=$43`, `READ=$52`, `WRITE=$57`, `STATUS=$53` |

Use `USE ATARI.SIO.DISK.COMMANDS AS DISK` for `request.command=DISK.READ`, or
alias the network constants as NETCMD when also importing the NET convenience
API. A separate namespace avoids collisions between constants such as STATUS
and routines such as Status in this case-insensitive language. Importing a
command module must not pull in its parent's protocol implementation.

Command meanings depend on the device protocol. Disk WRITE sends without verify;
disk WRITE_VERIFY and network WRITE both use `$57`. Do not introduce a universal
SIO.WRITE constant. The [Atari command equates](https://raw.githubusercontent.com/cc65/cc65/master/asminc/atari.inc)
and [FujiNet network commands](https://github.com/FujiNetWIFI/fujinet-firmware/wiki/SIO-Commands-for-Device-IDs-%2471-to-%2478)
are the source for these values.

Add JSON, CONTROL and APPKEY command modules with their corresponding slices,
using the same convention and verified protocol fixtures. Keep Request.command
as BYTE so extension commands remain possible. A named command identifies a
byte; its protocol wrapper supplies direction, count and auxiliary semantics.
SIO.Execute must not infer those from a global command table. Examples and
wrappers use the named constants; literal values remain in protocol reference
tables and independent expected-byte assertions.

## Public types

These declarations belong to `MODULE ATARI.SIO`:

```action
PUBLIC TYPE AuxBytes=[BYTE low,high]
PUBLIC TYPE Auxiliary=UNION [CARD word AuxBytes bytes]

PUBLIC TYPE Transfer=VARIANT [
  NO_DATA
  READ [BYTE POINTER buffer CARD length]
  WRITE [BYTE POINTER buffer CARD length]
  WRITE_READ [BYTE POINTER buffer CARD length]
]

PUBLIC TYPE Request=[
  BYTE device BYTE unit BYTE command BYTE timeout
  Auxiliary aux
  Transfer phase
]

PUBLIC TYPE Result=VARIANT [
  OK
  ERROR [BYTE status]
]
```

The main function is `Result FUNC Execute(Request request)`. It takes a value
snapshot of the small request and returns a constructed result by value. The
buffer pointers are borrowed: their payloads are not copied, allocated, retained
or freed by the library. A const-pointer type is not required for WRITE, but its
source bytes must remain stable during the operation.

`device`, `unit`, `command` and `timeout` are the raw DDEVIC, DUNIT, DCOMND and
DTIMLO bytes. Execute passes them to the OS unchanged. Normal device families
use one-based units; for ordinary non-overflowing values the OS derives the bus
address as `device + unit - 1`. A direct bus ID normally uses `unit=1`. The
transport neither computes that address nor imposes a universal unit range.
Keep `command` a BYTE for direct use of protocol and extension commands.
DTIMLO is nominally in OS seconds; it is not a deadline for the whole operation
including retries. Zero and other special values retain the selected OS's
behavior, with no library default or rejection. Auxiliary fields can be
set as a word or as two bytes. The target is little-endian, so `aux.word=$1234`
means AUX1=$34 and AUX2=$12. This relationship is specific to this Atari module.
See the [cc65 Atari OS equates](https://raw.githubusercontent.com/cc65/cc65/master/asminc/atari.inc)
for the DCB definitions and constants.

Directions are from the Atari's perspective. These alternatives describe the
data transfer; the command byte is selected separately.

| Transfer | Input DSTATS | Buffer contract |
| --- | ---: | --- |
| NO_DATA | `$00` | No data frame; pointer and length are emitted as zero. |
| READ | `$40` | Device fills a writable buffer of the requested length. |
| WRITE | `$80` | Atari sends the requested buffer bytes. |
| WRITE_READ | `$C0` | Send, then receive using the same buffer and length. |

WRITE_READ follows the independent send/receive bits used by the OS SIO routine;
it is not a promise that a particular peripheral implements that command. The
DCB cannot express different send and receive buffers or lengths in one call.
Protocols that need them require a device-specific sequence. NO_DATA means no data
frame, not a no-op: it still issues the command and awaits completion.

`length` is the transfer count, not a buffer capacity inferred by the compiler.
Execute copies it unchanged, including zero; OS/protocol behavior determines
what that count means. Requests cannot represent a data direction without its
buffer/count fields. This does not make
arbitrary pointers or lengths memory-safe.
Payload bytes are binary and unchanged: no ASCII/ATASCII conversion, string
prefix or checksum byte is added to the caller's span. Frame overhead belongs
to the OS.

## Result and caller contract

- Exactly status `$01` becomes OK. Every other OS status becomes
  ERROR(status), preserving the complete byte, including unknown codes.
  The exported BYTE constants are SUCCESS, BREAK_ABORT, TIMEOUT, NAK,
  FRAMING_ERROR, OVERRUN, CHECKSUM_ERROR and DEVICE_ERROR. Do not decode device error as necessarily
  write-protection. The [OS equates](https://raw.githubusercontent.com/cc65/cc65/master/asminc/atari.inc)
  identify these codes; they are separate from any status bytes returned by a
  peripheral's status command.
- There is no SIO.Error enum or INVALID alternative. Execute marshals the
  selected transfer, calls the OS and reports its status. It does not validate
  device/unit ranges, commands, timeouts, pointers or counts, inspect payloads,
  or infer transfer settings from the command. Unusual values pass through;
  that does not establish support for untested OS paths such as cassette I/O.
- The caller supplies ordinary accessible RAM, writable for READ/WRITE_READ,
  covering all bytes the OS will access and disjoint from the DCB, live code,
  OS workspace, stack and library working storage. Address-range and buffer-size
  correctness, banking and ownership are caller preconditions, not checked
  guarantees. Do not pass an array descriptor's address in place of its first
  element. An invalid memory span need not produce an ERROR result; it can corrupt
  memory before the OS returns.
- READ/WRITE_READ may modify a prefix or more of the buffer before ERROR.
  Preserve those effects; do not clear the buffer or claim transactional
  rollback. Failed writes may already have affected the peripheral. A successful
  command does not promise a later readback unless that command specifies one.
- The result contains no invented partial-byte count. There is no global
  last-error variable or implicit print/abort/retry policy. A caller can keep a
  LET result snapshot across subsequent requests.
- Construct Transfer values before use. A zeroed/corrupted variant tag is a
  language validity fault (Atari Error 105), outside the SIO.Result contract.
  The ordinary [variant rules](tutorials/VARIANTS.md) apply, including when a
  Request is captured as an argument. No raw tag inspection bypass is added.

Higher-level helpers own checks needed by the operations they implement. For
example, NET.Open checks that a URI fits its staging buffer, NET.ReadAvailable
bounds the requested count by caller capacity, and AppKey helpers check returned
lengths before copying. A helper that rejects input declares its own Error and
result alternative in its protocol module. Such checks do not prove that a
caller-supplied pointer is valid or introduce a validation phase into Execute.

Do not add library retries in the first version: one Execute performs
one OS call, whose internal retry policy remains in force. It cannot guarantee
exactly-once device execution. Optional retries later must be explicit and
consider command idempotence.

## Private OS representation and call boundary

Use a separate private overlay, never reinterpret Request or Result as the DCB:

```action
TYPE DcbFields=[
  BYTE device,unit,command,status
  CARD buffer
  BYTE timeout,reserved
  CARD length
  Auxiliary aux
]
TYPE DcbImage=UNION [DcbFields fields BYTE ARRAY bytes(12)]
VOLATILE DcbImage dcb=$0300

PROC OsSio=$E459()
```

The representation must be checked against literal OS offsets:

| Offset | OS field | Width |
| --- | --- | ---: |
| 0 | DDEVIC | 1 |
| 1 | DUNIT | 1 |
| 2 | DCOMND | 1 |
| 3 | DSTATS | 1 |
| 4 | DBUFLO/DBUFHI | 2 |
| 6 | DTIMLO | 1 |
| 7 | Unused/reserved byte | 1 |
| 8 | DBYTLO/DBYTHI | 2 |
| 10 | DAUX1/DAUX2 | 2 |

The union's byte view provides exact-image inspection for tests; its record
view names the normal operations. Assert SIZEOF=12 and every offset on the
Atari target. CARD is deliberate in the wire-facing fields. Host/native pointer
sizes must never change this layout silently.

Execution sequence:

1. Capture the request by value; use CASE to select the active transfer. Normal
   language tag checks apply; there is no library input-validation phase.
2. Write all twelve DCB bytes deterministically, including zero for the reserved
   byte and canonical pointer/count values for NO_DATA. Finish preparing the DCB
   before calling any external routine. Marshal named fields; no tagged-object
   copy into OS storage and no general memset over adjacent OS variables.
3. Call the parameterless fixed-address PROC. The OS returns status in Y and
   DSTATS; read the volatile DSTATS byte immediately after return and capture it
   before any other library call. This uses the existing PROC ABI rather than
   incorrectly declaring the OS entry as an Action! BYTE FUNC returning in A.
   The [SIO RETURN routine](https://atariwiki.org/wiki/Wiki.jsp?page=Atari+800+ROM+OS+Source+Listing)
   explicitly stores both results.
4. Construct Result from the captured byte. Leave the post-call DCB observable;
   do not save and restore it in the first API. A later request rewrites it.

Calls are synchronous and restricted to normal foreground code with the OS,
its interrupt services and required memory mapping available. The global DCB,
OS workspaces and Atari routine-static activation are not reentrant. No calls
from DLI/VBI/IRQ handlers, recursion, callbacks or concurrent DCB users; a busy
flag alone would not protect parameter homes overwritten before its check.
Do not surround SIO with SEI or disable VBLANK to simulate a lock. Sound and
other POKEY users need coordination with OS SIO; no register-preservation
promise beyond the audited OS interface is introduced.

The compiler must keep the fixed OS call and absolute/volatile storage effects
conservative, including accesses through transfer-buffer aliases. Do not mark
this call pure or infer that WRITE means the OS has no other memory effects.
Audit actual OS zero-page clobbers against compiler/runtime scratch conventions.
Only add a small assembly bridge if that audit demonstrates a need; no new
calling convention should be inferred from the public aggregate signatures.

The initial non-cassette audit compares the Rev B SIO/interrupt/timer routines
linked above with [compiler runtime scratch assignments](../src/codegen/storage.rs).
The SIO workspace at `$30..$42` is separate from the compiler's argument and
pointer scratch; shared BREAK state at `$11` belongs to the OS contract.
The implementation uses the unannotated fixed PROC boundary and needs no
assembly bridge for that path. Boundary tests overwrite `$30..$42` and return
different A and Y/DSTATS values. This checks compiler behavior under those
clobbers; it does not validate an arbitrary patched OS or custom interrupt handler.

UNION is intentionally limited to byte representation. The existing language
rejects inline variants inside unions, so neither Transfer nor Result belongs
inside DcbImage. Conversely, a union can be a variant payload. See the
[union contract](tutorials/UNIONS.md).

## FujiNet network contract

`ATARI.FUJINET.NET` is the normal application entry point. Initialize a small
Channel settings record with `DefaultChannel(unit)`, then pass it to each
operation. Its fields are BYTE unit, timeout, access and translation, plus CARD
chunkLimit.
The factory performs no I/O or allocation. Calls capture the settings by value;
there is no mutable global default timeout or implicit current channel.

The proposed call shapes below are design notation; complete Action! declarations
and result types are finalized and compiled against examples in slice 2:

| Call | Contract |
| --- | --- |
| DefaultChannel(unit) | Return channel settings initialized with named defaults. |
| Open(channel, uri) | Open from an Action! length-prefixed string. |
| OpenBuffer(channel, address, length) | Open from an explicit device-specification byte span. |
| Status(channel) | Return a typed snapshot of network status. |
| Read(channel, destination, length) | Issue one exact-length read; the caller establishes availability and storage size. |
| ReadAvailable(channel, destination, capacity) | Inspect status and perform at most one bounded read, returning data/wait/EOF/error. |
| Write(channel, source, length) | Write one explicit byte span. |
| WriteString(channel, text) | Write the content of an Action! string. |
| Close(channel) | Close the selected endpoint and return its result. |

Publish BYTE `DEFAULT_TIMEOUT=15` OS seconds and CARD `DEFAULT_CHUNK=256` bytes
as library policy, and initialize access to READ_ONLY and translation to BINARY.
Callers
can change the record once for a particular use. Reject zero timeouts/chunk
limits before I/O. Access and translation configure Open; changing them later
does not reconfigure an already open device. Explicit mode-changing operations
belong to their protocol modules. Separate helper names and an ordinary record
avoid dependencies on function overloading or default-argument language features.

Name protocol-sensitive access values clearly. READ_ONLY uses AUX1=4 and
READ_WRITE uses 12 for ordinary streams; provide HTTP_GET=12 as a separate named
choice for the HTTP example. FujiNet interprets HTTP mode 4 as a GET with file
resolution behavior, while 12 requests a direct GET. Preserve that distinction
instead of inferring options from a URI. See the
[upstream access modes](https://github.com/FujiNetWIFI/fujinet-firmware/wiki/N%3A-AUX1-Values).

The Channel record identifies an endpoint and its call settings; it is not a
unique handle or proof that a connection is open. Copying it does not create a
connection. Callers retain exclusive ownership of the chosen unit through Close
and must coordinate with other programs or CIO users. Avoid implicit allocation,
reopening or retries of stateful commands. Validate unit and settings on each
operation; explicit channels do not make the shared OS/DCB interface reentrant.
The record's unit selects the SIO endpoint. If a device specification contains
an explicit unit prefix such as N2:, require it to agree with that selection;
an unnumbered N: prefix leaves selection to the record.

Open and OpenBuffer share one checked implementation. It constructs the
protocol's 256-byte command payload in private storage, validates space for
termination, and initializes unused bytes. Open extracts the string's length
and content, never transmitting its Action! prefix. OpenBuffer uses exactly
the supplied span. Reject empty, oversized or internally terminated device
specifications before I/O; never silently truncate or read 256 bytes from a
shorter caller allocation. The accepted wire terminator and maximum content
length must be fixed against the chosen firmware. AUX1 and AUX2 carry independent
open options, which makes the auxiliary union's byte view useful. See the upstream
[Open command](https://github.com/FujiNetWIFI/fujinet-firmware/wiki/N%3A-SIO-Command-%27O%27---Open).

WriteString sends only the counted content, without adding a prefix, terminator
or newline. Embedded zero bytes remain data. After validating channel settings,
an empty string succeeds without an SIO call; raw zero-length transfer requests
through SIO.Execute are passed to the OS unchanged.
All nonempty writes use the same checked span path as Write and preserve its
result. Document buffer ownership for these helpers; they do not retain strings
or prove that a caller's pointer refers to a sufficiently large allocation.

Start with binary stream mode and explicit text translation options. Conversion
belongs in the network protocol wrapper or a text helper, never in SIO.Execute.
JSON channel mode is also explicit and owned by the JSON layer; it must not
silently change the behavior of an existing ordinary stream caller. Datagram
boundaries and listening sockets need their own later API rather than being
silently treated as an HTTP/TCP byte stream.

Status returns a value snapshot with a UNION view of its four received bytes:
a little-endian CARD count of bytes waiting, a connection byte, and an extended
error byte. Its raw byte-array view supports protocol fixtures; named fields
support application logic. Put that union in the success payload of a status
result VARIANT, following the same pattern as DiskStatus below. Receive into
untagged storage before constructing the result. The upstream
[Status command](https://github.com/FujiNetWIFI/fujinet-firmware/wiki/N%3A-SIO-Command-%27S%27---Status)
defines this payload; it is distinct from OS DSTATS.

Use variants for the higher-level read outcome: DATA(count), WAITING,
END_OF_STREAM, SIO_ERROR(status), NETWORK_ERROR(code) and INVALID(reason).
INVALID uses the network module's own Error reasons for its helper checks;
it is never forwarded from SIO.Execute. These are the intended alternatives,
with the complete public declarations to be settled alongside the first network
implementation. A zero available count alone is not EOF. Decode completion and
errors using the selected protocol's
documented status rules, and allow buffered data to be drained after the remote
side closes. Preserve unrecognized device errors as bytes.

ReadAvailable checks its capacity argument, obtains a status snapshot, and issues
at most one read of `min(available, capacity, configured chunk limit)` bytes.
It returns WAITING without a read when the stream is still pending and no data
is available. It must not poll forever internally. The application controls
polling, cancellation and an overall deadline; each SIO call still has its own
timeout. A status/read race may fail and follows the ordinary failure path.
The caller still provides writable storage for the declared capacity; the
helper cannot discover the allocation behind a pointer.

The read and write wrappers put the same transfer count in DBYT and the auxiliary
word. Never request more than the reported available count on the bounded read
path. DATA(count) is produced only after that exact request succeeds; it does
not add partial-transfer information to a failed SIO call. Each primitive write
uses the caller's explicit span and reports the actual transport result; larger
streaming operations account for successful chunks separately. These rules
follow the upstream [Read command](https://github.com/FujiNetWIFI/fujinet-firmware/wiki/N%3A-SIO-Command-%27R%27---Read)
and [Write command](https://github.com/FujiNetWIFI/fujinet-firmware/wiki/N%3A-SIO-Command-%27W%27---Write).

Preserve the distinction between local validation, SIO transport failure and
FujiNet protocol failure. A successful Status transaction can report a network
error in its payload. Conversely, an SIO device error does not contain the
extended cause. Keep the original result before an explicit diagnostic status
query; if that query fails too, it must not replace the original failure or
manufacture a network error. Likewise, cleanup Close failures must not erase
the preceding read/write failure.

JSON is the next consumer of these foundations: open a resource, select JSON
channel mode, parse, query a path and retrieve a bounded result. FujiNet performs
the parsing. Query strings, result lengths and truncation require explicit
contracts, and mode changes must be covered by sequence tests. The upstream
[channel-mode command](https://github.com/FujiNetWIFI/fujinet-firmware/wiki/N%3A-SIO-Command-%24FC---Set-Channel-Mode)
selects protocol or JSON mode. Adapter configuration, mounts and AppKeys follow
without expanding the transport's responsibilities.

## Optional buffering and notification

After the network and JSON milestones, add `ATARI.FUJINET.BUFFERED` for
interactive clients that benefit from byte-at-a-time consumption. Use a
caller-owned reader state and ring buffer; a 256-byte example is a useful
starting point, not a mandatory allocation in NET. Keep reader state separate
for each owned channel and prevent competing direct reads while it is active.
The reader references caller storage rather than copying a whole ring on calls.

Provide explicit Poll, ReadByte and ReadBuffer operations. Poll performs bounded
foreground work through NET. Empty reads return WAITING, EOF or an error as
appropriate; never return an undefined byte. Commit buffer positions/counts only
for successful transfers, including each part of a wrapped fill. Deliver already
buffered bytes before surfacing a pending terminal result, preserving that result
until drained. Failed reads retain the transport's partial-write limitations.

PROCEED notification is optional and must not be required for polling. Its ISR
only records an event; SIO calls and buffer work remain in foreground code.
Audit atomic flag consumption, the shared notification line, and preservation
or chaining of the previous vector and relevant PIA state. Attach/detach must
have an explicit owner and restore the prior configuration. Do not promise
one notification per packet or infer which channel is ready from the flag.

## Optional disk convenience layer

`ATARI.SIO.DISK` imports the core without adding disk behavior to Execute.
If added, start with the common 128-byte sector profile and explicit function
names. Its value is preparing the device ID, command, transfer direction/count
and auxiliary fields for common operations, plus returning a typed status
snapshot. Sector operations return SIO.Result directly, with the same error
handling as a generic request. There is no separate disk Error enum, Result
type or INVALID alternative, and no capacity argument that merely repeats a
caller-supplied allocation claim.

The module is optional: retain the constants and generic example as the
foundation, and add helpers when an example needs them. The FujiNet networking
milestone takes priority over expanding disk support.

| Function | Proposed parameters | Result |
| --- | --- | --- |
| ReadSector128 | BYTE unit, CARD sector, BYTE POINTER destination, BYTE timeout | SIO.Result |
| WriteSector128 | BYTE unit, CARD sector, BYTE POINTER source, BYTE timeout | SIO.Result |
| WriteVerifySector128 | Same as WriteSector128 | SIO.Result |
| Status | BYTE unit, BYTE timeout | StatusResult |

Pass unit, sector and timeout through unchanged, including zero or unusual
values; their interpretation remains with the OS/device. Sector wrappers issue
exactly one 128-byte transfer request, leaving any buffer tail alone. The caller
supplies at least 128 accessible bytes, writable for reads, subject to the same
memory and lifetime preconditions as Execute. The helper does not check pointer
validity or allocation size.

Use DEV.DISK, sector in the auxiliary word, and the READ, WRITE and WRITE_VERIFY
constants from `ATARI.SIO.DISK.COMMANDS`. Status uses its STATUS constant,
auxiliary zero and a four-byte receive buffer.
These command values are recorded in the
[Atari command equates](https://raw.githubusercontent.com/cc65/cc65/master/asminc/atari.inc).
Callers remain responsible for selecting the correct media/profile; these
functions cannot discover whether a sector exists or has that transfer size.

The disk module declares the following types after `USE ATARI.SIO AS SIO`:

```action
PUBLIC TYPE StatusFields=[BYTE flags,controller,timeout,reserved]
PUBLIC TYPE DiskStatus=UNION [StatusFields fields BYTE ARRAY bytes(4)]
PUBLIC TYPE StatusResult=VARIANT [
  OK [DiskStatus value]
  ERROR [BYTE status]
]
```

Status manages its own four-byte receive storage. Map SIO.Result.OK to
StatusResult.OK(value) and SIO.Result.ERROR(status) to StatusResult.ERROR(status),
preserving the complete transport status byte.
Preserve raw status bytes; hardware-specific interpretations belong in later
drive helpers. Receive into ordinary untagged private storage and construct OK
only after successful completion. A device must never write across a
compiler-generated variant tag. This is a useful union payload inside a checked
result variant.

Defer density/PERCOM discovery, 256-byte sector convenience functions and boot
sector exceptions to a separate drive-profile design. The generic transport
passes through the caller's length. Do not infer sector size from a status
bit or offer an ambiguous ReadSector that guesses it. Formatting is not needed
to prove the API and has no first-version convenience wrapper.

Raw sector access bypasses DOS file/cache coordination. Examples must use a
known test image and avoid concurrent DOS access to that disk; the library does
not flush DOS caches or implement filesystems.

## Examples using generic SIO and the disk helper

Both clients read drive 1, sector 1 using a 128-byte buffer and timeout 7.
The generic client is available as [read-sector.act](../samples/sio/read-sector.act),
with [build and usage instructions](../samples/sio/README.md). The disk-helper
client remains a proposed example until that optional module is added:

```action
MODULE SIO_DEMO
USE ATARI.SIO AS SIO
USE ATARI.SIO.DEVICES AS DEV
USE ATARI.SIO.DISK.COMMANDS AS CMD
USE SYS

BYTE ARRAY sector(128)

PROC Main()
  SIO.Request request
  request.device=DEV.DISK
  request.unit=1
  request.command=CMD.READ
  request.timeout=7
  request.aux.word=1
  request.phase=SIO.Transfer.READ(@sector(0),128)

  LET result=SIO.Execute(request)
  CASE result OF
  WHEN SIO.Result.OK THEN
    SYS.PrintE("Sector read")
  WHEN SIO.Result.ERROR(status) THEN
    SYS.PrintBE(status)
  ESAC
RETURN
ENDMODULE
```

For a device whose auxiliary bytes are independent, replace the word assignment
with assignments to `request.aux.bytes.low` and `.high`. For a command without
a data frame, assign `SIO.Transfer.NO_DATA`. No fake buffer is needed. Applications
should not treat an unconstructed Request as a default no-data command.

The optional disk helper reduces the command setup to one call. Its argument
order is unit, sector, destination, timeout; error handling remains identical:

```action
MODULE DISK_DEMO
USE ATARI.SIO AS SIO
USE ATARI.SIO.DISK AS DISK
USE SYS

BYTE ARRAY sector(128)

PROC Main()
  LET result=DISK.ReadSector128(1,1,@sector(0),7)
  CASE result OF
  WHEN SIO.Result.OK THEN
    SYS.PrintE("Sector read")
  WHEN SIO.Result.ERROR(status) THEN
    SYS.PrintBE(status)
  ESAC
RETURN
ENDMODULE
```

Both examples rely on the caller's buffer and chosen media using the expected
sector size. The helper provides no additional buffer validation or sector-size
discovery. Its separate StatusResult is useful because successful status calls
also return the four received bytes as a value.

## Implementation and acceptance slices

1. **Device/command constants and transport with OS-boundary tests (implemented).** Add
   `embedded/modules/atari/sio/devices.act` with the documented public BYTE
   constants, plus `embedded/modules/atari/sio/disk/commands.act` and
   `embedded/modules/atari/fujinet/net/commands.act` with the command catalogue.
   Then add `embedded/modules/atari/sio.act` with public types, status constants,
   DCB overlay and Execute, with only OK/ERROR results and unchanged input
   bytes/counts. The embedded VFS discovers these files;
   no SYS prelude addition or manual module registry is
   needed. ATARI is a reserved embedded namespace; a host file on a module search
   path cannot supply the missing production module. Verify the example through
   the real module loader once embedded. Test unsupported
   native use explicitly; do not claim portable execution merely because types
   can be laid out there. If earlier import rejection is desirable, use a
   general target-capability mechanism, not routine-name special cases. Add a
   focused SIO target under `tools/vm-runtime-tests/tests/`, using the existing
   pinned VM and union/variant
   compiler matrix. Intercept `$E459` or install a test-only 6502 service stub;
   the compiler-generated DCB stores, OS call, status load and result handling
   must execute. Do not substitute a host implementation of Execute. Include
   FujiNet command fixtures: a 256-byte open payload, four-byte network status,
   and read/write counts in the auxiliary word alongside generic directions.
2. **FujiNet primitives and convenience API.** Add
   `embedded/modules/atari/fujinet/net.act` with Channel/DefaultChannel, named
   defaults, Open/OpenBuffer, Status, Read/ReadAvailable, Write/WriteString and
   Close. Finalize the typed status/operation/read results and network-specific
   validation reasons, keeping the agreed Error naming within each module.
   Pin the firmware command contract. Compile small client examples before
   freezing signatures; common calls must need no SIO.Request, raw direction,
   device ID or repeated timeout arguments. Use a stateful test service for
   command ordering, equivalent string/span paths, settings snapshots,
   independent channels, ownership and cleanup. The implementation uses
   SIO.Execute and imports its network command constants; no separate DCB code
   in NET.
3. **First network application and acceptance.** Add an HTTP fetch example with
   a small fixed buffer, caller-controlled waiting/deadline, EOF handling and
   explicit Close. Initialize channel defaults once and select HTTP_GET by name;
   use the convenience API throughout. Fetch a deterministic resource larger
   than that buffer; test binary contents and a TCP echo write/readback path
   using named READ_WRITE access and both Write and WriteString. Record the
   code-size/copy measurements below before broadening the API. Run through an
   emulator with a verified FujiNet bridge/software device, or a physical Atari
   and FujiNet. Record OS, compiler/runtime, firmware revision, connection path
   and server fixture. An ordinary ATR mounted in an emulator proves no FujiNet
   networking behavior. Distinguish service stubs, accelerated SIO and serial
   emulation/hardware results; do not infer electrical timing from VM tests.
4. **JSON application milestone.** Add `embedded/modules/atari/fujinet/json.act`
   and a sample that selects a value from a controlled JSON response. Test mode
   changes, parse/query errors, missing paths, bounded results and cleanup.
   Keep adapter configuration and AppKeys as subsequent focused slices.

Subsequent slices have these bounded deliverables:

- **Adapter and AppKey conveniences.** CONTROL first exposes adapter/Wi-Fi
  status, then host/device slots, directory iteration and mount/unmount wrappers.
  APPKEY exposes explicit key identity, get/set operations and bounded data
  results. Preserve primary errors through close/cleanup, reject oversize writes
  and validate returned lengths before copying. Each slice gets a small client
  example and pinned protocol fixtures; neither requires a complete CONFIG UI.
- **Optional buffered reader.** Add caller-owned state/storage, bounded Poll and
  byte/block consumption. Verify ring wrap, full/empty transitions, partial
  failures, pending EOF/error after buffered data, and two independent readers.
- **Optional PROCEED notification.** Add explicit attach/detach and an ISR that
  records an event for foreground polling. Verify previous-handler/state
  restoration, notification races/coalescing, and operation with notification
  disabled. Validate with a FujiNet connection that supports the signal.

The disk companion can be added when an example needs it, with a known scratch
ATR and missing-drive/write-protected tests. Keep sector helpers as small
request builders returning SIO.Result; Status adds only its typed success
payload. It is not a prerequisite for slices 2–4. Completion of the SIO transport
alone is not completion of FujiNet support.

Focused checks must cover:

- Device constants have the documented literal values and BYTE type, resolve
  through qualified/aliased imports, and need no transport routines or runtime
  initialization. Verify base-plus-unit mapping separately, including D2=`$32`
  and the second FujiNet network channel=`$72` in the OS service fixture;
  Execute itself copies DDEVIC/DUNIT without computing the bus address. Preserve
  pass-through of custom device IDs outside the catalogue.
- Command constants have their protocol's literal BYTE values, coexist with
  similarly named routines through module aliases, and require no protocol
  implementation or runtime initialization. Check disk WRITE=`$50`, disk
  WRITE_VERIFY=`$57` and network WRITE=`$57` independently. Ensure command-only
  imports work before their parent wrapper modules exist, and extension
  commands pass through the generic transport unchanged.
- Literal DCB snapshots for all four directions, low/high bytes at page
  boundaries, auxiliary word/byte aliases, and a second request after the OS
  overwrites DSTATS. Every byte, including reserved and NO_DATA fields, is checked.
- Use a snapshot-only OS stub to check pass-through of zero/nonstandard
  device, unit, command and timeout bytes, zero counts and arbitrary pointer
  bits. It must not dereference these synthetic spans. Execute performs no
  range checks or payload accesses while marshalling. Invalid variant tags
  separately follow the language fault path before OS entry.
- Success, BREAK, documented errors and unknown status bytes. Set A to a
  deliberately different value from Y/DSTATS in the stub to catch a wrong ABI.
  Distinguish the SIO return status from a disk's returned status payload.
- Exactly one vector entry per constructed request, no hidden library retry;
  receive failures with partial writes, WRITE source preservation under the
  service contract, WRITE_READ send-before-overwrite and untouched buffer
  canaries/tails for valid spans. If disk helpers are included, check that their
  DCB bytes, payload effects and results match the equivalent generic requests,
  including untouched tails after a 128-byte transfer. Snapshot-only stubs
  verify that disk unit/sector/timeout values pass through unchanged; no local
  rejection or extra OS call is added. Verify the four-byte status success
  payload and unchanged ERROR status separately.
- Argument evaluation once, union/value snapshots, result retention across a
  later call, and writes visible through ordinary buffer aliases after SIO.
  Capture side-effecting arguments before DCB marshalling begins.
- Both public modern Atari backends and runtimes, plus existing raw/optimized
  NIR execution lanes where applicable. Normalize host fixture text and exercise
  LF/CRLF through actual module loading/instrumentation; preserve guest bytes.
- Exact Atari layout assertions, compatibility rejection and unsupported-target
  diagnostics. Do not assert 6502 layout on MIR68K or MIR65816.

FujiNet checks additionally cover:

- DefaultChannel initializes every field and causes no I/O. Named defaults,
  per-channel overrides and by-value capture are deterministic; modifying one
  channel's settings does not affect another. Validate zero timeout/chunk limits
  and unsupported access/translation settings before marshalling.
- Network helper rejections use NET.Error, cause no DCB writes/OS entry, and
  remain distinct from the raw SIO status. Check capacity bounds and URI staging
  limits without claiming pointer/allocation validation.
- Exact device/unit mapping, command-specific auxiliary bytes, fixed Open
  payload initialization, URL termination, overlong input rejection and rejection
  of conflicting channel/unit prefixes.
- Equivalent Open/OpenBuffer inputs produce identical DCB/payload bytes. Cover
  empty, one-byte, maximum accepted and oversized URIs plus embedded terminators.
  WriteString excludes the Action! prefix, preserves embedded binary bytes,
  appends nothing and handles an empty string with no SIO entry.
- Status and data counts above 255, several reads per response, empty responses,
  WAITING before data, EOF after buffered data, and disconnects during a request.
- Binary data containing zero, LF, CR, `$9B` and `$FF`; translation only when
  explicitly selected, with host LF/CRLF fixture handling kept separate.
- Absent FujiNet, invalid device specifications, device-side network failures,
  unknown extended codes and a failed diagnostic query after an original error.
- No endless internal polling, overall deadline/cancellation behavior, Close
  after success or failure, and preservation of the primary error if Close fails.
- Independent state for two network units, protocol/JSON mode transitions and
  unsupported optional commands. Tests must not require a public Internet
  service; use recorded protocol fixtures and controlled local endpoints.

## Usability and code-cost acceptance

Before broadening the API, compare the same short open/status/read/write/close
sequence through a handwritten Action! DCB caller, SIO.Execute and the NET
convenience functions. Include the matching Mad Pascal wrappers as a comparison
baseline when built with recorded compiler settings. Use identical payloads and
SIO service results, and distinguish differences in validation and behavior.

Record linked code bytes, static workspace and staging-buffer sizes, aggregate
argument/result copies and wrapper execution cycles with OS service time
excluded. Cover modern classic/MIR6502 and both Atari runtimes, recording runtime
dependencies separately. The 256-byte URI staging buffer is a real cost even
when the source call is short. Verify that unused JSON, CONTROL, APPKEY and
BUFFERED modules bring no code, workspace or initialization into a NET client.

Keep the first HTTP/TCP samples readable with one channel initialization,
named options, short operations and CASE handling. Compare those samples with
Mad Pascal's convenience calls as a usability check. Preserve documented
protocol-helper checks and returned results when comparing code sizes.
The first goal is correct and readable source; no numerical performance target
is claimed before measurement. If a compiler defect is exposed, repair it with
a general focused regression in a separate change.
Library/test-only changes use affected suites under [AGENTS.md](../AGENTS.md);
shared compiler changes require the broader checks there.

## Design validation

[Embedded-module tests](../tests/embedded_modules.rs) verify public BYTE
constants and independent command-module imports, private DCB layout/visibility,
compatibility rejection, native execution rejection and the generic sample's
four advertised backend/runtime builds.

[SIO VM tests](../tools/vm-runtime-tests/tests/sio.rs) execute the production
Execute body against a test OS vector. They check all twelve DCB bytes, all four
transfer forms, zero/nonstandard input pass-through, page-crossing spans,
partial reads, send-before-overwrite, buffer tails, argument/result snapshots,
all 256 return status bytes and invalid-tag faults before OS entry/DCB writes.
The service supplies only peripheral effects and a 6502 return stub; it does not
replace the transport. Coverage includes both modern Atari backends/runtimes
and raw/optimized NIR. Additional CRLF runs use identical embedded module bodies
under a temporary host namespace because reserved ATARI modules cannot be
overridden by host files.

The disk helper declarations and example were syntax-checked with temporary
implementations during design, but that optional module is not implemented.
FujiNet call shapes, defaults and later slices remain proposals. The boundary
fixtures include network-shaped Open/Status/Read/Write requests; they do not
establish firmware compatibility, serial timing or a working network connection.
No hardware/emulator FujiNet acceptance or Mad Pascal code-size comparison has
been validated yet.
