# MIR68K integer completion and benchmark acceptance

Status: multiplication and matrix1 are implemented. All 252 matrix1 vectors
pass in both native modes (504 executions) and four 6502 configurations
(1,008 executions). A separate NIR fix makes signed binary widening explicit;
snapshots and the 51-fixture sweep pass. The broad compiler run has only the
pre-existing untracked lines.act / SHARED.SCREEN sample failure. Division,
remainder and typed native faults now pass 1,674 host-oracle cases, terminal
fault checks, the full native suite and the affected compiler contract tests.
Binary search passes all 1,153 cases in raw/optimized native modes (2,306
executions) and all four 6502 configurations (4,612 executions); its unchanged
C-reference generator check passes. SHA passes all 155 native cases in both
modes (310 executions) and all four 6502 configurations (620 executions).
All three benchmark generator checks and the broad fixture NIR corpus pass.
The arithmetic and benchmark slices are complete. The previous milestone
passed Linux/macOS CI but exposed a Windows NIR-sweep stack overflow. The sweep
now reserves a 16 MiB compiler worker stack; a 128 KiB caller reproduces the
old crash and succeeds after the fix. The focused sweep tests and broad corpus
pass locally; remote verification of the new commits is pending.

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
