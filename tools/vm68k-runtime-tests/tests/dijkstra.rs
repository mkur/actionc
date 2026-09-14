mod common;
#[path = "common/compiler.rs"]
mod compiler;
#[path = "common/records.rs"]
mod records;
#[path = "common/reference.rs"]
mod reference;
use actionc_vm68k_tests::{Machine, Outcome};
use records::{Records, scalar, set};

const DRIVER: &str = include_str!("../../../fixtures/runtime/tacle/dijkstra/dijkstra.act");
const TYPES: &str = include_str!("../../../fixtures/runtime/tacle/dijkstra/types.inc");
const CORE: &str = include_str!("../../../fixtures/runtime/tacle/dijkstra/kernel.inc");
const VECTORS: &str = include_str!("../../../fixtures/runtime/tacle/dijkstra/vectors.txt");
const GRAPHS: &str = include_str!("../../../fixtures/runtime/tacle/dijkstra/graphs.txt");
const HEADER: &[(&str, usize)] = &[
    ("startNode", 2),
    ("endNode", 4),
    ("result", 6),
    ("checksum", 8),
    ("queueCount", 10),
    ("queueNext", 12),
    ("argNode", 16),
    ("argDist", 18),
    ("argPrev", 20),
    ("outNode", 22),
    ("outDist", 24),
    ("outPrev", 26),
    ("seedNext", 28),
];
fn word(bytes: &[u8], offset: usize) -> u32 {
    u32::from(u16::from_le_bytes(
        bytes[offset..offset + 2].try_into().unwrap(),
    ))
}
fn pool(text: &str) -> Vec<u8> {
    let mut bytes = reference::bytes(text);
    assert!(bytes.len() <= 8000);
    bytes.resize(8000, 0);
    bytes
}

fn check(optimize: bool) {
    let source = common::Source::new(&reference::text(DRIVER, optimize));
    let directory = source.0.parent().unwrap();
    for (name, text) in [("types.inc", TYPES), ("kernel.inc", CORE)] {
        std::fs::write(directory.join(name), reference::text(text, optimize)).unwrap();
    }
    let image = compiler::compile(
        &source.0,
        &directory.join("dijkstra.json"),
        if optimize { &[] } else { &["--no-opt"] },
    );
    let mut probe = Machine::from_image(&image).unwrap();
    set(&mut probe, &image, "command", 255);
    probe.run(1000).assert_completed();
    let query = |name| scalar(&probe, &image, name);
    let int = query("layoutIntSize");
    let pointer = query("layoutPointerSize");
    assert_eq!((int, pointer), (2, 4));
    let sizes = [
        query("layoutNodeSize"),
        query("layoutRowSize"),
        query("layoutQueueSize"),
    ];
    let node_offsets = [query("layoutNodeDist"), query("layoutNodePrev")];
    let queue_offsets = [
        query("layoutQueueNode"),
        query("layoutQueueDist"),
        query("layoutQueuePrev"),
        query("layoutQueueNext"),
    ];
    let row_cost = query("layoutRowCost");
    let graphs_text = reference::text(GRAPHS, optimize);
    let graphs: std::collections::BTreeMap<_, _> = graphs_text
        .lines()
        .filter(|l| !l.starts_with('#') && !l.is_empty())
        .map(|l| {
            let f: Vec<_> = l.split_whitespace().collect();
            assert_eq!(f.len(), 2);
            let graph = reference::bytes(f[1]);
            assert_eq!(graph.len(), 10000);
            (f[0], graph)
        })
        .collect();
    let vectors_text = reference::text(VECTORS, optimize);
    let vectors: Vec<_> = vectors_text
        .lines()
        .filter(|l| !l.starts_with('#') && !l.is_empty())
        .collect();
    assert_eq!(vectors.len(), 33);
    for line in vectors {
        let f: Vec<_> = line.split_whitespace().collect();
        assert_eq!(f.len(), 7);
        let input = reference::bytes(f[2]);
        let expected = reference::bytes(f[4]);
        let input_pool = pool(f[3]);
        let expected_pool = pool(f[6]);
        let expected_nodes = reference::bytes(f[5]);
        assert_eq!(
            (input.len(), expected.len(), expected_nodes.len()),
            (32, 32, 400)
        );
        let graph = &graphs[f[1]];
        let mut vm = Machine::from_image(&image).unwrap();
        let nodes = Records::new(&vm, &image, "nodes", 100, sizes[0]);
        let rows = Records::new(&vm, &image, "matrix", 100, sizes[1]);
        let items = Records::new(&vm, &image, "items", 1000, sizes[2]);
        assert!(items.base > 0xffff);
        set(&mut vm, &image, "command", input[0].into());
        for &(name, offset) in HEADER {
            set(&mut vm, &image, name, word(&input, offset));
        }
        set(
            &mut vm,
            &image,
            "headAddress",
            items.from_wire(word(&input, 14), 0x4001, 8).unwrap(),
        );
        for i in 0..100 {
            for &offset in &node_offsets {
                nodes.write(&mut vm, i, offset, int, 0);
            }
            vm.cpu
                .mem
                .write(
                    rows.address(i, row_cost, 100),
                    &graph[i as usize * 100..(i as usize + 1) * 100],
                )
                .unwrap();
        }
        for i in 0..1000 {
            for (field, &offset) in queue_offsets.iter().enumerate() {
                let wire = word(&input_pool, i as usize * 8 + field * 2);
                let value = if field == 3 {
                    items.from_wire(wire, 0x4001, 8).unwrap()
                } else {
                    wire
                };
                items.write(
                    &mut vm,
                    i,
                    offset,
                    if field == 3 { pointer } else { int },
                    value,
                );
            }
        }
        // The original entry performs 20 searches with linear FIFO-tail walks.
        // Retain the 6502 suite's billion-instruction bound for that command.
        let run = vm.run(if input[0] == 0 {
            1_000_000_000
        } else {
            100_000_000
        });
        if input[0] == 0 {
            eprintln!("Dijkstra {optimize}/{}: {} instructions", f[0], run.steps);
        }
        assert!(
            matches!(run.outcome, Outcome::Completed),
            "{optimize}/{}: {run:?}",
            f[0]
        );
        assert_eq!(scalar(&vm, &image, "done"), 0xa5);
        assert_eq!(scalar(&vm, &image, "command"), u32::from(expected[0]));
        for &(name, offset) in HEADER {
            assert_eq!(
                scalar(&vm, &image, name),
                word(&expected, offset),
                "{optimize}/{}: {name}",
                f[0]
            );
        }
        assert_eq!(
            items
                .to_wire(scalar(&vm, &image, "headAddress"), 0x4001, 8)
                .unwrap(),
            word(&expected, 14),
            "{}: head",
            f[0]
        );
        for i in 0..100 {
            for (field, &offset) in node_offsets.iter().enumerate() {
                assert_eq!(
                    nodes.read(&vm, i, offset, int),
                    word(&expected_nodes, i as usize * 4 + field * 2),
                    "{optimize}/{}: node {i}/{field}",
                    f[0]
                );
            }
            assert_eq!(
                vm.cpu
                    .mem
                    .bytes(rows.address(i, row_cost, 100), 100)
                    .unwrap(),
                &graph[i as usize * 100..(i as usize + 1) * 100],
                "{}: graph row {i}",
                f[0]
            );
        }
        for i in 0..1000 {
            for (field, &offset) in queue_offsets.iter().enumerate() {
                let actual = items.read(&vm, i, offset, if field == 3 { pointer } else { int });
                let wire = if field == 3 {
                    items.to_wire(actual, 0x4001, 8).unwrap()
                } else {
                    actual
                };
                assert_eq!(
                    wire,
                    word(&expected_pool, i as usize * 8 + field * 2),
                    "{optimize}/{}: queue {i}/{field}",
                    f[0]
                );
            }
        }
        nodes.check_padding(&vm, &node_offsets.map(|o| (o, int)), 0);
        rows.check_padding(&vm, &[(row_cost, 100)], 0);
        items.check_padding(
            &vm,
            &[
                (queue_offsets[0], int),
                (queue_offsets[1], int),
                (queue_offsets[2], int),
                (queue_offsets[3], pointer),
            ],
            0,
        );
        for (name, records, size) in [
            ("nodes", nodes, sizes[0]),
            ("matrix", rows, sizes[1]),
            ("items", items, sizes[2]),
        ] {
            assert_eq!(
                Records::new(&vm, &image, name, records.count, size).base,
                records.base,
                "array descriptor changed"
            );
        }
    }
}
#[test]
fn dijkstra_raw_matches_every_c_state() {
    check(false);
}
#[test]
fn dijkstra_optimized_matches_every_c_state() {
    check(true);
}
#[test]
fn queue_links_are_slot_identities_with_checked_full_width_conversion() {
    let pool = Records {
        base: 0x123400,
        stride: 10,
        count: 1000,
    };
    for index in [0, 1, 998, 999] {
        let native = pool.from_wire(0x4001 + 8 * index, 0x4001, 8).unwrap();
        assert_eq!(native, 0x123400 + 10 * index);
        assert_eq!(pool.to_wire(native, 0x4001, 8).unwrap(), 0x4001 + 8 * index);
    }
    assert_eq!(pool.from_wire(0, 0x4001, 8).unwrap(), 0);
    assert_eq!(pool.to_wire(0, 0x4001, 8).unwrap(), 0);
    for invalid in [0x4000, 0x4002, 0x4001 + 8000, u32::MAX] {
        assert!(pool.from_wire(invalid, 0x4001, 8).is_err());
    }
    for invalid in [0x1233ff, 0x123401, 0x123400 + 10000, u32::MAX] {
        assert!(pool.to_wire(invalid, 0x4001, 8).is_err());
    }
}
