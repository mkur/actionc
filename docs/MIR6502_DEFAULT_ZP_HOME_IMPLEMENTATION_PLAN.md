# Implementing default zero-page homes in MIR6502

In optimized MIR6502, enter every ordinary compiler-controlled local or parameter
that needs a memory home into ZP allocation. Use inline storage when its lifetime,
width or ABI capture requirements leave no suitable slot. Keep register-only and
elided values as they are. Capture the TN baseline described in slice 5 before
the first compiler change. Deliver each slice below as a separate commit.
[Placement design](MIR6502_PRIVATE_ZP_HOME_PLACEMENT.md)

## 1. Classify physical pool use for native and reviewed writes

Start with the current TN blocker, `Copy → Convert(v(j))`. The native routine
stores into the named array `fname` using an index. Teach the home-placement
query to use the typed base and addressing mode. A store to ordinary program
data can coexist with a private home. A fixed or numeric access excludes every
pool byte its effective address can reach, including through address wrap.
Keep the full pointee effects for ordering and other memory analyses.
[Native write analysis](../src/mir6502/analysis/known_callees/native_writes.rs)

Reuse the same distinction for a reviewed fixed-size or counted buffer write.
Carry its destination and extent through a native wrapper so an explicit
numeric actual stays visible. Include fixed scratch and argument lanes. If a
write can alter its own pointer or count and redirect later writes, use the
conservative physical footprint for that call.
[Reviewed buffer effects](../src/mir6502/analysis/known_callees/reviewed_buffers.rs)

**Acceptance.** A caller word survives an indexed store to ordinary data.
Passing an ordinary buffer to a reviewed copy permits an unrelated private home.
The same wrapper called with `$00A2` excludes the overlapping byte. Exercise
zero-means-256, wraparound and carrier overwrite using the existing copy tests.

## 2. Apply call conflicts to the values that cross each call

Replace the caller-wide exit on missing call facts with site-specific pool
conflicts wherever placement can change safely. Account separately for fixed
entry reads, outgoing argument setup, callee and descendant homes, scratch and
result delivery. Test a value's lifetime at each of those uses. A caller that
overwrites an implicit input before entry is unsafe even when its local dies
at the call. Preserve the existing ABI validation and result handoff checks.
[Cross-call selector](../src/mir6502/parameter_homes/cross_call.rs) · [Call facts](../src/mir6502/parameter_homes/calls.rs)

For opaque assembly, helpers, OS calls and callbacks, consume the selected ABI
and matching reviewed facts. An entry-only or stale review supplies no additional
return preservation. Values live through that boundary can use only bytes with
a preservation guarantee. Apply the same conflict rule during selection and
after expansion, using the facts for the layout being checked.
[Lifetime verification](../src/mir6502/parameter_homes/lifetimes.rs)

**Acceptance.** Use one caller with a value that dies before an opaque call, a
second that survives it, and a third defined afterwards. Only the crossing value
needs return preservation. In a nested native-call test, a grandchild's ZP
scratch rules out its actual bytes. The verifier rejects a deliberately
overlapping binding and accepts one in a free slot.

## 3. Allocate ordinary memory homes by default

Use the existing address-control facts and pool selector for both leaf routines
and routines with calls. Include leaf locals as well as parameters. Narrow
routine admission checks that merely require a recognized operation shape.
Keep a concrete reservation or conservative conflict wherever a machine effect
can touch the pool. Taking `@p` or exposing an alias constrains the cell itself.
Passing the pointer value in `p` leaves the cell eligible.
[Admission](../src/mir6502/parameter_homes/admission.rs) · [Address control](../src/mir6502/analysis/storage_ownership/source_objects.rs) · [Pool selector](../src/mir6502/parameter_homes/zero_page.rs)

Use the selected runtime's pool, respecting source-owned bytes and ABI
reservations. Check simultaneous lifetimes, loop back edges and contiguous word
lanes. Place an `SArgs` capture frame as one unit. A local read across
invocations needs a reservation for the full interval and its specified initial
contents. For a dedicated ZP home, reserve the bytes across other routines and
establish their contents at program entry. Try each suitable slot before using
inline storage for that object, and continue allocating its siblings.
[Lifetime and persistence rules](MIR6502_UNEXPOSED_OBJECT_STORAGE_DESIGN.md#zp-home-lifetimes-and-persistent-storage)

**Acceptance.** In a mixed routine, place a private byte while an address-taken
sibling stays addressable. In a fragmented pool, a byte can still fit when a word
or capture frame falls back. Test reuse after last use, overlapping loop
lifetimes, a repeated call reading retained state, and a parameter already
carried in registers. Execute the cases on cart and standalone.

## 4. Resolve contention and report the final layout

When candidates compete, prefer the home expected to save more at executed
uses. Start with a deterministic estimate from MIR loads and stores, weighting
loop uses and accounting for capture, spills and reloads. Break ties by stable
object identity. Submit the batch to the existing whole-image cost check. Verify
explicit physical reads and writes against the original and proposed layouts.
Retain the previous executable if verification or cost rejects the proposal.
[Batch selection](../src/mir6502/parameter_homes.rs) · [Physical verification](../src/mir6502/parameter_homes/physical.rs)

**Acceptance.** With one free byte, a repeatedly accessed loop local wins over
a cold local declared first. Reversing declaration order preserves that choice.
Extend the existing home trace to identify the responsible call, reserved
bytes, competing home or contiguous-width failure for an inline object.
Distinguish a rejected batch from pool exhaustion. Trace-on and trace-off builds
produce identical images, with no code-generation trial per object.

## 5. Qualify on shipped TN and TNDBG

Use the same sources, optimized MIR6502 and origin `$3000` before and after.
The table gives packaged XEX bytes at `9ba0a77a`. Use that revision for the
reference home maps, VM cycles and compiler timings.
[Baseline qualification](MIR6502_PRIVATE_ZP_HOME_PLACEMENT_IMPLEMENTATION_PLAN.md#input-review)

| Program | Cart | Standalone |
| --- | ---: | ---: |
| TN | 9,584 | 9,723 |
| TNDBG | 12,602 | 12,743 |

Report final ZP parameters and locals, their byte ranges, and the first concrete
conflict for remaining `Copy` fallbacks. Check that the indexed `fname` store in
`Convert` no longer blocks `mem`, `len` and `files`. For `Copy.j`, trace both
the panel copies under `SwapWin` and the later read after `Bget`. Use only the
facts available on each path, including any reachable CIO or returning handler.
A subsequent call can still require an inline home.

Compare final XEX sizes and VM cycles for the same panel switch and file-copy
workload, with identical inputs, hooks and measurement boundaries. Check copied
bytes, panel state and return behavior. Measure compiler time separately with
already-built release compilers, tracing off, and repeated warm runs. Report the
median for each runtime and any regression alongside the placement changes.
[Panel VM cases](../tools/vm-runtime-tests/tests/support/tn_panels.rs) · [Copy VM cases](../tools/vm-runtime-tests/tests/support/tn_copy.rs)

Run focused checks as each slice lands. For the default switch and final
qualification, run the affected MIR6502 backend suite, including these targets.

```sh
cargo test --lib mir6502::analysis::known_callees::
cargo test --lib mir6502::parameter_homes::
cargo test --test mir6502_parameter_homes --test mir6502_cross_call_homes
cargo test --manifest-path tools/vm-runtime-tests/Cargo.toml --test parameter_homes --test cross_call_homes --test callee_facts --test tn_dispatch
```
