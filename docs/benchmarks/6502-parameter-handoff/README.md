# Parameter-to-argument handoff measurements

The [implementation plan](../../MIR6502_PARAMETER_ARGUMENT_HANDOFF_IMPLEMENTATION_PLAN.md)
starts from compiler `471ef1c8`, with qualified leaf parameter placement enabled.
[baseline.json](baseline.json) freezes twelve builds: TN and independent wrapper
controls with inlining disabled/enabled, both runtimes, origins `$3000`/`$41C7`.
All four TN image hashes and compiler work counts exactly match the prior
[leaf qualification](../6502-parameter-homes/qualification-on.json).

| Subject | Code bytes | Ordinary parameter bytes | Bound ZP bytes |
| --- | ---: | ---: | ---: |
| TN Fnamecmp | 105 | 0 | 4 |
| TN Strcpy | 41 | 4 | 0 |
| TN Strcat | 113 | 4 | 0 |
| Independent ForwardWords | 26 | 4 | 0 |
| Independent CopyCounted | 41 | 4 | 0 |

TN images are 9,878 bytes (cart) and 9,985 (standalone); wrapper controls are
572 bytes in all configurations. Every measured traced output equals its full
untraced artifact. The receipt records source/library/image hashes, configuration,
routine sizes, upstream selections and downstream work counts. The single
sequential probe took 108.30 seconds, maximum RSS 321,830,912 bytes; it includes
traced and untraced compilation and excludes rebuilding the compiler/probe.
This is provenance, not a statistical performance claim.

Reproduce after sourcing `build/mir6502-layout-census/build-env.sh` and building
the current library:

```sh
python3 tools/compare6502/parameter_homes.py \
  --api-rlib PATH/TO/libactionc-HASH.rlib --enabled --suite handoff \
  --output build/mir6502-parameter-handoff/baseline
cargo test --test mir6502_parameter_argument_handoff
cargo test --manifest-path tools/vm-runtime-tests/Cargo.toml \
  --test parameter_argument_handoff
```

Baseline checks: two root tests and one VM test pass. VM coverage uses an
independent canonical-argument/ordered-memory oracle, both runtimes/origins,
actual LF/CRLF loading, all 256 length bytes, page boundaries, identical and
partially overlapping buffers, and stack balance. Additional controls cover
byte widening, parameter mutation/exposure, extra calls, post-call uses, source
ZP ownership and an extra callee entry read. Source offsets differ across
line endings; executable bytes, relocations and storage placement do not.

Slice 1's [gate-off receipt](slice1-off.json) matches all twelve baseline image
hashes, routine sizes, compiler work counts and upstream selections exactly.
`--handoff` independently enables the new extension in the measurement harness;
`--enabled` continues to control the existing leaf placement feature.
