mod common;
#[path = "common/compiler.rs"]
mod compiler;
#[path = "common/records.rs"]
mod records;
#[path = "common/reference.rs"]
mod reference;
use actionc_vm68k_tests::{Machine, Outcome};
use records::{Records, scalar, set};

const DRIVER: &str = include_str!("../../../fixtures/runtime/tacle/huff_dec/huff_dec.act");
const TYPES: &str = include_str!("../../../fixtures/runtime/tacle/huff_dec/types.inc");
const CORE: &str = include_str!("../../../fixtures/runtime/tacle/huff_dec/kernel.inc");
const VECTORS: &str = include_str!("../../../fixtures/runtime/tacle/huff_dec/vectors.txt");
const HEADER: &[(&str, usize, usize)] = &[
    ("command", 0, 1),
    ("inputLength", 2, 2),
    ("inputPos", 4, 2),
    ("outputPos", 6, 2),
    ("bitsLeft", 8, 1),
    ("reservoir", 10, 4),
    ("request", 14, 2),
    ("result", 16, 4),
    ("nodeCount", 22, 2),
];
fn number(bytes: &[u8], offset: usize, width: usize) -> u32 {
    reference::words(&bytes[offset..offset + width], width)[0]
}
fn filled(text: &str, length: usize, fill: u8) -> Vec<u8> {
    let mut bytes = reference::bytes(text);
    assert!(bytes.len() <= length);
    bytes.resize(length, fill);
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
        &directory.join("huff_dec.json"),
        if optimize { &[] } else { &["--no-opt"] },
    );
    let mut probe = Machine::from_image(&image).unwrap();
    set(&mut probe, &image, "command", 255);
    probe.run(1000).assert_completed();
    let query = |name| scalar(&probe, &image, name);
    let code_size = query("layoutCodeSize");
    let tree_size = query("layoutTreeSize");
    let bits = query("layoutCodeBits");
    let length = query("layoutCodeLength");
    let present = query("layoutCodePresent");
    let symbol = query("layoutTreeSymbol");
    let left = query("layoutTreeLeft");
    let right = query("layoutTreeRight");
    let card = query("layoutCardSize");
    let pointer = query("layoutPointerSize");
    assert_eq!((card, pointer), (2, 4));
    let vectors_text = reference::text(VECTORS, optimize);
    let vectors: Vec<_> = vectors_text
        .lines()
        .filter(|l| !l.starts_with('#') && !l.is_empty())
        .collect();
    assert_eq!(vectors.len(), 185);
    for line in vectors {
        let f: Vec<_> = line.split_whitespace().collect();
        assert_eq!(f.len(), 7);
        let input_header = reference::bytes(f[1]);
        let expected_header = reference::bytes(f[3]);
        assert_eq!((input_header.len(), expected_header.len()), (24, 24));
        let input = filled(f[2], 12288, 0xcc);
        let expected_output = filled(f[4], 1024, 0xcc);
        let expected_codes = filled(f[5], 257 * 35, 0);
        let expected_heap = filled(f[6], 514 * 6, 0);
        assert_eq!(
            reference::bytes(f[2]).len(),
            number(&input_header, 2, 2) as usize
        );
        let mut vm = Machine::from_image(&image).unwrap();
        let codes = Records::new(&vm, &image, "codes", 257, code_size);
        let heap = Records::new(&vm, &image, "heap", 514, tree_size);
        assert!(heap.base > 0xffff);
        for &(name, offset, width) in HEADER {
            set(&mut vm, &image, name, number(&input_header, offset, width));
        }
        set(
            &mut vm,
            &image,
            "rootAddress",
            heap.from_wire(number(&input_header, 20, 2), 0x6501, 6)
                .unwrap(),
        );
        vm.write_array(
            image.symbol("encoded").unwrap(),
            &input.iter().map(|b| u32::from(*b)).collect::<Vec<_>>(),
        )
        .unwrap();
        vm.write_array(image.symbol("decoded").unwrap(), &[0xcc; 1024])
            .unwrap();
        // Reference fields begin at zero; padding is independently poisoned.
        for records in [codes, heap] {
            vm.cpu
                .mem
                .write(
                    records.base,
                    &vec![0xcc; (records.count * records.stride) as usize],
                )
                .unwrap();
        }
        for i in 0..257 {
            vm.cpu
                .mem
                .write(codes.address(i, bits, 32), &[0; 32])
                .unwrap();
            codes.write(&mut vm, i, length, card, 0);
            codes.write(&mut vm, i, present, 1, 0);
        }
        for i in 0..514 {
            heap.write(&mut vm, i, symbol, card, 0);
            heap.write(&mut vm, i, left, pointer, 0);
            heap.write(&mut vm, i, right, pointer, 0);
        }
        let run = vm.run(60_000_000);
        assert!(
            matches!(run.outcome, Outcome::Completed),
            "{optimize}/{}: {run:?}",
            f[0]
        );
        assert_eq!(scalar(&vm, &image, "done"), 0xa5);
        for &(name, offset, width) in HEADER {
            assert_eq!(
                scalar(&vm, &image, name),
                number(&expected_header, offset, width),
                "{optimize}/{}: {name}",
                f[0]
            );
        }
        assert_eq!(
            heap.to_wire(scalar(&vm, &image, "rootAddress"), 0x6501, 6)
                .unwrap(),
            number(&expected_header, 20, 2),
            "{}: root",
            f[0]
        );
        let output = vm.read_array(image.symbol("decoded").unwrap()).unwrap();
        for (i, (&actual, &expected)) in output.iter().zip(&expected_output).enumerate() {
            assert_eq!(
                actual,
                u32::from(expected),
                "{optimize}/{}: output {i}",
                f[0]
            );
        }
        let encoded = vm.read_array(image.symbol("encoded").unwrap()).unwrap();
        for (i, (&actual, &expected)) in encoded.iter().zip(&input).enumerate() {
            assert_eq!(actual, u32::from(expected), "{}: encoded {i}", f[0]);
        }
        for i in 0..257 {
            let expected = &expected_codes[i as usize * 35..(i as usize + 1) * 35];
            assert_eq!(
                vm.cpu.mem.bytes(codes.address(i, bits, 32), 32).unwrap(),
                &expected[..32],
                "{optimize}/{}: code {i} bits",
                f[0]
            );
            assert_eq!(
                codes.read(&vm, i, length, card),
                number(expected, 32, 2),
                "{optimize}/{}: code {i} length",
                f[0]
            );
            assert_eq!(
                codes.read(&vm, i, present, 1),
                number(expected, 34, 1),
                "{optimize}/{}: code {i} present",
                f[0]
            );
        }
        for i in 0..514 {
            let expected = &expected_heap[i as usize * 6..(i as usize + 1) * 6];
            assert_eq!(
                heap.read(&vm, i, symbol, card),
                number(expected, 0, 2),
                "{optimize}/{}: tree {i} symbol",
                f[0]
            );
            for (offset, wire_offset) in [(left, 2), (right, 4)] {
                assert_eq!(
                    heap.to_wire(heap.read(&vm, i, offset, pointer), 0x6501, 6)
                        .unwrap(),
                    number(expected, wire_offset, 2),
                    "{optimize}/{}: tree {i} link {offset}",
                    f[0]
                );
            }
        }
        codes.check_padding(&vm, &[(bits, 32), (length, card), (present, 1)], 0xcc);
        heap.check_padding(
            &vm,
            &[(symbol, card), (left, pointer), (right, pointer)],
            0xcc,
        );
        for (name, records, size) in [("codes", codes, code_size), ("heap", heap, tree_size)] {
            assert_eq!(
                Records::new(&vm, &image, name, records.count, size).base,
                records.base,
                "array descriptor changed"
            );
        }
    }
}
#[test]
fn huff_dec_raw_matches_every_c_state() {
    check(false);
}
#[test]
fn huff_dec_optimized_matches_every_c_state() {
    check(true);
}
