# Union representation and copy costs

Measured during union release acceptance on 2026-09-09, using the pinned
actionc-vm revision `7ec0cc454ebf43b088b7bcd11515533085ea1964`.

Reproduce from the repository root:

```sh
cargo test --manifest-path tools/vm-runtime-tests/Cargo.toml --locked --test unions_codegen_audit -- --nocapture
```

The test prints CSV measurements, verifies raw and optimized NIR, and executes
every case against an independent guarded-memory oracle for seeds 0, 7, 127 and
255. Origin is `$3000`. Cycles include entry through the final completion-marker
store, excluding the idle loop. XEX bytes include load-file framing and required
runtime material; these are complete microprogram costs, not per-copy timings.
Cartridge and standalone measurements agree in these runtime-independent cases.

| Case (both forms and runtimes) | Backend | XEX bytes | Cycles | Optimized copy sites | Capture bytes |
| --- | --- | ---: | ---: | ---: | ---: |
| Direct typed views / explicit RAM aliases | classic | 49 | 42 | 0 | 0 |
| Direct typed views / explicit RAM aliases | MIR6502 | 45 | 38 | 0 | 0 |
| 33-byte union / ordinary record snapshot and copy | classic | 271 | 4,830 | 2 | 33 |
| 33-byte union / ordinary record snapshot and copy | MIR6502 | 580 | 16,859 | 2 | 33 |

Direct access writes a word, overwrites its first byte with a runtime seed, and
reads the word back. The union also contains a third byte which must stay
untouched. Both raw and optimized NIR have exactly four source-requested stores
(including result and completion), no copies, no comparisons and no calls.

The copy case initializes 33 bytes, captures an immutable LET, mutates the
source, and copies the snapshot to another address. The manual control is an
ordinary record with identical inline bytes and the same copy/snapshot semantics.
Both forms retain two CopyBytes sites and a 33-byte capture. This is deliberately
not compared with an alias, which would have different value semantics.

Both forms have equal measured sizes and cycles within each backend/runtime.
No union tag checks, implicit clears or runtime helper appear. The classic/MIR
copy gap belongs to existing aggregate lowering; it is not union overhead and
does not justify a union-specific optimization pass. The separate ADT copy/
initialization optimization backlog remains outside this delivery.
