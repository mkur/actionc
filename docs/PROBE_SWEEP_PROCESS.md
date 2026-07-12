# Probe Sweep Process

The original-compiler probe sweep is the quick compatibility gate for `actionc`.
It recompiles every probe that has a captured original Action! compiler output
and compares the resulting Atari load files.

## When To Run

Run the sweep after changes that can affect generated code, storage layout,
calling convention, expression evaluation, or load-file output.

Good moments:

- before and after codegen refactors
- after adding a compatibility peephole
- after changing ABI or runtime helper handling
- before committing a compatibility-sensitive change

## Standard Command

From the `actionc` repo root:

```sh
surveys/probes/original-compiler/sweep.sh
```

The script regenerates:

```text
surveys/probes/original-compiler/outputs/actionc/*.com
```

and compares those files with VM-captured original compiler outputs:

```text
surveys/probes/original-compiler/outputs/vm/*.COM
```

## Refresh Listings Too

Use this when you want the checked-in analysis artifacts updated as well:

```sh
surveys/probes/original-compiler/sweep.sh --update-artifacts
```

This also refreshes matching `.hex` and `.lst` files under
`outputs/actionc/`.

## Cargo Test Form

The same compatibility gate is available as an ignored integration test:

```sh
cargo test --test compatibility -- --ignored
```

This is useful before broad changes or when running a fuller local verification
pass.

## Reading Results

The sweep prints one line per probe:

- `OK`: byte-for-byte match with the original compiler capture
- `ALLOW`: documented accepted divergence
- `NOTE`: a previously accepted divergence now matches exactly
- `FAIL`: unexpected mismatch or missing input

At the end it prints:

```text
Probe sweep summary: exact=N accepted=N unexpected=N
```

The script exits nonzero when `unexpected` is not zero.

## Handling Failures

For an unexpected failure:

1. Check the first byte diffs printed by the sweep.
2. Regenerate a listing for the probe:

   ```sh
   cargo run --quiet --bin actionc-emit -- --emit-source-listing surveys/probes/original-compiler/<probe>.act
   ```

3. Compare against the original capture and any notes in
   `surveys/probes/original-compiler/outputs/vm/`.
4. If the difference is intentional and sane, document it in
   `surveys/probes/original-compiler/VM_PROBE_RESULTS.md` and add it to the
   sweep allowlist.
5. If it is a regression, fix the compiler and rerun the sweep.

Keep accepted divergences rare and explicit. The north star is compatibility
for valid Action! programs without reproducing clear original compiler bugs.

## Large Program Stability

The byte-exact probe sweep is intentionally narrow. TN is the large real-program
stability sentinel for changes that can shift layout, storage allocation,
include handling, broad codegen shape, or accumulated code size.

```sh
surveys/tn/check-stability.sh
```

The same check is wired into the ignored Cargo compatibility suite:

```sh
cargo test --test compatibility -- --ignored
```

TN is deliberately budget-based rather than byte-exact. A failure means the
compiler drifted enough that the generated load-file size should be investigated
before continuing with more compatibility or optimization work.
