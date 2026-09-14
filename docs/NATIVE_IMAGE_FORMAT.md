# Native image format, version 2

`compiler::native::artifacts` owns the executable transport for bare MC68000
programs. A UTF-8 JSON manifest names exact binary payloads in the same directory.
Move or copy the complete bundle; paths are relative to the manifest, independent
of the loader's working directory. A machine listing is optional inspection text
and is never needed to execute the image. This format is neither ELF nor an
Atari or Amiga load file.

The top-level fields are `format` (`"actionc-native"`), `version` (`2`), `target`
(`"motorola-68000"`), `endian` (`"big"`), `pointer_width` (`4` bytes),
`link_address_bits` (`24`), `entry`, `segments`, `zero_fill`, `symbols`, and optional
`machine_listing`. Numeric addresses, sizes, IDs and offsets are JSON integers.
Unknown fields and unsupported formats, versions or layouts are rejected.
Version 1 inspection dumps containing Rust debug type text are not executable
input to this reader.

Each segment contains `file`, `address`, `size`, `writable`, and `executable`.
The payload must contain exactly `size` bytes. Zero-fill regions contain
`address`, `size`, and `writable`; no payload is stored for BSS. Relocations have
already been applied. The reader checks arithmetic and the 24-bit bus limit,
nonempty segments, even executable ranges and entry, entry coverage, and overlap
across all mapped regions before allocating payloads. The manifest is bounded
to 16 MiB; payload reads are bounded by their verified declarations.

Payload names must be portable basenames, without path separators, drive prefixes
or parent references. Symlinks resolving outside the bundle are rejected. Listing
references obey the same basename syntax, but loading does not open the listing.

Each symbol has a display `name`, tagged `identity`, tagged `location`, `size`,
`alignment`, optional `type`, and optional `array`. Identities use a `kind` field:

| Kind | Identity fields |
| --- | --- |
| `global`, `static`, `routine` | `id` |
| `array_backing` | `owner` (global ID) |
| `static_local` | `routine`, `id` |
| `automatic` | `routine`, `object` |
| `parameter` | `routine`, `id` |

Locations are `{"kind":"absolute","address":…}` or
`{"kind":"frame","routine":…,"offset":…}`. Only parameters and automatic
objects are frame-relative; their owner must exist. Frames retain signed offsets
and cannot be inspected as globals after return. Routine extents must lie in
executable memory. Identities are unique; names need not be. Name lookup is
case-insensitive and rejects ambiguity. Aliases can describe overlapping storage
without adding overlapping mapped regions. External absolute symbols do not
allocate memory or grant access permissions.

Type layouts contain `kind`, optional `width` in bytes, and `signed`. Stable kinds
are `integer`, `boolean`, `data_pointer`, `callable`, `record`, and `opaque`.
Scalar integers have widths 1, 2 or 4; booleans use 1 and pointers/callables use 4.
Record field reflection and callable signatures are not transported. Static
initializer blocks whose element type does not describe their complete extent
are exported as opaque bytes. These are inspection facts, not executable IR.

Array metadata contains `element_width`, `stride`, optional `count`, `descriptor`,
and optional `backing_address`. Extents and multiplication are checked. A
descriptor starts with a four-byte pointer and can include a two-byte size word;
its backing address is an initial value, not an immutable pointer. VM array
access reads the descriptor's current guest value.
Direct arrays use their symbol address. Host adapters must respect the emitted
stride and target byte order.

The writer validates and serializes before publication. Every generation uses
new payload filenames; it stages outputs and replaces the manifest last. A
previous manifest's payloads remain intact. Failed writes clean up only files
created by that attempt. Old successful generations remain available until the
caller removes them. Listing publication is independent of the executable image.
Callers supply protected source paths to reject output/source collisions.

`ImageView` shares mapping/region verification between compiler images and imported
artifacts; `SymbolView` shares numeric and array inspection. Import does not
reconstruct NIR. Emulator memory reservations, stack setup, ABI guards and runtime
fault handling remain the VM's responsibility; the compiler does not depend on
r68k.
