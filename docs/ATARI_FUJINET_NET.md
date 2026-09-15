# ATARI.FUJINET.NET

Synchronous HTTP/TCP byte-stream helpers for Atari 8-bit programs. Import
`ATARI.FUJINET.NET AS NET`; the module uses `ATARI.SIO` and the existing OS SIO
vector. No CIO `N:` handler is required. Use modern classic (`--mode optimized`)
or MIR6502, with the cartridge or standalone runtime and a compatible Atari OS.
The library is foreground-only and nonreentrant.

The [implementation](../embedded/modules/atari/fujinet/net.act) fixes its wire
contract to **FujiNet firmware v1.6.1**, commit
`d7a5b2faac61a7889874849bf5db6f82718e42c1`. This is a source-level protocol
baseline, not a claim of live firmware/hardware acceptance. The
[HTTP example](../samples/fujinet/http-fetch.act) is ready for that next step.

## Channels and calls

```action
USE ATARI.FUJINET.NET AS NET
STRING uri="N:HTTP://example.com/"
BYTE ARRAY buffer(128)

; Inside a routine:
NET.Channel channel
channel=NET.DefaultChannel(1)
channel.access=NET.HTTP_GET
LET opened=NET.Open(channel,uri)
```

Pass named `STRING` variables to Open/WriteString in code shared by both
backends. The classic backend currently cannot capture a string literal
argument to these functions returning aggregate values. The named-string path
is covered by both backends; correcting literal capture is a separate compiler
follow-up. A STRING contains its byte count at index 0 and content at index 1.

`Channel` contains BYTE `unit`, `timeout`, `access`, `translation`, and CARD
`chunkLimit`. `DefaultChannel(unit)` performs no I/O or validation; it returns
the supplied unit with `DEFAULT_TIMEOUT=15` OS seconds, `READ_ONLY`, `BINARY`,
and `DEFAULT_CHUNK=256`. Configure it once and pass it by value to each call.

Own the selected unit exclusively through Close, including coordination with
CIO users. A copied Channel is another settings value for the same endpoint;
it does not allocate a new connection. There is no hidden current channel or
open-state cache. Access and translation affect Open only; changing those
fields does not reconfigure an existing connection. `chunkLimit` only bounds
ReadAvailable, not exact Read/Write.

| Function | Remaining arguments after Channel | Result |
| --- | --- | --- |
| Open | STRING uri | Result |
| OpenBuffer | BYTE POINTER address, CARD length | Result |
| Status | none | StatusResult |
| Read | BYTE POINTER destination, CARD length | Result |
| ReadAvailable | BYTE POINTER destination, CARD capacity | ReadResult |
| Write | BYTE POINTER source, CARD length | Result |
| WriteString | STRING text | Result |
| Close | none | Result |

Public access constants are `READ_ONLY=4`, `WRITE_ONLY=8`, `READ_WRITE=12`,
`HTTP_GET=12`, `HTTP_POST=13`, and `HTTP_PUT=14`. The HTTP implementation uses
mode 4 for GET with file-resolution behavior and 12 for direct GET; choose
HTTP_GET explicitly. Other firmware modes, directory operations, datagram
boundaries, listening sockets and HTTP header/channel modes are outside this
initial stream API. Protocol-specific suitability of a named mode and URI
syntax remains the firmware's responsibility.

Translation constants are `BINARY=0`, `TRANSLATE_CR=1`, `TRANSLATE_LF=2`, and
`TRANSLATE_CRLF=3`. These request FujiNet's ATASCII line-ending conversion;
NET performs no local conversion of stream bytes. Binary writes preserve
embedded zero bytes.

## Results and local checks

```action
PUBLIC TYPE Result=VARIANT [
  OK
  SIO_ERROR [BYTE status]
  INVALID [Error reason]
]
PUBLIC TYPE StatusFields=[CARD available BYTE connected BYTE error]
PUBLIC TYPE NetworkStatus=UNION [StatusFields fields BYTE ARRAY bytes(4)]
PUBLIC TYPE StatusResult=VARIANT [
  OK [NetworkStatus value]
  SIO_ERROR [BYTE status]
  INVALID [Error reason]
]
PUBLIC TYPE ReadResult=VARIANT [
  DATA [CARD count]
  WAITING
  END_OF_STREAM
  SIO_ERROR [BYTE status]
  NETWORK_ERROR [BYTE code]
  INVALID [Error reason]
]
```

`SIO_ERROR` contains the original OS DSTATS byte; only `$01` is successful.
`NETWORK_ERROR` contains the separate extended error from a successful network
Status response. `StatusResult.OK` preserves all four bytes, including unknown
connection/error values. Receive into the union first, then construct OK;
failed or partial Status transfers never expose a success payload.

NET's public `Error` enum has these stable values:

| Reason | Value | Checked condition |
| --- | ---: | --- |
| BAD_UNIT | 1 | Unit is outside 1..8. |
| BAD_TIMEOUT | 2 | Timeout is zero. |
| BAD_CHUNK_LIMIT | 3 | Chunk limit is zero. |
| BAD_ACCESS | 4 | Access is not one of the named supported values. |
| BAD_TRANSLATION | 5 | Translation is outside 0..3. |
| BAD_LENGTH | 6 | Exact Read/Write length or ReadAvailable capacity is zero. |
| BAD_URI_LENGTH | 7 | Open content length is outside 1..255. |
| BAD_URI_TERMINATOR | 8 | URI content contains NUL or ATASCII EOL `$9B`. |
| BAD_URI_UNIT | 9 | A numbered N/n prefix is not the selected single-digit unit followed by `:`. |

Every I/O operation checks channel settings in the order above, followed by
its length/content checks. Invalid input causes no DCB stores or SIO calls.
After channel validation, empty WriteString succeeds without I/O; nonempty
WriteString uses Write. These are NET helper policies. Generic `SIO.Execute`
still forwards input bits unchanged and has no INVALID result.

Pointers and strings remain caller-owned for the entire synchronous call.
Provide accessible source storage or writable destination storage of the stated
size. Helpers cannot discover allocations or make invalid pointer spans safe;
do not overlap buffers with the DCB, OS/runtime work areas or library storage.
NET does not retain pointers after returning.

## Wire contract

The DCB uses device base `$71`, DUNIT 1..8, and the captured timeout. Reserved
bytes and unused auxiliary bytes are zero. The OS forms physical IDs `$71..$78`.
Command constants are also available independently through
`ATARI.FUJINET.NET.COMMANDS`.

| Command | Direction/count | AUX1/AUX2 |
| --- | --- | --- |
| SET_TRANSLATION `$54` | No data, buffer/count zero | Both zero |
| OPEN `$4F` | Write exactly 256 bytes | Access / translation |
| STATUS `$53` | Read exactly 4 bytes | Both zero |
| READ `$52` | Read the exact caller count | Same count, little endian |
| WRITE `$57` | Write the exact caller count | Same count, little endian |
| CLOSE `$43` | No data, buffer/count zero | Both zero |

Open stages 1..255 content bytes, one NUL terminator and zero padding in private
256-byte storage. OpenBuffer reads exactly its span; Open excludes the Action!
length prefix and uses the same implementation. Neither reads 256 bytes from a
short caller allocation. `N:` leaves endpoint selection to Channel; `N2:` must
match unit 2. NET does not rewrite other URI syntax or duplicate the firmware's
URI parser. Use ordinary ASCII device specifications; upstream performs its own
normalization of high-bit characters and DOS-style syntax.

After staging and all validation, Open sends SET_TRANSLATION with zero, then
OPEN. This reset is necessary: the pinned firmware otherwise combines Open's
AUX2 with a sticky translation override left by earlier users. A failed reset
returns its SIO error and prevents OPEN; a failed OPEN returns its own error.
There are no implicit retries, diagnostic Status calls or cleanup calls.

WriteString sends exactly the counted content, without its prefix or an added
terminator/newline. Read and Write allow counts 1..65535, subject to accessible
caller storage; the configured chunk limit does not change those exact requests.
The firmware uses the auxiliary count, so it must equal DBYT.

## Bounded reads and failure ownership

ReadAvailable validates capacity, performs one Status, and at most one Read of
`min(available, capacity, chunkLimit)`. Its decisions are:

| Successful Status snapshot | Outcome |
| --- | --- |
| Error is neither `NETWORK_SUCCESS=1` nor `NETWORK_EOF=136` | NETWORK_ERROR with the unchanged byte, even if available is nonzero |
| Available is positive, error is success or EOF | Read a bounded chunk; DATA(count) only after exact SIO success |
| Available is zero, error is EOF | END_OF_STREAM |
| Available is zero, error is success | WAITING |

The connection byte alone does not establish EOF. Buffered bytes are drained
when an EOF status accompanies a positive available count. ReadAvailable does
not loop internally; the caller controls polling, cancellation and the overall
deadline. A Status/read race can still produce an SIO failure.

A failed read may have changed part of the destination. SIO provides no reliable
partial count, so the helper never returns DATA for that failure. A failed write
may already have sent bytes; do not retry it blindly. Keep the primary outcome
across explicit diagnostic Status or cleanup Close calls. Their failures are
separate values and must not replace the original cause. Applications needing
firmware-specific extended diagnostics after an SIO error query Status explicitly.

## References and validation

All source references below use the same pinned firmware commit:

- [Atari network commands](https://github.com/FujiNetWIFI/fujinet-firmware/blob/d7a5b2faac61a7889874849bf5db6f82718e42c1/lib/device/sio/network.cpp): `sio_open`, `sio_read`, `sio_write`, `sio_status`, `create_devicespec`, `sio_set_translation`.
- [Network device storage](https://github.com/FujiNetWIFI/fujinet-firmware/blob/d7a5b2faac61a7889874849bf5db6f82718e42c1/lib/device/sio/network.h) and [SIO bus](https://github.com/FujiNetWIFI/fujinet-firmware/blob/d7a5b2faac61a7889874849bf5db6f82718e42c1/lib/bus/sio/sio.h): 256-byte device specification and eight network endpoints.
- [Device-specification normalization](https://github.com/FujiNetWIFI/fujinet-firmware/blob/d7a5b2faac61a7889874849bf5db6f82718e42c1/lib/utils/utils.cpp): `util_devicespec_fix_9b` and `util_devicespec_fix_for_parsing`.
- [Protocol constants](https://github.com/FujiNetWIFI/fujinet-firmware/blob/d7a5b2faac61a7889874849bf5db6f82718e42c1/lib/network-protocol/Protocol.h), [HTTP](https://github.com/FujiNetWIFI/fujinet-firmware/blob/d7a5b2faac61a7889874849bf5db6f82718e42c1/lib/network-protocol/HTTP.cpp), [TCP](https://github.com/FujiNetWIFI/fujinet-firmware/blob/d7a5b2faac61a7889874849bf5db6f82718e42c1/lib/network-protocol/TCP.cpp), and [extended errors](https://github.com/FujiNetWIFI/fujinet-firmware/blob/d7a5b2faac61a7889874849bf5db6f82718e42c1/lib/network-protocol/status_error_codes.h) define the selected mode/status meanings.

[NET VM tests](../tools/vm-runtime-tests/tests/fujinet_net.rs) run the actual
library and transport through a test OS vector, with a stateful peripheral
service. They verify complete DCB frames, URI padding, sticky translation reset,
independent channels/settings, binary strings and page-crossing spans,
waiting/EOF/error transitions, partial reads, preserved primary errors and local
validation without DCB writes. They cover both Atari backends/runtimes, raw and
optimized NIR, and CRLF module loading. These fixtures model the pinned contract;
they do not execute the firmware or prove serial timing or live network behavior.

The [embedded-module tests](../tests/embedded_modules.rs) check constants, wire
layout and all four sample build configurations. Real HTTP/TCP acceptance and
code-size/copy comparisons remain slice 3 of the
[SIO design](ATARI_SIO_LIBRARY_DESIGN.md#implementation-and-acceptance-slices).
