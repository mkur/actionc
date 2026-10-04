# MIR65816 adjacent private scalar forwarding

## Objective and baseline

Implement priority 3 from the frozen Exec output audit: avoid copying a private
scalar source into a temporary when its sole adjacent consumer can read the
unchanged authoritative source directly. Start with LONGINT/LONGCARD, then
extend only closed consumer and width cases with demonstrated savings.

Compiler baseline: `e49813aa`. Frozen Exec revision:
`154bcf5a45605aeee57de5420ec4604e4ff7b30c`, 166 loaded source files and 899
routines. Compiler code is 330,552 bytes optimized unchecked, 443,016 bytes
optimized guarded and 377,888 bytes raw unchecked; initialized data is 1,385
bytes. Platform assembly and hosted Exec qualification are separate.

The current MIR still contains the audit's 624 candidate operations. Its older
transfer model totals 3,744 bytes: 379 four-byte captures account for 3,020 bytes
(204 parameter captures, 175 local captures). These are candidate savings,
not a promise: existing register forwarding, consumer selection and mode changes
can affect the result. Keep before/after evidence on identical frozen inputs.

## Contract and ownership

This is target-private MIR65816 selection. Do not change SemIR/NIR, ABI,
allocation, frame extents, argument padding or stack bounds. Retain allocated
capture homes initially; eliminate only their executable capture traffic.
Keep destination writes tied to allocated homes. A borrowed read must identify
its parameter or frame object, exact width/home, definition and sole use site.

Require one definition and one operand occurrence in the entire routine,
including terminators and edge arguments. The consumer must immediately follow
the Load in the same block. Reject volatile, external, absolute, indirect,
indexed, displaced, escaping, overlapping or ambiguous sources. Immutable
parameters must have no mutable authoritative frame home or hidden writes.
Local sources must be complete non-addressable local objects with no escape or
partial/aggregate aliases. Check source bounds and source/destination geometry.

Install read substitutions only for the proved consumer. Omitted captures keep
a conservative state barrier and publish no fabricated temporary definition.
Selected effects, home analysis and replay must describe actual source reads.
All other operations retain their normal selection and conservative barriers.
Do not borrow across calls, stores, branches or machine operations.

For terminal call/store consumers, read the source fully before the observable
operation, prove argument displacement/padding and destination overlap, and
expire the binding before the call. Indirect calls, unsupported helpers,
volatile stores and unresolved geometry retain captures. Narrow scalar work
must preserve existing A/N/Z forwarding and closed DP/X allocation profiles;
omit it when it would replace a cheaper existing path.

## Implementation commits

1. Commit this plan before compiler edits.
2. Add scoped scalar source bindings for immutable four-byte parameters and
   sole adjacent comparisons or native Add/Sub/AND/OR/XOR. Cover signed/unsigned
   values and both operand roles. Retain all homes and integrate exact read
   resolution into native scalar consumers. Add positive and refusal tests,
   independent VM value/flag checks, replay and an Exec measurement; commit.
3. Extend to complete non-escaping local LONG objects with independent ownership,
   bounds and disjointness checks. Cover writes before capture, hidden aliases,
   source mutation and IRQ/NMI restoration. Measure and commit separately.
4. Recount remaining consumers and widths. Add profitable direct call and final
   store consumers with complete preflight, then BYTE/word cases that do not
   conflict with accumulator demand, fusion or DP/X homes. Use separate commits
   for distinct proofs. Record refusals and defer any case whose safety or size
   benefit cannot be established without broadening the contract.

## Validation and completion

Use focused MIR65816 library and native runtime targets during development.
Check sole-use/definition counting, hidden address/edge uses, malformed homes,
mutability metadata, source escape, width/sign combinations, operand roles,
bank boundaries, canaries, call clobbers and replay. Add independent ca65/VM
checks for actual selected code, exact externally visible reads/writes and
IRQ/NMI restoration. Cover raw/optimized and guards on/off. For changed host
newline-sensitive fixture preparation, test LF and CRLF through that path.

After the final compiler change run the MIR65816 library and root integration
targets plus the complete native release suite through
`python3 -B tools/native65816-runtime-tests/qualify.py --release --no-fail-fast`.
Keep compiler/test inputs stable during qualification. Existing opt-in external
comparisons may remain ignored; do not exclude baseline failures. Scope checks
to this backend unless an actual shared compiler contract changes.

Rebuild all three frozen Exec profiles, verify source/layout hashes, and compare
ABI, frame/home/stack facts and initialized data. Record selected candidate
counts, actual savings and remaining refusals under `docs/benchmarks/`, with
bulky artifacts in `target/`. Update the emission contract and this plan with
implementation results. Use nonincremental builds with reduced host debug info.
Preserve unrelated working-tree changes and commit completed implementation.

## Implementation results

The immutable LONG-parameter arithmetic/comparison slice saves 933 bytes in
optimized unchecked Exec (330,552 to 329,619), with unchanged ABI, frames,
temporary homes, stack bounds and initialized data. Planner tests cover both
operand roles, signed/unsigned operations, extra definitions/uses, escape,
mutability, malformed homes and binding expiry. Independent ca65/VM checks
verify exact source reads, result stores, registers and flags; comparisons,
replay and IRQ/NMI checks pass. Existing LONG byte/traffic oracles now resolve
the checked source identity through independent ABI/frame metadata.

See [compact measurements](benchmarks/65816-scalar-forwarding/results.json)
for counts and validation scope as the remaining slices are completed.
