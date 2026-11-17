# MIR6502 fact repair priorities

Based on the [2026-11-16 fact-contract audit](MIR6502_FACT_CONTRACT_AUDIT_2026-11-16.md).
Correctness comes first, then contract consistency, then broader optimization
coverage.

**Issue 1 is implemented.** Read/write region summaries share a checked byte
projection that preserves identity, offset and extent. Unrepresentable ranges
produce Unknown. Regressions cover all supported region kinds, address limits
and caller invalidation of cached values from written bytes.

Validation: 62 focused analysis tests and the home-invalidation integration
control passed. Four of five callee-facts VM tests passed; the remaining test's
all-parameters-inline assertion for `KeepWriter` also fails on the unmodified
`2c5ae7ad` compiler. That separate placement expectation is unchanged here.

**Issue 2 is implemented.** Home effects retain possible writes for clobber
checks and expose a separate guaranteed-definition subset. Only guaranteed
definitions may kill liveness or reaching definitions, initialize private
aliases, or supply replacement reads. Explicit stores and helper result homes
still establish values; finite memory-effect annotations do not. The distinction
is documented in the [pseudo-machine contract](MIR6502_PSEUDO_MACHINE_CONTRACT.md).

Validation: the three new defect regressions failed before the fix and pass
after it. All 70 focused unit tests passed, as did two late-cleanup integration
controls and the caller-observation VM control. The staging-size integration
assertion (`ObservableAlias` in standalone mode) and the VM size-savings assertion
both fail identically with the unchanged `48eef2f6` compiler. Those existing
expectations are unchanged; the VM size assertion also prevents that test from
completing its full runtime/origin matrix. No full suite was run.

**Issue 3 is implemented.** Typed global accesses remain memory observations
without becoming caller-supplied inputs. Native and bounded-machine entry facts
share that distinction; literal scratch reads retain their requirements even
when a declared global overlaps the address. Existing source reservations and
read/write footprints remain authoritative.

Validation: 40 focused entry, handoff, residence and bounded-machine tests
passed. The new global-state VM control passed for default/optimized compilation
in both runtimes, including LF/CRLF input. Two older typed-global input
expectations were corrected, with raw-address negative controls retained. The
retained audit passed for nine controls and all 95 selected TN routines.
`ArenaCursor` now has no fixed-byte entry inputs, still reads both `allocp`
bytes, and retains the separate carry blocker. TN remains 9,811 bytes and is
byte-identical to the audited image. No full suite was run.

**Issue 4 is implemented.** Direct result metadata is independent of parameter
types. Native ActualEntry leaf and acyclic facts prove each result byte on all
returning paths; call sites reject conflicting known result contracts.

Validation: three focused lowering/metadata tests, the MIR6502 snapshot target,
39 entry/handoff/residence tests, and the pointer-parameter result VM control
passed. VM coverage includes both runtimes, LF/CRLF, page boundaries and carry
wraparound. The retained audit passed for nine controls and all 95 TN routines.
`Next` proves `$A0/$A1`; `Ord` retains only `$A3` as a fixed input, and `Items`
has none. TN remains 9,811 bytes, byte-identical to the issue-3 image.
`ArenaCursor` still has the separate carry blocker. See the
[qualification receipt](benchmarks/6502-call-facts-audit/typed-results-qualification.json).
No full suite was run.

**Issue 5 is implemented.** Ordinary liveness, entry proofs and local cleanup
share guaranteed register/flag definitions. Clobbers remain possible changes;
zero-CMP reuse, identity moves, zero shifts and conditional recipes cannot
invent definitions. The coarse entry query projects the precise query.

Validation: 373 focused unit tests, seven loop integration tests, three callee-fact
VM controls and the C/V observer VM matrix passed. The nine-control/95-routine
audit removes the false acyclic carry inputs of `ArenaCursor` and `Alloc` while
retaining issue-4 results. TN remains byte-identical at 9,811 bytes. Two older
A→X/precise-entry size assertions fail identically on pre-repair `2ce87047`;
expectations are unchanged. See the
[qualification receipt](benchmarks/6502-call-facts-audit/machine-definitions-qualification.json).
No full suite was run.

**Issue 7 is implemented.** Native read summaries no longer inherit write-only
uncertainty. They retain complete direct operands and pointer carriers, using
current virtual allocations and rejecting incomplete or overflowing extents.
Actual pointee reads remain Unknown; unknown writes still invalidate values.

Validation: 229 focused unit tests, two late-cleanup integration controls and
three callee-fact VM controls passed.
VM coverage includes both runtimes, default/optimized compilation, LF/CRLF,
page boundaries, direct-source aliasing and live caller state. Three scalar-call
elision assertions and the home-invalidation fixture's `CheckRoom` size assertion
(83 bytes versus the expected 89) also fail on clean pre-repair `9a9a1661`; their
expectations are unchanged. The retained audit passes 13 independent controls and all 95 TN
routines. Only `Push` gains a finite read footprint; writes, placement and MIR
remain unchanged. TN remains byte-identical at 9,811 bytes. See the
[qualification receipt](benchmarks/6502-call-facts-audit/directional-reads-qualification.json).
No full suite was run.

**Issue 8 is implemented.** Core summary consumers share an explicit validity
scope and compatibility check with terminal cleanup. A changed callee contract
restores all dependent core rewrites, home records and statistics before one
conservative retry. Register-reload folding uses the same scoped facts.

Qualification results are recorded in the
[implementation plan](MIR6502_FACT_VALIDITY_IMPLEMENTATION_PLAN.md) and
[receipt](benchmarks/6502-call-facts-audit/fact-validity-qualification.json).
TN's checked attempts reject `Pull`'s expanded physical writes after ZP remapping;
conservative fallback grows the image from 9,811 to 9,926 bytes (+115). Native
entry proofs and parameter/local home choices are unchanged. This establishes
a contract change, not a demonstrated TN miscompile. Recovering those bytes
requires preserving the earlier contract or replaying with narrower assumptions;
rebuilding a summary alone cannot do it.

## Prioritized issues

| Order | Issue to solve | Why it matters | Design note needed? |
| --- | --- | --- | --- |
| **1** | **Preserve memory-region offsets and extents** | Incomplete read/write footprints can allow stale-value reuse or unsafe deletion. | **No.** Targeted correction to an existing contract. |
| **2** | **Separate possible writes from definite definitions** | A conditional clobber currently makes earlier values appear dead. Fix operation effects, liveness and reaching definitions together. | **No new design.** The intended distinction is already documented; prepare a focused implementation plan. |
| **3** | **Separate ABI arguments from existing program state** | Reading `allocp` must not require callers to supply `$E8/$E9` as arguments. Preserve genuine implicit assembly inputs. | [**Implemented.**](MIR6502_CALL_FACT_CONTRACT_DESIGN.md) |
| **4** | **Represent typed results independently of parameter types** | Pointer parameters previously prevented proving return-home definitions, creating the spurious `$A0` requirement in `Ord` and `Items`. | [**Implemented.**](MIR6502_CALL_FACT_CONTRACT_DESIGN.md#typed-direct-result-contract) |
| **5** | **Unify guaranteed register/flag definitions** | Entry analyses disagree about ordinary comparisons, producing false carry requirements. | [**Implemented.**](MIR6502_CALL_FACT_CONTRACT_DESIGN.md#guaranteed-register-and-flag-definitions) |
| **6** | **Use backing and placement when checking physical overlap** | A write to a global at `$0600` should not exclude every ZP byte. Unknown placement must remain conservative. | [**Fixed-global slice implemented and qualified.**](MIR6502_FIXED_BACKING_OVERLAP_IMPLEMENTATION_PLAN.md) Checked backing/aliases feed preservation, invalidation and caller-byte checks; wider owner-aware private overlap remains separate. |
| **7** | **Finish separating read and write uncertainty** | Unknown pointee writes no longer erase bounded operand/carrier reads; actual pointee reads remain Unknown. | [**Implemented.**](MIR6502_DIRECTIONAL_READ_FACTS_IMPLEMENTATION_PLAN.md) Existing directional-effects contract restored. |
| **8** | **Make summary validity across transformations explicit** | Some consumers rely on undocumented assumptions about cached summaries surviving rewrites. No stale-summary failure was demonstrated. | [**Core slice implemented.**](MIR6502_FACT_VALIDITY_IMPLEMENTATION_PLAN.md) Scoped reuse, shared compatibility checks and complete core fallback; existing entry and terminal owners retained. |
| **9** | **Compose already-certified assembly leaf contracts** | Useful leaf knowledge was discarded by acyclic composition and residence admission. | [**Implemented and qualified.**](MIR6502_ASSEMBLY_LEAF_COMPOSITION_IMPLEMENTATION_PLAN.md) Existing ActualEntry certificates compose through native callers; live residence still requires complete writes. Independent control saves 7 bytes; TN saves 5. |

## Design work

The shared [call-fact contract design](MIR6502_CALL_FACT_CONTRACT_DESIGN.md)
covers the implemented contracts for **issues 3–5**.
It also specifies the backing/placement contract for **issue 6**. Its fixed-global
implementation is qualified; the initial slice retains the existing private-storage
exclusions rather than claiming new owner-aware layout precision.
Its **issue 8** section specifies implemented core fact lifetime and rewrite
compatibility, retaining the shared distinctions:

- Caller-supplied inputs versus existing state.
- Possible effects versus guaranteed definitions and results.
- Logical storage identities versus physical addresses.
- Fact validity and required recomputation after transformations.

Define contracts and ownership, then map existing producers and consumers onto
them. Separate notes for every symptom would add unnecessary fragmentation.

Issue 9 extends the [acyclic-call-chain design](MIR6502_PARAMETER_HOME_SELECTION_DESIGN.md#proposed-extension-certified-assembly-leaves-in-acyclic-call-chains)
with certified assembly leaves. The first scope connects existing entry and
write facts without expanding decoder coverage or inventing guaranteed result
definitions. Machine callees' own home policy and explicit observations remain
unchanged.

## Next implementation work

Issues **1–5, 7, 9 and the core slice of 8 are repaired**. Issue **6** is qualified
for fixed globals and aliases; wider owner-aware private overlap remains
separate. Issue **9** connects certified assembly entries and writes without
expanding decoder support or proving assembly result definitions. Recovering
issue 8's measured conservative fallback cost remains separate; retain the
existing compatibility and rollback guards when recovering size.
