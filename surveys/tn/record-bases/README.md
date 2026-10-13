# Record base reuse measurements

Stage 0 establishes generic record sequences, typed MIR before/after target
materialization, final listings and emitted execution checks. The
[implementation plan](../record-base-reuse-plan.md) defines the proof and cost
boundaries. [stage0.json](stage0.json) pins the fixture, compiler, reporter and
image hashes; full artifacts are retained under `target/record-bases/stage0`.

The generic fixture is 699 load-file bytes in cartridge and standalone modes.
`WriteFields` is 86 instruction bytes, `KnownFields` 69, `RebindParameter` 64
and `ReadCurrent` 46. Actual LF/CRLF source compilations produce identical
images, typed MIR and payloads. The VM target checks ordinary and page-crossing
record accesses, retained neighbor bytes and a pointer referring to its own
parameter cell. The latter changes the pointer through its first field write;
the subsequent field write must follow the changed pointer.

The [TN audit](../size-audit-2026-10-13/README.md) supplies 28 repeated setups:
27 after indirect stores and one after direct local captures. Typed MIR retains
the source cells and complete field displacements, but temp expansion turns
most preparations into byte loads/stores. The final post-materialization
boundary retains typed byte operands and provides the actual emitted storage
placement. This is the selection boundary for the new proofs. A provisional
materialization layout is insufficient when home elimination, coloring or
deferred storage changes final addresses.

Record types and ordinary Action ABI parameter classification supply no
universal disjointness proof. The self-cell execution case demonstrates why
unknown parameter pointers must retain fallback. Caller-only bounds also need
an explicit complete entry/caller contract; private parameter homes do not
establish that contract.

Reproduce the generic census with a supplied compiler and reporter:

```sh
CARGO_TARGET_DIR=target/record-bases/build CARGO_INCREMENTAL=0 \
  CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_DEV_OPT_LEVEL=1 \
  cargo build --bin actionc --example record_base_reuse_report -j2
python3 surveys/tn/measure-record-bases.py \
  --compiler target/record-bases/build/debug/actionc \
  --reporter target/record-bases/build/debug/examples/record_base_reuse_report \
  --out-dir target/record-bases/recheck
```

The focused execution target is `record_base_reuse` in
`tools/vm-runtime-tests`. Compiler production output is unchanged in Stage 0.

Stage 1 adds complete base proofs, byte dependencies and wrapping write
footprints. The optional emission proof map resolves stable storage IDs through
the converged final layout; ordinary output does not build the map. Unit cases
cover physical aliases, partial pair clobbers, unknown indirect destinations,
proven literal destinations, stale generations and forged proof fields.
Production selection remains unchanged in this slice.

Stage 2 supplies opt-in read-sequence selection through the shared rewrite
driver. It keeps the original high-byte load to preserve A/Z/N and any known
callee exit value already used by caller rewrites. The generic read sentinel
now publishes its captured first word to a different global before reading
the next field: the earlier fixture was already optimized by existing passes.
[stage2.json](stage2.json) compares this revised source with the feature off/on.
`ReadCurrent` changes from 62 to 55 code bytes in both runtimes; the complete
reporter payload changes from 703 to 696. Source homes and zero-page occupancy
do not grow. Raw/optimized NIR execution and LF/CRLF compilation preserve the
self-cell mutation fallback and field/neighbor checks.

Stage 3 admits BYTE stores and the existing direct-to-indirect WORD copy only
with a complete destination inside compiler-owned data. Numeric addresses into
code or foreign RAM retain fallback, as do incomplete WORD footprints. The
generic `KnownWord` case places a variable WORD store between field operations;
all RHS staging and low/high store order remain. VM checks execute both known
store cases with raw/optimized NIR, both runtimes and LF/CRLF sources. Incoming
record parameters and the self-cell mutation case remain generic.

Stage 4 qualifies parameter origins established by complete assignments inside
the current block. `RebindParameter` overwrites its incoming pointer with the
address of owned data; its field sequence can then retain the base. Entry
parameters still start unknown, and a partial assignment does not bound the
other byte. MIR's Action ABI does not certify closed entry: external/machine
entry can supply a pointer into a callee's own homes. Consequently caller-only
origins remain unavailable, including the TN header parameters. This is the
proved subset allowed by Stage 4; a future closed-entry contract must pass
through verified NIR before a backend may consume it.
[stage4.json](stage4.json) records `RebindParameter` at 64 → 50 code bytes
and complete reporter payloads at 780 → 738 bytes, with unchanged homes and
zero-page occupancy. Both runtimes compile actual LF/CRLF sources identically;
emitted execution checks also cover raw and optimized NIR.

Stage 5 enables the qualified subset in optimized MIR6502. Selection runs after
completed inlining and home selection; feature-off and feature-on therefore use
the same baseline decisions. The selector retains the original high-byte load,
checks identical frames and occupied zero page, and requires a strict final
image saving without growth in selected routine code or homes. Final data
relocation rebuilds each proof at its original snapshot sites, rejecting any
new alias. Absolute-backed named reads and hardware RHS reads end the region,
including reads hidden behind local or parameter IDs. Private spill identity
alone does not certify RAM placement. The initial owned-data window is
`$0400–$7FFF`, plus ordinary compiler-private zero-page carriers for reads.

The current generic source has 780 feature-off payload bytes and 738 with all
three selection flags enabled. `ReadCurrent` changes from 62 to 55 code bytes,
`KnownFields` 69 to 55, `KnownWord` 66 to 59 and `RebindParameter` 64 to 50.
The read sentinel's emitted call vector changes from 119 to 109 cycles and from
four to two writes to `$AA–$AF`, in both runtimes. Its four pointee reads remain
ordered and unchanged. These cycles include the common call harness; scratch
writes are a separate metric from total private-memory accesses.

The production scorecard uses complete cartridge images:

| Image | Before XEX bytes | After XEX bytes | Saving |
| --- | ---: | ---: | ---: |
| TN, MIR6502 | 16,617 | 16,610 | 7 |
| TNDBG, MIR6502 | 20,354 | 20,333 | 21 |
| TN, classic optimized | 18,076 | 18,076 | 0 |
| TNDBG, classic optimized | 21,563 | 21,563 | 0 |

`ActivateLocation` changes from 68 to 61 code bytes in both roots. TNDBG also
changes `Path` from 281 to 274 and `DrawWinFrame` from 423 to 416. TN now has
131 full setups and 27 listing-detected repeats; TNDBG has 150 and 27. Every
remaining detected repeat follows an indirect write. The typed parameter
origins for these writes remain unknown; `BatchBegin`, `TagsReset`, `TagsInit`
and `TagsExtent` retain their existing budgets. The original 280-byte candidate
budget is therefore not a qualified saving. Header-store reuse across routine
entry still needs a verified closed-entry/caller contract.

The final VM matrix checks feature controls, raw/optimized NIR, cartridge and
standalone runtimes, actual LF/CRLF inputs, source-cell mutation, captured
bounded bases, nested RHS preparations, BYTE/WORD store order and neighbor
bytes. Additional field windows cover a BYTE at offset 255, WORDs at
254/255/256, page crossing and a ROM-free read across `$FFFF/$0000`. No SemIR,
NIR, printer or shared compiler contract changes are involved.

Qualification passes 1,082 MIR6502 unit tests (two existing ignores), all 96
MIR6502 integrations and all 169 emission fixtures. Cargo builds the isolated
runtime harness; all 91 test executables pass, totaling 405 tests. Final runtime
execution used four independent workers with two test threads per executable,
covering the complete target set rather than a filtered subset.

The maintained [final.json](final.json) pins the qualified binaries, relevant compiler
sources, fixture sources, final images and scope of checks. Complete artifacts
are retained under `target/record-bases/qualified`, including typed MIR controls
and actual LF/CRLF production images. Reporter payloads and unpruned typed root
budgets are diagnostic; the production table above comes from CLI XEX files.
Blocker reports include mandatory first setups and repeated proof attempts;
the 27 remaining listing repeats are the separate candidate census.
The [remaining setup census](remaining-setups.csv) keeps those application
candidates separate from classic listing controls and mandatory typed setups.

Reproduce the final scorecard with the retained compiler and reporter:

```sh
python3 surveys/tn/measure-record-bases.py \
  --compiler target/record-bases/qualified/actionc-qualified \
  --reporter target/record-bases/qualified/reporter-qualified \
  --out-dir target/record-bases/recheck --controls --roots --jobs 4 \
  --baseline target/record-bases/stage0
```

The final selector does not add a calling-convention assumption, narrow an
unknown write effect, or extend reuse across calls, joins or loops. Subsequent
caller/entry contracts and CFG-wide reuse remain separately scoped work.
