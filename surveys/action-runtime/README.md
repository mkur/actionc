# ACTION! RunTime Disk Survey

This directory tracks current `actionc` support for the official ACTION!
RunTime Disk sources extracted under `corpora/action-runtime/extracted`.

Run:

```sh
surveys/action-runtime/survey.sh
```

The script builds `actionc`, checks each selected `.ACT` file with both
profiles (`legacy`, `modern`) and both backends (`classic`, `mir6502`), then
writes `RUNTIME_DISK_SURVEY.md`.

Modernized source files live under `samples/action-runtime/modern`, not under
`corpora/action-runtime/extracted`. The survey overlays those files onto a
temporary copy of the extracted runtime disk before compiling them, so local
`INCLUDE` directives still resolve beside the original runtime sources.

Sweep rules:

- `SYSALL.ACT` is the only `SYS*.ACT` source checked directly because it
  includes the other `SYS*.ACT` runtime files.
- Overlay files in `samples/action-runtime/modern` are used in place of the
  same-named extracted source when actionc needs a modernized variant.
- Each checked configuration emits a source listing so the sweep covers
  parsing, semantic analysis, include expansion, and backend lowering.

The report is the compatibility baseline for the extracted runtime disk
sources. It records both self-contained modules and disk programs that include
the runtime libraries with original Atari device paths such as `D:SYS.ACT`.
