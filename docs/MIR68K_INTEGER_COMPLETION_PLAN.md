# MIR68K integer completion and benchmark acceptance

Status: multiplication is implemented and passes the native suite and focused
compiler contracts. Remaining slices are in progress.

The next native milestone completes multiplication, division and remainder,
then executes matrix1, binary search and SHA against the existing reference
vectors. SemIR/NIR continue to own widths, signedness and fault semantics;
MIR68K legalizes those operations for the original MC68000.

## Delivery slices

1. Multiplication: original MULU.W plus modulo-32 partial products, checked
   against host arithmetic in raw and optimized modes. Preserve captured inputs,
   calls and narrow-result wrapping.
2. Matrix1: keep one portable algorithm, move 6502 transport into its adapter,
   and use native symbols for all types/shapes in the existing vector corpus.
3. Division/remainder: signed and unsigned 8/16/32-bit execution, wrapping
   MIN/-1, dividend-signed remainder, and a typed non-returning native fault
   adapter. Verify that faults neither store a result nor resume execution.
4. Binary search: preserve all existing integer variants, record layout and
   reference cases while moving host memory conventions to the adapters.
5. SHA: preserve the algorithm and vectors, make byte/word conversion explicit,
   and compare complete state and digests in both target adapters.

Run focused MIR68K and native VM checks for backend-only changes. Each fixture
split also runs its affected 6502 VM target, text handling under LF/CRLF, and
its C-reference generator check. Shared SemIR/NIR changes require snapshots,
the NIR sweep and the full compiler suite under AGENTS.md. The existing
Linux/Windows/macOS CI matrix includes both VM workspaces; local results do
not establish the status of remote jobs.

CLI/Amiga integration, register allocation and multidimensional arrays remain
follow-up work. Optimize emitted code only after measuring a correct execution
baseline.
