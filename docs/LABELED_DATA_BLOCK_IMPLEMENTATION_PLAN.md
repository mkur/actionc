# Labeled data block implementation plan

2026-11-16. Status: slices 1–2 implemented; TN qualification in progress.
Follow-up to the [unexposed-object qualification](benchmarks/6502-unexposed-object-storage/README.md).

## Contract

```action
DATA yesabort = [
  "Yes"   'Y
  "Abort" 'A
  $00
]
```

Declare named, mutable byte storage at module scope. `@yesabort` addresses its
first byte; the label is not a callable routine or a scalar variable cell.
Preserve the existing raw-block literal encoding, including length-prefixed
strings with trailing self-addresses, character bytes with `$9A` markers, and
supported address expressions. Add no descriptor,
terminator, routine entry or `RTS`. Preserve symbolic addresses as typed
relocations; do not guess unresolved operand widths. Placement may change while
the declared payload and symbolic address references remain correct.

Recognize `DATA` contextually in declaration position, preserving existing
identifiers and variant members named DATA. Separate the declaration from its
initializer in the AST/SemIR. Initially implement a literal-block initializer;
future generators can produce the same typed data image without changing label
semantics. Generator syntax and execution are outside this plan.

## Slices

1. **Add the declaration and typed lowering.** Parse module-level DATA blocks;
   resolve their labels and initializer references in semantic analysis/SemIR.
   Diagnose duplicate names, calls to data labels, unsupported local declarations
   and invalid initializers. Lower to existing NIR static-data objects with stable
   IDs, typed fragments and address targets. Emit no executable ForeignCode or
   MachineBlock for DATA. Extend verification, printing and boundary documentation
   so target backends consume resolved data facts without SemIR lookback.

2. **Connect emission and observation.** Reuse static-data emission, retention,
   maps and relocation handling; preserve exact payloads and protect declared
   ranges, address escapes and relocation targets. The instruction scanner visits
   executable machine blocks only; DATA never enters that scan. Keep undecodable
   raw code conservative, including mixed code/data raw blocks. Do not infer data
   from names, strings or absence of direct calls. Keep residence, scratch/callee
   checks, whole-image cost acceptance and rollback unchanged. Include the owner
   and machine-block ID in unresolved-observation diagnostics.

3. **Migrate TN and qualify.** Convert the six menu tables in modern TN and its
   maintained debug counterpart from `PROC name=*()` to DATA, retaining `@name`
   consumers and byte-for-byte payloads. Run one TN qualification against the
   recorded **9,838-byte** baseline. Inspect Sort's actual admission, proposal,
   selected homes and final cost; report any remaining blocker. Separate effects
   of the source/data migration from placement savings using existing trial
   before/after evidence. Retain assembly, facts and image hashes; update the
   design's recorded restriction. Do not promise a Sort saving.

Commit after each slice's relevant checks pass. Preserve unrelated working-tree
changes and historical evidence.

## Validation

Use grouped parser/semantic/NIR cases for contextual DATA, address references,
encoding/relocations, diagnostics and unchanged program-entry selection. Add one
pipeline control proving DATA does not block an eligible indexed caller, paired
with undecodable executable code that still rejects placement. Extend the existing
6502 VM control to read menu records and execute a resident indexed caller in
ActionCart and Standalone; check payload bytes, results and sentinels. Preserve
explicit physical-alias and rollback controls. Exercise LF/CRLF through any new
newline-sensitive fixture path; compare binary data exactly.

This adds a shared language/IR contract, so run focused checks while developing
and the required `cargo test nir_fixtures_match_snapshots`,
`cargo run --bin actionc-nir-sweep -- fixtures/nir`, and `cargo test` once before
completion. Include focused checks for other backends consuming the new static
declaration; do not restrict shared-contract validation to MIR6502.

## Validation record

Slice 1: six focused `cargo test --lib labeled_data` cases pass, covering
LF/CRLF parsing, typed fragments, forward/cyclic references, named-module linker
retention, target pointer widths, invalid uses, and rejection of an unlowered AST at the direct legacy codegen API. The new static relocation target required updating all three backend
consumers in this slice; slice 2 checks their emitted bytes and observations.

Slice 2: four exact-emission tests pass across classic 6502, MIR6502 (both
runtimes), 65816 and 68k. The raw-menu comparison verifies string self-addresses
and character `$9A` markers. Two indexed-caller pipeline controls pass with
LF/CRLF input; the paired opaque block still rejects placement and reports its
owner/block. The VM group passes with placement off/on in ActionCart and
Standalone, checking menu records/mutation, swaps, arguments, stack and sentinels.
The six frontend tests, four object-storage unit controls and exact selected-input
rollback control also pass. Classic SemIR projection reuses initialized inline
byte-array storage so maintained TN classic builds remain supported.
