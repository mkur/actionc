mod support;
use actionc::mir65816::image::Image;
use actionc_vm::native65816::{Access, Inputs};
use std::collections::BTreeMap;
use support::*;

fn exercise_unlink(image: &Image, nodes: [u32; 3], mask: u8) -> u64 {
    let [previous, item, following] = nodes;
    let caller = assemble_artifact(
        &format!(
            ".export returned\ntsc\nsec\nsbc #3\ntcs\nsep #$20\n.a8\nlda #{}\nsta 1,s\nlda #{}\nsta 2,s\nlda #{}\nsta 3,s\nrep #$20\n.a16\njsl ${:06X}\nreturned:\ntsc\nclc\nadc #3\ntcs\nstp\nnop",
            item & 255,
            (item >> 8) & 255,
            item >> 16,
            image.entry
        ),
        0x040000,
    );
    let mut h = Harness::new(image, &caller.bytes, mask);
    let mut contents: BTreeMap<_, _> = nodes.into_iter().map(|n| (n, [0xa5; 8])).collect();
    for (node, field, value) in [
        (previous, 0, item),
        (following, 3, item),
        (item, 0, following),
        (item, 3, previous),
    ] {
        contents.get_mut(&node).unwrap()[field + 1..field + 4]
            .copy_from_slice(&value.to_le_bytes()[..3]);
    }
    for (&node, bytes) in &contents {
        h.bus.map(node - 1, bytes, true);
        h.bus.watched.extend(node - 1..node + 7);
    }
    let removed = contents[&item];
    let mut expected_trace = Vec::new();
    for offset in [3, 0] {
        expected_trace.extend((0..3).map(|i| (item + offset + i, Access::Read)));
    }
    for (node, field, value) in [(previous, 0, following), (following, 3, previous)] {
        contents.get_mut(&node).unwrap()[field + 1..field + 4]
            .copy_from_slice(&value.to_le_bytes()[..3]);
        expected_trace.extend((0..3).map(|i| {
            (
                node + field as u32 + i,
                Access::Write((value >> (i * 8)) as u8),
            )
        }));
    }
    let mut begin = None;
    let mut elapsed = None;
    for _ in 0..10000 {
        if h.cpu.is_instruction_boundary() {
            if h.cpu.pc() == image.entry {
                begin = Some(h.cpu.cycles());
            }
            if h.cpu.pc() == caller.symbols["returned"] {
                elapsed = Some(h.cpu.cycles() - begin.unwrap());
                break;
            }
        }
        h.cpu.tick(&mut h.bus, Inputs::default()).unwrap();
    }
    h.run();
    h.guards(mask);
    for (node, bytes) in contents {
        assert_eq!(&h.bus.ram[node as usize - 1..node as usize + 7], &bytes);
    }
    assert_eq!(&h.bus.ram[item as usize - 1..item as usize + 7], &removed);
    let trace: Vec<_> = h.bus.trace.iter().map(|(_, at, op)| (*at, *op)).collect();
    assert_eq!(trace, expected_trace);
    elapsed.expect("routine did not return within its budget")
}

#[test]
fn unlink_matches_independent_reference_with_banked_and_aliased_nodes() {
    // Exercise actual compilation and assembly with both checkout conventions.
    for newline in ["\n", "\r\n"] {
        let source = fixture("unlink.act").replace('\n', newline);
        let assembly = fixture("unlink.s").replace('\n', newline);
        let mut reference = compile("PROC Remove(BYTE POINTER item) RETURN", false);
        let code = assemble(&assembly, reference.entry);
        reference
            .segments
            .iter_mut()
            .find(|s| s.executable)
            .unwrap()
            .bytes = code.clone();
        reference.routines[0].size = code.len() as u32;
        let reference = Image::from_json(&reference.to_json().unwrap()).unwrap();
        assert_eq!(reference.routines[0].size, 127);
        for optimize in [false, true] {
            let image = compile(&source, optimize);
            assert!(image.routines[0].size <= 223);
            for nodes in [
                [0x21ffff, 0x32fffc, 0x43fffe],
                [0x32fffc, 0x43fffe, 0x21ffff],
                [0x43fffe, 0x21ffff, 0x32fffc],
                [0x21ffff, 0x32fffc, 0x21ffff],
                [0x43fffe; 3],
            ] {
                for mask in [0, 4] {
                    assert_eq!(exercise_unlink(&reference, nodes, mask), 200);
                    let cycles = exercise_unlink(&image, nodes, mask);
                    assert!(cycles <= 420, "optimize={optimize}: {cycles} cycles");
                }
            }
        }
    }
}
