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
            assert!(image.routines[0].size <= if optimize { 144 } else { 223 });
            if optimize {
                assert_eq!(image.routines[0].fixed_frame, 0);
                assert_eq!(image.routines[0].spill_bytes, 0);
            }
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
                    if newline == "\n" && nodes == [0x21ffff, 0x32fffc, 0x43fffe] && mask == 0 {
                        eprintln!(
                            "unlink optimize={optimize}: {} bytes, {cycles} cycles, {} frame bytes",
                            image.routines[0].size, image.routines[0].fixed_frame
                        );
                    }
                    assert!(
                        cycles <= if optimize { 220 } else { 420 },
                        "optimize={optimize}: {cycles} cycles"
                    );
                }
            }
        }
    }
}

#[test]
fn pointer_leaves_match_stack_selection_for_swaps_chains_and_pressure() {
    use actionc::mir65816::{Mir65816Op, image::TemporaryHome};
    use support::context::{routine, symbol};
    let nodes = [0x21fffeu32, 0x32fffd, 0x43ffff, 0x54fffa];
    for (body, expected, resident) in [
        (
            "Node POINTER saved saved=p.next p.next=p.other p.other=saved",
            vec![(0, 1, 2), (0, 4, 1)],
            true,
        ),
        ("p.next=p.next.next.next", vec![(0, 1, 3)], true),
        ("p.other=p.next.other", vec![(0, 4, 3)], true),
        ("p.next=p.other", vec![(0, 1, 2)], true),
        (
            "Node POINTER a,b,c a=p.next b=p.other c=p.next.next a.other=b b.other=c c.other=a p.next=c",
            vec![(1, 4, 2), (2, 4, 2), (2, 4, 1), (0, 1, 2)],
            false,
        ),
    ] {
        let source = format!(
            "TYPE Cell=[BYTE tag Cell POINTER next,other] Cell POINTER selected \
            PROC Transform(BYTE prefix Cell POINTER p CARD suffix) {body} RETURN \
            PROC Main() Transform($A1,selected,$B2C3) RETURN"
        )
        .replace("Node POINTER", "Cell POINTER");
        for optimize in [false, true] {
            let prepared = prepare(&source, optimize);
            let image = prepared.compile(&layout()).unwrap().image;
            let address = routine(&image, "Transform");
            let map = image
                .routines
                .iter()
                .find(|r| r.address == address)
                .unwrap();
            if optimize {
                assert_eq!(
                    map.temporaries
                        .iter()
                        .any(|t| matches!(t.home, TemporaryHome::DirectPage { .. })),
                    resident,
                    "{body}"
                );
                if resident {
                    assert_eq!(map.fixed_frame, 0);
                }
            }
            // Force the existing stack selector by marking MIR accesses
            // observable after optimization. Access extents/order are identical;
            // the alternative uses byte transfers and no resident pointers.
            let mut stack = prepared.clone();
            for op in stack
                .mir
                .routines
                .iter_mut()
                .find(|r| r.name == "Transform")
                .unwrap()
                .blocks
                .iter_mut()
                .flat_map(|b| &mut b.ops)
            {
                match op {
                    Mir65816Op::Load { volatile, .. } | Mir65816Op::Store { volatile, .. } => {
                        *volatile = true
                    }
                    _ => {}
                }
            }
            let stack = stack.compile(&layout()).unwrap().image;
            let mut traces = Vec::new();
            for image in [&image, &stack] {
                let image = Image::from_json(&image.to_json().unwrap()).unwrap();
                let mut h = Harness::new(&image, &caller(image.entry), 0);
                let selected = symbol(&image, "selected") as usize;
                h.bus.ram[selected..selected + 3].copy_from_slice(&nodes[0].to_le_bytes()[..3]);
                let mut contents = [[0xa5; 9]; 4];
                for (i, &node) in nodes.iter().enumerate() {
                    for (field, target) in [(1, (i + 1) % 4), (4, (i + 2) % 4)] {
                        contents[i][field + 1..field + 4]
                            .copy_from_slice(&nodes[target].to_le_bytes()[..3]);
                    }
                    h.bus.map(node - 1, &contents[i], true);
                    h.bus.watched.extend(node - 1..node + 8);
                }
                for &(node, field, target) in &expected {
                    contents[node][field + 1..field + 4]
                        .copy_from_slice(&nodes[target].to_le_bytes()[..3]);
                }
                h.run();
                h.guards(0);
                for (i, &node) in nodes.iter().enumerate() {
                    assert_eq!(
                        &h.bus.ram[node as usize - 1..node as usize + 8],
                        &contents[i],
                        "{body}"
                    );
                }
                traces.push(
                    h.bus
                        .trace
                        .iter()
                        .map(|(_, at, access)| (*at, *access))
                        .collect::<Vec<_>>(),
                );
            }
            assert_eq!(traces[0], traces[1], "{body}");
        }
    }
}
