# o65 conformance fixtures

`vasm_native.o65` was emitted from `vasm_native.s` with vasm 2.0f:

```text
vasm6502_oldstyle -816 -Fo65 -o vasm_native.o65 vasm_native.s
```

The release archive hash is recorded in
[the assessment](../../docs/MIR65816_O65_ASSESSMENT.md). This narrow native
object exercises LONG, LOW, HIGH, WORD and BANK relocations and two exports.
It is a wire conformance fixture, not an Action application (no ABI descriptor).
Keep binary bytes exact; do not normalize their line endings. Tests use this
committed fixture; no external assembler is required to decode it.

SHA-256: binary `40e5fd88032e4ece0096d6699dca9ac8a64b61ab1212c424f14217fd91f21a04`;
source `24285296d3ecf23c197af4665f2647a8dcbe5c2f0fc92a34a017d9ff0942f819`.

`reference.o65` is a hand-specified application with a one-byte RTL routine,
BSS and full/split pointer initializers. Its builder uses literal wire fields,
not compiler APIs. Check it with `python3 fixtures/o65/build_reference.py --check`.
It tests reference loading without depending on the Rust writer.
