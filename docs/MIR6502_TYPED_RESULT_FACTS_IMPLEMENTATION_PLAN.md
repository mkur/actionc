# MIR6502 typed result facts implementation plan

Status: slice 1 complete; slices 2–3 pending, 2026-11-16.

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

## Follow-up — Design issue 5

After issue 4 is qualified, extend the shared design with the guaranteed
register/flag definition contract. Audit the existing effect classifier,
entry-flag analysis and machine liveness before specifying one shared transfer
authority. Preserve the distinction between possible clobbers and definitions,
including conditional operations, calls and machine blocks. Document migration
boundaries and focused acceptance cases; do not implement issue 5 in this task.
