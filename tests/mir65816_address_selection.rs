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
