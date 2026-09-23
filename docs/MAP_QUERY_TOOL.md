# `actionc-map-query`

`actionc-map-query` runs a batch of map queries against one generated
`actionc` profile.

Example:

```sh
cargo run --bin actionc-map-query -- \
  --profile legacy \
  --queries scratch/map-query.txt \
  fixtures/stress/pointers.act
```

There is also a saved pointers stress batch:

```sh
cargo run --bin actionc-map-query -- \
  --profile legacy \
  --queries fixtures/map-queries/pointers.mapq \
  fixtures/stress/pointers.act
```

Query files contain one command per line. Empty lines and lines starting with
`#` are ignored.

```text
owner $3026
source $3268
symbol BP
routine Main
range $3000 $3030
```

Supported commands:

- `owner <addr>`: storage, routine, skipped-range, and source ownership for an
  address
- `source <addr>`: nearest source range for an address
- `symbol <name>`: matching storage symbols and routines
- `routine <name>`: matching routine range
- `range <start> <end>`: overlapping map items; `end` is exclusive

The current renderer is plain text. Internally the tool builds structured
`QueryResult` values first, so a JSON renderer can be added later without
parsing text output or changing the query execution model.

## Source correspondence

Statement locations originate in the parser and survive SemIR lowering and
classic projection. A `RETURN` statement carries its own span, from the keyword
through the optional closing parenthesis; its returned expression keeps its
separate expression span. Bare returns must not use the start of the source
file as a substitute location.

Classic tail-call lowering records the call's source range over argument setup
and the final jump. A return merged into that jump has no separate instruction
to annotate. Byte-deleting optimizations relocate source ranges, including the
start of the statement currently being emitted. Completed parent ranges follow
their children so equally sized ranges retain the more specific source match.

The listing and map query use this metadata from the final code map. Repairing
source correspondence must not change the emitted executable.
