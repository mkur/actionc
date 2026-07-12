# MIR6502 Risk-Averse Code Quality Plan

This plan favors observability and mechanically safe cleanup before target
peepholes. The goal is to make MIR6502 easier to compare against legacy codegen
while avoiding changes that could hide materialization bugs.

## Principles

- Keep raw artifacts available. Normalized views must be additive and must not
  replace source listings, maps, load bytes, or verifier output.
- Prefer deterministic debug output before code-shape changes.
- Do not optimize across calls, machine blocks, pointer writes, OS/runtime
  calls, or unknown absolute memory until effects are strong enough.
- Run pre-emission verification before any emitted-object comparison.
- Require focused fixtures before enabling a MIR-level peephole by default.

## Low-Risk Slices

1. Comparison hygiene. Add normalized listings, instruction-only listings, and
   byte-size summaries to `tools/compare-codegen.sh`.
2. Stable symbol views. Add compact routine/data summaries grouped by symbol
   name so relocated addresses are easier to audit.
3. Printer-only MIR cleanup. Make materialized MIR printing more regular around
   spill names, ABI homes, and scratch pairs without changing MIR semantics.
4. Dead spill accounting. Add reports for spills allocated but pruned, then use
   the reports to find safe producer-consumer folds.
5. Verified local peepholes. Only after fixture coverage exists, enable narrow
   passes such as redundant adjacent load removal and compare/branch fusion.

## Peephole Admission Checklist

- The peephole operates on verified materialized MIR.
- The transformed MIR verifies again.
- It preserves barriers and does not cross blocks unless dominance/use facts are
  explicit.
- It has a positive fixture and a barrier-preservation fixture.
- It can be disabled with `Mir6502Config::enable_peepholes`.
