# MIR68K Amiga emulator validation

## 2026-09-14: AmigaOS 3.1 smoke run passed

The user ran the prepared Action! executables in vAmiga and exported the
**ActionC** disk. The exported executable, script, expected-output and manifest
bytes match the corrected bundle in `build/amiga-smoke-fixed/`. The host verifier
passed: all four redirected outputs match exactly, every numeric return code is
correct, execution succeeds after the deliberate fault, and the report ends with
`SMOKE PASS`.

| Evidence | Value |
| --- | --- |
| Compiler source revision | `9599469b6d2fde2dd2aac816fe4bd064a3356bed` |
| Bundle run ID | `1e31f01be81041888c91d783458b8a6b` |
| Exported ADF SHA-256 | `b9ef6c54989aba65451f3ff1a759f47fdfcb6d153afcb1fc08c6d35ebfa93090` |
| Exec report | `exec.library 40.10` |
| DOS report | `dos.library 40.3` |
| User-described machine | A600 configuration |
| Locally inspected vAmiga | 4.5, build 260807 |
| Script command stack | 65536 bytes |

The reports identify AmigaOS 3.1 libraries, not an AROS acceptance run. They do
not capture the active CPU/RAM settings or full Kickstart ROM revision. The
successful returned ADF is preserved locally as
`build/mir68k-amiga/acceptance-passed.adf`; OS disks and ROMs are not checked in.

All four program-output files match the committed expectations byte
for byte, including LF line endings and the final newline:

```text
Hello from Action! on the Amiga.
BYTE: 255
CARD: 65535
INT: -32768
```

```text
values: -6 11 14 7
sum: 26
```

```text
2 3 4 5 6 7 8 9 10 11
checksum: 65
check: 0
elements: 1
```

The deliberate division fault produces:

```text
before fault
Action! DivisionByZero
```

The complete status report is:

```text
BUILD 1e31f01be81041888c91d783458b8a6b
RC hello.console 0
RC hello.redirect 0
RC integer-array.console 0
RC integer-array.redirect 0
RC insertsort.console 0
RC insertsort.redirect 0
RC division-zero.console 20
RC division-zero.redirect 20
RC hello.recovery 0
SMOKE PASS
```

The verifier completed successfully with:

```sh
python3 tools/amiga_smoke.py verify --bundle build/amiga-smoke-fixed \
  build/amiga-returned-4fyjz_xv/ActionC/results
```

```text
AmigaOS smoke output/status checks passed for 9599469b6d2fde2dd2aac816fe4bd064a3356bed
```

This verifies the minimal Shell executable milestone on the recorded AmigaOS
configuration. It does not establish compatibility with older OS versions or
AROS, or independently record the console's visible text; the four captured
outputs are from redirected launches.

## Smoke-script correction

The first run exposed a script variable-name bug: unbraced Amiga Shell references
only consume letters and digits. `$actionc_rc` attempts to expand `$actionc` and
leaves `_rc` literal; an undefined reference remains unchanged. The generator
now uses the alphanumeric `actioncrc`, keeps capture immediately after each
executable, and prints failures visibly as well as recording them. The tooling
regression applies the documented expansion grammar to the generated logging
and conditional commands for return codes 0, 5 and 20. It rejects the original
script. See [RKRM AmigaDOS, section 15.1.5](https://developer.amigaos3.net/sites/default/files/downloads/2024-10/Amiga_ROM_Kernel_Reference_Manual_DOS.pdf).

The successful run used the corrected disk `build/actionc-smoke-fixed.adf`.
All four executable files are unchanged from revision `9599469`; only the script,
run ID and corresponding manifest hash changed. The rejected first export is
preserved locally as `build/mir68k-amiga/acceptance-initial.adf`.
