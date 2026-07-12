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
