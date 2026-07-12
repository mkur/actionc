# ABICALLS.COM Notes

Captured with the probe containing `SET *=$3000`.

The resulting Atari load file did not move the generated segment to `$3000`:

- code segment: `$27D4-$284D` (`122` bytes)
- RUNAD segment: `$02E2-$02E3`, value `$282B`
- first code/data bytes at `$27D4`: `00 30`

This shows that original `SET *=$3000` writes the value `$3000` at the current
code pointer rather than changing the compiler origin. For origin-controlled
captures, use:

```action
SET $491=$3000
SET $0E=$3000
```

where `$0491` is `codebase` and `$000E` is `qcode`.
