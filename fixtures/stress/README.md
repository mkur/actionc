# Stress Fixtures

Artificial Action! programs for compiler torture testing. These sources are
broader than the focused fixtures under `fixtures/mir6502`: they combine
pointer aliasing, arrays, calls, control flow, records, runtime helpers, and
storage layout in the same program.

The `.act` files in this directory are maintained test inputs. Generated
objects, original-compiler captures, status reports, and comparison workflows
live under [`surveys/stress`](../../surveys/stress/README.md).

Run the compile-only regression gate with:

```sh
scripts/check-stress-fixtures.sh
```

Compile an individual fixture with, for example:

```sh
cargo run --bin actionc-emit -- --emit-listing fixtures/stress/pointers.act
cargo run --bin actionc-emit -- --emit-load fixtures/stress/pointers.act \
  > target/pointers-stress.com
```

The stress suite intentionally includes `zero_page.act`, which exercises
`actionc`'s zero-page pointer-storage policy and is reported as an expected
failure by the compile-only gate.
