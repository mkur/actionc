# Strcat handwritten 6502 comparison

The current compiler emits a 113-byte `Strcat`. A handwritten version retaining
the incoming ABI, ordinary parameter homes and post-call reloads needs 97 bytes.
It saves 16 body bytes and 21 cycles on the lengths-3/8 sample. A more compact
52-byte version uses private pointer storage and stronger alias assumptions;
it is a useful target for qualified calls, but fails the parameter-home alias
cases admitted by the Action! version.

The best next compiler change is to fold the final `+1` into the carry input
of the existing pointer arithmetic operation. This accounts for six of the
sixteen local bytes and needs no new source-language or NIR behavior. Compiler
code is unchanged by this comparison.

## Source and comparison scope

[TN.ACT](../../../samples/tn/modern/TN.ACT) contains:

```action
PROC Strcat(CHAR POINTER s, t)
  MovePage(s+s^+1, t+1, t^)
  s^==+t^
RETURN
```

These are counted strings. The final statement rereads both lengths after the
copy; either length can have changed. The actual
[MovePage](../../../samples/tn/modern/LIB.ACT) copies from the highest offset
downwards and interprets a zero byte count as 256. All handwritten controls
call this same 19-byte helper, including its overlap and zero-count behavior.

The [frozen compiler assembly](compiler.asm) comes from optimized modern
MIR6502 with optimized NIR and entry-reachable linking at `d0c947cd`.
The four TN/TNDBG cartridge/standalone images match the SHA-256 hashes in the
[previous qualification](../6502-argument-x-bitwise/results.json). Each contains
a 113-byte `Strcat`. The assembly is also reproduced byte for byte by MADS.

The two primary handwritten routines are:

- [same-abi.asm](same-abi.asm): `s` arrives in A/X and `t` in Y/`$A3`, as in
  the compiler output. It writes the same four parameter-home bytes, retains
  the helper's ABI stores, and reloads `s` and `t` after the copy.
- [private-pointers.asm](private-pointers.asm): the same incoming register ABI,
  with `s` retained in `$AE/$AF` and `t` in `$AC/$AD`. It omits ordinary
  parameter-home side effects. The buffers must avoid pointer/ABI scratch,
  parameter homes, the stack and code, but may overlap each other. MovePage's
  declared pointer-pair preservation is required. One extra stack byte holds
  destination-low temporarily; it is popped before the helper call.

Both routines assume the compiler's binary-arithmetic invariant, D clear.
These are concrete comparison implementations, rather than minimum-size bounds.

## Measured bytes and cycles

Each row adds the indicated change to the preceding row. The intermediate
controls isolate the byte and cycle accounting.

| Implementation | Body bytes | Strcat cycles | Complete sample cycles |
| --- | ---: | ---: | ---: |
| Current compiler | 113 | 175 | 355 |
| Fold `+1` into carry input | 107 | 168 | 348 |
| Capture destination directly | 99 | 156 | 336 |
| Remove redundant `LDY #0`, same ABI and homes | 97 | 154 | 334 |
| Private pointer storage and scheduling | 52 | 95 | 275 |

The sample uses `s=$7000`, `s^=3`, `t=$7200`, `t^=8`. Complete cycles include
the unchanged helper's 163 cycles and the shared 17-cycle test wrapper.
Strcat cycles include its JSR and RTS; body sizes include those instructions,
but exclude the helper, callers, XEX framing and storage. These are body
opportunities; a future compiler implementation must measure final image costs.

All four program/runtime contexts agree at both callable fragment origins,
`$0800` and `$09C7`. The report also records destination/source page carry and
the zero-count case. For the latter, MovePage alone takes 4,627 cycles.
[results.json](results.json) records instruction-image hashes, identities,
cycle attribution and execution counts.

## Local differences the compiler can address

### Include the constant one in the carry chain

The existing selected MIR operation
`OffsetPointerByIndirectBytePlusOne` already represents the whole expression
`p + BYTE[p+offset] + 1`. Its
[emission recipe](../../../src/mir6502/emit.rs) first adds the byte with C clear,
then increments the low result in X and conditionally increments the high byte.
The handwritten carry-only control instead uses:

```asm
        LDY #0
        LDA $AE
        SEC
        ADC ($AE),Y
        STA $AC
        LDA $AF
        ADC #0
        STA $AD
```

SEC supplies the final one to the low-byte ADC. Since the loaded addend is a
byte, the low sum including one is at most 511; a single propagated carry
produces the complete 16-bit result. This replaces 21 bytes with 15. It saves
seven cycles on the normal sample and eleven when the old increment needed
high-byte repair.

The source-low, indirect byte and source-high reads retain their order. The
indirect byte is still read before the first result write, including when it
aliases the destination's incoming contents. Source and destination pointer
pairs must remain disjoint. A/X and arithmetic flags already have a clobber
contract for the selected operation; changing their incidental results needs
to remain within that contract. This is the narrowest implementation candidate.

### Capture the computed destination without a scratch round trip

The compiler produces the destination in `$AC/$AD`, then copies it to
`$A0/$A1`. The handwritten version writes `$A0` directly and keeps the high
byte in X while writing `$A1`. That removes eight more bytes and twelve cycles.

The `$A0/$A1` stores remain before the later `t^` read: that read can observe
them through an alias. The discarded `$AC/$AD` result is overwritten by the
original `t` pointer before the next indirect read. A compiler implementation
needs a bounded producer/consumer proof for this observation order and scratch
liveness. It belongs in MIR6502 selection, with the existing physical and
final-cost checks.

### Reuse the known Y value

The initial pointer arithmetic sets Y to zero. The intervening destination
capture and `t+1` calculation preserve it, but the compiler emits another
`LDY #0` before reading `t^`. Removing it saves two bytes and two cycles.
The following LDA defines N/Z, so this particular load's flag definition is
unobserved. A general implementation must check both Y's value and flags;
knowing the register value alone does not justify deleting every repeated load.

## What the compact version assumes

The remaining 45-byte difference between 97 and 52 bytes has different causes:

- Combining incoming pointer capture with private scratch placement saves
  eleven bytes compared with parameter homing plus the original-s setup.
- Stack capture, Y increment with carry repair, and source setup scheduling
  save fourteen bytes. The source length can be read before outgoing ABI cells
  change because these buffers cannot alias those cells.
- Retaining both original pointers across MovePage avoids twenty bytes of
  post-call parameter-home reconstruction.

Those twenty bytes are observable for ordinary Action! parameter homes. In a
tested case, the append overwrites `t`'s home from `$7200` to `$7300`. The old
source length is two and the new one is five. A destination length of 255 must
become four; the private-pointer routine produces one. Overwriting `s`'s home
similarly requires updating the newly selected destination. The 97-byte
routine preserves both cases; the 52-byte routine fails them as expected.

Private placement therefore needs a proven absence of these aliases, or a
storage model that makes those homes private. A helper's pointer-register
preservation annotation alone cannot establish that its copy leaves ordinary
parameter homes unchanged.

## Validation and reproduction

The generator binds four qualified program images to five comparison bodies,
checks the helper's exact instructions, assembles both origins with actual LF
and CRLF sources, and compares all 80 assembly outputs. The VM check executes:

- 9,600 normal calls, including every destination length at six low-byte
  boundaries, every source count, self-concatenation, overlaps, length-byte
  overwrite, page carries and zero-count copying;
- 64 parameter-home alias calls and 96 scratch-alias calls for the four
  variants retaining Action!'s parameter homes;
- 16 expected counterexamples for private pointer retention.

An independent host model performs the descending copy on mutable memory and
rereads the final lengths. Tests compare complete buffer regions, ordered
buffer accesses, helper arguments, returned A/X/Y/flags/SP and, where retained,
parameter writes. Alias cases also compare surrounding memory and returned
state. The cartridge cases load the ROMs; standalone calls run without them.
Manifest, listing and symbol parsing also pass LF/CRLF variants.

```sh
python3 tools/compare6502/tn/strcat_idiomatic.py --assemble-only
CARGO_INCREMENTAL=0 cargo test --locked \
  --manifest-path tools/vm-runtime-tests/Cargo.toml \
  --test strcat_idiomatic -- --ignored --nocapture
python3 tools/compare6502/tn/strcat_idiomatic.py --check
```

The first run after intentionally changing a control uses the generator without
`--check` to update results. MADS must be on PATH or selected by `ACTIONC_MADS`.
The frozen TN images and metadata must first exist under
`build/argument-x-bitwise/measured`; the
[previous report](../6502-argument-x-bitwise/README.md#qualification-and-reproduction)
gives the corpus exporter command. Generated fragments, manifests and VM cycle
samples remain under ignored `build/strcat-idiomatic`.
