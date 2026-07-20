# MIR6502 peephole liveness audit

Date: 2026-07-20

Baseline: `82763b4` (`mir6502: retain live staged word homes`), with the current
uncommitted public-ABI worktree changes present.

Scope: the MIR6502 materialization, producer/consumer, structural peephole,
SSA-lite, and spill-cleanup pipeline used by the modern profile. The classic
backend and final emitter/branch relaxation are outside this audit.

## Executive result

The liveness framework itself is not the immediate problem. It models whole
temps, byte lanes, terminator uses, and CFG live-in/live-out sets, and the passes
that use it are generally sound. The problem is adoption: several older
producer/consumer and structural rewrites still use block-suffix scans, or no
post-window check at all.

The audit found two systemic correctness gaps and one separate machine-state
proof gap:

1. **Pre-home temp rewrites do not receive CFG live-out.** `materialize_ops`
   receives the block terminator but not `MirTempLiveSet`. After
   `lower_block_arguments`, a value can be defined in a predecessor and used
   directly in a successor, so checking only later operations and the
   terminator is insufficient.
2. **Post-home structural rewrites remove spill/zero-page stores without a
   successor-safe memory proof.** The narrow staged-word fix covers one such
   rewrite, but the same omission remains in staged byte/word, indirect, word
   array, and indexed-base rewrites.
3. **A small number of rewrites change register/flag or fixed pointer-pair
   state without an explicit tail-liveness proof.** The clearest example is
   deleting a same-register parameter reload, which also deletes the reload's
   N/Z flag update.

These are proof gaps, not evidence that every firing is a miscompile. A firing
is unsafe only when an eliminated value or machine state is observed outside
the matched window. The staged-word regression demonstrated that this situation
does occur in real code, so the remaining gaps should be treated as correctness
work rather than optional optimization hardening.

## Liveness domains

The pipeline needs four related, but distinct, safety checks.

| Domain | Relevant phase | Required proof before removal |
| --- | --- | --- |
| Logical temp or temp lane | Before `materialize_temp_ops` | No suffix use, no terminator use, and no CFG live-out use. Removing a whole temp is blocked by either live byte lane. |
| Materialized private home | After `materialize_temp_ops` | No read before overwrite on any remaining path. The existing conservative rule may instead reject every successor block unless the home is overwritten locally first. |
| Register or flags | Late/materialized MIR | No read before overwrite/clobber, including flag-test terminators and ABI-observable return/register state. |
| Address-consumer pair | Late/materialized MIR | The fixed/virtual zero-page pointer pair is not reused before being rematerialized, including on successor paths. |

`terminator_uses_temp` is necessary but not sufficient after block arguments
are lowered. `lower_block_arguments` converts edge arguments into predecessor
operations and clears block parameters. A target temp can therefore be defined
in one block and used directly in another without appearing in the predecessor
terminator. That is exactly what `analyze_temp_liveness` is designed to expose.

## Pipeline audit

| Pipeline stage | Status | Audit result |
| --- | --- | --- |
| Initial temp cleanup before block-argument lowering | Conditional pass | It checks the current terminator, including edge arguments. This is safe only while verifier-clean input requires all cross-block values to travel through block arguments. That invariant should be explicit. |
| Block-argument lowering | Pass | Parallel copies preserve edge values. It deliberately creates predecessor definitions whose target temps can be live into successors. Later passes must therefore use routine live-out. |
| Pre-branch compare producer folding and compare narrowing | **Gap** | Checks block suffix and terminator, but not CFG live-out. |
| Unique word-load address forwarding | Pass | Computes routine temp liveness and checks suffix, terminator, live-out, use count, and source stability. This is the reference implementation for pre-home forwarding. |
| Main `materialize_ops` producer/consumer rewrites | **Systemic gap** | The API has no live-out argument. Many subpasses either check only `def_is_used_after`/`temp_is_used_after`, or perform no dead-result check. |
| Pre-home copy propagation and temp cleanup fixed point | Pass | Recomputes routine liveness on every round and checks terminator plus exact/full live-out requirements. |
| Temp-home materialization and indirect-load spill folding | Pass | The spill fold receives precomputed live-out and retains stores for full-temp or exact-lane successor uses. |
| Structural peepholes | **Mixed; systemic home gap** | Register/flag cleanup and three scratch-removal rewrites are guarded. Several older staging/indirect rewrites remove private homes without the same guard. |
| Indexed base-pointer staging | **Gap** | Proves source stability and scans later operations in the block, but has no terminator/successor-home check. |
| Terminator materialization | Pass for this audit | Converts remaining logical terminator state after structural lowering. It cannot recover a temp/home removed earlier. |
| SSA-lite local and single-predecessor forwarding | Pass | Tracks memory facts conservatively, treats calls/barriers as kills, checks reload flag observability, skips joins and backedges for cross-block seeding. |
| Dead spill-store removal | Pass | Uses CFG-recursive read-before-write analysis with conservative handling of unknown effects. |
| Basic-block/routine spill coloring | Separate allocator concern | Routine coloring has CFG liveness/interference. This audit did not find a peephole-style omitted suffix check here; allocator pairing and interference invariants should remain covered by their own tests. |
| Final-layout structural/SSA/dead-store rerun | Inherits earlier status | The same structural gaps run a second time after final layout. |

## Detailed findings

### P0: pre-home temp-definition elision lacks CFG live-out

The central defect is the `materialize_ops` interface. It operates one block at
a time and receives `&MirTerminator`, but not that block's routine liveness.
Consequently, even transforms that correctly reject a terminator use can remove
a definition needed in a successor.

Affected families:

- `compare_branch.rs`
  - `fold_compare_operand_producers_before_branches` /
    `collect_compare_operand_plan`;
  - `narrow_byte_bitwise_zero_compares`;
  - `try_fuse_byte_binary_compare_consumer`;
  - `try_fuse_byte_compare_consumer`, including the one-load form with no
    post-use check and the two-load form with only a suffix check.
- `calls.rs`
  - `fold_call_arg_producers`;
  - `try_materialize_call_arg_expr_producers`;
  - `forward_return_slot_call_result_args` (terminator-aware, but not
    live-out-aware);
  - `try_forward_param_word_store_consumer`;
  - `try_fuse_call_result_store_consumer` and
    `try_fuse_loaded_arg_call_result_store_consumer`.
- `store_consumers.rs`
  - address, cast, direct-copy, word-store, byte-store, and store-expression
    producer/consumer folds;
  - several subforms have a block-suffix check but no terminator/live-out
    check; several adjacent loaded/cast/binary forms have neither;
  - `collect_store_expr_plan` receives the terminator, but currently uses it
    for flag observability only;
  - the inc/dec forms have detailed register/flag tail checks, but still need
    the eliminated temp definitions to be dead at routine scope.
- `pointers.rs`
  - `rematerialize_direct_pointer_temp_derefs`;
  - `try_fuse_pointer_temp_deref`.
- `indexes.rs`
  - delayed byte-index producer suppression;
  - indexed byte/word copy;
  - dynamic inline byte indexing;
  - dynamic byte/word index preparation, including LEA-based forms.
- `materialize.rs` / `temps.rs`
  - `try_fuse_address_store_consumer`;
  - `is_unused_lea_addr`, whose name describes only block-local use and which
    checks neither the terminator nor live-out.

This category should be fixed centrally. Adding isolated
`terminator_uses_temp` calls would still leave successor uses unprotected.

### P0: post-home scratch-store removal is inconsistently guarded

`private_scratch_store_removal_is_safe_after` is the current conservative
reference rule. It allows removal when the home is overwritten before any read;
otherwise it requires a terminal block and no later read. The staged-word fix
now applies it to both removed word lanes.

The following structural rewrites remove one or more `MirMem::Spill` or
`MirMem::ZeroPage` definitions without applying the equivalent proof:

- `next_style_word_store_forward_at` removes `staged_lo` and `staged_hi`;
- `staged_byte_word_update_at` and
  `forwarded_staged_byte_word_update_at` remove combinations of `value_slot`,
  low/high operand slots, and low/high result slots;
- `indirect_byte_compound_at`;
- direct and forwarded indirect byte compound forms;
- immediate and delayed indirect byte constant compound forms;
- `indirect_byte_const_store_at`;
- `indirect_y_const_store_at`;
- indirect byte direct-store forms;
- `word_array_store_value_staging_at`;
- `fold_indexed_base_pointer_staging` removes the low/high staging loads and
  stores after checking only same-block uses.

The already-guarded structural rewrites are:

- `staged_word_store_forward_at`;
- staged compare RHS and staged binary RHS folding;
- dead private scratch-store removal;
- immediate register spill forwarding and complete spill store/reload removal;
- indirect-load spill consumer folding, which uses logical temp live-out.

The immediate safety repair is to pass the terminator into every structural
subpass that removes a private-home store and apply the existing conservative
helper to every removed home. A more precise later version can use CFG home
liveness to keep valid folds in nonterminal blocks.

### P0/P1: fixed address-consumer state is not always preserved

Several indirect structural rewrites replace a matched fixed pointer pair with
the canonical `$AC/$AD` pair. Their matched memory effect is equivalent, but
they do not prove that the original pair is dead after the window. A later
`LoadIndirect`, `StoreIndirect`, `AdvanceAddress`, or
`IndirectByteCompound` may reuse the original pair before another
materialization, including because `materialize_temp_ops` suppresses redundant
address staging.

This overlaps the private-home issue but deserves an explicit invariant:

- either retain the original `MirAddressConsumer` in the replacement;
- or prove both bytes of the old pair dead before overwrite on all paths.

The memory helpers already know how an address consumer reads its pair, so a
home-liveness implementation should reuse that representation rather than add
special cases for `$AC`/`$AD`.

### P1: same-register parameter reload deletion loses N/Z production

`forward_param_reload` returns `None` when the known source register is already
the requested destination register. That deletes the load entirely. The value
is preserved, but a 6502 load also establishes N/Z, while deleting it preserves
whatever flags happened to exist before the load.

The cross-register replacement uses a register transfer and normally
re-establishes N/Z, so the same-register case is the primary concern. It should
be allowed only when flags are dead until overwritten, or replaced with an
operation that intentionally refreshes the required flags. Add branch-on-N/Z
and fall-through tests.

### P1: machine-state equivalence is distributed rather than declarative

The safer late rewrites already have good local helpers:

- `tail_does_not_read_reg` for registers;
- `binary_flags_may_be_live_after` and `terminator_consumes_flags` for flags;
- `tail_allows_inc_dec_update_after` for the combined A/carry/overflow case;
- call/barrier effect checks in SSA-lite.

However, each rewrite decides independently which final A/X/Y/C/Z/N/V state
must match. Long structural replacements should declare their changed machine
state and prove every changed component dead. This is particularly valuable for
the direct byte-to-word update pseudos and pointer-pair canonicalization, even
where no concrete bug was established by this audit.

## TN exposure

The current `target/tn-liveness-fix/TN.peepholes` report shows that the affected
families are active in TN:

| Reported transformation | TN count | Audit status |
| --- | ---: | --- |
| compare operand consumer before branch | 112 | Terminator-aware, missing live-out |
| delayed byte-index producer suppression | 53 | Missing terminator/live-out |
| byte-store consumer | 75 | Mixed local checks, missing routine proof |
| direct-copy store consumer | 41 | Suffix-only proof |
| call-result store consumer | 34 | Suffix-only proof |
| word-store consumer | 28 | Mixed/no suffix checks |
| call-argument expression consumer | 21 | Suffix-only proof |
| address-store consumer | 11 | No dead-result proof |
| indexed byte/word copy | 1 / 1 | No dead-result proof |
| store-expression consumer | 1 | Terminator used for flags, not temp liveness |
| staged byte/word update | 3 | Missing successor-home proof |
| word-array value staging | 3 | Missing successor-home proof |
| indirect byte compound / const compound / direct store | 1 / 2 / 1 | Missing home and pointer-pair proof |
| staged word-store forwarding | 1 | Guarded by `82763b4` |

The same report records eight gross cross-block home-demand lanes and one
retained cross-block lane after home planning. Those counts do not identify a
specific bad firing, but they confirm that cross-block value lifetimes exist in
TN and cannot be dismissed as an unreachable MIR shape.

The aggregate counts are execution counts of optimizer sites, not unique source
expressions, and some families contain both safe and unsafe subforms. They are
useful for prioritization, not for claiming that TN is currently miscompiled at
every listed site.

## Existing test coverage

Strong successor-liveness tests currently cover:

- exact/full temp-lane liveness;
- pre-home copy propagation with successor uses;
- indirect spill-load consumers with successor uses;
- dead private scratch stores;
- staged RHS homes;
- spill store/reload removal;
- the newly fixed staged word-store forwarding case.

Most producer/consumer tests demonstrate the optimized output shape but do not
add a second use in the terminator or successor. The structural indirect,
staged byte/word, word-array, call-result, call-argument, pointer, index, cast,
and copy families need negative liveness fixtures.

Each producer-removing family should have, at minimum:

1. an ordinary forwardable case;
2. a later-use case in the same block;
3. a `BoolValue` terminator use where the temp type permits it;
4. a successor-only exact-lane use;
5. a successor-only full-temp use for word definitions.

Each private-home-removing family should have:

1. a later read of every eliminated home;
2. a successor read of every eliminated home;
3. a local overwrite-before-read case that remains optimizable;
4. an unknown call or machine-block effect barrier;
5. a later reuse of every changed address-consumer pair.

## Recommended implementation order

### Slice 1: centralize pre-home deadness

1. Compute routine temp liveness immediately after block-argument lowering for
   pre-branch compare folding.
2. Recompute it after compare expansion/CFG cleanup and before the per-block
   `materialize_ops` loop.
3. Pass each block's `MirTempLiveSet` into `materialize_ops` and its
   producer/consumer subpasses.
4. Add one lane-aware helper equivalent to:

   ```text
   temp_dead_after(ops, end, terminator, live_out, temp)
       = no suffix use
      && no terminator use
      && neither full-temp nor exact-lane live-out
   ```

5. Route every temp-definition-eliding rewrite through that helper. If a
   rewrite removes several producers, prove each one independently.
6. Add successor-use tests first, then retain existing positive shape tests to
   measure lost optimization opportunities.

Do not fix this category with terminator-only checks.

### Slice 2: complete conservative private-home protection

1. Pass `&MirTerminator` through staged byte/word, word-temp, indirect,
   word-array, and indexed-base structural subpasses.
2. Enumerate the stores removed by each successful match.
3. Require `private_scratch_store_removal_is_safe_after` for each eliminated
   spill/zero-page byte.
4. Preserve the matched address consumer where possible; otherwise check the
   old pair with the same read-before-overwrite rule.
5. Add per-lane later-read and successor tests.

This slice is deliberately conservative and may reduce peephole counts in
nonterminal blocks. Correctness should precede recovering those sites.

### Slice 3: add precise materialized-home CFG liveness

1. Generalize the dead-spill read-before-write analysis to query whether a
   specific `MirMem` byte is live after an operation.
2. Include `Spill`, virtual zero-page, and fixed address-consumer pair bytes.
3. Use exact memory effects and address-consumer reads; stop at calls, barriers,
   and machine blocks unless structured effects prove safety.
4. Replace the conservative “any successor blocks removal” rule with this query.
5. Restore profitable nonterminal structural folds and record blocked/applied
   counters.

### Slice 4: register/flag proof cleanup

1. Give `forward_param_register_homes` terminator/tail context.
2. Guard same-register reload deletion with N/Z liveness.
3. Define a compact machine-state effect summary for structural rewrites:
   reads, writes, and required final A/X/Y/C/Z/N/V plus address-consumer pairs.
4. Migrate the longest rewrites first and add flag-branch tests.

### Slice 5: enforcement and observability

1. Add debug assertions or a test helper that compares removed definitions with
   suffix, terminator, and live-out uses.
2. Require new producer-removing helpers to accept a liveness context rather
   than raw `ops` alone.
3. Add counters for `blocked-terminator-live`, `blocked-successor-live`,
   `blocked-home-live`, and `blocked-pointer-pair-live`.
4. Rebuild TN and compare the final listing, peephole counts, and emulator smoke
   behavior after every slice.

## Bottom line

The audit does not recommend replacing the current liveness analysis. It
recommends making liveness context mandatory at the two transformation
boundaries where definitions are erased:

```text
pre-home rewrite   -> temp suffix + terminator + CFG live-out
post-home rewrite  -> memory read-before-overwrite on all CFG paths
```

The highest-value next change is Slice 1 because it closes one systemic API gap
covering most active TN producer/consumer fusions. Slice 2 should follow before
adding more structural peepholes, because it completes the same safety repair
already applied narrowly to staged word-store forwarding.
