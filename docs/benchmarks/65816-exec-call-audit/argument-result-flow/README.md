# Exec816 argument and result flow investigation

Start with native call results consumed by byte/word zero tests, then capture
results directly into their final private local destinations. Extend bounded
private scalar borrowing into multi-argument calls alongside that foundation.
Wide argument flow remains the larger follow-up: it needs a complete schedule
for native lanes, guards and argument construction.

The recurring constraint is ownership at the call boundary. Calls produce
values in declared native lanes, but ordinary selection immediately requires a
temporary memory home. The common planner does not currently admit calls as
register producers, and its accumulator argument consumer requires one argument.
This leaves useful producer/consumer relationships outside the existing plans.

## Measured cohorts

This investigation joins typed MIR operands and storage facts with actual
selected accesses from the [original audit](../README.md). It uses compiler
`af7c504c`, Rust 1.99.0 and frozen Exec `57df0d7`, optimized release. The image,
MIR inventory, placement census and call observations all reproduce the original
audit byte for byte. The original audit and stage-7 records remain unchanged.

| Structural cohort | Values | Capture store instruction bytes | Consumer home-read instruction bytes |
| --- | ---: | ---: | ---: |
| Byte/word call result immediately compared with zero | 692 | 1,384 | 1,384 |
| Call result immediately assigned to private frame storage | 287 | 988 | 988 |
| Bounded private byte/word reads used only as call arguments | 121 | 242 | 260 |

These are existing instruction footprints, not measured or promised savings.
Consumers still need comparisons, final destination writes or ABI argument
construction. Mode changes, cleanup, physical bounds and lane preservation affect
the complete cost. The first two cohorts describe result consumers; the third
describes a separate private-load producer cohort.

### Results tested against zero

There are 665 byte and 27 word results with a sole adjacent `Eq`/`Ne` comparison
against literal zero. In 686 cases the comparison has a sole adjacent branch
consumer. Every one of the 692 currently performs both a capture store and a
read from that home. This is the clearest initial cohort: no source memory read
needs to be delayed and no address needs to be prepared for the consumer.

The exact [`SDFSFILE.Open` excerpt](../examples.lst) shows the pattern: the caller
restores S while preserving A, stores the returned byte, reloads it, then branches
on zero. A planned native result lifetime could let that consumer use the
returned value directly.

The remaining adjacent comparison cases include nonzero constants, other temp
operands, wide values and signed comparisons. Keep their requirements separate
from the first zero-test slice. Equality against zero is a particularly small
starting contract; it does not require ordering normalization or signed overflow
handling.

### Results assigned to locals

Of 381 adjacent result-to-`Store` cases, 377 use the result as the stored value;
four use it as the destination address. Of the value stores, 287 target unindexed,
nonvolatile private frame objects. Their widths are 37 bytes, 43 words,
132 three-byte values and 75 four-byte values.

These sites capture into a compiler temp, then read that temp to write the local.
The final local store is required. A checked capture into the final local home
could remove the intervening temp transfer while preserving the store's position
after the call. `HEAPCORE.Deallocate` supplies a four-byte example in the case
table. This direction reaches wide values without keeping them through arbitrary
subsequent instructions.

The other stored-value cases target 84 indirect destinations, including four
indexed forms, and six static destinations. Address preparation can overwrite A,
use Y or demand other scratch. Four address-use cases require the result as a
pointer base rather than as payload. Do not admit all `Store` consumers through
one unchecked register path.

### Private arguments and wide flow

The conservative private-source screen finds 84 immutable-parameter reads and
37 private local reads, each with a sole call use. They comprise eight bytes and
113 words. All 121 feed calls with multiple arguments; only two are immediately
adjacent to the call. The other 119 have intervening operations within the same
block, so extending only adjacent accumulator forwarding would miss most of them.

The screen requires a complete canonical home, rejects mixed/volatile or exposed
views, excludes frame objects backing parameters, and stops at intervening calls,
stores, copies or volatile accesses. It is an investigation screen, not complete
compiler admission. Physical displacements during partial argument construction,
ownership and selector compatibility still need compiler proofs.

`TASKPOLICY.PointerResult` illustrates an immutable word parameter captured
before preparing a second argument. The existing borrowing machinery already
supports bounded pointer and LONG sources; extending its common ownership model
to byte/word inputs would cover this pattern without keeping the value in A
through the intervening work.

The full argument cohort remains 1,603 captured sole-call values, of which
**1,214 are three or four bytes**. Computed addresses, pointer loads, wide casts
and results passed to subsequent calls dominate that larger group. Many read
through record pointers. Their evaluation order cannot be preserved by treating
those reads as immutable private backing storage.

The 94 adjacent call-result-to-call cases include 47 three-byte and 33 four-byte
results. For wide values, the next call's guard can clobber X before pushes;
argument order also determines which lanes must remain available while other
arguments are constructed. A complete schedule must handle that lifetime or
retain a capture. One/two-byte arguments already have a Y-preservation mechanism,
but it must be preflighted against the complete multi-argument plan before reuse.

### Casts already sharing storage

Among the 153 captured results followed by casts, 90 have unchanged three-byte
width. Of those, 83 share their canonical home with the cast destination and
perform no home read at the cast. For example, `HEAPPOLICY.Start` casts a returned
pointer to another pointer type with a zero-byte cast span, then passes it to a
call. This is already effective storage coalescing. Its remaining capture belongs
to the longer result-to-argument flow, not an extra cast copy.

The remaining casts include extension, truncation and signed forms. Eleven
unsigned byte-to-word casts are a small later cohort that can use the byte
result ABI's zero-extension guarantee. Any downstream capture and use obligations
still remain.

## Where the compiler needs to change

The relevant owners are all in MIR65816:

- [`home_demand::Plan`](../../../../src/mir65816/emit/home_demand.rs) chooses omitted
  homes before allocation. Its producer match admits loads, selected arithmetic
  and casts; `Call` falls back to `UnsupportedProducer`. Its call consumer accepts
  only one one/two-byte temp argument at offset zero.
- [`Builder::call_with_accumulator`](../../../../src/mir65816/emit/select.rs#L2516)
  preserves native A/X through cleanup, then ordinarily captures them.
  [`call_result`](../../../../src/mir65816/emit/call_copies.rs#L194) requires the
  result temp's allocated home even when immediate return forwarding omits the
  physical store. A planner that omits that home needs a different preflight.
- [`placement`](../../../../src/mir65816/emit/placement.rs) owns register lifetimes,
  borrowed inputs and redirected local captures. Extend those checked contracts
  so a native result has a declared lane origin and a specific consumer or final
  destination. Reuse stable routine, block, temp and storage identities.
- [`resources`](../../../../src/mir65816/emit/resources.rs) treats calls as
  barriers. Keep that protection for pre-call values. Newly returned outputs need
  a checked definition at the call's exit; this does not imply that any incoming
  register or DP cache survived the call.
- [`call_pushes`](../../../../src/mir65816/emit/call_pushes.rs) owns complete
  argument construction. Borrowed homes and live native inputs must participate
  in that plan's exact widths, partial S deltas, padding and cost comparison.

The first foundation should separate declared native result validation from the
choice of capture destination: a temp home, an admitted register lifetime, or a
final private destination. Missing homes must follow a complete checked decision.
Generalize the existing immediate-return preservation mechanism through that
model rather than using a return-specific exception for every new consumer.

Result flags need explicit handling. Calls do not promise useful flags; cleanup
changes them. `TYA` restores A and establishes N/Z, and the byte ABI guarantees
zero in hidden B, but the current tracked call state records fresh word values
without a dedicated zero-extension value fact. Either prove the appropriate
native result/flag relation or emit a fresh comparison. Preserve the typed call
effects and boundary contracts in both cases.

## Suggested implementation slices

The proposed [call-flow design](../../../MIR65816_CALL_FLOW_DESIGN.md) defines
the ownership, lifetime, construction and fallback contracts for these slices.
It does not claim implementation or measured savings.

1. **Native result origins and scalar zero tests.** Add the checked result-use
   decision, preflight and placement ownership, then consume sole adjacent
   byte/word zero tests directly. Keep other result uses on the current capture
   path. Include branch and materialized Boolean consumers.
2. **Native captures into private locals.** Extend the destination decision to
   the final frame object for admitted adjacent assignments across widths 1–4.
   Validate destination identity, mutability, complete bounds and source-span
   ownership. Preserve the required local store and conservative call effects.
3. **Bounded private scalar arguments.** Extend common source ownership to
   byte/word parameter and local reads through same-block multi-argument
   preparation. Preserve barrier and physical-displacement checks. Measure this
   separately from wide native argument scheduling.

Then assess wide result-to-argument flow and identity-cast chains using the same
native origin and destination model. Keep source reads at their original points
unless structured storage/effect proofs permit moving them. Retain capture when
guard or argument schedules cannot protect the complete value profitably.

Existing [native home-demand tests](../../../../tools/native65816-runtime-tests/tests/home_demand.rs),
[call-return tests](../../../../tools/native65816-runtime-tests/tests/call_returns.rs),
[argument construction tests](../../../../tools/native65816-runtime-tests/tests/call_pushes.rs)
and [terminal pointer call tests](../../../../tools/native65816-runtime-tests/tests/terminal_pointer_calls.rs)
provide relevant harnesses. New slices need independent assembly callees, exact
external-access and argument/padding oracles, raw/optimized and guarded/release
execution, relocated images, and task/IRQ/NMI interruption at changed boundaries.
Test zero/nonzero extremes, poisoned unspecified flags/lanes, aliasing and
fallback shapes. Rebuild all frozen profiles and measure complete code, frame,
peak, traffic and compiler cost before claiming an improvement.

## Evidence and validation

[results.json](results.json) retains structural counts.
[result-cases.csv.gz](result-cases.csv.gz) and
[argument-cases.csv.gz](argument-cases.csv.gz) retain every classified site,
actual home-read/store footprints and screening details.
[provenance.json](provenance.json) binds typed observations, probe/compiler inputs,
commands and original-audit identity;
[evidence-sha256.json](evidence-sha256.json) authenticates generated files.

```sh
python3 -B tools/compare65816/exec_call_flow.py collect
python3 -B tools/compare65816/exec_call_flow.py report
python3 -B tools/compare65816/exec_call_flow.py report --check
python3 -B -m unittest discover -s tools/compare65816 -p 'test_exec_call_*.py'
```

The optional `record_probe --features flow-analysis` exports typed operands and
storage facts only for emitted routines. Bulky artifacts stay under
`target/exec-call-flow`; the existing audit probe build cache is reused. This
investigation changes observation tooling and documentation, not compiler policy
or IR contracts. The current probe passes an all-features check; 16 focused tool
tests, both report checks, formatting and documentation links pass. LF and CRLF
capture CSV inputs pass through the actual parsing and typed-join path. No new
VM execution, backend qualification or measured savings are claimed here.
