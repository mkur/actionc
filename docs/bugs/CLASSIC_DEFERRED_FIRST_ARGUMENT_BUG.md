# Classic deferred first byte argument corrupts later ABI registers

The classic backend's modern profile (`--mode optimized`) could overwrite a
later argument while loading a deferred first byte argument. The puLse demo
exposed this through calls such as `plot10(x(n),y(n),0)` and
`plot10(x(n),y(n),c(n))`.

## Reproducer

```action
BYTE ARRAY xs(2)=[0 23],ys(2)=[0 71]
BYTE n,first=$0600,second=$0601,third=$0602

PROC Capture(BYTE a,b,c)
  first=a
  second=b
  third=c
RETURN

PROC Main()
  n=1
  Capture(xs(n),ys(n),0)
RETURN
```

With standalone runtime, the expected bytes at `$0600..$0602` are
`$17,$47,$00`. Affected optimized builds write `$17,$47,$01` instead.

## Cause and invariant

`can_defer_modern_first_byte_staged_register_arg` allowed indexed and indirect
loads to be deferred until `emit_load_staged_call_registers` loaded A, after X
and Y already held later ABI argument bytes. A byte-array load using
`LDY index; LDA array,Y` replaced the third argument in Y with the array index.
More complex address calculations can also use X.

The fix keeps these first arguments staged whenever the call has another
argument byte. Single-byte calls retain the deferral, as do simple accumulator
expressions whose lowering preserves X/Y. Direct scalar/immediate deferral is
unchanged. This is a call-argument loading bug; removing the shift and SArgs
helpers from optimized output does not cause it.

## Regression coverage

`src/codegen/tests/call_arguments.rs` executes emitted 6502 code under both
classic profiles. It covers immediate, indexed and computed third arguments,
computed first indexes, pointer reads, a word second argument, byte-index
boundaries, and ordinary/tail calls.

The supplied puLse and pulse2 drawing/animation routines were also executed
with deterministic initial state for 120 animation steps. Both optimized
programs had 360 differing framebuffer bytes before the fix and zero after it;
compatibility and MIR6502 had zero in both runs. This checks drawing and motion,
not real Atari display timing or OS integration.
