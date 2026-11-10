# TN C reference port

Implementation follows the [translation plan](../../../docs/TN_C_OSCAR64_TRANSLATION_PLAN.md).
Slices 0–3 are qualified. Remaining UI/command integration follows.

The target is Atari native 6502 with Oscar64 pinned by
[the build driver](../../../tools/compare6502/tn/build.py). Bootstrap tools and
freeze the Action! references with:

```sh
python3 tools/compare6502/tn/build.py bootstrap
python3 tools/compare6502/tn/build.py freeze
python3 tools/compare6502/tn/build.py probe
python3 tools/compare6502/tn/build.py platform
python3 tools/compare6502/tn/build.py kernels
python3 tools/compare6502/tn/literals.py --check
python3 tools/compare6502/tn/build.py panels
cargo test --locked --manifest-path tools/vm-runtime-tests/Cargo.toml --features tn-c-reference --test tn_c_reference
```

The external compiler test is enabled explicitly through `tn-c-reference` and
requires built artifacts; it never silently skips absent inputs. Generated
binaries and listings stay under `build/tn-c-oscar64/`. Source and compact
qualification evidence are tracked.

Oscar64's standard XEX RUNAD is `$02E0`. The pinned VM convenience loader instead
recognizes Action!'s `$02E2` INITAD. The C adapter loads the original image and
explicitly starts its real RUNAD, without patching the executable or either
compiler. This difference is part of the loader adapter, not code-generation cost.

`tn_target.h` retains TN's fixed zero-page cells and restricts automatic global
placement to `$EA..$FD`. Its volatile compiler temporary bank is 16 rather than
32 bytes; the saved bank begins at `$B5`. The startup probe checks the source-cell
sentinels. Larger routines still require a complete temporary-footprint audit.
The current RAM/stack region is a provisional upper bound; final copy-buffer
placement is qualified with the application, not inferred from this probe.

The platform probe executes inherited conversion/copy/keyboard algorithms, CIO
argument and filename adapters, byte carry, arena push/pull and terminal transfer.
Mocked OS vectors inject eight nested nonlocal errors per execution; both CPU
and software stack return to their original checkpoint. The explicit volatile
recovery flag avoids relying on the pinned compiler retaining a varying setjmp
return. The ordinary source error/UI behavior is integrated in later slices.

`TN_DIAGNOSTIC` preserves callable kernel entries for controlled measurements.
Production builds leave normal reachability and inlining enabled. The
[kernel scorecard](../../../surveys/tn/c-oscar64/kernels.json) reports callee
bytes/cycles including nested calls, excluding caller argument placement. It
is not a whole-image comparison; ABI and C automatic-storage differences remain
part of these diagnostic controls.
