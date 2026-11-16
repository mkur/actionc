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

## Prioritized issues

| Order | Issue to solve | Why it matters | Design note needed? |
| --- | --- | --- | --- |
| **1** | **Preserve memory-region offsets and extents** | Incomplete read/write footprints can allow stale-value reuse or unsafe deletion. | **No.** Targeted correction to an existing contract. |
| **2** | **Separate possible writes from definite definitions** | A conditional clobber currently makes earlier values appear dead. Fix operation effects, liveness and reaching definitions together. | **No new design.** The intended distinction is already documented; prepare a focused implementation plan. |
| **3** | **Separate ABI arguments from existing program state** | Reading `allocp` must not require callers to supply `$E8/$E9` as arguments. Preserve genuine implicit assembly inputs. | **Yes.** This changes the meaning and representation of entry requirements. |
| **4** | **Represent typed results independently of parameter types** | Pointer parameters currently prevent proving return-home definitions. This creates the spurious `$A0` requirement in `Ord` and `Items`. | **Yes.** Define the MIR result contract and its composition rules. |
| **5** | **Unify guaranteed register/flag definitions** | Entry analyses disagree about ordinary comparisons, producing false carry requirements. | **Yes, for consolidation.** Adding another isolated CMP exception would leave the underlying inconsistency. |
| **6** | **Use backing and placement when checking physical overlap** | A write to a global at `$0600` should not exclude every ZP byte. Unknown placement must remain conservative. | **Yes.** Specify which address evidence is authoritative at each phase. |
| **7** | **Finish separating read and write uncertainty** | An unknown pointee write currently makes otherwise bounded reads unknown. | **No.** Restore the existing directional-effects contract. |
| **8** | **Make summary validity across transformations explicit** | Some consumers rely on undocumented assumptions about cached summaries surviving rewrites. No stale-summary failure was demonstrated. | **Yes, as documentation and hardening.** Avoid introducing a general cache framework without evidence it is needed. |
| **9** | **Compose already-certified assembly leaf contracts** | Useful leaf knowledge is discarded by acyclic composition and residence admission. | **Extend the existing acyclic design after the repairs.** This is additional coverage, not a correctness fix. |

## Design work

Prepare **one short shared design note for issues 3–6 and 8**, covering:

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

Issue **2 can proceed without waiting for the shared design note**. It is a
confirmed unsafe analysis answer, although the audit did not demonstrate a new
TN runtime failure. Keep the fix small and test the affected facts plus a
consuming analysis.
