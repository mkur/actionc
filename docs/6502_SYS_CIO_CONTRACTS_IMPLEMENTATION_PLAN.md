# 6502 SYS CIO contracts implementation plan

Review the five public CIO routines used by the graphics batch on ActionCart
and standalone. Share the reviewed evidence between classic and MIR6502.
Use the existing runtime binding and freshness checks. [Graphics plan](6502_STANDARD_LIBRARY_CONTRACTS_IMPLEMENTATION_PLAN.md) ·
[Public SYS interfaces](../embedded/modules/sys.act)

## Shared CIO boundary

Record the public ABI, Atari IOCB commands, wrapper scratch and result staging,
argument-relative buffer accesses, and the status/error continuation as
independent facts. Match the selected implementation and its verified local
dependencies from checked-in bytes and linked operands. Keep the device
dispatch at CIOV and the returning error continuation explicit.

An IOCB number identifies a channel. The currently installed handler and its
effects require separate evidence. Replaceable device and error handlers can
observe or overwrite compiler-private ZP. A full normal-return footprint
therefore requires evidence for both dispatches. Until a caller can establish
it, use the known wrapper facts and keep transitive reads, writes, entry and
result definitions conservative. A successful result staged in `$A0` gives
no result guarantee for an unreviewed error return. Existing TN review
annotations describe TN's own implementations and installed callback.
[TN CIO implementations](../samples/tn/modern/LIB.ACT) ·
[Runtime coverage](MIR6502_RUNTIME_LIBRARY_CONTRACT_COVERAGE.md)

## Slice 1. PutD and GetD

- Review the two entries, their shared `CCIO` path and the status handler.
  Cover command `$0B` with the output character in A and command `$07`
  with the successful input character staged in `$A0`. Check that both
  entries set IOCB length to zero for a single character.
- Bind the reviewed records in both backends. Connect `Plot` and `Locate`
  to their provider's CIO dependency while keeping the open continuation
  visible to ZP home admission. Check direct and graphics callers and
  changed linked dependencies.
- Run the graphics VM checks plus direct character I/O tests on both
  providers and backends. Include a returning error handler that writes to
  a ZP home and a case that observes the successful character result.

## Slice 2. Close and Open

- Review `Close`'s command `$0C` and `Open`'s command `$03`, their
  shared `XIOstr` setup and the path to status handling. Account for
  the copied counted filename and the provider-specific staging buffer.
  Bind the cartridge's internal open entry separately from public
  `SYS.Open` where `Graphics` calls it directly.
- Expose both dependencies through the reviewed `Graphics` boundary.
  Verify channel selection, counted filenames, mode/auxiliary bytes and
  the returning failure control. Run the existing device and graphics
  fixtures in both backends on both providers.

## Slice 3. XIO

- Review command and auxiliary staging and conditional access to the
  counted string. Keep the device-specific footprint separate from IOCB
  setup. Qualify screen commands `$11` and `$12` through `DrawTo` and
  `Fill`, including the preserved IOCB auxiliary values for zero AUX1.
- Exercise a direct call with the public ABI and an arbitrary command.
  Keep the transitive footprint unknown when the handler or command is
  unresolved. Compare both backends' bindings and confirm that an unrelated
  same-named routine never acquires the review.

Record qualified fields and remaining dispatch obligations in the coverage
inventory. Run the focused contract, graphics/device VM and 6502
integration checks. Keep the remaining SYS families queued separately.

## Qualification

The shared reviews bind all five entry paths and the graphics dependencies on
both providers. The cartridge mode-setting sequence uses the internal open
entry and bypasses the public filename-copy wrapper. The reviewed XIO setup
reads the counted string's length and skips CIOV when it is zero. The VM
fixtures exercise the screen and editor transitions, character I/O, and a
returning error handler that writes into the ZP pool. Full caller preservation
remains open at the device dispatch and the returning status/error path.
[Direct CIO fixture](../fixtures/runtime/resident_cio_contracts.act) ·
[Coverage inventory](MIR6502_RUNTIME_LIBRARY_CONTRACT_COVERAGE.md)
