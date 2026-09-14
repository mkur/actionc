# MIR68K compared with MC68000 GCC

The [reference command](../tools/mir68k-c-reference/README.md) compiles equivalent
C and runs it in the same r68k VM as Action!. This supplements the previous
Action!-only baseline with an independent compiler reference. It also retains
both sets of disassemblies so optimization decisions can follow actual code.

## Recorded baseline

Measured with the Action! compiler at `545d8e3`, GCC 16.2.0 and GNU binutils
2.47.20260726. The compiler source was clean. Both C modes and optimized
Action! pass all 209 insertion-sort vectors and all 22 matrix1 vectors for the
LONGINT/10×10×10 variant. Original reference fixtures and generators are unchanged.

The [CSV](mir68k-c-comparison.csv) contains exact measurements, including
stack traffic and instruction totals across the complete selected corpus.
Default benchmark entry measurements:

| Benchmark | Compiler | Executable bytes | Executed instructions |
| --- | --- | ---: | ---: |
| Insertion sort | Action! optimized | 2,242 | 10,617 |
| Insertion sort | GCC `-O2` | 574 | 1,426 |
| Insertion sort | GCC `-Os` | 440 | 1,499 |
| Matrix1 | Action! optimized | 1,790 | 162,876 |
| Matrix1 | GCC `-O2` | 306 | 20,230 |
| Matrix1 | GCC `-Os` | 298 | 22,032 |

Against GCC `-O2`, Action! executes 7.45× as many instructions for the default
insertion sort and 8.05× for matrix1. Across the reference corpus, the ratios
are 7.75× and 7.62×. These are instruction ratios, not execution-time ratios.
Both sides include initialization, result checking and required linked helpers.
The C compiler can inline and transform loops; Action! retains its present
optimization policy. This measures the complete compiler outputs without
artificially disabling GCC optimizations to match our current capabilities.

## What the disassembly shows

**Pointer alignment is a major matrix1 gap.** Action!'s `Multiply` is 786 bytes;
GCC `-O2` emits 94 bytes for that routine plus its shared multiplication helper.
Our pointer loads reconstruct a longword with four byte loads and shifts, and
stores split it into four bytes. That is required when alignment is unknown.
Here the pointers originate in aligned arrays and advance by four bytes, so
even-address alignment could be proven and preserved. GCC uses longword loads
and postincrement addressing. Action! permits odd pointers, so optimization must
carry a proof rather than adopt C's typed-pointer alignment assumption.

**Registers need to retain values across loop blocks.** GCC keeps matrix
pointers and the accumulator in registers, saving callee-saved registers once.
Our output repeatedly loads local pointer/counter cells, stages temporaries and
writes them back. Default matrix1 performs 72,256 stack-byte reads and 42,304
writes; GCC `-O2` performs 16,860 and 12,064, including the multiply helper's
stack arguments. Insertion sort is more pronounced: 3,506 reads and 2,586 writes
versus 186 and 120. The previous local forwarding pass has limited scope and
does not promote source locals or retain values across blocks.

**Control flow still materializes short-lived booleans.** Our conditions often
compare, execute Scc, normalize to 0/1, store/reload a temporary, and then branch.
GCC branches directly from condition flags. Our physical layout also retains
avoidable jumps around conditional edges. Eliminate boolean materialization
only for branch-only values, and preserve flag liveness and edge transfers.

**Instruction selection has smaller, concrete opportunities.** GCC's 32-bit
multiply helper uses SWAP for 16-bit word rearrangement; ours uses repeated
eight-bit shifts and operand reloads. GCC also uses postincrement addressing,
direct memory arithmetic and two-byte branches. These are useful focused
improvements after, or alongside, the larger storage and alignment work.

## Improvement cycle

1. Preserve proven pointer alignment through assignments, loop joins and
   constant-stride updates. Retain conservative byte access for unknown or odd
   pointers, with focused aligned/unaligned and volatile regressions.
2. Promote eligible private locals and extend register allocation across blocks.
   Start with nonescaping counters/pointers, respect calls and alias facts, and
   retain the conservative backend for differential execution.
3. Fuse compare/branch use and simplify physical fallthrough, then add measured
   addressing and word-rearrangement choices.
4. After each focused change, rerun the paired comparison, confirm all reference
   states, record the new CSV and inspect the changed hot loops. Keep GCC
   versions/flags stable when attributing changes to Action!.
5. Extend C coverage to SHA and DCT once this comparison is established. Their
   shifts, rounding and wrapping must match the Action! adaptations explicitly.

Compiler CLI/platform integration remains separate. This baseline establishes
where the current bare CPU backend spends instructions; it does not change
compiler behavior or promise that all observed GCC transformations are already
safe under Action!'s memory semantics.
