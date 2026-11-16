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

## Prioritized issues

| Order | Issue to solve | Why it matters | Design note needed? |
| --- | --- | --- | --- |
| **1** | **Preserve memory-region offsets and extents** | Incomplete read/write footprints can allow stale-value reuse or unsafe deletion. | **No.** Targeted correction to an existing contract. |
| **2** | **Separate possible writes from definite definitions** | A conditional clobber currently makes earlier values appear dead. Fix operation effects, liveness and reaching definitions together. | **No new design.** The intended distinction is already documented; prepare a focused implementation plan. |
| **3** | **Separate ABI arguments from existing program state** | Reading `allocp` must not require callers to supply `$E8/$E9` as arguments. Preserve genuine implicit assembly inputs. | [**Implemented.**](MIR6502_CALL_FACT_CONTRACT_DESIGN.md) |
| **4** | **Represent typed results independently of parameter types** | Pointer parameters currently prevent proving return-home definitions. This creates the spurious `$A0` requirement in `Ord` and `Items`. | [**Design prepared.**](MIR6502_CALL_FACT_CONTRACT_DESIGN.md#typed-direct-result-contract) Implementation pending. |
| **5** | **Unify guaranteed register/flag definitions** | Entry analyses disagree about ordinary comparisons, producing false carry requirements. | **Yes, for consolidation.** Adding another isolated CMP exception would leave the underlying inconsistency. |
| **6** | **Use backing and placement when checking physical overlap** | A write to a global at `$0600` should not exclude every ZP byte. Unknown placement must remain conservative. | **Yes.** Specify which address evidence is authoritative at each phase. |
| **7** | **Finish separating read and write uncertainty** | An unknown pointee write currently makes otherwise bounded reads unknown. | **No.** Restore the existing directional-effects contract. |
| **8** | **Make summary validity across transformations explicit** | Some consumers rely on undocumented assumptions about cached summaries surviving rewrites. No stale-summary failure was demonstrated. | **Yes, as documentation and hardening.** Avoid introducing a general cache framework without evidence it is needed. |
| **9** | **Compose already-certified assembly leaf contracts** | Useful leaf knowledge is discarded by acyclic composition and residence admission. | **Extend the existing acyclic design after the repairs.** This is additional coverage, not a correctness fix. |

## Design work

The shared [call-fact contract design](MIR6502_CALL_FACT_CONTRACT_DESIGN.md)
covers **issue 3** and proposes the typed result contract for **issue 4**.
Extend it for **issues 5–6 and 8**, covering:

- Caller-supplied inputs versus existing state.
- Possible effects versus guaranteed definitions and results.
- Logical storage identities versus physical addresses.
- Fact validity and required recomputation after transformations.

Define contracts and ownership, then map existing producers and consumers onto
them. Separate notes for every symptom would add unnecessary fragmentation.

Issue 9 should extend the acyclic-call-chain section of the
[parameter home selection design](MIR6502_PARAMETER_HOME_SELECTION_DESIGN.md)
after the shared contracts are repaired.

## Next implementation work

Issues **1–3 are repaired**. Issue **4** now has a design note and is ready
for implementation planning. Issue **7** remains an independent targeted
correction that does not need to wait for the remaining shared design work.
