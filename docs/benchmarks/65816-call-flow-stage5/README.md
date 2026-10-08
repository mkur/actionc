# Call flow: bounded private scalar inputs

Stage 5 admits full canonical byte/word Loads with a sole terminal Direct-call
argument use within sixteen same-block operations. Incoming parameters must be
immutable and locals unexposed. Calls, writes, copies, volatile/external reads,
partial views and parameter-backed frame aliases exclude the interval. All
bindings and ordinary operands share complete argument/padding preflight and
actual construction selection before an atomic allocation trial omits homes.
Final sources and schedules are rederived; broken sealed ownership is an error.
Borrowing ends before the callee, separately from its fresh output. Existing
wide scalar, pointer and bounded accumulator strategies retain their contracts.

| Profile | Complete code | Incremental saving | Total saving vs stage 0 | Narrow bindings | Newly deferred reads |
| --- | ---: | ---: | ---: | ---: | ---: |
| Optimized release | 425,954 | 501 | 4,137 | 283 | 91 |
| Optimized guarded | 575,068 | 597 | 4,697 | 283 | 91 |
| Raw guarded | 639,980 | 926 | 8,983 | 374 | 166 |

The other 192 optimized/208 raw bindings already borrowed adjacent storage but
still reserved a canonical temp home. Their removal is distinct from deferring
a previously captured Load. Of the 121 optimized candidates screened by the
investigation, 91 are admitted: 24 cross external reads, two terminate at Helpers,
three cross reads of parameter-backed pointer frames and one widens a two-byte
value into a three-byte argument slot. Those 30 retain complete capture fallback.
No routine grows in code, frame, spill extent or local peak versus stage 4 or
stage 0. Optimized frame/spill/peak sums each decrease another 80 bytes; raw sums
each decrease 96. All prior result admissions remain.

Full reservation bounds cover every partial push depth and exact-width source
tail. Near-limit shapes outside that stronger bound retain fallback even if a
particular push order might fit. Several narrow bindings can share a complete
call alongside constants, symbols and wide captures; no general register
argument or wide result-to-call chain is added.

Validation: 394 emission tests, 47 root integration tests and 19 observer tests
pass. Two opt-in inventory integration cases remain ignored under their usual
manifest requirements. The qualified native suite passes 22 tests (`call_inputs`,
`scalar_forwarding`, `call_pushes`, `call_copies`, `call_padding`,
`terminal_pointer_calls`, `call_flow`, `call_flow_interruptions`, `replay`).
Independent assembly checks complete argument bytes/padding and fresh outputs
with poisoned unspecified registers/flags, several narrow inputs, a wide value
and a symbolic pointer. Verified MIR probes exercise both incremental pushes and
full reservation/store construction; raw/optimized, guarded/release and both
interrupt masks pass. Existing fixtures cover repeated private reads, nested-call
fallback, source order, task/IRQ/NMI construction boundaries and two o65 placements.
Actual source compilation checks LF/CRLF identity. The observer now distinguishes
separately qualified zero-frame terminal wrappers from ordinary Call reads;
a focused native regression prevents fictitious read observations.

All 441 frozen native vectors retain exact values, arguments, external traces,
cycles/private accesses and peaks. This corpus is a regression oracle; the new
independent fixtures execute the admitted inputs. [Binding census](argument-bindings.json.gz),
[incremental resources](incremental-results.json), [native results](native-results.json),
[results](results.json), [provenance](provenance.json) and compressed tables bind
the measurements. The refused observation draft is not published as evidence.
Serial compiler-cost acceptance and carried application gates remain for stage 6.

Use the stage-0 reproduction commands with `target/call-flow-stage5`, stage `5`,
and the stage-0 reference. Focused native command:

```sh
python3 -B tools/native65816-runtime-tests/qualify.py \
  --test call_inputs --test scalar_forwarding --test call_pushes \
  --test call_copies --test call_padding --test terminal_pointer_calls \
  --test call_flow --test call_flow_interruptions --test replay -j2
```
