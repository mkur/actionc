# MIR6502 Materialization Gap Closure Plan

Snapshot date: 2026-06-03.

This note is a Codex-ready implementation plan for closing the current MIR6502
fixture gap reported by `target/mir6502-fixture-dumps`. It focuses on reusable
consumer-home materialization paths instead of one-off fixture patches.

Related documents:

- `docs/MIR6502_PSEUDO_MACHINE_CONTRACT.md`
- `docs/MIR6502_IMPLEMENTATION_PLAN.md`
- `docs/MIR6502_FULL_LANGUAGE_EXPANSION_PLAN.md`
- `docs/MIR6502_OBJECT_EMISSION_PLAN.md`
- `docs/MIR6502_TRACKED_EMISSION_BOUNDARY.md`
- `docs/MIR6502_ADDRESS_CONSUMER_MATERIALIZATION_PLAN.md`
- `docs/bugs/MIR6502_WORD_STORE_CONSUMER_SPILL_BUG.md`
- `docs/bugs/MIR6502_LOOP_CONSUMER_SPILLS_BUG.md`
- `docs/bugs/MIR6502_CALL_ABI_FIRST_BYTE_ARG_BUG.md`

## Current Gap Snapshot

The fixture dump summary shows:

```text
fixtures: 117
materialized MIR succeeded: 72
source listings succeeded: 68
command failures: 94
```

Representative errors include:

```text
pre-emission MIR cannot contain virtual temp
pre-emission MIR cannot contain word-width pseudo ops
pre-emission MIR cannot contain abstract bool branch conditions
computed index addresses must be materialized before emission
pointer-cell addresses must be materialized before emission
pointer-cell values must be materialized before emission
```

These failures are not random. They show that materialization is still leaving
abstract values, address forms, pointer-cell forms, and branch conditions in MIR
that is being checked as pre-emission MIR.

## North Star

Pre-emission MIR must not contain unresolved producer values that still require
semantic or target-strategy decisions.

The core invariant is:

```text
If a value has a single immediate consumer, materialize it directly into that
consumer's required home. Do not assign it an ordinary temp/spill unless a durable
home is actually needed.
```

Consumer homes include:

```text
store destination memory
return value home
call argument ABI home
zero-page pointer pair for address consumers
compare/branch flag path
indexed-element address form
runtime helper argument home
```

This is materialization infrastructure, not broad optimization.

## Red Lines

Do not close the gap with fixture-specific hacks.

Do not implement these as part of this plan:

- global constant propagation;
- dead store elimination;
- alias-sensitive load/store forwarding;
- common subexpression elimination;
- general register allocation;
- cross-block value propagation;
- automatic zero-page allocation;
- replacing pointer dereferences with direct absolute stores;
- `INC` / `DEC` peepholes as the primary fix;
- source-name lookup or SemIR recovery in MIR emission.

If a fixture needs one of those to improve code quality, defer that improvement
until pre-emission coverage is stable.

## Execution Rules For Codex

Implement this plan in small, test-gated slices.

For each milestone:

1. fix one consumer class only;
2. add or update focused fixtures;
3. run the narrowest relevant tests first;
4. run the MIR fixture dump/sweep after the narrow tests pass;
5. commit before starting the next milestone;
6. do not mix materialization infrastructure, emission support, and peepholes in
   the same commit.

Suggested checks:

```sh
cargo test -q mir6502 --lib
cargo test -q mir6502_fixtures_match_snapshots
cargo run --bin actionc-mir6502-sweep -- fixtures/mir6502
scripts/dump_mir6502_fixtures.sh
```

If a command is too broad for the inner loop, run the focused test first and the
broader sweep before committing.

## Milestone 1: Consumer-Home Materialization Helpers

Goal: add the reusable helper family that later milestones call.

Add helper functions or an equivalent module that can materialize values into
explicit consumers:

```text
materialize_value_to_a(value)
materialize_value_to_mem(value, dst_mem, width)
materialize_value_to_return_home(value, width)
materialize_value_to_call_arg(value, arg_home, width)
materialize_value_to_zp_pair(value, pair)
materialize_value_to_compare_home(value, width)
```

The exact Rust names can differ. The invariant matters: consumers request the
home they need; values are not first forced into ordinary temp/spill homes.

Initial support:

```text
constants
virtual temps already byte-split
register A
plain direct memory values
word values as low/high byte lanes
simple address values where already supported
```

Acceptance criteria:

- Existing scalar fixtures remain green.
- The helper layer has tests or call sites proving it can place byte and word
  values into a store destination without ordinary spills.
- The pre-emission verifier remains strict; this milestone should not weaken it.

Suggested commit:

```text
mir6502: add consumer home materialization helpers
```

## Milestone 2: Scalar Store Consumers

Goal: eliminate virtual temps and ordinary spills for values immediately consumed
by stores.

Implement direct store-consumer materialization for:

```text
BYTE / CHAR values
CARD values
INT values for Add/Sub/And/Or/Xor and direct moves
pointer-sized word values where they are plain word stores
casts whose result is immediately stored
word direct memory source copied to word destination
constant word source stored to word destination
```

Representative failure buckets:

```text
casts.materialized-mir.err -> virtual temps remain
word-store consumer spill cases
array element value immediately stored to scalar destination
```

Expected behavior:

```text
Store(dst, ConstWord)
  -> store low byte directly to dst+0
  -> store high byte directly to dst+1

Store(dst, LoadWord(src))
  -> load src+0 and store dst+0
  -> load src+1 and store dst+1

Store(dst, BinaryWord(Add/Sub/And/Or/Xor, ...))
  -> byte-lane operation directly into dst lanes
```

Acceptance criteria:

- `casts` no longer fails because cast-result virtual temps remain.
- Direct word stores do not create ordinary word spills.
- Store-consumer fixture output contains direct byte-lane stores to the final
  destination.
- Existing pointer/address staging fixtures remain green.

Suggested commit:

```text
mir6502: materialize scalar store consumers directly
```

## Milestone 3: Return-Value Consumers

Goal: materialize function return values directly into return homes.

Representative failures:

```text
func_returns_byte.materialized-mir.err
func_returns_word.materialized-mir.err
```

Implement:

```text
return byte constant/direct memory/temp -> byte return home
return word constant/direct memory/temp -> low/high return homes
return cast result -> return home
return arithmetic result -> return home, when already supported by store-consumer logic
```

Acceptance criteria:

- `func_returns_byte` materializes successfully.
- `func_returns_word` materializes successfully.
- No word-width pseudo op reaches pre-emission for simple word returns.
- No virtual temp reaches pre-emission for simple return values.

Suggested commit:

```text
mir6502: materialize function return values
```

## Milestone 4: Direct Call Argument Consumers

Goal: materialize direct-call arguments into ABI homes instead of placeholder
homes or virtual temps.

Representative failures:

```text
call_with_address_arg.materialized-mir.err
pass_byte_array_param.materialized-mir.err
pass_card_array_param.materialized-mir.err
pass_unsized_array_param.materialized-mir.err
builtin_putchar_byte.materialized-mir.err
builtin_print_string.materialized-mir.err
```

Start with direct calls and known builtins.

Implement:

```text
first byte arg -> A
second byte arg -> X or documented ABI home
later byte args -> documented argument area / SArgs-style homes
word args -> low/high byte ABI homes
address args -> low/high byte ABI homes
inline array arg -> base address
unsized array arg -> pointer-cell value
card/int array arg -> backing pointer when descriptor-backed
```

This milestone should reuse pointer-cell and address materialization helpers as
needed, but it can stage only the cases required by direct calls first.

Acceptance criteria:

- The first byte argument never uses `stack $0000+0` or an equivalent placeholder.
- `builtin_putchar_byte` no longer fails with a virtual temp remaining.
- Direct address argument fixtures materialize into ABI homes.
- Array parameter fixtures either materialize correctly or fail only on the next
  explicit pointer/indexed-address milestone.

Suggested commit:

```text
mir6502: materialize direct call arguments into ABI homes
```

## Milestone 5: Pointer-Cell Values And Addresses

Goal: make unsized arrays and pointer-backed shapes materialize through reusable
pointer-cell paths.

Representative failures:

```text
unsized_byte_array_const_read.materialized-mir.err -> pointer-cell addresses
pass_unsized_array_param.materialized-mir.err -> pointer-cell values
unsized_card_array_const_read.materialized-mir.err
unsized_card_array_dynamic_read.materialized-mir.err
unsized_byte_array_dynamic_read.materialized-mir.err
```

Implement two separate paths:

```text
materialize_pointer_cell_value_to_word_home(pointer_cell, dst)
materialize_pointer_cell_address_to_zp(pointer_cell, pair)
```

Use them for:

```text
unsized array value passed as argument
pointer-backed constant index read/write
pointer-backed dynamic index read/write
pointer nonzero condition, where appropriate
pointer compare, where appropriate
```

Acceptance criteria:

- Pointer-cell value errors disappear for unsized array parameter fixtures.
- Pointer-cell address errors disappear for unsized array const-read fixtures.
- Pointer dereference fixtures remain correct.
- No pointer-cell path asks emission to rediscover source-level pointer meaning.

Suggested commit:

```text
mir6502: materialize pointer-cell values and addresses
```

## Milestone 6: Constant Indexed Elements

Goal: materialize constant-index array elements as direct byte-lane memory
accesses when the backing storage is known.

Representative fixtures:

```text
byte_array_const_read
card_array_const_read
unsized_*_const_read after pointer-cell support
initialized_*_storage where constant backing references are present
```

Implement:

```text
inline byte array constant index -> direct base+offset memory
inline word array constant index -> direct base+index*2 memory
record/field constant offset path where already represented by NIR/MIR facts
word element load consumed by store -> direct byte-lane copy to destination
```

Acceptance criteria:

- Constant indexed elements no longer create index/address spills.
- `CARD ARRAY words(4); x = words(1)` materializes as direct loads from element
  low/high bytes and direct stores to `x` low/high bytes.
- No computed-index address remains for constant-index fixtures.

Suggested commit:

```text
mir6502: materialize constant indexed elements directly
```

## Milestone 7: Dynamic Indexed Elements

Goal: materialize computed indexes into the selected address strategy before
pre-emission.

Representative failures:

```text
byte_array_dynamic_read.materialized-mir.err -> computed index addresses
byte_array_dynamic_write.materialized-mir.err
char_array_dynamic_read.materialized-mir.err
card_array_dynamic_read.materialized-mir.err
card_array_dynamic_write.materialized-mir.err
large_byte_array_dynamic_read.materialized-mir.err
local_card_array_dynamic_read.materialized-mir.err
```

Implement in layers:

1. dynamic inline byte array index;
2. dynamic inline word array index with scaling;
3. pointer-backed byte array index;
4. pointer-backed word array index;
5. descriptor-backed arrays.

Materialization homes:

```text
byte index -> selected index register/home
word index -> scaled byte offset or staged element address
pointer/descriptor base -> zero-page pointer pair
final element access -> selected direct/indexed/indirect address form
```

Acceptance criteria:

- Computed index address errors disappear for inline byte dynamic fixtures.
- Descriptor-backed and local card array fixtures may be left for later sub-slices
  only if they fail with a more specific unsupported diagnostic.
- No dynamic index reaches pre-emission as an abstract computed address.

Suggested commits:

```text
mir6502: materialize dynamic inline byte indexes
mir6502: materialize dynamic word indexes
mir6502: materialize descriptor-backed array indexes
```

## Milestone 8: Compare/Branch Consumers

Goal: remove abstract bool branch conditions and compare-only virtual temps.

Representative failures:

```text
signed_int_compare.materialized-mir.err -> abstract bool branch conditions
short_circuit_and.materialized-mir.err -> virtual temp remains
short_circuit_or.materialized-mir.err
pointer_condition.materialized-mir.err
pointer_nonzero_condition.materialized-mir.err
pointer_compare.materialized-mir.err
for_loop_word.materialized-mir.err
return_from_if.materialized-mir.err
```

Implement in layers:

```text
byte Eq/Ne branch consumers
byte unsigned relational branch consumers
word Eq/Ne branch consumers
word unsigned relational branch consumers
signed INT relational branch consumers
pointer nonzero and pointer compare consumers
short-circuit CFG/condition materialization
```

Rules:

- Do not materialize a bool byte unless the bool value is used as a value.
- If the compare feeds only a branch, materialize to flags/control flow.
- Preserve signedness semantics for INT relational operations.

Acceptance criteria:

- No abstract bool condition reaches pre-emission for simple byte branches.
- `signed_int_compare` either materializes correctly or fails only on a specific
  signed-compare unsupported diagnostic before pre-emission.
- Short-circuit fixtures no longer leave virtual bool temps in pre-emission MIR.

Suggested commits:

```text
mir6502: materialize byte compare branch consumers
mir6502: materialize word compare branch consumers
mir6502: materialize signed int branch consumers
mir6502: materialize short-circuit branch conditions
```

## Milestone 9: Initialized Storage And Static Payload Consumers

Goal: make initialized aggregate/static data self-contained before emission.

Representative failures:

```text
initialized_byte_array.materialized-mir.err
initialized_byte_array_storage.materialized-mir.err
initialized_card_array_layout.materialized-mir.err
initialized_unsized_card_array_storage.materialized-mir.err
local_initialized_array_storage.materialized-mir.err
```

Implement:

```text
initialized byte array payloads as authoritative bytes
initialized word array payloads as authoritative low/high bytes
descriptor and backing storage records
unsized initialized array pointer cells
local initialized array backing records
zero-fill records
```

Rules:

- Do not reconstruct payload bytes from display text during emission.
- Descriptor/backing relationships must be printable and verifiable.
- Zero-fill should be explicit in MIR storage/layout records.

Acceptance criteria:

- Initialized byte-array fixtures materialize without virtual temps.
- Descriptor-backed initialized card-array fixtures either materialize or fail on
  one specific unsupported descriptor/backing case.
- Storage layout remains visible in materialized MIR dumps.

Suggested commit:

```text
mir6502: materialize initialized storage payloads
```

## Milestone 10: Builtins, Runtime, OS, Machine Blocks

Goal: route non-user-call targets through the same value/call/effect machinery.

Representative failures:

```text
builtin_putchar_byte
builtin_print_string
os_call_opaque_barrier
machine_block_label_ref
machine_block_global_ref
```

Implement:

```text
builtin target mapping
builtin argument materialization via ABI homes
runtime helper target resolution where needed
OS opaque call effects
structured machine-block payload emission/materialization
```

Rules:

- Builtins should not lower through source-name lookup in emission.
- OS calls are opaque barriers unless precise effects exist.
- Machine blocks need structured payload/effect facts or precise unsupported
  diagnostics.

Acceptance criteria:

- Builtin byte/string fixtures no longer fail due to virtual temps.
- OS call fixture preserves opaque effects.
- Machine block fixtures either emit structured payloads or fail before emission
  with precise unsupported diagnostics.

Suggested commit:

```text
mir6502: materialize builtin and opaque call targets
```

## Milestone 11: Indirect Calls And Callable Values

Goal: support typed callable values after direct call ABI homes are stable.

Representative failures:

```text
indirect_proc_call.materialized-mir.err
indirect_func_call_byte.materialized-mir.err
indirect_func_call_word.materialized-mir.err
callable_param_forwarding.materialized-mir.err
```

Implement:

```text
routine address values
callable variable value materialization
indirect call target homes
indirect call ABI planning
callable parameter forwarding
```

Rules:

- Indirect calls must be typed before MIR materialization.
- Do not recover signatures from SemIR or source names during emission.
- Effects should remain conservative unless known.

Acceptance criteria:

- Indirect call fixtures no longer leave callable temps in pre-emission MIR.
- Callable parameter forwarding uses explicit ABI/callable homes.
- Direct call ABI regressions remain green.

Suggested commit:

```text
mir6502: materialize indirect call targets
```

## Milestone 12: Re-run Fixture Dump And Re-bucket

Goal: verify progress and avoid continuing with stale assumptions.

After each two or three milestones, run:

```sh
scripts/dump_mir6502_fixtures.sh
```

Then summarize:

```text
fixtures total
materialized MIR successes
source listing successes
remaining errors grouped by diagnostic text
```

If a bucket still has many failures, add or update a bug note under `docs/bugs/`.

Acceptance criteria:

- The remaining failures become more specific after each milestone.
- Generic `virtual temp remains` failures decrease significantly.
- No new source-listing failures appear for fixtures that previously emitted.

Suggested commit:

```text
mir6502: refresh fixture materialization gap snapshot
```

## Suggested First Codex Task

```text
Implement MIR6502 materialization gap closure Milestone 1 and the smallest part of
Milestone 2.

Goal:
- Add consumer-home materialization helpers for byte/word values.
- Use them for simple Store consumers only.
- Cover constants, direct memory values, cast results, and simple word low/high
  lanes.
- Add regressions for casts and direct byte/word store consumers.
- Do not implement dynamic arrays, pointer-cell arrays, signed compares, indirect
  calls, initialized aggregate payloads, or peepholes.

Acceptance:
- casts.materialized-mir.err no longer reports virtual temps for simple casts.
- simple byte/word store-consumer fixtures materialize without ordinary temp
  spills.
- existing scalar, pointer, direct-call, and object-emission fixtures remain green.

Suggested commit:
- mir6502: materialize scalar store consumers directly
```
