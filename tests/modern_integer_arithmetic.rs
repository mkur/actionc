use actionc::nir::NirBinaryOp;
use actionc::semantic::{SemanticOptions, analyze_with_options};
use actionc::target::{ByteSize, TargetId};

#[test]
fn arithmetic_fault_preserves_prior_fixed_local_stores_for_error() {
    use actionc::nir::{NirOp, NirStorageId, NirValue, direct_storage_id};

    for operation in ["/", "MOD"] {
        for divisor in ["b", "2"] {
            let source = format!(
                "CARD a,b,output PROC Main() CARD state,q \
                 state=41 q=a {operation} {divisor} state=42 output=state RETURN"
            );
            let ast = actionc::parser::parse(&actionc::lexer::tokenize(&source).unwrap()).unwrap();
            let model = analyze_with_options(&ast, SemanticOptions::modern()).unwrap();
            let semir = actionc::semantic::ir::lower_program(&ast, &model);
            let nir = actionc::nir::lower_program(&semir);
            let main = nir
                .routines
                .iter()
                .find(|routine| routine.name == "Main")
                .unwrap();
            let state = NirStorageId::Local(
                main.locals
                    .iter()
                    .find(|local| local.name == "state")
                    .unwrap()
                    .id,
            );
            let optimized = actionc::nir::optimize_program(&nir).unwrap();
            let main = optimized
                .routines
                .iter()
                .find(|routine| routine.name == "Main")
                .unwrap();
            let ops: Vec<_> = main.blocks.iter().flat_map(|block| &block.ops).collect();
            let saved = ops.iter().position(|op| {
                matches!(op,
                    NirOp::Store { place, src: NirValue::ConstU16(41), .. }
                    if direct_storage_id(place) == Some(state)
                )
            });
            if divisor == "b" {
                let fault = ops
                    .iter()
                    .position(|op| {
                        matches!(
                            op,
                            NirOp::Binary {
                                op: NirBinaryOp::Div | NirBinaryOp::Mod,
                                ..
                            }
                        )
                    })
                    .expect("dead result must not remove a possible fault");
                assert!(
                    saved.is_some_and(|saved| saved < fault),
                    "{operation}: {main:#?}"
                );
            } else {
                assert!(
                    saved.is_none(),
                    "a proven nonzero operation needs no error-observable store"
                );
            }
        }
    }
}

#[test]
fn typed_array_bound_folding_preserves_large_shift_counts() {
    for expression in ["(1 LSH 16)+1", "(256 RSH 16)+1", "(INT(-513)/INT(256))+3"] {
        let source = format!("BYTE ARRAY table({expression}) PROC Main() RETURN");
        let ast = actionc::parser::parse(&actionc::lexer::tokenize(&source).unwrap()).unwrap();
        let model = analyze_with_options(&ast, SemanticOptions::modern()).unwrap();
        let semir = actionc::semantic::ir::lower_program(&ast, &model);
        let declaration = semir
            .modules
            .iter()
            .flat_map(|module| &module.items)
            .find_map(|item| {
                if let actionc::semantic::ir::SemItem::Declaration(declaration) = item {
                    Some(declaration)
                } else {
                    None
                }
            })
            .unwrap();
        if let actionc::semantic::ir::SemDeclarationStorage::Array { array_type, .. } =
            &declaration.storage
        {
            assert_eq!(array_type.length, Some(1), "{expression}");
        } else {
            panic!("array declaration expected");
        }
    }
}

#[test]
fn native_canaries_preserve_integer_division_domain_and_source_width() {
    let source = "INT s,t CARD u,v,q,r PROC Main() q=s/t r=s MOD t q=u/v r=u MOD v RETURN";
    let ast = actionc::parser::parse(&actionc::lexer::tokenize(source).unwrap()).unwrap();
    for target in [
        TargetId::Motorola68000,
        TargetId::Wdc65816Native,
        TargetId::Wdc65816Small,
    ] {
        let model =
            analyze_with_options(&ast, SemanticOptions::modern().with_target(target)).unwrap();
        let semir = actionc::semantic::ir::lower_program(&ast, &model);
        let nir = actionc::nir::lower_program(&semir);
        let actual: Vec<_> = if target == TargetId::Motorola68000 {
            let mir = actionc::mir68k::lower_program(&nir).unwrap();
            mir.routines
                .iter()
                .flat_map(|r| &r.blocks)
                .flat_map(|b| &b.ops)
                .filter_map(|op| {
                    if let actionc::mir68k::Mir68kOp::Binary {
                        operation,
                        width,
                        signed,
                        ..
                    } = op
                    {
                        Some((*operation, *width, *signed))
                    } else {
                        None
                    }
                })
                .collect()
        } else {
            let mir = actionc::mir65816::lower_program(&nir).unwrap();
            mir.routines
                .iter()
                .flat_map(|r| &r.blocks)
                .flat_map(|b| &b.ops)
                .filter_map(|op| {
                    if let actionc::mir65816::Mir65816Op::Binary {
                        operation,
                        width,
                        signed,
                        ..
                    } = op
                    {
                        Some((*operation, *width, *signed))
                    } else {
                        None
                    }
                })
                .collect()
        };
        assert_eq!(
            actual,
            vec![
                (NirBinaryOp::Div, ByteSize::new(2), true),
                (NirBinaryOp::Mod, ByteSize::new(2), true),
                (NirBinaryOp::Div, ByteSize::new(2), false),
                (NirBinaryOp::Mod, ByteSize::new(2), false)
            ],
            "{target:?}"
        );
    }
}
