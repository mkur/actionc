# Call flow: common result declarations and routes

Stage 1 separates native ABI declarations from physical destination validation.
The common placement owner binds Call point, target, complete ABI, result temp,
width and native lanes to the immutable MIR and its complete logical census.
Capture, discard and immediate Return are explicit routes. Recomputed placement
and selected-transfer checks reject changed route tables, targets, contracts,
duplicate transfers and missing transfers. Allocated destinations retain their
complete width/range checks; discarded declarations also validate native lanes.

All five artifacts per profile match the [stage-0 baseline](../65816-call-flow-stage0/README.md)
byte for byte: optimized release 430,091 bytes, optimized guarded 579,765 and raw
guarded 648,963. Frames, spills, peaks, source spans, captures and reads are
unchanged. Return homes remain reserved. This slice claims no code savings.

Validation: 389 emission tests and the new route-forgery regression pass;
45 root ABI/contract/emission tests pass. Qualified native `call_copies`,
`call_returns`, `replay` and `call_flow` pass; final manifest is
`tools/native65816-runtime-tests/target/qualification/run-jk4vxih6/manifest.json`.
An earlier run was refused when a test source changed; the listed rerun used
stable inputs. Frozen collection uses Rust 1.99 and the stage-0 build settings.
[Provenance](provenance.json) and [results](results.json) bind all five artifacts.

The next slice implements atomic trial allocation and final native-output
validation on these logical facts. Compiler-cost acceptance remains a final
serial before/after obligation against the recorded stage-0 medians.

```sh
python3 -B tools/compare65816/exec_call_candidate.py collect \
  --output target/call-flow-stage1 --stage 1
python3 -B tools/compare65816/exec_call_candidate.py report \
  --output target/call-flow-stage1 --reference target/call-flow-stage0 \
  --destination docs/benchmarks/65816-call-flow-stage1 --check
python3 -B tools/native65816-runtime-tests/qualify.py \
  --test call_copies --test call_returns --test replay --test call_flow -j2
```
