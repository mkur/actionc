# Returned record addresses

Slice 0 establishes the conservative baseline at `9624da9a`. Compiler and
runtime input hashes match the authenticated call-flow stage-6 images in all
three profiles. [Baseline](baseline.json) preserves complete `MYDOSFILE.Chain`
bytes, MIR, source spans, homes, resources and artifact identities.

| Profile | Bytes | Instructions | Frame/spills/local peak |
| --- | ---: | ---: | --- |
| Optimized release | 50 | 29 | 8/8/8 |
| Optimized guarded | 72 | 39 | 8/8/8 |
| Raw guarded | 72 | 39 | 8/8/8 |

The independent `address_returns` native target passes with the conservative
compiler. It covers direct fields, zero-index chains and nonzero CARD array
elements, low-word carry and bank wrap, poisoned A/X/Y, both I states, exact
three-byte inputs, adjacent canaries and absence of pointee accesses. All four
raw/optimized × guarded/release configurations compile LF and CRLF source to
identical images. Its pinned-VM qualification manifest is embedded in the
baseline. Existing full-workload compiler-cost measurements remain the
call-flow stage-6 reference; its numerical application gates remain unchanged.

Reproduce the fixture with:

```sh
CARGO_INCREMENTAL=0 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 \
  python3 -B tools/native65816-runtime-tests/qualify.py --test address_returns -j2
```

[Implementation plan](../../MIR65816_ADDRESS_RETURN_PLAN.md).
