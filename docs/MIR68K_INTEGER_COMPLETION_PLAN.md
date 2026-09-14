# MIR68K integer completion and benchmark acceptance

Status: complete. Multiplication, division, remainder and typed native faults
are implemented, with 2,970 multiplication and 1,674 division/remainder
host-oracle cases plus focused composition and terminal-fault checks. A separate
NIR fix makes signed binary widening explicit; snapshots and the 51-fixture
sweep pass without snapshot changes.

Benchmark acceptance passes against the unchanged C-reference vectors:

- Matrix1: 252 cases, 504 native executions and 1,008 6502 executions.
- Binary search: 1,153 cases, 2,306 native executions and 4,612 6502 executions.
- SHA: 155 cases, 310 native executions and 620 6502 executions.

All three generator checks and the broad fixture NIR corpus pass. The NIR sweep
reserves a 16 MiB compiler worker stack to prevent Windows main-thread stack
overflow; the regression check invokes it from a 128 KiB caller.

[Cross-platform CI](https://github.com/mkur/actionc/actions/runs/34796179839)
passed on Linux, Windows and macOS for code commit `e3ece64`: sample builds,
the complete compiler suite, both VM suites and Python tooling tests. The
subsequent completion record changes documentation only.

This milestone completes multiplication, division and remainder,
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
