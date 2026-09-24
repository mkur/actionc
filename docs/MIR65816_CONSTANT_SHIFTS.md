# Constant shifts and final index scaling

Status: implemented. See the
[measurements and qualification](benchmarks/65816-constant-shifts/README.md).

The selector removes the last ASL/ROL/ROL of every indexed address calculation.
Only the accumulated 24-bit pointer escapes scaling. The scaled index is private
scratch, and subsequent address arithmetic establishes its own carry; loads,
stores and address materialization do not consume the removed shift's flags.
Earlier shifts and additions retain their full 24-bit wrap and carry behavior.
Expected saving: six bytes per indexed address, with unchanged source accesses.

Numeric constant shifts use the entire count. Counts at least the scalar bit
width produce zero. Other counts split into byte displacement and a residual
of zero through seven bits. Capture the required input bytes in existing
domain-owned result scratch before writing any destination; this permits
overlapping allocated homes and preserves mutable parameter selection.
Byte displacement uses copies and zero fill. Even residual widths use A16
ASL/ROL or LSR/ROR pairs; odd widths use A8 chains. Choose unrolling or a fixed
X16 counter from encoded instruction sizes. No runtime count checks remain.
Signed right shifts remain logical, as already required by NIR semantics.

Validate complete source/destination homes before emission, even for zero
results. Three-byte private copies may overlap their two word transfers, but
never touch a fourth byte. Source loads and volatile access ordering remain
separate MIR operations. Variable and symbolic counts retain the checked loop.
No helper, reservation, ABI, frame, guard or interrupt contract changes.

Qualification compares against `74731616`: focused exact-encoding, rejection,
all-width/count and 24-bit carry tests; raw/optimized VM execution; IRQ/NMI;
independent ca65 encodings; full native debug/release suites; LF/CRLF corpus
builds; corpus and frozen Exec size/traffic measurements.
