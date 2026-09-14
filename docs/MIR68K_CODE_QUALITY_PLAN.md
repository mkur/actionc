# MIR68K code quality milestone

This milestone adds native DCT and ADPCM acceptance, records a reproducible
code-generation baseline, reduces temporary stack traffic within basic blocks,
and improves measured instruction selection. Each major slice is committed
separately. Public CLI output and Amiga startup remain the following milestone.

Native benchmark acceptance is implemented. It exposed missing native array
descriptor loads and named record-pointer dereferences, now explicit in NIR.
Six native snapshots change for that contract fix; the Atari representation is
preserved. DCT passes 181 cases per mode, the decoder 2,713 checkpoints and the
encoder 1,561 checkpoints. Reference generator checks pass unchanged.

## Delivery and acceptance

1. Run every existing jfdctint, adpcm_dec and adpcm_enc C-reference vector in
   raw and optimized native modes. Decode reference integers before serializing
   them in target byte order. Address state through compiler symbols and compare
   intermediate states as well as final results. Exercise LF and CRLF through
   actual source instrumentation and compilation.
2. Record code bytes, executed instructions and maximum individual frame size
   for the native benchmarks. Measure their ordinary entry points, excluding
   test instrumentation, with a repeatable command and explicit compiler modes.
3. Retain temporary values in registers within individual physical blocks.
   Track actual instruction clobbers and widths; preserve flags, calls, volatile
   accesses, scratch storage, and control-flow boundaries. Retain a conservative
   materialization path for differential execution and measurement.
4. Improve instruction selection where baseline listings show avoidable cost.
   Qualify new original-MC68000 encodings independently, preserve flags and
   full-width semantics, and measure the result before expanding the scope.

Backend changes run focused compiler checks and the complete native suite.
Shared compiler contracts require the checks in AGENTS.md. Benchmark adapters
alone do not require repeating unaffected 6502 tests; the original algorithms
and their existing adapters remain shared validation inputs. Full CI continues
to cover all workspaces on Linux, Windows and macOS.

Register allocation across blocks, multidimensional arrays and platform output
formats remain separate work. NIR continues to own typed computation and
effects; instruction selection and physical register decisions belong to MIR68K.
