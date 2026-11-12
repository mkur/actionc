# Word expression stores through same-width moves

The targeted MIR6502 fix lets the existing expression-store consumer retain
word Add/Sub provenance through bit-preserving word moves. In checked TN,
`CARD POINTER(captured+request)` can now write its result directly into
`allocp` at `$E8/$E9`, eliminating the `$E2/$E3` result staging.

`Alloc` shrinks **87 → 79 bytes**. Successful `Alloc(1)` takes **207 → 195
cycles**, including the same 17-cycle wrapper. The original request save,
validated cursor, exhaustion checks, return value and fault behavior remain.

| Program | Runtime | Previous XEX | Current XEX |
| --- | --- | ---: | ---: |
| TN | cart | 10,298 | 10,290 |
| TN | standalone | 10,423 | 10,415 |
| TNDBG | cart | 13,393 | 13,385 |
| TNDBG | standalone | 13,518 | 13,510 |

`Alloc` is the only routine whose size changes. These are additional savings
against `71325d37`, after the home-value invalidation work. The library sources
are unchanged. [current.json](current.json) binds source digests, before/after
image hashes and sizes, final Alloc listings, per-routine changes and VM cycles.
The generator verifies its frozen baseline source receipt against that commit.
Historical benchmark files are not refreshed.

The added permission is deliberately bounded. A complete fixed zero-page
word destination must be disjoint from all pure input reads. Exact fixed RAM
ranges and declared ordinary scalar frame roots above the main origin provide
the proof. Estimated object addresses do not. Overlaps, unknown placement,
local alias views, initialized frame views, hardware/indirect accesses,
register inputs, width changes and escaping results retain their existing
paths. See the [MIR6502 contract](../../MIR6502_PSEUDO_MACHINE_CONTRACT.md).

A naive expression-copy change failed the overlap control: writing a result
low byte over an input high byte changed `0 + 1` into `$0101`. The final guard
rejects that transformation. The [source regression](../../../fixtures/runtime/word_move_store.act)
covers Add/Sub, chained casts, both high-byte overlaps, an in-place destination
and captured values across a call.

Validation:

- Five focused expression/driver tests, including width changes, multiple uses,
  escaped definitions, origins inside zero page, unsupported views and stability.
- Full MIR6502 library suite: **1,266 passed**, 12 ignored.
- All 25 MIR6502/checked-arena integration targets: **113 passed**, 9 ignored.
- New source VM controls: **4,592 calls** across raw/optimized NIR, both runtimes,
  two origins and actual LF/CRLF source files; carries, borrows and wrap covered.
- Seven affected VM targets: **15 tests passed**, one historical measurement
  ignored. These include the existing **1,120** checked-arena calls and TN UI
  dispatch/panel controls. Fresh measured images also pass **44 Alloc calls**.
- MIR6502 emission sweep: **169 passed**.
- Eight explicitly selected MADS tests: **64 assemblies**. Four real images
  also reassemble with LF/CRLF at `$2C00` and relocate to match fresh direct
  output at `$3100`: **16 assemblies**.
- The new generator's `--vm --check` passes, including fresh cycle receipts.

Reproduction from the repository root:

```sh
CARGO_INCREMENTAL=0 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_DEV_OPT_LEVEL=1 cargo build --bin actionc
python3 tools/compare6502/tn/word_move_store.py --compiler target/debug/actionc --vm --check
cargo test --test mir6502_word_move_store
cargo test --release --manifest-path tools/vm-runtime-tests/Cargo.toml --test word_move_store
cargo test --test mir6502_word_move_store mads -- --ignored
```

The measurement command checks the saved receipt; omit `--check` only when
intentionally refreshing this benchmark. Without `--vm`, a check may reuse
cycles only if the complete fresh source/image receipt still matches. Host
compiler executable hashes are provenance and are excluded from deterministic
comparison. Binary images remain byte-exact; host text hashing normalizes CRLF.
