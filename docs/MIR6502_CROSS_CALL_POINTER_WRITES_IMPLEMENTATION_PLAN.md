# MIR6502 implementation plan for ordinary pointer writes across calls

Let a caller keep a live, unexposed object in ZP across a native pointer
write when the candidate bytes survive the call. TN's `Print.i` and the
two-byte cell holding `Print.s` are the acceptance case. `Putchar` receives
one character and writes it through `Screen`. Eligibility for those homes
depends on who controls their addresses and which physical bytes the call
touches. [Design](MIR6502_UNEXPOSED_OBJECT_STORAGE_DESIGN.md#ordinary-pointer-writes-across-calls)

Land each slice with its tests in a separate commit. Use MIR6502's typed
operations, matched assembly and runtime contracts, and existing placement
trial. At baseline `d00dd8b5`, modern TN at origin `$3000` has 47 named ZP
homes on both runtimes. The complete cartridge XEX is 9,687 bytes and the
standalone XEX is 9,826 bytes. Both traces reject `Print.i` and `Print.s` at
`Putchar` with `unknown residence callee writes`. [Baseline qualification](MIR6502_REVIEWED_CALL_COMPOSITION_IMPLEMENTATION_PLAN.md#calcadr-return-definitions)

## 1. Describe the writes of a native pointer store

In the residence analysis, use `writes_ordinary_pointee` on the selected MIR
operation. Record the write through the pointer alongside all physical writes
made by its addressing sequence. Use the existing `MirKnownMemoryWrites`
representation where it can express those facts. Resolve scratch, spills and
bound frame slots in the routine that owns them. Keep pointer-cell reads and
source-visible store ordering in their existing analyses. A missing physical
effect, opaque operation or unmatched assembly contract yields unknown writes.
[Effect classifier](../src/mir6502/analysis/effects.rs) · [Callee summaries](../src/mir6502/analysis/known_callees.rs)

Test a byte store through a program pointer and a word store with a fixed
scratch write. The general memory query must still report a possible write
to an arbitrary address. The residence summary must retain the scratch
byte, including a byte restored before return. Keep automatic admission on
the existing path until both placement and verification consume the facts.

## 2. Carry destination evidence through direct calls and wrappers

Union possible writes across branches and acyclic callees. Track whether the
destination comes from ordinary program storage, an explicit numeric address
or a pointer formal. In `Putchar(ch)`, the destination comes from `Screen`;
the presence of the unrelated `ch` parameter introduces no pointer obligation.
For a forwarded pointer formal, retain its identity, access width and known
offset or index range until the actual call site. Reuse the ABI argument-value
lookup for materialized registers and ZP lanes. Keep cached routine facts
independent of the actual values at any one site. An unresolved explicit
destination that might overlap the home, or an opaque descendant, declines
the proof. [Call-site queries](../src/mir6502/parameter_homes/calls.rs) ·
[Existing buffer argument lookup](../src/mir6502/runtime_contracts/buffers.rs)

Call the same writer through a wrapper with `$A2` at one site and `$7100` at
another, using scratch disjoint from `$A2`. Only the first explicit byte store
overlaps a proposed home at `$A2`. Cover a word whose second byte overlaps,
16-bit address wrap, a branch where only one path writes the candidate, and
a stale descendant review.
These cases must pass before caller placement uses the new summary.

## 3. Use one preservation rule in selection and verification

For each live object, establish `ObjectAddressControl` before choosing a
slot. Use the private-byte query for an object whose address the compiler
controls. Check argument setup, callee homes, scratch, results and explicit
numeric ranges against every byte of the proposed home. For exposed or
fixed-address objects, use the ordinary physical-overlap checks. Apply the
same test to supplied bindings in expanded lifetime verification.
[Discovery](../src/mir6502/parameter_homes/cross_call.rs) · [Lifetime verifier](../src/mir6502/parameter_homes/lifetimes.rs)

Extend `has_pointee_writes` so a native store in a descendant activates the
same physical-observation checks as a store in the caller. Recheck explicit
observations against both the baseline and candidate layouts. Exercise the
normal planner and a manually supplied binding. A disjoint live home must
pass both paths. A single overlapping lane must fail at the call. Passing
`@i` to the writer must disqualify `i` from the private-home rule while a
separate unexposed local remains eligible. Retain the whole-image cost check
and rollback. [Physical observations and layout replay](../src/mir6502/parameter_homes/physical.rs)

## 4. Qualify TN on cartridge and standalone

Trace modern TN at `$3000` with the existing `CalcAdr` and `Internal` reviews.
Show that both `Print.i` and `Print.s` reach candidate selection across the
screen store. Force a live home overlapping `$A0` as a rejection control for
`Internal`'s result write. Record the selected addresses or the specific
remaining rejection for each object, the total named homes and both XEX size
deltas. The allocator's measured cost decides the final placements.

Run the real `Print` loop on both runtimes. Check multiple characters, the
adjacent screen sentinels and the final cursor position. In a small wrapper
fixture, check the value stored through the pointer and the caller's surviving
word after return. Recheck existing TN assertions that depend on a particular
home, including `PrintNonzeroDigit.b`, against the VM behavior and the new
placement evidence. [TN VM checks](../tools/vm-runtime-tests/tests/tn_dispatch.rs) ·
[Cross-call VM checks](../tools/vm-runtime-tests/tests/cross_call_homes.rs)

Compare failures with the baseline under the same build settings. In
particular, the parameter-home unit target already reports a carry assertion
in `an_extra_expanded_entry_requirement_restores_the_complete_selected_artifact`.

```sh
cargo test --lib mir6502::analysis::known_callees::
cargo test --lib mir6502::parameter_homes::
cargo test --test mir6502_cross_call_homes --test mir6502_parameter_pointers
cargo test --manifest-path tools/vm-runtime-tests/Cargo.toml --test cross_call_homes --test tn_dispatch
```

## TN qualification

Both modern MIR6502 builds at origin `$3000` select `$E3/$E4` for the pointer
cell `Print.s` and `$E5` for the loop index `Print.i`. The expanded lifetime
verifier rejects a supplied `Print.i` binding at `$A0` at the call to `Putchar`,
where `Internal` writes its result.

| Runtime | Previous XEX | Selected XEX | Change | Named ZP homes |
| --- | ---: | ---: | ---: | ---: |
| Cartridge | 9,687 bytes | 9,638 bytes | −49 bytes | 47 → 53 |
| Standalone | 9,826 bytes | 9,777 bytes | −49 bytes | 47 → 53 |

The other new homes are `PrintFname.s`, `PrintFname.i`, `PrintNonzeroDigit.b`
and `DrawMenu.menu`. The digit routine keeps `b` at `$E3` and its divisor at
`$E2`. The VM overwrites the dead divisor at `Putchar` and checks the emitted
digit and remainder. On both runtimes the real `Print` loop writes the four
expected bytes, preserves the two screen sentinels and advances from column
2 to column 6 on row 1. A separate native wrapper test checks its destination
and the live caller word, including a carry and a 16-bit wrap.
[VM tests](../tools/vm-runtime-tests/tests/cross_call_homes.rs)

The numeric-destination tests cover separate calls to the same wrapper, an
overlap on either word lane, wrapping at `$FFFF`, and whole-image rollback
when the destination aliases the original inline cell. Taking one local’s
address leaves an independent private object eligible. [Placement and rollback tests](../tests/mir6502_cross_call_homes.rs)

For a pointer captured from `$A3/$A4`, the analysis follows the existing
`SArgs` contract back to the staged bytes. The regression changes the
diagnostic operands while preserving those staged bytes, then invalidates the
capture contract and checks that the caller loses its write guarantee.
[Call-site tests](../src/mir6502/analysis/known_callees/native_writes/tests.rs)

The callee-summary suite passed 91 tests. The parameter-home suite passed 85,
with the same pre-existing carry assertion documented above. Both VM targets
passed all 22 tests. The cross-call integration target also passed its
separately invoked MADS reconstruction and relocation check with LF and CRLF.
