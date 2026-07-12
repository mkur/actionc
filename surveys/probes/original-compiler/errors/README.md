# Original Compiler Error Fixtures

This directory contains intentionally malformed Action! source files. They are
negative fixtures for compiler diagnostics, with each file mapped to the closest
original Action! compiler error code from `../ERROR_CODES.md`.

These fixtures are source-facing errors: they should fail before producing an
object file. Resource and runtime-condition codes from the original compiler
are listed in `manifest.tsv`, but are not represented by stable source fixtures
unless a deterministic source form exists.

Run the local compiler check with:

```sh
surveys/probes/original-compiler/errors/run-actionc.sh
```

The goal is not byte-for-byte original diagnostics. The goal is to keep a broad
negative corpus that exercises the same categories of source failures the
original compiler reported.
