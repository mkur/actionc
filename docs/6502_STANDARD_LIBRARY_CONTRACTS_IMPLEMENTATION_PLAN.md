# 6502 standard library contracts implementation plan

The first batch covers all seven public `SYS` graphics routines on ActionCart
and standalone. Implement the fixed-address routines first, then qualify the
screen and mode-setting calls as their CIO dependencies are reviewed. Use the
same reviewed records from classic codegen and MIR6502. Track the guarantees
available for each binding in the existing inventory. `SYS.COLOR` is volatile
drawing state shared by three of the routines. `SYS.Point` belongs with
`SYS.Note` in file positioning.
[Public interface](../embedded/modules/sys.act) ·
[Current coverage](MIR6502_RUNTIME_LIBRARY_CONTRACT_COVERAGE.md)

| Slice | Public routines | Implementation dependencies |
| --- | --- | --- |
| 1 | `Position`, `SetColor` | `Position` falls through into `Pos1`. `SetColor` is self-contained. |
| 2 | `Plot`, `Locate` | `Pos1` or `Position`, followed by the `PutD` or `GetD` screen CIO path. |
| 3 | `DrawTo`, `Fill` | Shared `GrIO` setup and screen XIO commands `$11` and `$12`. |
| 4 | `Graphics` | Close and reopen E: and S: through each provider's CIO and status-handling paths. |

[Standalone graphics source](../corpora/action-runtime/extracted/SYSGR.ACT) ·
[Cartridge bindings](../embedded/bindings/sys-cart.act) ·
[Standalone bindings](../embedded/bindings/sys-standalone.act)

## Contract boundaries

- Review assembly once, with LLM assistance where useful, and check in the
  facts and implementation fingerprint. During compilation, match the
  reviewed identity and apply the facts. A missing or stale match leaves
  the call on its conservative path.
- Reuse the shared 6502 callee facts and existing runtime binding machinery.
  Key the review to the typed public interface, ABI and selected provider.
  For the cartridge, tie it to the supported ROM entry and reviewed bytes.
  For standalone, match the reviewed body after runtime-source normalization,
  with relocation and dependency evidence where applicable. A snapshot of
  the selected implementation supports later freshness checks. It needs an
  initial match against the checked-in review.
- Keep canonical arguments, guaranteed definitions, possible writes and
  normal-return preservation distinct. Keep the hardware effects and call
  ordering visible to optimizers. The shared record states the guarantees.
  Each backend projects the fields its consumers can represent.
  [Shared review design](6502_SHARED_CALLEE_FACTS_DESIGN.md) ·
  [Runtime binding design](MIR6502_RUNTIME_HELPER_CONTRACT_DESIGN.md#preserve-identity-through-runtime-selection)

## Slice 1. Position and SetColor

Use these two entries to wire one shared provider record into the MIR library
invocation path and the classic call-effects projection. Bind it while the
typed interface and selected implementation are both available. Revalidate
after remapping or linking whenever the evidence changes. Each backend uses
the fields it can represent.
[MIR interface resolution](../src/mir6502/interfaces.rs) ·
[MIR library invocation](../src/mir6502/runtime_contracts/library.rs) ·
[Classic cartridge projection](../src/codegen/semir.rs) ·
[Classic standalone linking](../src/codegen/standalone.rs) ·
[Classic fact projection](../src/codegen/reviewed.rs)

`Position` takes the column in A/X and the row in Y. Cover the complete
entry through the return in `Pos1`. The public entry writes `$5A–$5C`
and `$54–$56`. Record `Pos1` separately for the callers that enter there
directly. Verify both coordinate copies with a nonzero high column byte.

The current cartridge entry at `$A6CE` and linked `SYSGR_SetColor` implement
the three-byte Action! ABI in A, X and Y. There is no result or extra
machine input. On the path that sets a color, the routine may write
`$A0` and `$A2`, then writes `$02C4+register` and `$D016+register`.
The value is `(hue << 4) | (luminance & $0F)`, truncated to a byte.
The early return leaves the scratch bytes untouched. Y survives both paths.
[Cartridge binding](../embedded/bindings/sys-cart.act) ·
[Standalone binding](../embedded/bindings/sys-standalone.act) ·
[Resident source](../corpora/action-runtime/extracted/SYSGR.ACT)

The actual `CMP #$05` / `BPL` skips the stores for register values 5–132.
Values 133–255 reach the indexed stores. Certify ABI sufficiency for the
entire BYTE input domain. In this slice, grant the finite normal-return
write footprint only when the caller proves the register is in 0–4.
Keep general memory and device effects conservative. Use the existing
argument-value query for this check, starting with a known constant, and
leave other values on the conservative write path. This scope covers
`SetColor(2,0,0)` in `PMG.DM1`. [Demo call](../samples/toolkit/modern/PMG.DM1)

For `SetColor`, test a dynamic register and the boundaries 4, 5, 132,
133 and 255. Observe both the shadow and hardware writes. Exercise the
high indices in an isolated VM call. Check seeded ZP, stack balance, Y
preservation and the early return. Use a focused MIR caller with a
profitable live local across a valid call and controls for scratch
overlap and unproven register values. Rebuild `PMG.DM1` to verify that
`SetColor` clears admission and record the next blocker, the selected homes
and the image size.

## Slice 2. Plot and Locate

Review the channel-6 paths from `Plot` through `Pos1` and `PutD`, and
from `Locate` through `Position` and `GetD`. Record the volatile
`SYS.COLOR` read by `Plot` as program state. Record `Locate`'s BYTE
result on successful pixel reads and check the definition at the public
result location on every path for which the contract claims it.

Start with the wrapper's certified entry and its known effects. Review the
CIO vector, screen handler and error continuation before publishing
transitive preservation. A replaceable handler or an unreviewed returning
error path keeps the corresponding footprint unknown. Test a plot followed by
a locate, plus a failed read with a returning error handler where supported.
Compare both the pixel and the caller-visible result. [Resident I/O source](../corpora/action-runtime/extracted/SYSIO.ACT)

## Slice 3. DrawTo and Fill

Review `GrIO` once per provider and reuse that evidence for both public
entries. Cover coordinate setup, the volatile color read and the
`$02FB` write, outgoing argument lanes, and the screen XIO path.
Keep command `$11` for the line and `$12` for the fill distinct.
Reuse the CIO boundary from slice 2 for the facts it establishes about both
commands. Verify the command, channel, endpoints and changed pixels for
each call. [Graphics VM fixture](../fixtures/runtime/resident_graphics_io.act)

## Slice 4. Graphics

Review the close and open sequence, mode-derived auxiliary bytes, state
changes and error paths. Follow each provider's actual dependency chain.
The cartridge reaches its lower-level open sequence and status handler.
The linked source calls `Open` and tail-jumps to it. Review the required
`Close` and `Open` paths as dependencies of `Graphics`, with broader
public CIO coverage queued separately. Use the existing TN CIO reviews
as a reference for how to represent a returning callback. Test the sequence
and modes, including a failing open, and keep screen and OS state changes
observable. [Resident graphics source](../corpora/action-runtime/extracted/SYSGR.ACT) ·
[TN CIO reviews](../samples/tn/modern/LIB.ACT)

## Acceptance for each slice

- Check qualified and compatibility-prelude bindings and conservative
  fallback for a user-defined shadow, a different binding and a stale body
  or dependency. Compare checked-in guarantees in classic and MIR6502 for
  both providers.
- Run the existing resident graphics state and I/O fixtures across the
  four backend/provider combinations. Add focused tests for effects the
  fixtures miss. Use a caller with a live home and overlap controls wherever
  the new contract claims preservation. Use a returning-handler control
  before granting preservation across a CIO boundary. VM success on the
  default screen path alone establishes behavior for that tested path.
  [Contract tests](../src/mir6502/runtime_contracts/library_tests.rs) ·
  [Home tests](../tests/mir6502_cross_call_homes.rs) ·
  [VM tests](../tools/vm-runtime-tests/src/lib.rs)
- Record the verified fields and any unknown dependencies in the coverage
  inventory, then commit the slice. Pick the next library family after
  qualifying these seven entries. The 96-entry public interface remains the
  long-term coverage target.
