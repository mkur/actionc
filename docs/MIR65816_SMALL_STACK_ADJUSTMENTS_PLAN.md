# MIR65816 small stack adjustments

## Objective and baseline

Implement priority 2 from the Exec compiler-output analysis: shorten proved
one-, two- and three-byte stack reservations and releases. First repair the ten
pre-existing native runtime test failures recorded by the
[mode-state measurement](benchmarks/65816-mode-state/results.json), in a separate
commit. Tests must retain meaningful coverage of the current compiler contract;
do not ignore failures or weaken memory, ABI or interrupt checks.

Compiler baseline: `2f950d43`. Frozen Exec inputs: revision
`154bcf5a45605aeee57de5420ec4604e4ff7b30c`, 166 loaded source hashes and 899
routines. Code totals are 334,256 bytes optimized without guards, 444,271 bytes
optimized with guards and 381,526 bytes raw without guards. Initialized data is
1,385 bytes. Platform assembly and hosted boot qualification are outside this
measurement. The current unchecked image has 1,926 candidate adjustment windows,
with 3,694 bytes of modeled savings before flag-liveness restrictions.

## Ownership and proof

This is MIR65816 work. Keep SemIR/NIR, ABI, frame allocation, argument padding,
result placement, memory access order and guard policy unchanged. Use selected
typed instructions and checked machine liveness, never decoded-byte matching
inside the compiler.

For A16 binary-mode stack arithmetic, replace eligible `CLC; ADC #n` or
`SEC; SBC #n` with `n` accumulator increments or decrements, for `1 <= n <= 3`.
Require a tracked stack-address value and a contiguous local instruction window
without entries or protected events. Preserve the surrounding TSC/TCS and
result-preservation instructions. The final A and N/Z must agree; C/V may differ
only when verified dead after the window. Guard arithmetic consumes carry and
must remain unchanged. Unknown proofs retain the original selection.

Add typed INC A encoding/effects/replay and preserve stack-address equations
through A16 INC/DEC. Byte operations must preserve hidden B and must not invent
a full stack equation. Rebuild selected identity, CFG, layout, fixups, source
spans and trace observations after a checked replacement. The verifier must
continue to reject TCS without a proved stack equation.

## Commit sequence

1. Commit this plan before implementation.
2. Repair the ten runtime baseline failures. Review forwarding/home assumptions,
   upper-half DP ownership, incoming byte returns, zero-frame guard policy,
   relocated interface dependencies and instruction coverage. Add or adapt
   focused cases to preserve the intended behavior checks. Run all eight
   affected runtime targets without exclusions, then commit the repair.
3. Add the typed increment/decrement facts and narrow checked optimization.
   Cover both directions, amounts 0 through 4, A8 rejection, live C/V rejection,
   CFG boundaries, replay/generation and stack equations. Compare independent
   assembled sequences and VM execution across stack boundaries and flag
   combinations; check result lanes and IRQ/NMI restoration. Update intentional
   emission expectations and the state-tracker contract. Commit after validation.

## Validation and completion

During development run focused backend unit and native runtime targets. Preserve
raw/optimized, guard on/off, result-width and relocation coverage. Exercise
changed newline-sensitive fixture paths with LF and CRLF. Use the qualified VM
runner and keep its compiler/test inputs stable while it runs. Use nonincremental
builds and reduced host debug information to limit build-cache growth.

Final checks: MIR65816 library and root integration targets, the full native
runtime release suite without baseline exclusions, and focused debug stack,
guard, state/replay, call/result and interrupt tests. Existing opt-in external
compiler comparisons remain separate. Broaden only for actual shared-contract
changes or failures; do not run unrelated backends by default.

Rebuild all three frozen Exec profiles and verify source/layout hashes. Record
actual bytes saved, remaining candidates/fallbacks, unchanged data and ABI/frame
facts, exact test scope and compact provenance under `docs/benchmarks/`. Keep
bulky generated artifacts under `target/`. Results must distinguish measured
savings from the original candidate model and compiler VM coverage from hosted
Exec qualification.

## Baseline repair

All 58 tests in the eight affected targets pass in focused debug runs without
exclusions. Existing preemption coverage passes with actual post-arithmetic
instruction boundaries and upper-half DP homes. The relocated task harness now
binds and maps its Move/Clear runtime imports. Relocated comparison coverage
checks all eight dispatches, including the optimized direct global load.

Private comparison and parameter-bridge probes retain deliberate extra uses so
new sole-use forwarding cannot erase the behavior they test. Byte-return checks
include the actual incoming/local read when it feeds A directly. The zero-frame
test now checks the published inherited-stack contract; stack-fault tests retain
responsibility for new reservations. Host LF/CRLF variants pass through the
changed comparison/parameter fixture preparation paths.

## Implemented behavior and measurement

The closed selected-code rewrite shortens eligible A16 binary stack adjustments
and replays the resulting routine before publication. INC/DEC preserve checked
stack equations only at word width. Live C/V, intervening selected events and
unproved state keep the original sequence. The state-boundary snapshot changes
are intentional emission changes: shorter bytes and shifted labels/spans, with
identical frame descriptions.

The frozen inputs retain all 166 source hashes and the layout hash. All 899
routine signatures, argument/result layouts, frames, temporary homes, spill
counts and stack bounds are unchanged, as are data and zero-fill regions.

| Profile | Before | After | Saved |
| --- | ---: | ---: | ---: |
| Optimized, unchecked | 334,256 | 330,552 | 3,704 |
| Optimized, guarded | 444,271 | 443,016 | 1,255 |
| Raw, unchecked | 381,526 | 377,888 | 3,638 |

Unchecked optimized output replaces all 1,926 modeled windows, saving 3,694
bytes directly and another 10 through shorter branches. Guarded output replaces
692 windows and retains 1,198 conservative fallbacks, whose static model totals
2,370 bytes. The raw profile replaces all 1,895 windows. Branch relaxation adds
three bytes of savings in guarded output and five in raw output.

Independent ca65/VM comparisons cover both directions, stack wraparound, both
accumulator widths, hidden B, flags, full result lanes and interrupt restoration.
One- and two-byte adjustments save three and one cycles respectively. Three-byte
adjustments save one byte but cost one extra cycle; this implements the planned
code-size tradeoff. Compact measurements and validation provenance are recorded
in [results.json](benchmarks/65816-small-stack/results.json). These measurements
do not claim a hosted Exec boot qualification.

Final validation passed: 350 MIR65816 library tests, 86 root integration tests,
73 focused debug runtime tests across 12 targets, and all 346 active tests in
the unfiltered native release suite. Six existing opt-in release tests remain
ignored; there are no baseline exclusions. The disassembler's 10 tests and
both LF/CRLF snapshot reads pass. The release qualification manifest verifies
stable compiler/test inputs and records the pinned VM plus its status-timing
patch. No shared frontend or NIR contract changed, so validation stays scoped
to the 65816 backend.
