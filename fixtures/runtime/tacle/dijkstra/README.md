# TACLeBench Dijkstra

`dijkstra.act` ports the TACLeBench/MiBench graph-search workload using existing
Action! records, self-referencing record pointers, pointer parameters, and arrays
of records containing rows. It retains **100 graph nodes**, the **1,000-entry
queue pool**, and the original driver performing 20 searches. No new language
constructs or heap allocator are needed.

This is a host-driven compiler fixture, with the graph and queue state supplied
in RAM. The modern classic (`--mode optimized`) and MIR6502 backends run with
both cartridge and standalone runtimes. Compile at **`$8000`** to keep generated
code above the host data and below the cartridge at `$A000`.

## Provenance

- [Pinned upstream directory](https://github.com/tacle/tacle-bench/tree/c6a0d73e47bbd2bc86e34637156fb26dd4d5cf08/bench/sequential/dijkstra),
  revision `c6a0d73e47bbd2bc86e34637156fb26dd4d5cf08`, source header version 2.0.
- Upstream identifies the author as unknown and the source as MiBench's network
  section. It identifies the license as GPL, without a version.
- `dijkstra.c`, `input.h`, and `input.c` are unmodified upstream files. The full
  original 100-by-100 matrix is retained and used by the benchmark test.
- The Action! adaptation was added on 2026-09-13 under GNU GPL; see the
  repository [license](../../../../LICENSE).
- LF-normalized SHA-256 hashes:
  `dijkstra.c`: `ffa37cd0725262ef946bc1946ca39f168d19d95b816b6483939b322bd44df43c`;
  `input.h`: `2c942256da80e579b805ee6baed2bba3186b59668a48b10f486568c6c1d4a474`;
  `input.c`: `31de2ec9cb93102533bc9ce26872e853c0bd8a5861b60019db6af7705ae82fee`.

## Action! adaptations and preserved behavior

The C structures become packed Action! records. Each node has two INT fields
(four bytes); each queue entry has three INT fields and a QueueItem POINTER
(eight bytes). The C matrix becomes `Row ARRAY matrix(100)`, with each row
containing `BYTE ARRAY cost(100)`. Access remains `matrix(node).cost(i)`.

C integers become signed 16-bit INT in both the port and the reference. Graph
weights remain unsigned bytes. Search distances, intermediate additions, loop
indexes, and checksums in these cases fit in INT without overflow. Pointer
outputs retain the original procedure-call structure. C increments and the
assignment inside a condition become separate statements in the same order.
The volatile XOR-zero initialization loop is retained.

The workload uses FIFO relaxation with a linked queue, not a priority queue.
The queue pool is allocated monotonically and reset after each completed search
in the benchmark driver. Several unusual upstream behaviors are preserved:

- `9999` means both an unknown node/distance and a missing edge, but the byte
  matrix cannot store 9999. Every matrix element therefore represents an edge;
  zero-weight entries are real edges. The port keeps the original sentinel
  comparison and does not claim disconnected-graph coverage.
- When start equals end, Find returns success while leaving every node's
  distance and predecessor at 9999, rather than setting the source distance to
  zero. This behavior is tested explicitly.
- Enqueue captures the candidate slot, increments the allocation index, then
  checks the limit. Consequently, slots 0..998 can be written; slot 999 is
  rejected. A failed insertion increments the index but leaves queue links,
  count, and slot bytes unchanged. Hosts must not keep allocating beyond the
  pool: the tests exercise the first rejection with valid input indexes.
- On exhaustion during relaxation, the node update preceding the failed
  Enqueue remains visible, together with the remaining pending queue. The
  original benchmark stops and subtracts one from its checksum.

## Host memory contract

| Address | Meaning |
| --- | --- |
| `$0600` | Command (0..3) |
| `$0602`, `$0604` | Start and end node, INT |
| `$0606`, `$0608` | Result and checksum, INT |
| `$060A`, `$060C` | Queue count and next allocation index, INT |
| `$060E` | Queue-head pointer |
| `$0610`, `$0612`, `$0614` | Enqueue node, distance, predecessor arguments |
| `$0616`, `$0618`, `$061A` | Dequeue outputs; unchanged for an empty queue |
| `$061C` | Initial allocation index for command 1 |
| `$06FF` | Completion marker, written as `$A5` |
| `$0801..$0990` | 100 node records |
| `$1001..$3710` | Full 100-by-100 byte matrix |
| `$4001..$5F40` | 1,000 queue records |

The entry point imports/exports the head through the CARD at `$060E`; the
working head is an ordinary typed QueueItem POINTER. This avoids treating a
numeric POINTER initializer as a fixed-storage binding, whose interpretation
currently differs between backends. All words and pointer addresses are
little-endian. Queue pointers are null or
`$4001 + 8*index`, pointing into the actual record pool. The host must supply valid
node indexes and acyclic queue chains; there is no new runtime
bounds or pointer validation in the Action! fixture.

| Command | Operation |
| --- | --- |
| 0 | Init, original 20-search benchmark, original checksum-return check |
| 1 | Init, apply initial allocation index, Find(start,end) |
| 2 | Enqueue into the host-supplied queue |
| 3 | Dequeue from the host-supplied queue, result=0 |

## Coverage

`graphs.txt` stores 11 complete matrices. `vectors.txt` stores **33 cases**,
executed across four backend/runtime combinations (**132 VM executions**):

- The original full benchmark, reproducing checksum **25**, and an adversarial
  benchmark that exhausts the queue through repeated distance improvements.
- 16 individual searches: original and deterministic random graphs, zero/one/
  maximum byte weights, ties, a backwards zero-cost chain, equal start/end
  nodes, and allocation limits.
- 11 enqueue and four dequeue cases, including empty/nonempty chains, signed
  payloads, indexes above 255, page-crossing fields, and the final pool slots.

The C oracle records **21/22 if/while outcomes**. The only unexecuted outcome
is the impossible missing-edge comparison described above; regeneration checks
this exact coverage set. This is branch coverage, not a full path-coverage claim.

Every case compares the complete 32-byte control area, all 100 node records,
and all 1,000 queue records, including every pointer and unused entry. It also
checks completion, the unchanged graph, and every surrounding guard byte in
`$0600..$7FFF`. Data structures start at unaligned addresses and span pages.
Standalone runs load no ROMs. LF and CRLF source, graph, and vector text pass
through the actual compiler, parser, and VM path.

The four configurations are separate Rust tests and can run in parallel. The
full 20-search driver remains enabled in each one, so the debug VM target can
take several minutes; execution limits count emulated instructions rather than
elapsed time. A fifth test validates the text fixtures and original results.

```sh
cd tools/vm-runtime-tests
cargo test --locked --test dijkstra
```

## Regenerating vectors

From the repository root:

```sh
python3 tools/generate_dijkstra_vectors.py
python3 tools/generate_dijkstra_vectors.py --check
```

Regeneration requires Python 3 and a GCC-compatible compiler (`--cc clang` is
supported). It verifies pinned source hashes and compiles the C implementation
with explicit 16-bit integers. The wrapper maps pointer addresses to/from host
C pointers and serializes individual fields, independent of host pointer width,
structure padding, and endianness. It preserves the C algorithm, instruments
conditions without changing their evaluation order, and checks that the matrix
is unchanged.

Successful individual searches are additionally checked against an independent
Python implementation of dense Dijkstra, including all distances and predecessor
edge consistency. Expected Action! results always come from C. Normal VM tests
need neither a C compiler nor network access.

Text vectors encode binary data as hex. Queue fields may omit trailing zero
bytes (`-` means an entirely zero pool); the parser restores the full 8,000-byte
pool before comparison. This reduces fixture size without skipping unused bytes.
