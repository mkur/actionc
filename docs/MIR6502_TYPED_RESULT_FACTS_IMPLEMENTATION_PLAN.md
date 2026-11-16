# MIR6502 typed result facts implementation plan

Status: slices 1–3 and the issue-5 design follow-up complete, 2026-11-17.

Implements issue **4** in the [fact repair priorities](MIR6502_FACT_REPAIR_PRIORITIES.md)
using the [typed direct result contract](MIR6502_CALL_FACT_CONTRACT_DESIGN.md#typed-direct-result-contract).
Commit each slice separately. Keep tests focused on MIR6502 and the changed consumers.

## Slice 1 — Carry the independent result contract

- Add `MirDirectResultAbi` and `MirRoutine.direct_result_abi`, distinguishing
  Unknown, Void, byte, word and data-pointer results.
- Lower the result from verified NIR independently of parameters and variadic
  status. Preserve the existing integer-only `scalar_signature` and direct
  parameter admission rules.
- Centralize the supported Action result-home mapping. Preserve metadata through
  existing routine cloning; generated entries without typed interfaces use Unknown.
- Print the result contract in MIR diagnostics and update hand-built routines
  explicitly. Do not infer result typing from their instructions.
- Check metadata lowering, unsupported types, printer distinctions, and compilation
  of constructor consumers. This slice must not change executable output.

Completed: three focused lowering/metadata tests and the MIR6502 snapshot target
passed. The 168 snapshot changes add only `result-abi` diagnostic lines; the
fixture harness's original lowering path verified every other line unchanged.
The snapshot target also checks CRLF normalization. Generated runtime entries
and untyped test constructors explicitly retain Unknown.

## Slice 2 — Prove and compose returned definitions

- Replace the scalar-signature gate in the existing CFG result proof with the
  independent result contract. Require a reachable normal return and prove each
  declared byte separately on all returning paths.
- Use the same result proof for native ActualEntry leaf and acyclic facts;
  PlannedCapture and bounded assembly publish no new result guarantees.
- Keep call inputs before result definitions. Check known consumed result width
  and canonical home at each call site; reject inconsistent contracts.
- Cover pointer/integer parameter parity, partial and conditional word writes,
  may-writes, unreachable returns, incoming result reads, Void/Unknown, ignored
  results and transitive invalidation after a result-only metadata change.
- Run focused entry-fact and call-consumer unit tests. Preserve existing entry,
  parameter, call-closure and storage-safety restrictions.

Completed: all 39 focused entry, handoff and residence tests passed, including
six new result-proof groups. ActualEntry leaf/acyclic answers agree, result-only
metadata changes invalidate ancestors, and every consumed result site is checked.
The old untyped-result regression now removes the direct result contract rather
than the independent integer-specialization signature.

## Slice 3 — Qualify and document issue 4

- Add one focused VM control retaining caller state across pointer-parameter
  calls with byte and word results. Exercise carry/page boundaries, default and
  optimized compilation, both runtimes, and LF/CRLF source paths.
- Extend the retained fact-audit probe to report typed result metadata and proven
  result bytes. Preserve historical baseline artifacts.
- Re-run the independent controls and TN audit. Check that `Next` proves `$A0/$A1`,
  `Ord`/`Items` lose only the false inherited `$A0` requirement, and `Ord` keeps `$A3`.
  Record actual image size and remaining blockers; savings are not required.
- Update contract and priority status with qualification. Run diff/link checks;
  broaden tests only for a failure or newly affected consumer.

Completed: the focused VM control passed across default/optimized compilation,
both runtimes and LF/CRLF inputs, checking returned values, retained caller state
and unchanged pointees over page/carry boundaries. The retained audit passed for
nine controls and all 95 selected TN routines. `Next` proves `$A0/$A1`; `Ord`
keeps `$A3` and `Items` has no fixed-byte input. TN remains **9,811 bytes**, with
the same image hash as issue 3. The independent Unknown-result control still
requires `$A0`. The [receipt](benchmarks/6502-call-facts-audit/typed-results-qualification.json)
records the facts and checks. No full suite was run.

## Follow-up — Design issue 5

After issue 4 is qualified, extend the shared design with the guaranteed
register/flag definition contract. Audit the existing effect classifier,
entry-flag analysis and machine liveness before specifying one shared transfer
authority. Preserve the distinction between possible clobbers and definitions,
including conditional operations, calls and machine blocks. Document migration
boundaries and focused acceptance cases; do not implement issue 5 in this task.

Completed: the [shared design](MIR6502_CALL_FACT_CONTRACT_DESIGN.md#guaranteed-register-and-flag-definitions)
now specifies one guaranteed-definition authority in the existing machine effect
structure, migration of entry/liveness and compatibility consumers, conservative
call handling, and emission/elision acceptance cases. Issue 5 is not implemented.
