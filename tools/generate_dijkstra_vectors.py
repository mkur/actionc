#!/usr/bin/env python3
"""Generate Dijkstra VM vectors from pinned TACLeBench C, never from Action!."""

import argparse
import ctypes
import hashlib
from pathlib import Path
import random
import re
import subprocess
import tempfile


ROOT = Path(__file__).resolve().parents[1]
FIXTURES = ROOT / "fixtures/runtime/tacle/dijkstra"
HASHES = {
    "dijkstra.c": "ffa37cd0725262ef946bc1946ca39f168d19d95b816b6483939b322bd44df43c",
    "input.h": "2c942256da80e579b805ee6baed2bba3186b59668a48b10f486568c6c1d4a474",
    "input.c": "31de2ec9cb93102533bc9ce26872e853c0bd8a5861b60019db6af7705ae82fee",
}
NODES = 100
POOL_SIZE = 1000
POOL_BASE = 0x4001
OUTPUT_BYTES = 32 + NODES * 4 + POOL_SIZE * 8


def pinned(name):
    text = (FIXTURES / name).read_text()  # normalize host CRLF before hashing
    if hashlib.sha256(text.encode()).hexdigest() != HASHES[name]:
        raise ValueError(f"Pinned {name} changed; review provenance and adaptations")
    return text


def reference_source():
    header = pinned("input.h")
    source = pinned("dijkstra.c").replace('#include "input.h"', "")
    source = source[:source.index("int main( void )\n{")]
    source = source.replace("int main( void );", "")
    source = re.sub(r"\bint\b", "int16_t", source)
    # Observe branch outcomes without changing source evaluation order.
    conditions, edits, impossible_false = 0, [], None
    for match in re.finditer(r"\b(?:if|while)\s*\(", source):
        start = match.end()
        end, depth = start, 1
        while depth:
            depth += (source[end] == "(") - (source[end] == ")")
            end += 1
        if "dijkstra_AdjMatrix" in source[start:end - 1]:
            # BYTE costs cannot equal the source's 9999 sentinel.
            impossible_false = conditions * 2
        edits.extend([(start, f"oracle_cond({conditions}, !!("), (end - 1, "))")])
        conditions += 1
    for offset, value in sorted(edits, reverse=True):
        source = source[:offset] + value + source[offset:]
    prefix = f"""#include <stdint.h>
#include <string.h>
#include <stdlib.h>
static uint8_t oracle_branches[{conditions * 2}];
static int oracle_cond(int id, int value) {{
  oracle_branches[id * 2 + !!value] = 1;
  return value;
}}
unsigned char dijkstra_AdjMatrix[100][100];
"""
    wrapper = r"""
static uint16_t load16(const uint8_t *p) {
  return (uint16_t)p[0] | ((uint16_t)p[1] << 8);
}
static void store16(uint8_t *p, uint16_t value) {
  p[0] = (uint8_t)value; p[1] = (uint8_t)(value >> 8);
}
static struct _QITEM *decode_pointer(uint16_t address) {
  if (!address) return NULL;
  if (address < 0x4001 || (address - 0x4001) % 8 || (address - 0x4001) / 8 >= 1000)
    abort();
  return &dijkstra_queueItems[(address - 0x4001) / 8];
}
static uint16_t encode_pointer(struct _QITEM *pointer) {
  if (!pointer) return 0;
  return (uint16_t)(0x4001 + 8 * (pointer - dijkstra_queueItems));
}
void oracle_run(const uint8_t *graph, const uint8_t *header, const uint8_t *pool,
                uint8_t *output, uint8_t *coverage) {
  memcpy(dijkstra_AdjMatrix, graph, 10000);
  memset(dijkstra_rgnNodes, 0, sizeof dijkstra_rgnNodes);
  memset(oracle_branches, 0, sizeof oracle_branches);
  dijkstra_checksum = (int16_t)load16(header + 8);
  dijkstra_queueCount = (int16_t)load16(header + 10);
  dijkstra_queueNext = (int16_t)load16(header + 12);
  dijkstra_queueHead = decode_pointer(load16(header + 14));
  for (int i = 0; i < 1000; ++i) {
    dijkstra_queueItems[i].node = (int16_t)load16(pool + 8*i);
    dijkstra_queueItems[i].dist = (int16_t)load16(pool + 8*i + 2);
    dijkstra_queueItems[i].prev = (int16_t)load16(pool + 8*i + 4);
    dijkstra_queueItems[i].next = decode_pointer(load16(pool + 8*i + 6));
  }
  int16_t result = 0;
  int16_t node = (int16_t)load16(header + 22);
  int16_t dist = (int16_t)load16(header + 24);
  int16_t prev = (int16_t)load16(header + 26);
  switch (header[0]) {
    case 0:
      dijkstra_init(); dijkstra_main(); result = dijkstra_return(); break;
    case 1:
      dijkstra_init(); dijkstra_queueNext = (int16_t)load16(header + 28);
      result = dijkstra_find((int16_t)load16(header + 2), (int16_t)load16(header + 4));
      break;
    case 2:
      result = dijkstra_enqueue((int16_t)load16(header + 16), (int16_t)load16(header + 18),
                               (int16_t)load16(header + 20));
      break;
    case 3:
      dijkstra_dequeue(&node, &dist, &prev); break;
    default: abort();
  }
  if (memcmp(graph, dijkstra_AdjMatrix, 10000)) abort();
  memcpy(output, header, 32);
  store16(output + 6, result); store16(output + 8, dijkstra_checksum);
  store16(output + 10, dijkstra_queueCount); store16(output + 12, dijkstra_queueNext);
  store16(output + 14, encode_pointer(dijkstra_queueHead));
  store16(output + 22, node); store16(output + 24, dist); store16(output + 26, prev);
  for (int i = 0; i < 100; ++i) {
    store16(output + 32 + 4*i, dijkstra_rgnNodes[i].dist);
    store16(output + 32 + 4*i + 2, dijkstra_rgnNodes[i].prev);
  }
  for (int i = 0; i < 1000; ++i) {
    store16(output + 432 + 8*i, dijkstra_queueItems[i].node);
    store16(output + 432 + 8*i + 2, dijkstra_queueItems[i].dist);
    store16(output + 432 + 8*i + 4, dijkstra_queueItems[i].prev);
    store16(output + 432 + 8*i + 6, encode_pointer(dijkstra_queueItems[i].next));
  }
  memcpy(coverage, oracle_branches, sizeof oracle_branches);
}
"""
    assert impossible_false is not None
    return prefix + header + source + wrapper, conditions * 2, impossible_false


def distances(graph, start):
    """Independent dense Dijkstra oracle for successful C searches."""
    dist, visited = [100000] * NODES, set()
    dist[start] = 0
    for _ in range(NODES):
        node = min((i for i in range(NODES) if i not in visited), key=lambda i: dist[i])
        visited.add(node)
        for target in range(NODES):
            dist[target] = min(dist[target], dist[node] + graph[node * NODES + target])
    return dist


def generate(cc):
    original = pinned("input.c").split("=", 1)[1]
    graphs = {"original": bytes(map(int, re.findall(r"\d+", original)))}
    assert len(graphs["original"]) == NODES * NODES
    for name, weight in [("zero", 0), ("ones", 1), ("max", 255)]:
        graphs[name] = bytes([weight]) * (NODES * NODES)
    graphs["backward"] = bytes(0 if i == j or j == i - 1 else 255
                               for i in range(NODES) for j in range(NODES))
    graphs["ties"] = bytes(0 if i == j else (1 if j % 2 else 2)
                           for i in range(NODES) for j in range(NODES))
    # Later FIFO entries repeatedly improve earlier ones, exhausting the pool.
    graphs["exhaust"] = bytes(0 if i == j else (200 - j if i == 0 else (0 if j < i else 255))
                              for i in range(NODES) for j in range(NODES))
    rng = random.Random(0x44494A4B)
    for number in range(4):
        graphs[f"random-{number}"] = bytes(rng.randrange(256) for _ in range(NODES * NODES))

    source, branches, impossible_false = reference_source()
    with tempfile.TemporaryDirectory(prefix="actionc-dijkstra-") as temporary:
        directory = Path(temporary)
        cfile, library = directory / "oracle.c", directory / "oracle.so"
        cfile.write_text(source)
        subprocess.run([cc, "-std=c99", "-O2", "-shared", "-fPIC", "-Wno-unknown-pragmas",
                        str(cfile), "-o", str(library)], check=True)
        oracle = ctypes.CDLL(str(library)).oracle_run
        pointer = ctypes.POINTER(ctypes.c_uint8)
        oracle.argtypes = [pointer] * 5
        oracle.restype = None
        vectors, covered = [], set()

        def buffer(data):
            return (ctypes.c_uint8 * len(data)).from_buffer_copy(data)

        def word(data, offset, value):
            data[offset:offset + 2] = (value & 0xFFFF).to_bytes(2, "little")

        def short_hex(data):
            # Unwritten pool entries are zero; omitted suffix bytes are zero.
            return bytes(data).rstrip(b"\0").hex() or "-"

        def run(label, graph_name="original", command=1, start=0, end=50, seed=0,
                chain=(), next_slot=0, args=(99, -1234, 9999)):
            header, pool = bytearray([0xCC] * 32), bytearray(POOL_SIZE * 8)
            header[0] = command
            for offset, value in [(2, start), (4, end), (8, 0x1234), (10, len(chain)),
                                  (12, next_slot), (14, POOL_BASE + 8 * chain[0] if chain else 0),
                                  (16, args[0]), (18, args[1]), (20, args[2]),
                                  (22, -30000), (24, 0x1234), (26, 30000), (28, seed)]:
                word(header, offset, value)
            for position, index in enumerate(chain):
                following = POOL_BASE + 8 * chain[position + 1] if position + 1 < len(chain) else 0
                for field, value in enumerate([index % 100, -1000 + index, 9999 - index, following]):
                    word(pool, 8 * index + 2 * field, value)
            output, coverage = (ctypes.c_uint8 * OUTPUT_BYTES)(), (ctypes.c_uint8 * branches)()
            graph = graphs[graph_name]
            oracle(buffer(graph), buffer(header), buffer(pool), output, coverage)
            output = bytes(output)
            covered.update(i for i, value in enumerate(coverage) if value)
            result = int.from_bytes(output[6:8], "little", signed=True)
            if command == 1 and start != end and result == 0:
                expected_distances = distances(graph, start)
                actual = [int.from_bytes(output[32 + 4*i:34 + 4*i], "little", signed=True)
                          for i in range(NODES)]
                assert actual == expected_distances, label
                for node in range(NODES):
                    prev = int.from_bytes(output[34 + 4*node:36 + 4*node], "little", signed=True)
                    if node == start:
                        assert prev == 9999
                    else:
                        assert 0 <= prev < NODES
                        assert actual[node] == actual[prev] + graph[prev * NODES + node]
            vectors.append((label, graph_name, header.hex(), short_hex(pool), output[:32].hex(),
                            output[32:432].hex(), short_hex(output[432:])))
            return output

        original_result = run("benchmark-original", command=0)
        assert int.from_bytes(original_result[6:8], "little", signed=True) == 0
        assert int.from_bytes(original_result[8:10], "little", signed=True) == 25
        exhausted = run("benchmark-exhaust", "exhaust", command=0)
        assert int.from_bytes(exhausted[8:10], "little", signed=True) == -1
        for start, end in [(0, 50), (63, 99), (99, 0), (0, 0), (99, 99)]:
            run(f"original-{start}-{end}", start=start, end=end)
        for graph, start, end in [("zero", 0, 99), ("ones", 63, 0), ("max", 99, 0),
                                   ("backward", 99, 0), ("ties", 0, 99)]:
            run(graph, graph, start=start, end=end)
        for number, start in enumerate([0, 31, 63, 99]):
            run(f"random-{number}", f"random-{number}", start=start, end=(start + 50) % 100)
        for seed in [998, 999]:
            output = run(f"find-pool-{seed}", seed=seed)
            assert int.from_bytes(output[6:8], "little", signed=True) == -1
        for next_slot in [0, 31, 32, 255, 256, 998, 999]:
            run(f"enqueue-empty-{next_slot}", command=2, next_slot=next_slot)
        for chain, next_slot in [((0,), 1), ((31, 32, 127), 256), ((0, 31, 255), 998),
                                 ((0, 31, 255), 999)]:
            run(f"enqueue-chain-{len(chain)}-{next_slot}", command=2, chain=chain, next_slot=next_slot)
        for chain in [(), (0,), (31, 32, 255), (998,)]:
            run(f"dequeue-{'-'.join(map(str, chain)) or 'empty'}", command=3,
                chain=chain, next_slot=max(chain, default=-1) + 1)

        assert covered == set(range(branches)) - {impossible_false}, "C branch coverage changed"
        vector_header = [
            "# Generated by tools/generate_dijkstra_vectors.py; do not edit.",
            f"# nodes=100 pool_entries=1000 cases={len(vectors)} if_while_outcomes={len(covered)}/{branches}",
            "# label graph input_header32 input_pool_zero_tail expected_header32 expected_nodes400 expected_pool_zero_tail",
        ]
        vector_text = "\n".join(vector_header + [" ".join(row) for row in vectors]) + "\n"
        graph_text = "# Generated by tools/generate_dijkstra_vectors.py; label matrix10000_hex\n"
        graph_text += "\n".join(f"{name} {graph.hex()}" for name, graph in graphs.items()) + "\n"
        return {"graphs.txt": graph_text, "vectors.txt": vector_text}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cc", default="cc", help="GCC-compatible C compiler")
    parser.add_argument("--check", action="store_true", help="verify committed graphs and vectors")
    args = parser.parse_args()
    for name, text in generate(args.cc).items():
        path = FIXTURES / name
        if args.check:
            if path.read_text() != text:
                raise SystemExit(f"{name} is stale; regenerate Dijkstra vectors")
        else:
            path.write_text(text)
        print(f"{name}: SHA-256 {hashlib.sha256(text.encode()).hexdigest()}")
        if name == "vectors.txt":
            print(text.splitlines()[1])


if __name__ == "__main__":
    main()
