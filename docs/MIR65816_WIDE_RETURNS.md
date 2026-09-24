# Native 24/32-bit returns

Status: implemented. See the
[measurements and qualification](benchmarks/65816-wide-returns/README.md).

MIR65816 selects an exact-width numeric constant, null, numeric address, captured
temp or parameter for an authoritative `A16X8ZeroExtended` or `A16X16` result.
A contains the low word; X contains the bank byte, zero-extended to sixteen bits,
or the complete high word. Signed LONGINT results retain their bit pattern.
Symbolic addresses retain existing byte relocations. Width mismatches retain
the existing fallback; signed widening remains an explicit cast.

Before emission, validate the temp annotation or authoritative parameter home,
including mutable parameter storage, and the complete source extent. Stack
accesses include the transient delta and must fit d,S; allocated direct-page
homes must fit the ABI scratch domain. No allocation or new home is introduced.

Numeric constants use `LDX #high; LDA #low` in native widths. Captured 32-bit
values use `LDA home+2; TAX; LDA home`. Captured 24-bit values use
`LDA home+1; XBA; AND #$00FF; TAX; LDA home`. Both overlapping word reads stay
inside the three-byte private home; byte one is read twice, and no fourth byte
is accessed. Source loads from volatile, absolute or indirect memory remain
separate operations with their original access widths and ordering.

The existing terminal-boundary A16 restoration, A/X-preserving frame release
and RTL remain authoritative. These preparations make no scratch stores,
pushes or calls and change no ABI, guards, frame sizes or bank-zero reservations.
Selected `LDX` immediate has explicit X16 effects, changes N/Z, preserves A/Y
and carry, and cannot clobber a reserved loop counter.

At an A16 boundary the constant paths save 24 bytes for 24-bit results and
28 bytes for 32-bit results. Captured paths save 21 and 29 bytes respectively.
They eliminate four result-scratch byte reads and seven/eight scratch byte
writes per return. DP-resident sources still require their own reads.

Qualification covers raw/optimized code, independent ca65 callers and tails,
zero/nonzero frames, immutable/mutable parameters, direct/indirect calls,
recursion, zero/high-bit/bank-boundary patterns, relocated o65, exact bus reads,
volatile/alias ordering across full scratch clobbers, rejected malformed homes,
and IRQ/NMI restoration. Host source fixtures exercise LF and CRLF compilation.
