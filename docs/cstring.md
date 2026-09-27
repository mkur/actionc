# C strings on native 65816

`c"Hello\n"` (also `C"..."`) emits single-byte text followed by one NUL.
There is no length prefix or allocation. Ordinary Action! strings are unchanged.
Supported escapes are `\n \r \t \b \f \v \a \\ \" \' \? \0` and `\xHH`
with exactly two hexadecimal digits. Physical newlines, unknown escapes and
characters above 255 are errors. Embedded NUL is allowed; the final NUL is still
appended. `\n` is byte 10, independent of the host's newline convention.

`SYS.CSTRING`, or contextual `CSTRING`, is a read-only borrowed byte view.
It occupies the native data pointer's three bytes, including in parameters,
function results and record fields. Assignment copies the address; equality
compares addresses. `text(i)` and `text^` read bytes; stores through the view
are rejected. `SIZEOF(CSTRING)` is 3, not the text length. `CSTRING(0)` is null,
distinct from the non-null empty literal `c""`.

Use `CSTRING(buffer)` to borrow byte storage and `BYTE POINTER(text)` to bridge
to an existing raw-pointer API. Both are explicit, nonallocating casts. The raw
pointer bridge does not make literal backing writable. These are not ownership,
lifetime or bounds checks. Literal-expression backing is immutable and lasts
as long as its containing image; the source buffer controls a borrowed view's
lifetime. There is no implicit conversion from Action! strings or raw pointers.

```action
BYTE ARRAY greeting=c"Hello\n" ; seven writable bytes, including NUL
BYTE ARRAY path(32)=c"D1:"    ; excess capacity is zero-filled
CSTRING text
PROC Main()
  text=c"static backing"
  LET borrowed=CSTRING(path)
RETURN
```

Array initialization includes NUL and rejects insufficient capacity. Scalar
Action! declarations have storage/address initialization rules: initialize a
CSTRING view with an assignment in a routine or a `LET`, rather than a bare
`CSTRING text=c"..."` declaration. Array lifetime follows the existing global
or per-activation local declaration rules. Near/far qualifiers are deferred.
This first implementation explicitly rejects CSTRING on other targets.

## Library

`USE CSTRING AS STR` imports the external interface. A host supplies the eight
versioned providers in [the contract](../embedded/modules/cstring/contract.json).
Only referenced functions become imports. `USE CSTRING.IMPL AS STR` explicitly
includes the same implementation for standalone programs; ordinary module
inclusion currently includes all eight functions. No allocator, OS services,
global working storage, initialization, errno or cleanup are involved. Decimal
conversion shares a 40-byte table of powers of ten, which it never modifies. Calls use the
normal checked native ABI and the caller's stack.

| Function | Contract |
| --- | --- |
| `SIZE strlen(CSTRING text)` | Count bytes before NUL. |
| `SIZE strnlen(BYTE POINTER bytes, SIZE maximum)` | Inspect at most maximum bytes; raw buffer need not be terminated. |
| `INT strcmp(CSTRING left, CSTRING right)` | Unsigned byte comparison; negative, zero or positive result. |
| `INT strncmp(CSTRING left, CSTRING right, SIZE maximum)` | Compare at most maximum bytes, stopping at NUL. |
| `CSTRING strchr(CSTRING text, BYTE value)` | First match or null; searching zero finds the terminator. |
| `SIZE strlcpy(BYTE POINTER destination, CSTRING source, SIZE capacity)` | Copy at most capacity−1 bytes and terminate if capacity > 0; return full source length. |
| `SIZE strlcat(BYTE POINTER destination, CSTRING source, SIZE capacity)` | Append after the bounded destination scan; return bounded initial length plus full source length. |
| `SIZE u32toa(LONGCARD value, BYTE POINTER destination, SIZE capacity)` | Convert unsigned 32-bit value to decimal; write at most capacity−1 digits and terminate if capacity > 0; return full digit count. |

Capacity includes the terminator. A copy/append result `>= capacity` means the
complete terminated result did not fit. An unterminated destination within the
append bound stays unchanged. Zero capacity performs no destination access
(and permits a null destination); source length is still counted. Zero compare
or scan bounds perform no input reads. Copy/append do not zero unused padding.
Source and destination must not overlap. Other pointers must identify live,
accessible storage and CSTRING sources must have a reachable terminator.
Counts and their sums must fit SIZE; address wrap is unsupported.

`u32toa` is an Action library extension, not a standard C function. It emits
`0` for zero and otherwise no leading zeros, covering `0` through `4294967295`.
Capacity includes NUL; 11 bytes always suffice. Like `strlcpy`, its result is
the required length excluding NUL, even on truncation: `result >= capacity`
means the complete terminated result did not fit. Zero capacity permits a null
destination and makes no destination accesses. Unused padding stays unchanged.
Only unsigned decimal conversion is provided; signed, radix and general format
strings are outside this small API. Repeated subtraction avoids division helpers.

The [native example](../examples/native65816/cstring.act) appends a filename
and checks capacity. The interface generator's `--check` mode checks source
consistency; frontend tests also compare every semantic signature against the
implementation and contract. Native execution tests cover raw/optimized code,
canaries and observed bounds, large bank-crossing strings and relocation.
