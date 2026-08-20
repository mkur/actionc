# SemIR Bridge Sweep

`actionc-semir-sweep` compares the AST-backed classic codegen path with the
SemIR bridge path. It is a bridge-fidelity tool: for every source that the AST
path can compile, the SemIR bridge should produce byte-identical output.

Run the default sweep:

```sh
cargo run --bin actionc-semir-sweep --
```

The default roots are:

- `surveys/probes/original-compiler`
- `fixtures/stress`
- `corpora/toolkit/original/extracted`

Run selected files or directories, choose a profile or origin, or emit a
markdown report:

```sh
cargo run --bin actionc-semir-sweep -- fixtures/semir
cargo run --bin actionc-semir-sweep -- --profile modern fixtures/stress
cargo run --bin actionc-semir-sweep -- --origin '$4000' fixtures/semir
cargo run --bin actionc-semir-sweep -- --report markdown fixtures/semir
```

Result classes:

- `MATCH`: AST and SemIR bridge output match exactly.
- `MISMATCH`: both paths compiled, but their output differs. Treat this as a
  bridge bug until proven otherwise.
- `SEMFAIL`: AST codegen succeeded, but SemIR bridge codegen failed or panicked.
- `ASTFAIL`: the AST path failed too, so there is no successful reference output.
- `LOADFAIL`: parsing, include expansion, or semantic analysis failed before
  codegen.

The tool exits nonzero for `MISMATCH` or `SEMFAIL`. `ASTFAIL` and `LOADFAIL` are
reported but do not fail the sweep because no successful AST result exists to
compare.

## SemIR Shape Fixtures

The bridge sweep proves output equivalence. Snapshot fixtures in
`fixtures/semir` separately prove that the SemIR text shape stays stable for
key language constructs:

```sh
cargo test --test semir_fixtures
```

Refresh an intentional fixture change with:

```sh
cargo run --bin actionc-emit -- --emit-semir fixtures/semir/name.act > fixtures/semir/name.semir
```
