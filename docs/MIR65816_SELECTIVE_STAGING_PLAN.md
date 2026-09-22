# Native 65816 selective staging implementation plan

Status: implemented in `8af3541` and [qualified](MIR65816_SELECTIVE_STAGING.md)
on 2026-09-22. The original plan below was frozen against main `d62f0e8`, whose
qualified compiler implementation was `49a0aae`; its forecasts remain unchanged.
The [frozen baseline and forecasts](benchmarks/65816-selective-staging/baseline.json)
use the [compact-staging results](MIR65816_STAGING_RESERVATIONS.md).

## Objective and scope

For eligible cyclic word edges, capture only sources that an earlier destination
assignment would overwrite. Capture those values before any destination write,
then perform every assignment in original MIR argument order. Preserve the
existing single-word and acyclic schedules, including their final A/N/Z reload
when required. Apply the same strategy to raw and optimized MIR.

Keep public ABI v1, image v3, o65 profile v1, temp/object homes, call conventions,
stack guard logic, interrupt reserves and Exec816's compiler pin. Change only
private edge-copy selection and its required staging capacity. No self-copy
elimination, home coalescing, source deduplication, scratch reuse within an edge,
X/Y/DP residence, new forwarding or NIR optimization belongs to this slice.

The shared planner now owns actual staging requirements. Extend it with a compact
capture-to-slot mapping in the same behavioral slice, so selective emission does
not reintroduce unused reservations. This is a change to MIR65816 allocation and
emission contracts, not a new public MIR form or serialized image field.

## Rechecked baseline and forecast

Planning verified all 224 saved artifact hashes across 56 builds, the identical
264 debug/release records, and all 420 qualified compiler/fixture input hashes.
The baseline is `target/staging-reservations-after`; historical snapshots stay
immutable. The earlier [copy inventory](MIR65816_COPY_INVENTORY.md) describes the
dependency rule, but its PCs, reservations and cumulative forecasts predate the
acyclic and compact-staging slices. Do not rerun its historical exporter on main.

Only the optimized `loop_rotation` backedge changes in the current corpus. Its
private stack word assignments are `$06 → $0A`, `$0A → $06`, `$0C → $08`.
The second source must be captured before the first destination write. The
counter assignment needs no staging. All six input vectors execute this edge
eight times; initialization remains the existing direct acyclic sequence.

| `loop_rotation(13)`, optimized | Qualified baseline | Forecast |
| --- | ---: | ---: |
| Code bytes | 148 | 140 |
| Cycles | 1,116 | 956 |
| Instructions | 262 | 230 |
| Stack-byte reads | 175 | 143 |
| Stack-byte writes | 172 | 140 |
| Reserved staging bytes | 6 | 2 |
| Fixed frame / observed peak | 20 | 16 |
| Spill bytes | 16 | 12 |
| Incoming argument body displacement | 24 | 20 |

The one retained capture uses pool slot 0 at S+$0E. Temporary homes end at
S+$0D, so the two-byte staging word ends at S+$0F and the aligned extent is 16.
The incoming ABI offset remains zero; only its displacement from body S changes.
Guard reservation/fault-report and teardown immediates must use the new extent.

With A16 already established, the predicted copy window is:

```asm
; Capture original $0A before any assignment.
LDA $0A,S
STA $0E,S
; Original assignment order and final accumulator value.
LDA $06,S
STA $0A,S
LDA $0E,S
STA $06,S
LDA $0C,S
STA $08,S
JML loop_header
```

The copy window starts at $010068. The transfer moves from $010080 to $010078
and still targets $010045. Following code and later routines move by eight bytes;
finalization must remap every affected position and reference.

These are forecasts, not measurements of an implemented compiler. Across six
vectors, per incoming I state, expect 48 selected edge executions and 96 avoided
staging pairs: 192 fewer instructions, 960 cycles, 192 stack-byte reads and 192
writes. The static saving is eight bytes once, not eight per vector. Expect all
27 other Action corpus builds and all vbcc artifacts/results to remain unchanged.
In particular, `sum_loop(13)` stays 120 bytes / 1,212 cycles / 12 stack bytes.

## Shared planning and correctness

### Eligibility and strategy representation

Work in [copies.rs](../src/mir65816/emit/copies.rs). Replace the overloaded
`WordCopies.order: Option<_>` decision with an explicit private strategy that
separates direct schedules, selective staging and complete staging. Keep checked
source/destination operands separate from logical capture indices and resolved
physical scratch displacements. An absent capture must never become a sentinel
stack address or an implicit request for the same-index argument slot.

Selection precedence is:

1. Preserve current single-word behavior, including its safe own-source overlap.
2. Preserve the current acyclic scheduler and its exact tie-breaking/reload policy.
3. Admit selective staging only when destinations are disjoint words and every
   source/destination overlap is a complete word, with a proven cyclic dependency.
4. Retain complete word staging for unproved/partial overlaps. Retain bytewise
   staging for mixed widths and unsupported word operands. Malformed targets,
   arities, widths and homes remain errors, not optimization fallbacks.

`acyclic_word_order` currently returns `None` for both cycles and bad overlap
geometry. Distinguish these cases explicitly; failure to find a direct order is
not by itself permission to use selective staging. Continue complete preflight
before any mode prefix, capture, assignment or transfer is emitted. Check both
accessed bytes after transient S movement, authoritative mutable parameter homes,
all required captures and the target label. An unsupported earlier source must
not conceal a malformed later operand.

### Capture rule

For assignments `D[i] <- S[i]` in original order, define:

```text
capture(i) = S[i] is a stack word
             and there exists j < i with bytes(D[j]) intersecting bytes(S[i])
```

For the eligible geometry, any such overlap is an exact word match. Immediates
never require capture. Enumerate captured move indices in increasing order.
First load and save each captured source into a distinct scratch word; then emit
all destination assignments in original order, loading from scratch only for a
captured move. Every self-copy and dead destination still loads and stores.

Before the capture phase, sources have their original values. Scratch writes are
disjoint from every source, destination and live home. During assignment i, a
captured source is its saved original value. An uncaptured source cannot have
been changed by an earlier destination; the complete load also precedes its own
store. Each assignment therefore matches a simultaneous-copy reference model.
Repeated at-risk sources get separate captures in this slice. Multiple cycles
and both rotation directions are covered: retaining destination order can require
several captures for a single cycle, so do not assume one scratch word per cycle.

Planning checked this rule against a simultaneous-copy model on 8,400 input
combinations: one to four assignments, five stack sources, two immediates and
three initial word patterns. Destination values and final A matched in every
case. This abstract check is not emitted-machine-code qualification.

The last assignment loads the same original final source as complete staging.
It establishes identical full A and N/Z without an extra reload. LDA/STA preserve
C/V and X/Y/S/D/DBR/I; existing mode handling still establishes A16/X16. There is
no helper or ordinary call inside an edge transfer. Calls before or after it keep
the current barriers and invocation-owned homes. Source-language memory accesses,
volatile ordering and pointer alias behavior do not change.

### Compact allocation and validation

In [allocation.rs](../src/mir65816/emit/allocation.rs), `edge_copies[k]` becomes
pool slot k in capture order, rather than necessarily argument k. A selective
edge with captured moves `[1, 4]` requests two word slots and maps them to pool
indices `[0, 1]`. Full word/byte staging still captures every argument, so its
mapping remains identical to the current argument-index mapping.

Each edge supplies a dense list of requested capture widths. Direct edges supply
none; selective word edges supply one width-2 entry per capture; complete staging
supplies every argument width. Reserve the maximum width per pool index across
all edges, including unreachable blocks and both branch arms. Do not deduplicate
captures or overlap their lifetimes within an edge. Alignment and separation from
frame objects and all temp homes remain unchanged.

Preserve the current two-stage allocation proof: plan using the smallest frame
containing all private homes, then reserve scratch and independently recheck the
final plans, count, widths, mappings, byte ranges and frame/peak accounting.
Immutable incoming arguments remain above the entire frame as it grows; mutable
ones use fixed object homes. Revalidate every incoming last byte after the final
extent changes. Neither provisional unused slots nor removed reservations may
cause a false overflow or weaken the 254-byte fixed-frame limit.

Resolve logical pool indices to checked displacements in
[select.rs](../src/mir65816/emit/select.rs) before emission. Allocation, verification
and selection must consume the same strategy/capture contract. The verifier must
reject missing, duplicate, out-of-range or undersized capture mappings and
collisions with source/destination/live storage. Keep the existing typed transfer,
fallthrough, branch finalization and relocation machinery.

## Independent evidence and measurement changes

The test [word-edge window](../tools/native65816-runtime-tests/tests/support/word_edge.rs)
currently distinguishes only fully direct and fully staged execution. Give it an
explicit selective form and per-move capture information. Add a test-only selective
edge index, grounded in verified MIR edge identity, physical homes and typed
`mir_transfers`, with routine-relative capture/copy/transfer ranges.

Validate captures and assignments from final instruction bytes using a separate
byte-memory snapshot oracle. Do not use the compiler's chosen capture mask or
scheduler as the expected answer. Capture mapping may be checked against the
allocated pool, but correctness and necessity must come from original typed
source/destination ranges. Reject missing captures, late captures, wrong saved
sources, reuse of a live scratch word, unexpected operand changes, wrong targets
and truncated windows.

Update [multi-word indexing](../tools/native65816-runtime-tests/tests/support/multi_word_edge.rs):
it currently tries complete staging and direct scheduling, then rejects unknown
encodings. Account for selective windows explicitly. Suppress interior suffixes
of all indexed windows so a selective assignment tail cannot count as another
edge. Count once at the first LDA, never at REP or a later source reload. Preserve
independent decoding and coverage of the complete-staging fallback.

Keep existing `word_edges`/`edge_words` totals and single-word, acyclic, fusion and
forwarding metrics unchanged in meaning. Add separate Action-only metrics:
`selective_word_edges`, `selective_edge_words`, `selective_staged_words`,
`selective_direct_words` and `selective_word_edge_sites`. For each rotation vector
these should be 8, 24, 8, 16 and one site with count 8. Other corpus records have
zero new selective counts; vbcc records keep their existing schema.

For the corpus, require reconstructed compiler images to match saved serialized
artifacts before extracting proof identities; execute the saved bytes. Carry
routine-relative evidence through o65 relocation. Update exact traces in
`word_edges.rs`, `o65.rs`, `preemption.rs` and related support consumers. Check the
whole stack outside predicted writes; do not seed canaries in removed slots.

Add a dedicated comparison checker under `tools/compare65816` using the frozen
baseline. Check complete instruction streams and images: selective capture/copy
reordering, the two removed staging pairs, compact scratch operands, changed
frame/argument/guard operands and required label/branch/PER/relocation remapping.
Also cover later uncounted driver routines whose addresses move. A loose size or
cycle ceiling is insufficient. Every existing measurement field must either
match or have an independently declared delta; normalized private-stack traces
must account for the changed body S, while external-memory traces stay exact.

## Regression and qualification matrix

| Area | Required evidence |
| --- | --- |
| Planner | Swaps; three-/longer cycles in both directions; multiple cycles plus independent/immediate moves; self/repeated sources; a captured final move; unchanged reordered acyclic schedules and final-A reloads. Exhaustive small graphs checked against simultaneous byte copies. |
| Preflight/fallback | Both incoming and mutable parameters; A8/A16/unknown entry knowledge; partial overlaps and unsupported source forms retain fallback; mixed byte/word/three-/four-byte edges; missing targets/homes/captures, wrong widths and transient-offset overflow fail before emission. |
| Allocation | Dense capture mapping across differently shaped edges, per-slot mixed-width maxima, captured move index beyond pool length, corrupt mappings/overlap, alignment padding and incoming last-byte limits. Former high-pressure cyclic cases may now fit; preserve genuinely overflowing cases and the exact 254-byte boundary. |
| Machine execution | Independent ca65 selective and fully staged sequences with boundary word values, full A/status, exact cycles and bus accesses. Goto, fallthrough, backedges, ordinary/fused branch arms and same-target/different-argument edges. Construct verifier-clean cyclic MIR for both frontend modes: raw corpus alone has no nonempty edges. |
| Guards and ABI | Correct new prologue/teardown and mutable-parameter copy displacements; exact floor, one-byte-short failure before writes, ceiling/wrap errors, outgoing-call checks and local/whole-call bounds. Public argument/result placement, all live homes and interrupt reserve remain. |
| Effects and preemption | Live values around helper and direct/indirect scratch clobbers; aliased, volatile and bank-crossing memory. IRQ at every newly selected capture/assignment boundary in both task domains; seeded IRQ/NMI; full CPU restoration while A, flags and scratch snapshots are live. |
| Relocation and proof tools | Both branch arms and cycles at the existing two o65 placements; typed fixups, PER, spans and trace boundaries after shortening. Reject forged/truncated/suffix windows and stale capture/target metadata; exercise long and short dispatch boundaries. |

Keep all existing independent and fallback coverage. Update an old staging-size
assertion only after independently deriving the new requirement. In particular,
a large rotation that formerly failed due to full staging may now be legal;
replace its negative case with one whose real capture requirements exceed the
limit. The existing emission-boundary fixture should remain byte-identical:
its affected sum loop is already direct. Review any difference instead of
regenerating snapshots blindly.

## Implementation sequence and commits

1. **Freeze evidence.** Use this plan's baseline hashes and forecasts. Add semantic
   probes and independent ca65 references that also run against the old fully
   staged implementation; inventory all window/index consumers. Commit the
   baseline/probes without changing emitted code. Reconstruct missing baseline
   artifacts from `49a0aae` in an isolated checkout, with new provenance rather
   than overwriting frozen records.
2. **Implement the vertical slice.** Add explicit strategy/capture planning,
   compact pool requirements, verifier checks and selective emission together.
   Update typed evidence, execution traces and focused tests in the same slice.
   Existing direct/acyclic schedules remain; changed incoming displacements must
   still be reflected in their operands. Update the
   [emission contract](MIR65816_EMISSION_CONTRACT.md) and
   [allocation contract](MIR65816_TEMPORARY_ALLOCATION.md). Commit after focused
   selection, allocation, execution, guard, relocation and interruption checks.
3. **Qualify and publish measurements.** Run the complete native suite in both
   hosts and the full raw/optimized comparison. Save the strict delta, listings,
   source/tool/artifact hashes and qualification record; update the
   [quality plan](MIR65816_CODE_QUALITY_PLAN.md). Commit qualification separately.
   Preserve all unrelated local files throughout.

Affected root checks:

```sh
cargo test --lib mir65816 --features native65816-state-proof
cargo test --test mir65816_state_boundary --test mir65816_abi \
  --test mir65816_contract --test mir65816_emission --test mir65816_o65 \
  --test actionc_65816_cli --test actionc_65816_o65_cli
```

Finish with the qualified native VM, comparison-tool tests and corpus build:

```sh
CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 \
  python3 -B tools/native65816-runtime-tests/qualify.py
python3 -B tools/native65816-runtime-tests/qualify.py --release
python3 -B -m unittest discover -s tools/compare65816 -p 'test_*.py'
cargo build --release --bin actionc-65816
python3 -B tools/compare65816/build.py --output target/selective-staging-after --verify-crlf
```

Run both external `code_quality` host commands from the
[comparison workflow](../tools/compare65816/README.md), using the new directory's
manifest and separate debug/release results. Preserve all 264 records and paired
I-state checks (528 executions per host). The optimized vbcc `unlink` vector-0
failure must remain explicit; it is not permission to ignore additional failures.
Save new results under `docs/benchmarks/65816-selective-staging/after`, without
rewriting historical data. Finish comparison-tool changes before the corpus build
so recorded input hashes remain valid. Verify LF/CRLF through the actual fixture
paths, rebuilding affected embedded-fixture tests in an isolated CRLF checkout.

No semantic/NIR boundary change is planned, so a repository-wide NIR sweep or full
root suite is not required unless implementation crosses those contracts. Check
formatting only on edited files, documentation links and staged diffs. Completion
requires the forecast to hold (or an independently justified revised baseline),
all Action cases correct, matching host artifacts, preserved ABI/guards and
unchanged unrelated local work. Board and Exec816 integration remain separate.
