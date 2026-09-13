use actionc::target::{ByteSize, TargetId};
use actionc::{lexer, mir68k::*, nir, parser, semantic};

fn lower(source: &str, optimized: bool) -> Mir68kProgram {
    let ast = parser::parse(&lexer::tokenize(source).unwrap()).unwrap();
    let model = semantic::analyze_with_options(
        &ast,
        semantic::SemanticOptions::modern().with_target(TargetId::Motorola68000),
    )
    .unwrap();
    let program = nir::lower_program(&semantic::ir::lower_program(&ast, &model));
    let program = if optimized {
        nir::optimize_program(&program).unwrap()
    } else {
        program
    };
    lower_program(&program).unwrap()
}

#[test]
fn global_storage_descriptors_and_signed_long_facts_survive_lowering() {
    for optimized in [false, true] {
        let mir = lower(
            "LONGINT a,b,result LONGCARD ARRAY table(3)=[1 2] BYTE flag\nPROC Entry()\nresult=a-b flag=a<b RETURN",
            optimized,
        );
        assert_eq!(mir.entry, Some(mir.routines[0].id));
        let result = mir.data.iter().find(|d| d.name == "result").unwrap();
        assert_eq!(result.size.get(), 4);
        assert_eq!(result.zero_fill.get(), 4);
        assert_eq!(result.alignment.get(), 2);
        let table = mir.data.iter().find(|d| d.name == "table").unwrap();
        let backing = mir
            .data
            .iter()
            .find(|d| d.name == "table.__backing")
            .unwrap();
        assert_ne!(table.id, backing.id);
        assert_eq!(table.size.get(), 6);
        assert_eq!(backing.size.get(), 12);
        assert_eq!(backing.bytes, [0, 0, 0, 1, 0, 0, 0, 2]);
        assert_eq!(backing.zero_fill.get(), 4);
        assert!(matches!(
            table.relocations[0].target,
            Mir68kRelocationTarget::ArrayBacking(_)
        ));
        let ops: Vec<_> = mir
            .routines
            .iter()
            .flat_map(|r| &r.blocks)
            .flat_map(|b| &b.ops)
            .collect();
        assert!(ops.iter().any(
            |op| matches!(op, Mir68kOp::Binary { signed: true, width, .. } if width.get() == 4)
        ));
        assert!(ops.iter().any(
            |op| matches!(op, Mir68kOp::Compare { signed: true, width, .. } if width.get() == 4)
        ));
    }
}

#[test]
fn verifier_rejects_lost_storage_entry_types_and_callees() {
    let original = lower(
        "LONGINT a,result LONGINT FUNC Inc(LONGINT x) RETURN(x+1) PROC Entry() result=Inc(a) RETURN",
        false,
    );
    let mut bad = original.clone();
    bad.data.clear();
    assert!(verify::verify_contract(&bad).is_err());
    let mut bad = original.clone();
    bad.entry = None;
    assert!(verify::verify_contract(&bad).is_err());
    let mut bad = original.clone();
    bad.routines[0].temps[0].1.width = Some(ByteSize::ONE);
    assert!(verify::verify_contract(&bad).is_err());
    let mut bad = original;
    bad.routines.remove(0);
    assert!(verify::verify_contract(&bad).is_err());
}

#[test]
fn block_parameters_and_edge_values_survive_and_are_checked() {
    let ast = parser::parse(&lexer::tokenize("BYTE result PROC Entry() result=7 RETURN").unwrap())
        .unwrap();
    let model = semantic::analyze_with_options(
        &ast,
        semantic::SemanticOptions::modern().with_target(TargetId::Motorola68000),
    )
    .unwrap();
    let mut program = nir::lower_program(&semantic::ir::lower_program(&ast, &model));
    let r = &mut program.routines[0];
    let store = r.blocks[0].ops.pop().unwrap();
    let nir::NirOp::Store { place, ty, src } = store else {
        panic!("expected store")
    };
    let id = nir::TempId(r.temps.len() as u32);
    let block = nir::BlockId(1);
    r.temps.push(nir::NirTemp {
        id,
        ty: ty.clone(),
        def: nir::NirTempDef {
            block,
            op_index: None,
        },
    });
    r.blocks[0].terminator = nir::NirTerminator::Goto(nir::NirEdge {
        target: block,
        args: vec![src],
    });
    r.blocks.push(nir::NirBlock {
        id: block,
        label: "join".into(),
        params: vec![nir::NirBlockParam {
            dest: id,
            ty: ty.clone(),
        }],
        ops: vec![nir::NirOp::Store {
            place,
            ty: ty.clone(),
            src: nir::NirValue::Temp { id, ty },
        }],
        terminator: nir::NirTerminator::Return(None),
    });
    nir::verify_program(&program).unwrap();
    let mut mir = lower_program(&program).unwrap();
    assert_eq!(mir.routines[0].blocks[1].params[0].0, id);
    let Mir68kTerminator::Goto(edge) = &mut mir.routines[0].blocks[0].terminator else {
        panic!()
    };
    assert_eq!(edge.args.len(), 1);
    edge.args.clear();
    assert!(verify::verify_contract(&mir).is_err());
}
