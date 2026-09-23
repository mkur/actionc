# Native 65816 BYTE and pointer comparisons implementation plan

Status: proposed on 2026-09-23 against actionc main `f96b9b76`. Planning only;
implementation has not started. This is item 3 from the Exec816 size audit and
the next user-selected code-size task. It does not activate the other
[code-size backlog](BACKLOG.md#native-65816-code-size-reduction) items.

## Objective and limits

Reduce generated code for unsigned BYTE comparisons and native three-byte
pointer equality. Select compact comparisons both when a Boolean value is
needed and when only the immediately following branch consumes the result.
Preserve physical ABI v1, image v3, the experimental o65 profile, every stack
guard, frame allocation, DP ownership and preemption contracts.

This is MIR65816 instruction selection over verified computation. Reuse the
existing state tracker, typed effects, selected actions, replay and branch-use
proof. Do not add a new optimization pass or comparison framework. Initial
selection changes are distinct from post-selection rewrites: any later removal
of a selected instruction still belongs to the checked rewrite driver.

Excluded: signed ordering, 32-bit comparison selection, pointer ordering,
address construction, constant-shift optimization, source-load folding,
nonadjacent fusion, Boolean-expression chasing, allocation/frame changes,
broader branch relaxation, guard compaction and ABI changes. In particular,
`DOSWIRE.IsPointer` compares a shifted LONGCARD; its 322-byte implementation is
not a promised win from this narrower plan.

## Evidence and current implementation

The 2026-09-23 Exec audit rebuilt Exec `8e1ff57` with its actual compiler pin
`ae1f555e77d0ae6caffbfc7453cbf06e3d098bee` and current actionc main. Both produced
identical optimized shell image contents and XEX bytes. Eight Task slots,
console enabled and `config/shell-mydos.json` were used. Existing local changes
were preserved; generated Exec version strings included its dirty marker.

| Baseline | Bytes / sites |
| --- | ---: |
| Optimized shell executable segments | 574,884 bytes |
| Raw shell executable segments | 607,177 bytes |
| Optimized shell XEX | 592,708 bytes |
| Optimized compiler routines, excluding assembly support | 567,319 bytes |
| Generic three-arm Boolean materializations | 1,714 sites / 30,852 bytes in result arms |
| `EXECLISTS.IsListEmpty`, including guard | 262 bytes |
| `SHELLAPP.ShellParse`, including guards | 7,793 bytes |
| Generic materializations inside `ShellParse` | 49 sites |

The generic-site counts include other widths and unsupported predicates. They
are an inventory, not predicted savings for this slice, and overlap branch
relaxation opportunities. Full measurements, listings and input metadata are
available locally in Exec's `build/size-audit-20260923/`; freeze the relevant
evidence in the baseline commit rather than making permanent documentation
depend on that ignored directory.

Relevant existing code:

- [`select.rs`](../src/mir65816/emit/select.rs): `word_condition` checks operands
  and the Boolean destination; `word_compare` materializes two outcomes;
  `compare_branch` combines an adjacent Compare/Branch. The generic `compare`
  emits less/greater/equal paths even for equality and BYTE inputs.
- [`liveness.rs`](../src/mir65816/emit/liveness.rs): `sole_branch_conditions`
  excludes all other uses, including edge arguments and unreachable blocks.
- [`tracked.rs`](../src/mir65816/emit/tracked.rs),
  [`selected.rs`](../src/mir65816/emit/selected.rs) and
  [`effects.rs`](../src/mir65816/emit/effects.rs): the needed A8/A16 loads, CMP,
  branches and mode changes already have typed encoding and effects.
- [`allocation.rs`](../src/mir65816/emit/allocation.rs) and
  [`scalar.rs`](../src/mir65816/emit/scalar.rs): current DP promotion does not
  admit the proposed byte/pointer-comparison routines. Leave those whitelists
  unchanged. Existing word comparison and X-loop selection remain intact.

## Selection contract

Preflight the complete comparison before allocating labels, emitting a prefix
or changing tracker state. Validate the one-byte result home even for fusion;
retain its allocation and metadata when its runtime store disappears. Invalid
IDs, home widths or displacement bounds are errors. Legal forms outside the
new selector's scope fall back without partial emission.

| Input form | Initial selection |
| --- | --- |
| Width 1, unsigned Eq/Ne/Lt/Le/Gt/Ge | Native A8 comparison |
| Width 1, signed Eq/Ne | Same bit-equality operation, if present in verified MIR |
| Width 3, Eq/Ne | Low word plus bank-byte equality; equality itself is independent of signedness |
| Existing eligible width-2 comparisons | Existing word selector, including DP and X cases |
| Signed ordering, width 4, width-3 ordering | Existing fallback |

Admit exact-width captured stack temps and parameter values, and representable
literal/null/absolute-address values. Check all physical bytes with the current
stack delta, including the third byte of a pointer. Do not silently truncate
wider operands or invent widening rules; unsupported mixed widths retain the
existing path. Resolve mutable parameters through the existing parameter-home
rules. Direct symbolic data/routine address operands retain relocation-aware
fallback; an already captured address temp can use the new selector normally.

The first slice needs no new DP-resident operands or scratch writes. Do not
relax `word_home`'s two-byte/scalar-DP contract to obtain a pointer's low word.
Instead, validate the low two-byte subrange of a checked three-byte stack home.
Keep existing word DP selection unchanged and reject unsupported homes before
emission. Do not widen the pointer-leaf allocator to admit comparisons here.

Keep condition preflight shared between materialized and fused emission, using
a small private representation for the admitted byte, word and three-byte
forms. Preserve the existing word path's selected instructions. Retain the
former fallback's operation barrier for new forms; do not introduce A8 value
forwarding or persistent flag facts as part of comparison selection.

### BYTE emission

Select `SEP #$20; LDA left; CMP right`, using immediate or stack-relative
addressing directly. No `RIGHT` scratch store is needed. Map predicates as in
the existing word selector:

| Relation | Operand order | True predicate |
| --- | --- | --- |
| Eq | left, right | BEQ |
| Ne | left, right | BNE |
| Lt | left, right | BCC |
| Ge | left, right | BCS |
| Gt | right, left | BCC |
| Le | right, left | BCS |

Swapping is restricted to captured values, not source memory accesses. Restore
A16 before either outgoing edge; `REP #$20` preserves the C/Z flags consumed by
the following branch. Never interpret A8 CMP's N/V as a signed-order result.

### Pointer equality emission

Compare the low word in A16; on mismatch the equality result is already known.
Otherwise compare the bank byte in A8 and restore A16 before transferring to an
outcome. Eq is true only if both parts match; Ne is true if either differs.
Use existing typed labels/branches and their long-transfer fallback. Do not
add raw short-branch encodings or change general layout policy.

For null equality, use each loaded part's zero flag instead of an unnecessary
CMP against zero. Apply the same two-part decision and check the bank even when
the low word is zero. This requires no OR scratch or new instruction form.

Load exactly bytes 0–1 and byte 2. No fourth-byte read and no two-byte read
starting at byte 2 are allowed. Changed order or short-circuiting of private
captured-value reads does not authorize changing the earlier observable loads.
Do not retain a two-byte value identity for a complete pointer in the tracker.
Mode and flag observations must be correct on the early-mismatch path as well
as the bank-comparison path; hidden accumulator B is not a Boolean result.

### Boolean consumers and edges

For stored, returned, passed or reused comparisons, materialize exactly one
canonical BYTE 0 or 1 through two outcome arms and store it to the checked
result home. Three less/greater/equal arms are unnecessary for these forms.
Preserve BYTE return zero-extension through the existing return emitter.

Fuse only a final Compare followed immediately by Branch on that exact
one-byte result when `sole_branch_conditions` proves there are no other uses.
Do not duplicate that use analysis or extend it to casts/logical expressions.
Skip both Boolean store and reload, retain fused source-span attribution, and
send both outcomes through the existing `edge`/`edge_last` machinery. Same-target
edges may have different arguments; neither copies nor scheduling may be skipped.
Consume comparison flags before any edge copy. Keep all frame/guard metadata
unchanged even when the Boolean home is no longer written.

Calls, runtime helpers, machine operations and volatile/aliased source accesses
stay in their original order. Nothing is assumed resident across a call, and
all 64 call-clobbered scratch bytes remain clobberable. No new DP or bank-zero
reservation is introduced: reserved-byte and per-task deltas are zero.

## Commit-sized slices

### 0. Freeze the baseline and add semantic probes

- Add focused `byte_comparisons` and `pointer_comparisons` targets under
  [`native65816-runtime-tests/tests`](../tools/native65816-runtime-tests/tests),
  plus selector preflight/use-shape tests alongside existing compare tests.
  These semantic tests must pass before selection changes.
- Compile runtime-input probes in raw and optimized modes. Cover returned,
  stored, passed and reused results separately from sole-branch comparisons;
  inspect actual MIR so promotion/constant folding cannot hide eligibility.
- Record before listings, image hashes, routine sizes, frames, guard counts,
  cycles and stack/DP traffic under a new comparison benchmark directory.
  Copy compact Exec baseline facts/provenance into tracked evidence. Retain
  binaries and large listings in ignored output directories with hashes.
- Classify reached/selected sites by width, predicate and consumer. Partition
  the 1,714-site Exec inventory before forecasting this slice's benefit.
  Set per-probe size ceilings from the frozen listings before implementation.
- Keep original small-corpus and Dijkstra snapshots immutable. Commit tests and
  baseline evidence together; production output must remain unchanged.

### 1. Native BYTE comparisons and adjacent branch consumption

- Add checked byte operands/conditions in `select.rs`, retaining all validation
  and barrier rules above. Share selection between materialized and fused paths.
- Emit native A8 predicates, two-outcome Boolean materialization and direct
  branch consumption using existing typed effects/replay. Keep word selection
  and the loop-X special case unchanged.
- Update tests that currently classify BYTE as a fallback. Keep the established
  word-window decoder word-specific; add independently checked byte-window
  evidence instead of accepting any nearby CMP as proof of successful fusion.
- Require strict local byte reductions for the representative materialized and
  fused stack-input probes, zero new DP scratch traffic, and unchanged homes,
  guards and externally visible memory traces. Measure the Exec shell delta.
- Commit after focused emitted-code, call-clobber and preemption checks pass.

### 2. Native three-byte Eq/Ne and null comparisons

- Extend the same condition selection with checked low-word/bank operands and
  both outcomes. Implement general equality and null tests for materialized and
  branch-only consumers together. Do not add pointer ordering or symbolic-CMP
  relocation forms.
- Test early mismatch, equal low words/different banks, equal banks/different
  low words and exact equality. Ensure both paths reach A16 edges correctly.
- Validate source subranges and home overlap, replay/CFG joins, result canaries,
  and the retained fallback for unsupported operands. Execute both true and
  false nonempty edges, including same-target edges and backedges.
- Require strict reduction of representative pointer probes and
  `EXECLISTS.IsListEmpty` relative to the frozen baseline; measure the separate
  incremental shell delta. Commit after focused validation passes.

### 3. Final qualification and measured results

- Run the complete affected native suite in debug and release once the code is
  stable, plus root MIR65816/CLI/o65 checks and the affected CRLF paths.
- Rebuild the small corpus, Dijkstra and Exec empty-core/console/shell images
  with the same inputs. Compare raw and optimized final executable bytes by
  routine and whole image, XEX bytes, stack/DP traffic and available VM cycles.
  Dijkstra is a regression check, not a promised signed-ordering improvement.
- Explain any changed word-path or fallback measurements. Separate relocation
  address changes from instruction changes and branch-layout side effects.
  Require net code-size reduction in the targeted probes and shell; report
  measured savings instead of extrapolating from generic-arm totals.
- Record the compiler override when building Exec; do not update its pin as part
  of this plan. Run its relevant hosted task/IRQ/console/filesystem acceptance
  before claiming hosted qualification. Compilation and compiler VM tests alone
  are not that claim.
- Update the comparison contract/results and quality-plan status; commit the
  qualification record and compact measurements. Do not mix another size
  optimization into this commit.

## Correctness and validation gates

| Area | Required cases |
| --- | --- |
| BYTE relations | All six predicates over runtime boundary pairs `0,1,$7F,$80,$FE,$FF`; both operand orders and immediate/parameter/temp forms; Eq/Ne signed-bit interpretation where admitted |
| Pointer relations | Eq/Ne with null on either side; `$000000,$000001,$00FFFF,$010000,$010001,$12FFFF,$130000,$FFFFFF`; high-bank-only and low-byte-only differences; compare invalid-to-dereference bit patterns without dereferencing them |
| Boolean consumers | Exact 0/1, BYTE-return zero extension, stored/passed/reused conditions, two branch uses, condition as either edge argument, and an intervening operation/call |
| Homes and bounds | Stack delta, final valid byte/word/pointer displacements, invalid extents/IDs, immutable fallback state, adjacent canaries, aliased source homes and distinct live edge values |
| Effects | Volatile BYTE and three-byte pointer captures, captures crossing a bank boundary, aliasing writes between captures, and direct/indirect assembly calls clobbering A/X/Y/P and all scratch |
| State and execution | Independent ca65 instruction bytes, actual final-byte decoding, A8 hidden B, C/Z truth, A16/X16 edge entry, typed replay equivalence and conservative joins |
| Async and guards | Both incoming I states; IRQ at reached load/CMP/mode-restore/decision/materialization windows for both pointer paths, seeded IRQ/NMI, full context restoration, existing stack-fault and headroom checks |
| Relocation | JSON and serialized o65 at two placements with different banks; captured data/routine addresses, nulls and retained direct-symbol fallback; short/long dispatch outcomes and final fixups |

Use host-computed expected results independent of the emitter. Batch values in
one compiled image per distinct shape; add instruction-wide IRQ sweeps only to
representative byte/pointer probes, not the entire value cross-product. Normalize
host fixture text where appropriate and exercise LF/CRLF through compilation;
preserve exact binary and memory-trace bytes.

Focused implementation commands, once the proposed test targets exist:

```sh
cargo test --lib --features native65816-state-proof mir65816::
python3 -B tools/native65816-runtime-tests/qualify.py \
  --test byte_comparisons --test pointer_comparisons --test compare_branch \
  --test word_comparisons --test state_tracking --test preemption \
  --test stack_faults --test o65
```

At the final gate, use `qualify.py` and `qualify.py --release` for the full native
suite and run the affected root targets: `mir65816_emission`, `mir65816_contract`,
`mir65816_abi`, `mir65816_state_boundary`, `mir65816_o65`, `actionc_65816_cli` and
`actionc_65816_o65_cli`. Scope local testing to MIR65816 under
[AGENTS.md](../AGENTS.md#backend-test-scope); unrelated 6502/68k suites are not
required. A shared NIR/frontend/verifier contract change is outside this plan
and would trigger the broader required checks.
