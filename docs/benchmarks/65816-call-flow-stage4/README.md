# Call flow: final private local destinations

Stage 4 sends native one-through-four-byte results directly to a sole adjacent
private local Store. The final object remains allocated; the intermediate temp
has no fictitious home. Checked output lanes survive actual outgoing cleanup,
then are consumed at Store. The semantic write stays in Store's source span,
with an exact one-byte tail for a three-byte result.

| Profile | Complete code | Incremental saving | Total saving vs stage 0 | Admitted Stores |
| --- | ---: | ---: | ---: | ---: |
| Optimized release | 426,455 | 1,985 | 3,636 | 286 |
| Optimized guarded | 575,665 | 2,016 | 4,100 | 286 |
| Raw guarded | 640,906 | 6,351 | 8,057 | 901 |

The optimized investigation screened 287 private assignments. The remaining
FSWORKER.Worker assignment has an indirect callee and keeps capture fallback.
No routine grows in code, frame, spill extent or local peak versus stage 3 or
stage 0. Optimized frame/spill/peak sums each decrease another 58 bytes; raw sums
each decrease 178. All prior native Return and zero-test admissions remain.

Validation: 393 emission tests, 52 affected root integration tests, 19 observer
tests and 21 qualified native tests pass (`call_flow`, `call_flow_interruptions`,
`home_demand`, `memory`, `replay`). The native fixtures cover all four widths,
bank-valued pointers, full 32-bit values, poisoned unspecified lanes/flags,
canaries, exact external accesses and LF/CRLF through compilation. A verified
MIR record-field fixture checks every destination byte at nonzero displacement,
including neighboring canaries and no widened read. Current record lowering does
not set the whole object's mutable fact; ordinary source retains fallback until
that stronger fact is available. The fixture explicitly supplies it rather than
changing frontend contracts. Task/IRQ/NMI probes suspend every reached changed
cleanup/Store boundary in two domains and compare full state/private memory
against independent uninterrupted CPU execution.

All 441 frozen vectors retain values, arguments, exact external accesses,
cycles/private traffic and peaks. Their corpus remains a regression oracle;
independent fixtures execute the new routes. [Native results](native-results.json),
[incremental resources](incremental-results.json), [results](results.json),
[provenance](provenance.json) and compressed tables bind the evidence.
Serial compiler-cost acceptance and existing application gates remain open
for stage 6.

Reproduce with the stage-0 commands using `target/call-flow-stage4`, stage `4`,
and stage-0 reference. Focused native command:

```sh
python3 -B tools/native65816-runtime-tests/qualify.py \
  --test call_flow --test call_flow_interruptions --test home_demand \
  --test memory --test replay -j2
```
