# MIR6502 fixed-address observations: implementation plan

2026-11-16. Status: both slices implemented and qualified.
Applies the existing [unexposed-object contract](MIR6502_UNEXPOSED_OBJECT_STORAGE_DESIGN.md)
and [storage-observation design](MIR6502_STORAGE_OBSERVATION_DESIGN.md).

## Outcome and baseline

Make a typed reference to explicitly fixed storage yield the same physical
observations as its numeric address, while retaining storage identity.
Knowing the address of a pointer cell does not establish its pointee footprint.

At `8fd0f0f8`, [TN qualification](benchmarks/6502-labeled-data/README.md) records
**9,838 bytes**, with Sort at 250 code bytes, three mapped inline-home bytes and
no ZP homes. Discovery stops at Block/m7: `(screen),Y` retains global identity
g9, but the scanner does not resolve its fixed `$E6` backing. Its pointer fetches
should be recorded at `$E6/$E7`. Reuse the recorded image, facts and receipt;
no historical compiler rebuild is needed.

This is a MIR6502 analysis correction. Keep source syntax, NIR, emission recipes,
DATA handling, public pointee-effect contracts, placement admission rules,
whole-image cost acceptance, preservation replay and rollback unchanged. Use
typed IDs and MIR backing facts; add no source-name lookup or optimization pass.

## Slice 1 — resolve and record fixed observations

Primary code: [machine observations](../src/mir6502/analysis/known_callees/observations.rs),
[known-callee helpers](../src/mir6502/analysis/known_callees.rs), and
[physical observations](../src/mir6502/parameter_homes/physical.rs).

1. Provide one small, shared fixed-address query for these consumers. Resolve
   numeric addresses, explicit fixed-ZP slots, and global IDs whose backing is
   absolute or an alias chain ending at absolute storage. Include byte offsets
   and already-applied relocation addends exactly once. Bound alias traversal;
   missing IDs, cycles and address overflow must fail conservatively.
2. Leave layout-dependent globals, statics and compiler-selected homes outside
   this query. Retain their existing logical/final-layout handling. Do not use a
   previous emission's addresses as discovery facts, or rewrite logical MirMem
   identities into numeric addresses throughout the program.
3. Use the query for indirect pointer fetches in the scanner. For `(zp),Y`,
   require a valid ZP operand and record both fetches, including `$FF → $00`.
   For NMOS `JMP (abs)`, preserve the same-page high-byte fetch, including
   `$12FF → $1200`. Record wrapped bytes separately; do not represent them as
   an ordinary consecutive two-byte range. Keep unknown pointees unknown and
   retain the existing full-ZP observation for `(zp,X)`.
4. Use the same query in the physical collector's `numeric_base` path so aliases
   do not lose the low-byte observation after the scanner resolves the high
   byte. Retain the original logical reference alongside physical evidence;
   check both old and proposed placements through the existing transaction.
5. Preserve relocation selector, operand-width and object-bound checks. A
   selected low/high byte is not automatically a full symbolic address; a known
   non-ZP address cannot certify a ZP operand. Unresolved indirect operands and
   undecodable executable blocks must still reject with owner/block diagnostics.
   Keep the change local to observations rather than broadening all callee
   summary decoding as a side effect.

Use a grouped `fixed_observation_` unit regression covering numeric versus typed
fixed references, an alias chain with offsets/addends, both wrap boundaries,
ordinary unresolved storage, invalid fixed references and a non-ZP operand.
Assert the actual physical fetch bytes, retained identity and conservative
pointee effects. Exercise explicit overlap with either fetched byte so a newly
complete inventory cannot erase a real observation.

Run that group and the existing affected `object_storage_` unit controls.
Commit the resolver, both consumers and their focused tests together.

## Slice 2 — exercise placement and qualify TN

1. Extend [object_storage.act](../fixtures/runtime/object_storage.act) with a
   generic executable ASM reader through an explicitly fixed pointer cell.
   Keep the existing indexed callers and DATA menu. Assert that lowering really
   supplies a typed global relocation to the scanner, rather than only testing
   legacy named or numeric machine operands.
2. Extend the existing [pipeline controls](../tests/mir6502_cross_call_homes.rs):
   the fixed pointer must no longer make discovery incomplete, and the positive
   callers must actually select ZP homes in the existing single trial. Keep the
   opaque-block rejection and independent-caller fallback control.
3. Extend the existing [VM group](../tools/vm-runtime-tests/tests/cross_call_homes.rs)
   to execute that ASM reader as well as the resident callers. Check its result,
   declared pointer bytes, neighboring sentinels and stack balance with placement
   off/on in ActionCart and Standalone. Retain the swap/call-input oracles. Use
   declared addresses and emitted maps, never guessed private homes.
4. Run one [TN facts collection](../tools/compare6502/tn/leaf_facts.py). Verify
   that Block/m7's fixed-pointer observation resolves. Report Sort's actual
   admission, proposed/selected homes and final cost, or the next concrete
   blocker. Preserve Ord/Items/FindItem/Copy savings and inspect Fnamecmp/Next
   contracts. Further unresolved blocks are a reportable outcome, not permission
   to expand this slice into another scanner feature.
5. Compare all routine code/home counts and complete image size against the
   DATA baseline. Retain assembly, selected MIR, facts, image/relocation hashes,
   decisions and work counts under a new qualification directory. Check exact
   rollback once. If TN's selected code or homes change, run its existing
   `tn_dispatch` VM target once. Update the design's recorded restriction and
   this plan's status, then commit qualification and execution coverage.

A smaller TN image is not required for completion. Fixed symbolic references
must be handled correctly without weakening any remaining proof or cost guard.

## Focused validation

The new unit prefix below is proposed; verify nonzero selected test counts.
Run groups in the slice that changes their behavior and do not repeat passing
groups without a relevant change or failure.

```sh
cargo test --lib fixed_observation_
cargo test --lib object_storage_
cargo test --test mir6502_cross_call_homes object_storage_
cargo test --manifest-path tools/vm-runtime-tests/Cargo.toml --test cross_call_homes object_storage_
cargo test --lib later_trials_keep_the_exact_selected_input_and_executable
```

Reuse `build/checked-arena/shared-target`, dev/test opt-level 1, debug information
off, incremental off and two build jobs. Preserve unrelated working-tree edits.
Normalize host text before newline-sensitive fixture instrumentation and check
LF/CRLF through that actual path; preserve exact binary/ATASCII bytes.

No full suite, NIR sweep or baseline rebuild is planned for this backend-only
correction. Broaden checks only for actual affected consumers or failures, as
required by AGENTS.md. Any shared frontend/NIR contract change belongs in a
separate slice with the mandatory shared-contract checks.

The five optimizer failures and stale enum snapshot already reproduced on the
pre-DATA baseline are [recorded separately](benchmarks/6502-labeled-data/final/validation.json).
Their repair is a follow-up task; this plan must neither change their expectations
to hide a failure nor claim that repository-wide validation is green.

## Validation record

Slice 1: four `fixed_observation_` unit tests pass, covering typed/numeric and
aliased pointer fetches, wrap boundaries, invalid/unresolved operands, selected
bytes, conservative pointee effects and both fetched bytes' overlap protection.
The physical collector still excludes generated scratch traffic. The four
existing `object_storage_` unit controls and exact selected-input/executable
rollback test also pass. No shared IR contract or emission recipe changed.

Slice 2: both pipeline controls pass, including typed storage relocation input,
actual ZP selection, declared-byte exclusion, LF/CRLF image identity and opaque
fallback. The VM group passes with placement off/on in both runtimes, exercising
the ASM reader, unchanged pointer/sentinels, swaps, arguments, results and stack.
The diagnostic collector now retains scoped explicit observations for each
machine block and hashes the shared observation implementation in its receipt.

The [completed qualification](benchmarks/6502-fixed-address-observations/README.md)
records Block/m7's resolved `$E6/$E7` fetches. TN remains byte-identical at 9,838
bytes, with all routine footprints, leaf/caller facts, accepted trial costs and
work counters unchanged. Sort's first unresolved observation is now _Cio/m11's
layout-dependent `JMP (ErrorHandler)`. _StrNam/m13, Range/m34 and MakeJmp/m29 also
remain unresolved. These are reported rather than added to this slice's scope.
The unchanged TN image did not require repeating its dispatch VM test. No full
suite, NIR sweep or historical baseline rebuild ran.
