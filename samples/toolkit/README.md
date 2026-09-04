# ACTION! Toolkit Samples

This directory contains maintained Toolkit sources intended for day-to-day
`actionc` compilation work.

```text
modern/
  hand-modernized source copies for the `modern` profile
  filenames match the original Toolkit names where practical
```

The byte-exact Toolkit ATR and extracted original files live under
`corpora/toolkit/`. Toolkit survey scripts, VM captures, and comparison reports
live under `surveys/toolkit/`.

`KALSCOPE.DEM`, `MUSIC.DEM`, and the maintained `.DM1`/`.DM2` demonstrations
are executable sample roots. The adjacent `.ACT` files are their local library
dependencies; in particular, `MUSIC.DEM` uses the maintained `IO.ACT` copy and
does not depend on an extracted corpus path. `ALLOCATE.ACT` is retained as
readable library source but currently has no maintained executable owner. The
hidden `.PMG_trace_default_no_appmhi.DM1` file is a compiler trace input, not a
user-facing demo.

These roles and the known-good build combinations are enforced by
`tests/sample_build_matrix.rs`.
