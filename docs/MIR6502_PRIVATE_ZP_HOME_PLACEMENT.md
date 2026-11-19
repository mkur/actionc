# Private zero-page homes in MIR6502

**Proposed placement rule.** An ordinary parameter or local has the same
protection from unrelated pointer accesses in zero page as it has in inline
storage. MIR6502 can choose a ZP home when two conditions hold.

1. **The compiler controls the object's address.** Check exposure before
   choosing a home. Taking `@mem`, creating an alias to the cell, or fixing
   its address sends it through the existing addressable-storage rules.
   Passing a pointer value held in `mem` keeps the address of the cell private.
   [Address-control analysis](../src/mir6502/analysis/storage_ownership/source_objects.rs)
2. **Every byte of the home is available for the value's lifetime.** Include
   loop back edges, live values across calls, and any value that must survive
   between invocations. A word needs both bytes. Fixed program storage, ABI
   inputs and results, scratch, and other live homes occupy their actual
   bytes. A byte can be reused after the last use when its next owner can
   establish its own value. [Lifetime rules](MIR6502_UNEXPOSED_OBJECT_STORAGE_DESIGN.md#zp-home-lifetimes-and-persistent-storage)

An ordinary store through a computed pointer can modify the program data it
addresses. An unexposed local can stay live across that store in any
otherwise available home. A physical access identified by the program or
the target ABI occupies the referenced bytes, including an explicit numeric
destination passed to a callee.

## TN example

`Copy.mem`, `Copy.len`, and `Copy.files` are live across `Tag(j)`. Inside
`Tag`, `Store(v(i), a!TAG_MASK)` changes one directory-row byte. The caller
passes an index to `Tag` and keeps the addresses of those three locals private.
Their placement can therefore be decided without tracing how `SetWin` fills
`v` or how `Sort` permutes it. [TN source](../samples/tn/modern/TN.ACT)

The general `Store(p, value)` writes `$A0`, `$A1`, and one byte through its
first argument. Under this design its review records that fact once. The call
with `v(i)` occupies the two scratch bytes. For `Store($00A2, value)`, all
three bytes are occupied. The allocator also accounts for `Tag`'s own homes
and scratch bytes and those used by its other callees.

## Compiler contract

Today `Store` has a matched entry review with an open write footprint.
MIR6502 rejects a live home when the callee's writes summarize to `Unknown`.
The proposed call check asks which candidate ZP bytes the invocation can use.
Native MIR supplies its physical uses. Opaque assembly, runtime helpers, and
OS calls supply them through a matched contract or the target ABI. An entry
guarantee alone gives no evidence about scratch. If a call or callback can
use an unaccounted candidate byte, that byte is unavailable across the call.
[Current call check](../src/mir6502/parameter_homes/calls.rs) · [Shared assembly contracts](6502_SHARED_CALLEE_FACTS_DESIGN.md)

Compose these facts through native callers. Discovery and verification after
expansion must agree for the selected runtime and physical layout. The
general memory-effects query still reports the indirect write for alias
analysis and ordering. In the TN acceptance case, all three locals remain
candidates across the row write. Forcing a live local into `$A1` must fail.
Passing the local's address to `Store` must invoke the addressable-storage rules.
Other calls in `Copy` and final image cost can still determine its placement.
[Home selection](../src/mir6502/parameter_homes/cross_call.rs) · [Residence verification](../src/mir6502/parameter_homes/lifetimes.rs)
