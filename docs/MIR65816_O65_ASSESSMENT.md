# o65 suitability for native 65816 applications

Assessment: 2026-09-21, actionc `f9eb285`. This is a format assessment and
proposed qualification slice, not an implemented output format or loader.

The [implementation plan](MIR65816_O65_IMPLEMENTATION_PLAN.md) defines the
experimental profile, commit-sized slices and execution qualification gates.

## Recommendation

Use o65 as a candidate for an Exec816 relocatable application format. Its
native-65816 mode and relocation encodings fit the compiler's main address
fixups. Adoption needs a documented Exec816 profile and a correct writer;
the tested stock vlink output is not sufficient for general banked images.
Keep the existing JSON image and XEX bootstrap during qualification.

o65 is Andre Fachat's format, supported by vasm/vlink in the vbcc toolchain.
Choosing this container neither requires vbcc nor establishes compatibility
with its generated code's calling convention. It does not require new
near/far/huge source pointer types: Action's current native 24-bit pointers
can remain unchanged.

## Fit with actionc

The [original specification](https://github.com/fachat/xa65/blob/master/xa/attic/doc/fileformat.txt)
describes native 65816 selection, 16- or 32-bit header fields, text/data/BSS/
bank-zero storage, named imports/exports, and relocation tables. That archived
document is version 1.2; the [vlink manual](https://server.owl.de/~frank/vbcc/docs/vlink.pdf)
identifies its `o65-816` target as version 1.3. The implementation was checked
in addition to those documents.

| Compiler requirement | o65 mapping or remaining work |
| --- | --- |
| Full 24-bit code/data references | `SEGADR` relocation |
| Separate low/high/bank address bytes | `LOW`, `HIGH`, `SEG`; carry information accompanies the latter two |
| Low 16 bits of an address | `WORD` relocation |
| Runtime/helper names | Undefined symbol table; Exec must separately validate the ABI |
| Zero-initialized allocation | BSS; the profile must require clearing |
| Fixed hardware addresses | Remain absolute; never relocate them as allocated data |
| Local branch/PER displacements | Resolve within the routine; preserve routine layout and bank containment |

The existing [emitter fixups](../src/mir65816/emit/code.rs) retain target
identity, addends, and byte selectors. The [image linker](../src/mir65816/image.rs)
currently resolves them before serializing JSON. A useful implementation
boundary is to preserve these facts through placement and encode relocations
from them, rather than recover references by scanning final machine code.

Data needs an explicit audit too. A 24-bit address in a four-byte scalar can
use a three-byte relocation plus a fixed zero high byte when its range is
proven. General 32-bit arithmetic relocations are a different requirement.
`ImageEnd` currently means the maximum end of allocated code/data; independently
moving segments can change which end is greatest. It needs a defined loader
symbol, constrained placement, or an unsupported diagnostic in an initial
profile. Storage aliases must resolve to their underlying allocation plus
offset, without creating a second allocation.

## Confirmed tool limitation

Built the official release archives in an isolated temporary directory:

| Tool | Reported version | Download SHA-256 |
| --- | --- | --- |
| [vlink](http://sun.hasenbraten.de/vlink/release/vlink.tar.gz) | 0.18a | `8d151cdd30a4feb575a364e68810c2bc300fe1a7c074dbbee6fd1175a6c5bfae` |
| [vasm](http://sun.hasenbraten.de/vasm/release/vasm.tar.gz) | 2.0f | `c84b2de1cbb87831795fe64a85c5d9a7002a766e3a7c30b0a2d7d5e99d878f49` |

In vlink's `t_o65.c`, `o65outsize` is fixed at two. The reader recognizes
the format's 32-bit size mode, but the writer does not select it. The tested
vasm o65 header writer also uses narrow fields. Native CPU mode and wide
header mode are separate properties.

Two actual vlink output probes returned success without diagnostics:

| Requested output | Encoded result |
| --- | --- |
| Text at `$01:8000`, data at `$12:FFF0` | Header bases `$8000` and `$FFF0`; the JSL operand correctly contains `$12:FFF0` |
| 65,536-byte text section, supplied through vobj | Header text length zero, although the file contains the payload |

This is an implementation limitation, not a 64 KiB limit in the o65
specification. It also does not prevent a correctly encoded small module
with low original bases from being relocated to upper RAM. It does prevent
relying on this writer for arbitrary current actionc image addresses/sizes.
An actionc writer using the standard wide fields, or an upstream writer fix,
is needed before claiming that coverage.

Reproduce with the built binaries on PATH, in a disposable directory:

```sh
cat > probe.s <<'EOF'
 section .text,"acrx"
 global start
start:
 jsl target
 rtl
 section .data,"adrw"
 global target
target:
 byte <target, >target, ^target
 word target
 defl target
EOF
cat > high.ld <<'EOF'
SECTIONS {
 .text 0x018000 : { *(.text) }
 .data 0x12fff0 : { *(.data) }
}
EOF
vasm6502_oldstyle -816 -Fo65 -o probe.o65 probe.s
vlink -b o65-816 -T high.ld -o probe-linked.o65 probe.o65
cat > large.s <<'EOF'
 section .text,"acrx"
 global start
start:
 rtl
 dsb 65535,0
EOF
vasm6502_oldstyle -816 -Fvobj -o large.vobj large.s
vlink -b o65-816 -o large.o65 large.vobj
python3 - <<'PY'
from pathlib import Path
import struct

for name in ('probe.o65', 'probe-linked.o65', 'large.o65'):
    data = Path(name).read_bytes()
    mode = struct.unpack_from('<H', data, 6)[0]
    kind = 'I' if mode & 0x2000 else 'H'
    fields = struct.unpack_from('<' + kind * 9, data, 8)
    print(name, 'file bytes', len(data), 'mode', hex(mode),
          'text base/length', tuple(map(hex, fields[:2])),
          'data base/length', tuple(map(hex, fields[2:4])))
PY
```

The small linked payload is `22 f0 ff 12 6b` for text and
`f0 ff 12 f0 ff f0 ff 12` for data. This checks the emitted address forms;
it is not a guest execution or a runtime-loader qualification.

## Required Exec816 profile

- **Bank placement:** Preserve the low 16 bits of linked code addresses by
  initially relocating text in multiples of 64 KiB. The current linker keeps
  each routine and its continuations within one bank. Generic o65 alignment
  flags, whose largest alignment is 256 bytes, do not express that requirement.
  One text segment requires a contiguous extent; arbitrary scattered banks
  need a later module/segment design. Chaining alone does not define it.
- **Section policy:** Combine readonly data with text in the initial profile,
  or explicitly constrain separately placed readonly data. A single o65 module
  does not have an arbitrary list of independently placed sections.
- **Entry and ABI:** Define an entry symbol or descriptor, profile version,
  Action ABI version, and import contracts. Symbol names alone do not describe
  signatures, register widths, DBR, D, or helper clobbers. Preserve the
  [native ABI](MIR65816_PHYSICAL_ABI_V1.md). In the inspected vlink source,
  header options other than filename are skipped on input: do not depend on
  custom ABI options surviving an ordinary relink. A retained data descriptor
  or metadata attached after final linking is safer.
- **Direct page and preemption:** Start with an empty o65 zero segment for
  application scratch. Exec continues to allocate each task's ABI DP domain
  and establishes D on entry/context switch. An image-wide zero allocation
  must not silently replace task/interrupt context storage.
- **Stack:** Preserve compiler guards, overflow handling, task floor/ceiling
  initialization, and interrupt reserve. The o65 stack field cannot substitute
  for whole-call-chain bounds, especially with recursion and indirect calls.

## Proposed first implementation slice

Preserve typed post-placement relocations and add an experimental o65 writer
plus a host-side reference relocator for a deliberately limited profile.
Use standard 32-bit fields from the start and reject unsupported layouts or
relocation expressions explicitly. No source-language or public ABI change
is necessary.

Compile representative programs in raw and optimized modes, serialize once,
then relocate each exact artifact to two valid upper-RAM placements and
execute both on the independent native VM. Cover full addresses, split-byte
carries at `$xx:FFFF`, data/function pointers, imports, aliases, BSS, code-bank
boundaries, and stack-guard failure. Include 65,535/65,536/65,537-byte section
encoding cases and malformed relocation bounds. An independent decoder must
check the wide header fields, rather than only round-tripping our own writer.

Only after that evidence should Exec gain a runtime application loader and
change its packaging contract. The XEX cold-boot path can remain independent
of the application executable format.
