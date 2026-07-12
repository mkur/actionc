# ARRREF.COM observations

Source: `surveys/probes/original-compiler/array_refs.act`

Original load-file layout:

- Code/data segment: `$3000..$3054`
- RUNAD segment: `$02E2..$02E3 = $3007`

Storage layout inferred from generated code:

- `ba`: two-byte byte-array base pointer at `$3000..$3001`
- `ca`: two-byte card-array base pointer at `$3002..$3003`
- `x`: `$3004`
- `w`: `$3005..$3006`
- `Main` trampoline/RUNAD: `$3007`
- `Main` body: `$300A`

Unsized array lowering:

- `BYTE ARRAY ba` does not allocate inline element storage. It allocates a two-byte pointer.
- `CARD ARRAY ca` likewise allocates a two-byte pointer.
- `ba(0)` loads the pointer from `$3000/$3001` into `$AE/$AF` and accesses `($AE),Y`.
- `ca(0)` loads the pointer from `$3002/$3003` into `$AE/$AF`; card element stores/loads use `Y=1` for the high byte and `Y=0` for the low byte.

Important contrast with `ARRAYS.COM`:

- Sized `BYTE ARRAY ba(8)` is inline storage.
- Sized `CARD ARRAY ca(8)` is a four-byte descriptor plus backing storage after the saved code segment.
- Unsized arrays are just two-byte base pointers, presumably intended to be supplied by library/user setup.

Current actionc comparison:

- actionc now represents unsized global arrays as two-byte pointer-backed references.
- actionc now loads unsized array pointers into `$AE/$AF` for compatible indexing.
- Remaining differences are mostly code-shape and instruction-order deltas rather than storage-layout deltas.

Likely compatibility work:

- Add instruction-selection peepholes where original Action! uses shorter array forms.
- Continue comparing against original load files as broader array features are added.
