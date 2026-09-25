use actionc::{
    lexer,
    mir65816::{self, Mir65816Op, emit, image},
    nir, parser, semantic,
    target::TargetId,
};

fn mir(source: &str, optimize: bool) -> mir65816::Mir65816Program {
    let ast = parser::parse(&lexer::tokenize(source).unwrap()).unwrap();
    let model = semantic::analyze_with_options(
        &ast,
        semantic::SemanticOptions::modern().with_target(TargetId::Wdc65816Native),
    )
    .unwrap();
    let p = nir::lower_program(&semantic::ir::lower_program(&ast, &model));
    let p = if optimize {
        nir::optimize_program_with_promotion(&p, nir::NirPromotionPolicy::Native65816).unwrap()
    } else {
        p
    };
    mir65816::lower_program(&p).unwrap()
}

#[test]
fn symbolic_addresses_use_exact_direct_fixups_without_pointer_scratch() {
    for optimize in [false, true] {
        let p = mir(
            "TYPE Pair=[BYTE first BYTE last] Pair object BYTE initialized=[42] ADDRESS a,b PROC Main() a=ADDRESS(@initialized) b=ADDRESS(@object.last) RETURN",
            optimize,
        );
        let machine = emit::materialize(&p).unwrap();
        let mut sites = 0;
        for r in &p.routines {
            let code = &machine.routines.iter().find(|m| m.id == r.id).unwrap().code;
            for block in &r.blocks {
                for (i, op) in block.ops.iter().enumerate() {
                    if matches!(op, Mir65816Op::AddressOf { .. }) {
                        sites += 1;
                        let span = code.mir_spans[&(block.id, i)].clone();
                        assert!(span.len() <= 14);
                        assert_eq!(
                            code.fixups
                                .iter()
                                .filter(|f| span.contains(&f.offset))
                                .count(),
                            3
                        );
                    }
                }
            }
        }
        assert_eq!(sites, 2);
        let layout: image::LinkOptions = serde_json::from_str(r#"{"code_origin":65536,"data_origin":3342332,"zero_fill_origin":4325372,"stack_overflow":294912,"nmi_extra_stack":0,"imports":[]}"#).unwrap();
        image::link(&p, &machine, &layout)
            .unwrap()
            .verify()
            .unwrap();
        mir65816::o65::inspect(
            &mir65816::o65::write(&mir65816::o65::prepare(&p, &Default::default()).unwrap())
                .unwrap(),
        )
        .unwrap();
    }
}

#[test]
fn constant_address_chains_fold_only_interior_offsets_and_keep_frames() {
    for optimize in [false, true] {
        for offset in [0, 1, 255, 256, 299, 300, 65535] {
            let p = mir(
                &format!(
                    "BYTE ARRAY bytes(300) ADDRESS result PROC Main() result=ADDRESS(@bytes({offset})) RETURN"
                ),
                optimize,
            );
            let machine = emit::materialize(&p).unwrap();
            let r = p.routines.iter().find(|r| r.name == "Main").unwrap();
            let m = machine.routines.iter().find(|m| m.id == r.id).unwrap();
            m.frame.verify_stack(r).unwrap();
            assert_eq!(m.frame.temps.len(), r.temps.len());
            assert_eq!(machine.prepared, p);
            let sites: Vec<_> = r
                .blocks
                .iter()
                .flat_map(|b| {
                    b.ops.iter().enumerate().filter_map(move |(i, op)| {
                        matches!(op, Mir65816Op::AddressOf { .. }).then_some((b.id, i))
                    })
                })
                .collect();
            assert_eq!(sites.len(), 2);
            let producer = &m.code.mir_spans[&sites[0]];
            let consumer = &m.code.mir_spans[&sites[1]];
            if offset < 300 {
                assert!(producer.is_empty());
                assert!(consumer.len() <= 14);
                let refs: Vec<_> = m
                    .code
                    .fixups
                    .iter()
                    .filter(|f| consumer.contains(&f.offset))
                    .collect();
                assert_eq!(refs.len(), 3);
                assert!(refs.iter().all(|f| f.addend == offset));
            } else {
                assert!(!producer.is_empty());
                assert!(consumer.len() > 14);
            }
        }
    }
}

#[test]
fn constant_byte_accesses_are_direct_but_volatile_and_one_past_keep_fallback() {
    for optimize in [false, true] {
        for volatile in [false, true] {
            for offset in [0, 299, 300] {
                let mut p = mir(
                    &format!(
                        "BYTE ARRAY bytes(300) BYTE input=$7100,output=$7101 PROC Main() output=bytes({offset}) bytes({offset})=input RETURN"
                    ),
                    optimize,
                );
                if volatile {
                    for op in p
                        .routines
                        .iter_mut()
                        .flat_map(|r| &mut r.blocks)
                        .flat_map(|b| &mut b.ops)
                    {
                        match op {
                            Mir65816Op::Load { volatile, .. }
                            | Mir65816Op::Store { volatile, .. } => *volatile = true,
                            _ => {}
                        }
                    }
                }
                let m = emit::materialize(&p).unwrap();
                let mut selected = 0;
                for r in &p.routines {
                    let code = &m.routines.iter().find(|m| m.id == r.id).unwrap().code;
                    for block in &r.blocks {
                        for (i, op) in block.ops.iter().enumerate() {
                            let address = match op {
                                Mir65816Op::Load { address, .. }
                                | Mir65816Op::Store { address, .. } => address,
                                _ => continue,
                            };
                            if address.index.is_none() {
                                continue;
                            }
                            selected += 1;
                            let span = &code.mir_spans[&(block.id, i)];
                            if !volatile && offset < 300 {
                                assert!(span.len() <= 8);
                                assert_eq!(
                                    code.fixups
                                        .iter()
                                        .filter(|f| span.contains(&f.offset))
                                        .count(),
                                    1
                                );
                            } else {
                                assert!(span.len() > 8);
                            }
                        }
                    }
                }
                assert_eq!(selected, 2);
            }
        }
    }
}
