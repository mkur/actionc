# Captured BYTE returns

Status: implemented. See the
[measurements and qualification](benchmarks/65816-captured-byte-returns/README.md).

Starting from `2a3e90d1`, select an exact one-byte stack temp or parameter
directly for an `A8ZeroExtended` ABI result. Reuse the existing BYTE operand
classifier to validate the typed home and stack displacement, including the
transient delta, before emitting any instruction. Mutable parameters use their
current frame home. Other widths, result homes and unsupported operand/home
forms keep their existing paths; this slice does not broaden allocation.

Load only the owned byte in A8, restore A16 and apply `AND #$00FF` to clear hidden
B. The existing result-preserving frame release and RTL remain authoritative.
No result scratch is read or written. X is unspecified by the BYTE result ABI
and needs no initialization. Source loads, including volatile and indirect
accesses, remain separate MIR operations; no reads move across calls or aliases.

The replacement removes thirteen bytes per eligible return, independent of its
incoming accumulator width. It removes four DP byte reads and five DP byte
writes per executed return. Frames, argument/result placement, stack peaks,
guards, reservations, image/o65 formats and interrupt contracts remain fixed.
The optimized frozen Exec inventory contains 160 captured BYTE return sites:
the primary forecast is 2,080 bytes, before any further branch relaxation.

Qualification covers exact encodings, both entry widths, all byte values and
poisoned hidden B, zero/nonzero frames, mutable and incoming parameters, offset
255, invalid homes with no partial emission, exact source/tail bus traffic,
direct/indirect clobbering calls, alias/volatile ordering, relocated o65 and
IRQ/NMI restoration. Compile LF/CRLF fixtures through their real paths. Compare
raw/optimized frozen Exec, corpus and Dijkstra against the baseline, separately
accounting for return selection and any layout changes.
