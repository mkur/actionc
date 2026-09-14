# Amiga Shell executables

The experimental MC68000 backend can produce one relocatable Amiga HUNK
executable with integer arithmetic, control flow, calls, arrays and console
output. This path uses modern Action! semantics and MIR68K.

```sh
cargo build --locked --bin actionc
target/debug/actionc --target motorola-68000 --runtime amiga \
  -o build/hello.amiga samples/amiga/hello.act
```

The default output name is `<source-stem>.amiga`. Copy the executable to the
Amiga; it needs no symbol file or payload companions. The extension is optional.
From an Amiga Shell:

```text
Stack 65536
hello.amiga
Echo $RC
```

The runtime requests DOS library version 40 (AmigaOS 3.1). A manual vAmiga run
with Exec 40.10 and DOS 40.3 passed all four smoke samples: exact redirected
output, status 0 for normal runs, status 20 for the deliberate division fault,
and successful execution after the fault. See the
[emulator validation record](MIR68K_AMIGA_EMULATOR_VALIDATION.md).
Older OS versions and AROS have not been validated. An older DOS library makes
startup return 20 before any output is available.

Use an MC68000 configuration, at least 1 MiB RAM and a 64 KiB command stack for
the initial smoke test. The program uses the inherited stack. The shim measured
184–404 bytes of stack use across the supplied samples with raw and optimized
NIR, including the test caller but excluding real library internals. These
measurements are not a bound for arbitrary programs or real OS stack use.

## Console calls

| Call | Output |
| --- | --- |
| `Put(byte)` | One unchanged byte |
| `PutE()` | LF, byte 10 |
| `Print(text)`, `PrintE(text)` | Counted string payload, with optional LF |
| `PrintB(value)`, `PrintBE(value)` | Unsigned BYTE decimal, with optional LF |
| `PrintC(value)`, `PrintCE(value)` | Unsigned CARD decimal, with optional LF |
| `PrintI(value)`, `PrintIE(value)` | Signed INT decimal, with optional LF |

These are the existing SYS declarations. Named modules use `USE SYS` and
qualified calls such as `SYS.PrintE("Hello")`. Legacy-style source can use the
unqualified names, as the samples do. Strings retain their length prefix in
memory; `Print` omits it. Payload bytes, including NUL and bytes above 127, pass
unchanged. There is no implicit ATASCII conversion. Use printable ASCII and the
E routines for portable text examples.

Normal completion returns status 0. Startup failure, failed console output or
a typed Action! fault returns 20 after cleanup. Fault messages identify the
reason, for example `Action! DivisionByZero`. Partial writes are completed;
zero progress or a negative result terminates output and the program.

The program entry must be a parameterless PROC. Other SYS services, LONGINT/
LONGCARD decimal formatting, REAL, input, graphics, general file APIs, argument
parsing and Workbench launch are outside this runtime. Do not launch these
executables as Workbench applications. Source ORG, Atari SET origin controls
and `--origin` are invalid because the OS chooses load addresses.

Bare remains the default MC68000 runtime. Explicit `--backend mir68k` and
`--profile modern` are accepted; Atari `--mode` and runtime settings are not.
`--module-path`, `--no-opt` and `--no-codegen-opt` retain their native meanings.
`--listing` writes physical instruction inspection text. `actionc-emit` supports
NIR inspection, an Amiga map using section/offset locations, and HUNK byte output.
The native bare JSON transport remains version 2 and is unchanged.

## Reproduce the real-OS smoke test

Build the [four samples](../samples/amiga/README.md) and a Shell script:

```sh
python3 tools/amiga_smoke.py build --compiler target/debug/actionc
```

The bundle in `build/amiga-smoke/` records its run ID, repository revision,
compiler identity and file hashes. It contains the executables, `smoke`, and
independently specified expected output. The deliberate fault returns 20; the
script checks that status and then runs the greeting again. It also exercises
console output, file redirection and repeated launches.

A tested transfer method uses the pure Python filesystem tools from
[amitools 0.8.1](https://pypi.org/project/amitools/0.8.1/):

```sh
python3 -m venv build/amiga-host-tools
build/amiga-host-tools/bin/python -m pip install amitools==0.8.1
build/amiga-host-tools/bin/xdftool -f build/actionc-smoke.adf \
  pack build/amiga-smoke ofs + relabel ActionC
```

This makes a data disk, not an OS boot disk. The generated 880 KiB OFS image
has been unpacked and all four executables compared byte for byte with the
compiler output. Keep ROMs and OS disks outside the repository.

In vAmiga, use a **copy** of an AmigaOS 3.1 machine and its disks. Verify the CPU,
memory, Kickstart and DOS versions, then insert `actionc-smoke.adf` in DF1.
The locally inspected emulator is vAmiga 4.5, build 260807. Keep the Workbench
boot disk mounted too, so disk commands such as Execute and Version remain
available. Boot the OS, open a Shell, and run:

```text
CD ActionC:
Protect #?.amiga +e
Execute smoke
MakeDir ActionC:results
Copy RAM:actionc-#? ActionC:results
```

The script saves each program's return code immediately after execution and
writes `SMOKE PASS` only after the expected statuses, including recovery. Inspect
the visible console output too. Its saved-status variable is `actioncrc`:
unbraced Amiga Shell variable names contain only letters and digits, so
`$actionc_rc` would refer to `$actionc` followed by literal `_rc`.
See [RKRM AmigaDOS, section 15.1.5](https://developer.amigaos3.net/sites/default/files/downloads/2024-10/Amiga_ROM_Kernel_Reference_Manual_DOS.pdf).
Use a freshly generated script if a status report contains literal variables.

Export the modified **ActionC** disk (normally DF1) as
`build/actionc-results.adf`, then unpack and verify the returned files:

```sh
mkdir -p build/amiga-returned
build/amiga-host-tools/bin/xdftool -r build/actionc-results.adf \
  unpack build/amiga-returned
python3 tools/amiga_smoke.py verify build/amiga-returned/ActionC/results
```

Use an extraction directory without an existing `ActionC` subdirectory.
With an existing destination directory, amitools creates a subdirectory named
after the volume. An exported Workbench disk does not contain the test reports.

Verification requires exact output bytes, the current run ID, every expected
return code, the final marker, and recorded Exec/DOS version 40. Missing markers,
wrong output or OS errors fail the check. Record the machine configuration and
vAmiga version alongside the reports; a successful emulator process exit alone
does not establish acceptance.

The mandatory Linux/Windows/macOS CI checks use r68k, an independent HUNK reader
and a small OS-call shim, without ROM dependencies. They execute the generated
startup, console and library-adapter instructions. Real AmigaOS testing remains
a separate acceptance step. See the [execution contract](MIR68K_EXECUTION_CONTRACT.md)
for ABI, relocation and fault details.
