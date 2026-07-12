# AltirraBridge Notes

AltirraBridge is installed outside this repository, under the shared
workspace tools directory:

```sh
../tools/AltirraBridge-nightly-macos-arm64
```

Important files:

- `../tools/AltirraBridge-nightly-macos-arm64/AltirraBridgeServer`
- `../tools/AltirraBridge-nightly-macos-arm64/docs/PROTOCOL.md`
- `../tools/AltirraBridge-nightly-macos-arm64/docs/COMMANDS.md`
- `../tools/AltirraBridge-nightly-macos-arm64/sdk/python/altirra_bridge`
- `../tools/AltirraBridge-nightly-macos-arm64/skills/altirra-bridge`

The bridge is a line-oriented JSON protocol over a local socket. The
server writes a token file on startup; clients read the bound address
and token from that file, send `HELLO`, then issue one command at a
time. The Python SDK is pure stdlib and can be used by adding its
directory to `PYTHONPATH`.

## Starting The Server

Use the headless server for compiler probes:

```sh
../tools/AltirraBridge-nightly-macos-arm64/AltirraBridgeServer \
  --bridge=tcp:127.0.0.1:0 \
  --no-basic \
  --machine=800XL \
  --memory=64K
```

The server logs lines like:

```text
[Bridge] listening on tcp:127.0.0.1:56166
[Bridge] token-file: /var/.../altirra-bridge-46644.token
[Bridge] log-file: /var/.../altirra-bridge-46644.log
```

The sandbox may reject the loopback bind. If that happens, rerun the
server command with escalation. This is expected for bridge work.

The server reads ROM/profile settings from:

```text
~/.config/altirra/settings.ini
```

The smoke-tested install reported Atari XL/XE OS ver.2 and accepted
`--no-basic --machine=800XL --memory=64K`.

## Minimal Python Client

```sh
PYTHONPATH=../tools/AltirraBridge-nightly-macos-arm64/sdk/python \
python3 - <<'PY'
from altirra_bridge import AltirraBridge

token_file = "/var/.../altirra-bridge-46644.token"
with AltirraBridge.from_token_file(token_file) as a:
    print(a.ping())
    print(a.regs())
    print(hex(a.peek16(0xfffc)))
    print(a.quit())
PY
```

This was smoke-tested successfully:

- `PING` returned `{"ok": true}`.
- `REGS` returned CPU state.
- `PEEK16 $FFFC` returned the reset vector.
- `QUIT` cleanly shut down `AltirraBridgeServer`.

## Useful SDK Calls

The Python client exposes the commands we need for compiler testing:

- lifecycle: `ping`, `pause`, `resume`, `frame`, `quit`
- load/reset: `boot`, `mount`, `cold_reset`, `warm_reset`
- state: `regs`, `peek`, `peek16`, `memdump`, `hwstate`
- hardware: `antic`, `gtia`, `pokey`, `pia`, `dlist`, `pmg`
- input: `key`, `joy`, `consol`
- rendering: `screenshot`, `rawscreen`, `render_frame`
- debugger: `disasm`, `history`, `eval_expr`, `callstack`, `memmap`
- break/watch: `bp_set`, `bp_clear`, `bp_clear_all`, `bp_list`, `watch_set`
- symbols/profiling: `sym_load`, `sym_resolve`, `sym_lookup`,
  `profile_start`, `profile_stop`, `profile_dump`
- config: `config`, including `config("debugbrkrun", "true")`

`frame(N)` is the deterministic timing primitive. The command response
is gated by the emulator main loop; the next command waits until the
requested frames complete and the emulator is paused again.

## Actionc Probe Pattern

Compile a load file normally:

```sh
cargo run -q --bin actionc -- \
  --backend mir6502 \
  --output target/pmg_bridge_mir.com \
  samples/toolkit/modern/PMG.DM1
```

Then boot and inspect through the bridge:

```python
from altirra_bridge import AltirraBridge

with AltirraBridge.from_token_file(token_file) as a:
    a.config("debugbrkrun", "true")
    a.boot("target/pmg_bridge_mir.com")
    # Prefer small staged frame counts or RUNAD breakpoints for load files.
    a.frame(1)
    print(a.regs())
    print("PM mode/base:", a.peek(0x3000, 3).hex())
    print("HiMem:", hex(a.peek16(0x02e5)))
    print("AppMHi:", hex(a.peek16(0x000e)))
```

For generated Action load files, do not start with a blind `frame(300)`.
A quick experiment showed:

- `BOOT target/pmg_bridge_mir.com` was accepted and dispatched.
- The client then hung during `FRAME 300`.
- The bridge log confirmed the last processed command was `FRAME 300`.

Use `debugbrkrun`, PC breakpoints, or smaller staged frame counts first.
Once the loader/run-address behavior is understood, longer frame gates
are fine.

## PMG Findings From The First Probe

The PMG demo was useful for separating VM/headless-object behavior from
real emulator behavior.

Default-origin legacy object, run in the existing in-repo VM without
artificial memory pokes:

- `EndProg = $38F6`
- `AppMHi = $38F6`
- initial VM `HiMem = $9C1F`
- `PMGraphics(2)` computed `PM_BaseAdr = $9400`
- `PM_BaseAdr < AppMHi` was false
- `PM_Mode` became `2`
- repeated `PMClear`/`PMCreate`/`PMMove` calls did not touch `$344B`

An earlier low-memory reproduction deliberately poked `HiMem` down
around `$3980`; that forced `PM_BaseAdr` to `$3400`, made
`PM_BaseAdr < AppMHi` true, left `PM_Mode = 0`, and later made
`PMClear(4)` underflow `PM_BSize(PM_Mode)-1` to `$FFFF`. That explains
the low-memory failure mode, but it is not evidence for the default
case.

For MIR6502, the generated PMG listing also patched `EndProg`
correctly:

- object ended around `$3A33`
- `EndProg` storage contained `$3A33`
- `main` copied it into `$0E/$0F` before `PMGraphics(2)`

So if an emulator run sees `PM_BaseAdr < AppMHi` with plenty of memory,
inspect runtime `HiMem` at `$02E5/$02E6` first.

## Operational Notes

- The token file contains both connection address and auth token.
- The persistent log is next to the token file and is safe to inspect;
  the token itself is not written there.
- `peek` and `memdump` are debugger-safe reads. Reading I/O addresses
  does not trigger hardware side effects.
- `watch_set(addr, mode="rw")` returns breakpoint ids and can be used
  to stop on unexpected writes such as `$344B`.
- `history(N)` and `callstack(N)` are the first things to read after a
  breakpoint/watchpoint hit.
- Use `screenshot()` or `rawscreen()` instead of macOS screen capture;
  they return emulator pixels through the bridge without inspecting the
  host desktop.
