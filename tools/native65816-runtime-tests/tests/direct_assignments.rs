mod support;
use actionc::mir65816::image::Image;
use actionc_vm::native65816::Access;
use support::*;

fn image(source: &str, optimize: bool, origin: u32) -> Image {
    let mut options = layout();
    options.data_origin = origin;
    let compiled = prepare(source, optimize).compile(&options).unwrap();
    Image::from_json(&compiled.image.to_json().unwrap()).unwrap()
}

#[test]
fn direct_assignments_copy_exact_bytes_across_banks_without_scratch_writes() {
    for (ty, bytes) in [
        ("BYTE", 1),
        ("CARD", 2),
        ("BYTE POINTER", 3),
        ("LONGCARD", 4),
    ] {
        for optimize in [false, true] {
            for origin in [0x120000, 0x12ffff] {
                let image = image(
                    &format!("{ty} p,q PROC Main() q=p RETURN"),
                    optimize,
                    origin,
                );
                let p = context::symbol(&image, "p");
                let q = context::symbol(&image, "q");
                for mask in [0, 4] {
                    let mut h = Harness::new(&image, &caller(image.entry), mask);
                    let payload = [0x34, 0x12, 0xfe, 0x81];
                    h.bus.ram[p as usize..p as usize + bytes].copy_from_slice(&payload[..bytes]);
                    h.bus.ram[q as usize..q as usize + bytes].fill(0xa5);
                    h.run();
                    h.guards(mask);
                    assert_eq!(
                        &h.bus.ram[p as usize..p as usize + bytes],
                        &payload[..bytes]
                    );
                    assert_eq!(
                        &h.bus.ram[q as usize..q as usize + bytes],
                        &payload[..bytes]
                    );
                    let reads: Vec<_> = h
                        .bus
                        .reads
                        .iter()
                        .copied()
                        .filter(|a| (p..p + bytes as u32).contains(a))
                        .collect();
                    assert_eq!(reads, (p..p + bytes as u32).collect::<Vec<_>>());
                    let writes: Vec<_> = h
                        .bus
                        .writes
                        .iter()
                        .copied()
                        .filter(|(a, _)| (q..q + bytes as u32).contains(a))
                        .collect();
                    assert_eq!(
                        writes,
                        payload[..bytes]
                            .iter()
                            .enumerate()
                            .map(|(i, b)| (q + i as u32, *b))
                            .collect::<Vec<_>>()
                    );
                    assert!(
                        h.bus
                            .writes
                            .iter()
                            .all(|(a, _)| !(0x2080..0x20c0).contains(a))
                    );
                }
            }
        }
    }
}

#[test]
fn direct_frame_and_parameter_assignments_preserve_values_and_guards() {
    for (ty, bytes) in [
        ("BYTE", 1),
        ("CARD", 2),
        ("BYTE POINTER", 3),
        ("LONGCARD", 4),
    ] {
        let source = format!(
            "{ty} p,argument,q,localResult,mutableResult,directResult \
             PROC ReadOnly({ty} incoming) directResult=incoming RETURN \
             PROC Work({ty} incoming) {ty} first,second \
             first=incoming second=first localResult=second \
             first=p q=first incoming=p mutableResult=incoming RETURN \
             PROC Main() ReadOnly(argument) Work(argument) RETURN"
        );
        for optimize in [false, true] {
            let image = compile(&source, optimize);
            let p = context::symbol(&image, "p") as usize;
            let argument = context::symbol(&image, "argument") as usize;
            for mask in [0, 4] {
                let mut h = Harness::new(&image, &caller(image.entry), mask);
                let payload = [0x34, 0x12, 0xfe, 0x81];
                let incoming = [0x56, 0x78, 0x23, 0x91];
                h.bus.ram[p..p + bytes].copy_from_slice(&payload[..bytes]);
                h.bus.ram[argument..argument + bytes].copy_from_slice(&incoming[..bytes]);
                h.run();
                h.guards(mask);
                for (name, expected) in [
                    ("q", payload),
                    ("mutableResult", payload),
                    ("localResult", incoming),
                    ("directResult", incoming),
                ] {
                    let at = context::symbol(&image, name) as usize;
                    assert_eq!(
                        &h.bus.ram[at..at + bytes],
                        &expected[..bytes],
                        "{ty}/{optimize}/{name}"
                    );
                }
            }
        }
    }
}

#[test]
fn direct_pointer_fields_preserve_neighbor_bytes() {
    let source = "TYPE Pair=[BYTE before BYTE POINTER p BYTE POINTER q BYTE after] Pair object PROC Main() object.q=object.p RETURN";
    for optimize in [false, true] {
        let image = image(source, optimize, 0x12fffe);
        let at = context::symbol(&image, "object") as usize;
        let mut h = Harness::new(&image, &caller(image.entry), 0);
        h.bus.ram[at..at + 8].copy_from_slice(&[0xa5, 0x34, 0x12, 0xfe, 0xcc, 0xcc, 0xcc, 0x5a]);
        h.run();
        h.guards(0);
        assert_eq!(
            &h.bus.ram[at..at + 8],
            &[0xa5, 0x34, 0x12, 0xfe, 0x34, 0x12, 0xfe, 0x5a]
        );
        assert_eq!(
            h.bus
                .writes
                .iter()
                .filter(|(a, _)| (at..at + 8).contains(&(*a as usize)))
                .count(),
            3
        );
    }
}

#[test]
fn overlapping_absolute_and_indirect_assignments_still_capture_before_writing() {
    for indirect in [false, true] {
        let source = if indirect {
            "TYPE Cell=[BYTE POINTER value] Cell POINTER p=$7110,q=$7113 PROC Main() q.value=p.value RETURN"
        } else {
            "BYTE POINTER p=$7100,q=$7101 PROC Main() q=p RETURN"
        };
        for optimize in [false, true] {
            let image = compile(source, optimize);
            let mut h = Harness::new(&image, &caller(image.entry), 0);
            h.bus.ram[0x7100..0x7104].copy_from_slice(&[0x12, 0x34, 0x56, 0x78]);
            h.bus.ram[0x7110..0x7116].copy_from_slice(&[0, 0x71, 0, 1, 0x71, 0]);
            h.bus.watched.extend(0x7100..0x7104);
            h.run();
            h.guards(0);
            assert_eq!(&h.bus.ram[0x7100..0x7104], &[0x12, 0x12, 0x34, 0x56]);
            assert_eq!(
                h.bus
                    .trace
                    .iter()
                    .map(|(_, a, k)| (*a, *k))
                    .collect::<Vec<_>>(),
                vec![
                    (0x7100, Access::Read),
                    (0x7101, Access::Read),
                    (0x7102, Access::Read),
                    (0x7101, Access::Write(0x12)),
                    (0x7102, Access::Write(0x34)),
                    (0x7103, Access::Write(0x56)),
                ]
            );
        }
    }
}
