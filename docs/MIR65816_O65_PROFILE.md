# Experimental Action native o65 profiles v1 and v2

Identities: `actionc.o65.experimental.v1` and `.v2`. Native ABI: `action65816.native.v1`.
This is an experimental application container; it does not change the ABI or
JSON image contracts. See the [implementation plan](MIR65816_O65_IMPLEMENTATION_PLAN.md)
and [format/tool assessment](MIR65816_O65_ASSESSMENT.md).

Options default to `actionc.o65.experimental.v2`, permitting the arithmetic
fault extension. The writer chooses the minimum required format: v2 only when
`__a816_arithmetic_fault_v1` is imported, otherwise byte-compatible v1.
Explicit v1 options reject programs requiring that extension. Old readers
reject the new descriptor export/version before interpreting its contract.

## Wire contract

The o65 header is the standard six-byte magic `01 00 6f 36 35 00`, followed by
little-endian mode `$A202`: native 65816, 32-bit sizes, executable, bytewise
relocations, BSS clearing, four-byte alignment. Simple/chained flags are clear.
All nine header `.size` fields are u32, as are import/export counts, import
indices and export values. Original text/data/BSS bases are zero and the zero
segment is empty. The stack-size hint is zero (unknown). Header options end
with zero. No timestamps or paths are emitted.

Text contains bank-contained routines, readonly objects and the descriptor;
data contains initialized writable objects, including explicit zero tails;
BSS contains entirely zero-initialized writable allocations. Gaps are zero.
Code offsets stay fixed within banks. Each routine is at most 65,535 bytes and
leaves the final byte of its bank unused. Aliases do not allocate storage.

Relocations use the standard LOW `$20`, HIGH `$40`, WORD `$80`, SEG `$A0`, and
SEGADR `$C0` encodings. Segment IDs are undefined=0, absolute=1, text=2, data=3,
BSS=4, zero=5. HIGH carries the original low byte; SEG carries the original
low word. Undefined entries contain the u32 import index before carry bytes.
The stream cursor starts at -1; `$FF` advances by 254; zero terminates.
Exports contain a segment ID and original symbol value, not a file offset.
For this profile original bases are zero, so symbol values equal offsets.

The writer uses LONG/LOW/HIGH/BANK for movable compiler references. The
structural codec also understands WORD, but the profile rejects movable full
one-/two-byte addresses rather than changing checked narrowing to truncation.
A four-byte address container has a three-byte LONG patch and a zero fourth
byte. Absolute addresses are patched once. ImageEnd, nonzero import addends,
stronger alignment, fixed routine placement and unresolved runtime interfaces
have explicit diagnostics. Internal signed addends must yield an effective
offset inside the target section or one-past, with no 24-bit overflow at load.

## Descriptor

Two text exports are required: `__a816_entry_v1` and
`__a816_o65_profile_v1` (version 1) or `__a816_o65_profile_v2` (version 2). The latter locates this packed descriptor (no implicit
padding). All numeric fields are unsigned little-endian except byte tags.
Vectors have a u32 count; strings have a u32 byte length followed by UTF-8,
without a terminator. Strings are at most 4096 bytes, vectors at most 1,000,000
items, the file at most 128 MiB, and each section below 16 MiB. Decoding also
checks counts against remaining bytes before allocation. Linker names use
case-sensitive ASCII `[A-Za-z_][A-Za-z0-9_]*`.

| Record | Fields in wire order |
| --- | --- |
| Descriptor header | `A8O1` (4 bytes), total descriptor length u32, version u16=1 or 2, required features u16=0, pointer bytes u8=3, endian u8=0, NMI extra stack u16, entry text offset u32 |
| Descriptor body | routine vector, object vector, import vector, relocation-check vector |
| Contract | ABI string, signature u32, argument vector, result u8, incoming extent u32, stack peak u16, IRQ effect u8, kind u8, domains u8 |
| Argument | offset u32, size u32, alignment u32 |
| Routine | ID u32, debug name string, text offset u32, size u32, contract, frame u16, spills u16, local peak u32 |
| Object | kind u8, ID u32, debug name string, location tag u8, offset/address u32, size u32, alignment u32, mutable u8, alias u8 |
| Import | linker name string, contract |
| Relocation check | site section u8, site offset u32, encoding u8, target tag u8, import index u32, complete effective target offset u32, zero-extend u8 |

Object kinds are global=0, static=1, array backing=2. Location tags are
absolute=0, text=2, data=3, BSS=4. Relocation target tags are import=0 or the
section ID; a section target's import-index field is zero. Boolean bytes are
exactly zero/one. Result tags are void=0, A8 zero-extended=1, A16=2,
A16/X8 zero-extended=3, A16/X16=4. IRQ effects are preserve=0,
save-disable=1, restore=2. Domains are task=1, IRQ=2, both=3; never NMI.
Kind 0 denotes an ordinary returning checked scalar interface. Kind 1 is the
raw, nonreturning overflow adapter, with its specified register inputs rather
than stack arguments. Its signature/argument/result/incoming/peak/effect fields
are zero and domains=3. Kind 2 is the raw terminal arithmetic-fault adapter
with the same zero fields and domains=3, admitted only in profile v2; its A/X
register meanings differ from overflow. Unknown tags/features are rejected.

Complete effective offsets in the relocation-check vector preserve the bounds
information missing from split-byte relocations. They are independently checked
against the standard relocation records and original payload bytes before
patching. This redundancy is intentional for the experiment and included in
size measurements. Descriptor bytes cannot be relocation sites. No compiler
sidecar is required. Metadata does not prove arbitrary code follows the ABI.

Debug names and numeric IDs are only maps. Entry identity comes from the MIR
entry fact. External names come from the explicit binding table, not parsing
source/debug names. Each provider must match both the signature identity and
physical contract; numeric signature equality alone is insufficient.

## Loading and ownership

The host provides three final bases, allowed/reserved regions, NMI allowance
and named providers with extents/contracts. The platform NMI allowance must
match the descriptor; the task/IRQ floor setup uses that declared allowance. Nonempty application sections must
fit upper RAM; text starts on a 64 KiB boundary, data/BSS on a four-byte
boundary. Check all allocation, import and reserved extents for overlaps and
24-bit overflow. Validate routine containment and all complete target offsets,
including one-past bounds, before writing a private output copy. A failure
publishes nothing and writes no guest memory. BSS is cleared explicitly.

`__a816_stack_overflow_v1` is always the first named raw import. Version 2
also imports `__a816_arithmetic_fault_v1`, with contract kind=2, signature=0,
no ordinary arguments/result/incoming area, stack peak=0, preserved IRQ effect
and task/IRQ domains=3. `Contract::arithmetic_fault()` constructs this exact raw
contract. At transfer A16=1 denotes DivisionByZero, X16=S, native M=X=0,
decimal clear, DBR=0, D/I unchanged; JML adds no return address. The entry is
terminal, with no unwind or result store. It cannot be supplied as an ordinary
returning routine or the stack-overflow provider. Providers must match the
entire named contract and supply nonoverlapping, valid address/size extents.
The relocated image exposes `arithmetic_fault()` as an optional address.

Compiler-owned arithmetic helpers occupy ordinary routine records and text
fixups; their stable typed identities drive dependency selection. Only needed
helpers are emitted, once each, and each body stays in one program bank.

 Ordinary providers
must implement checked reservations and the declared IRQ effect/domain rules.
No application scratch is allocated through o65's zero segment. The host owns
DP, stack, vectors, task lifetime and scheduling; it initializes D and stack
floor/ceiling using ABI headroom (task 26+NMI extra, IRQ 13+NMI extra). Local
frame/peak maps never claim a bound for recursion or unknown indirect depth.
The entry retains its native signature. There is no new process-startup ABI.

## Qualification and measured costs

The [2026-09-21 qualification](abi/action65816-o65-qualification.json) passes all
44 native tests in debug and release, including seven o65 execution groups.
Sixteen root codec/loader/CLI tests pass in both builds; a separate range-index
regression checks empty, nested, adjacent and disjoint regions against individual
containment/overlap semantics. Independent fixtures/decoding cover all five wire
relocation forms, wide counts and carry bytes. The compiler profile still
rejects full narrow moving addresses, `ImageEnd` and nonzero import addends.

Each raw or optimized artifact is reused unchanged at two placements. Tests
cover direct/indirect calls, mixed scalar ABI results, 64-byte scratch clobbers,
split/full static pointers, aliases, initialized zero tails, zeroed BSS, fresh
mutable loads, multiple code banks, relocated faults and two live task domains
under IRQ/NMI. They execute the emitted bytes
on the independent native VM. No Exec816 loader or hardware qualification is
claimed.

Representative sizes in bytes, shown as raw / optimized:

| Probe | Machine code | Total file | Descriptor | Observed stack |
| --- | ---: | ---: | ---: | ---: |
| Pointer/indirect call | 477 / 435 | 1533 / 1491 | 850 / 850 | 36 / 24 |
| Live values across imports | 775 / 691 | 2017 / 1933 | 995 / 995 | unmeasured |
| Recursive task contexts | 7174 / 6854 | 15657 / 15338 | 7433 / 7433 | unmeasured |
| Multi-bank code | 74673 / 74673 | 78662 / 78662 | 2545 / 2545 | unmeasured |

Stack observations include the independent caller's outgoing bytes and complete
call chain, relative to its initial S; they are fixture measurements, not general
task bounds. The pointer probe takes 773 / 669 VM cycles. Descriptor redundancy
is a substantial cost for small programs. Reducing it needs a separate profile
change that preserves validation of complete address values and ABI maps.
