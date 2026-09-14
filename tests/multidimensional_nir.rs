use actionc::nir::{self, NirBinaryOp, NirOp, NirPlaceKind, NirTypeKind, NirValue};
use actionc::semantic::{self, SemanticOptions};
use actionc::target::{ByteSize, TargetId, TargetLayout};

fn lower(source: &str, target: TargetId) -> nir::NirProgram {
    let ast = actionc::parser::parse(&actionc::lexer::tokenize(source).unwrap()).unwrap();
    let model = semantic::analyze_with_options(
        &ast,
        SemanticOptions {
            multidimensional_arrays: true,
            ..SemanticOptions::modern().with_target(target)
        },
    )
    .unwrap();
    let program = nir::lower_program(&semantic::ir::lower_program(&ast, &model));
    nir::verify_program(&program)
        .unwrap_or_else(|errors| panic!("{}\n{errors:#?}", nir::format_program(&program)));
    nir::optimize_program(&program).unwrap();
    program
}

fn ops(program: &nir::NirProgram) -> impl Iterator<Item = &NirOp> {
    program
        .routines
        .iter()
        .filter(|r| r.name == "Main")
        .flat_map(|r| &r.blocks)
        .flat_map(|b| &b.ops)
}

#[test]
fn multidimensional_nir_widens_each_coordinate_before_generated_arithmetic() {
    let source = "BYTE i,j,k LONGINT ARRAY volume(2,3,129) PROC Main() volume(i,j,k)=123456 RETURN";
    for target in [
        TargetId::Atari6502,
        TargetId::Wdc65816Small,
        TargetId::Wdc65816Native,
        TargetId::Motorola68000,
    ] {
        let program = lower(source, target);
        let bits = TargetLayout::for_target(target).address_integer_bits;
        let arithmetic = ops(&program)
            .filter_map(|op| match op {
                NirOp::Binary {
                    ty,
                    op: NirBinaryOp::Add | NirBinaryOp::Mul,
                    ..
                } => Some(ty),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(arithmetic.len(), 4, "{target:?}");
        assert!(arithmetic.iter().all(|ty| {
            ty.kind
                .integer()
                .is_some_and(|i| i.bits == bits && !i.signed)
        }));
        let casts = ops(&program).filter(|op| matches!(op,NirOp::Cast { from,to,.. }
            if from.width == Some(ByteSize::ONE) && to.kind.integer().is_some_and(|i| i.bits == bits))).count();
        assert_eq!(casts, 3, "{target:?}");
        let indexes = ops(&program)
            .filter_map(|op| match op {
                NirOp::Store { place, .. } if matches!(place.kind, NirPlaceKind::Index { .. }) => {
                    Some(place)
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(indexes.len(), 1);
        assert!(
            matches!(&indexes[0].kind,NirPlaceKind::Index { elem_size,index: NirValue::Temp { ty,.. },.. }
            if elem_size.get() == 4 && ty.kind.integer().unwrap().bits == bits)
        );
    }
}

#[test]
fn multidimensional_nir_captures_base_coordinates_then_rhs_and_compound_load() {
    let source = "TYPE Tile=[CARD ARRAY pixels(2,129)] Tile first,second Tile POINTER p \
        VOLATILE BYTE column=$0600 BYTE calls \
        BYTE FUNC Row() calls==+1 p=second RETURN(1) \
        CARD FUNC Value() calls==+1 first.pixels(1,128)=40 RETURN(2) \
        PROC Main() p=first p.pixels(Row(),column)==+Value() RETURN";
    for target in [TargetId::Atari6502, TargetId::Motorola68000] {
        let program = lower(source, target);
        for program in [program.clone(), nir::optimize_program(&program).unwrap()] {
            let operations = ops(&program).collect::<Vec<_>>();
            let calls = operations
                .iter()
                .enumerate()
                .filter_map(|(i, op)| match op {
                    NirOp::Call {
                        callee: nir::NirCallee::User { name, .. },
                        ..
                    } => Some((i, name.as_str())),
                    _ => None,
                })
                .collect::<Vec<_>>();
            assert_eq!(
                calls.iter().map(|(_, name)| *name).collect::<Vec<_>>(),
                ["Row", "Value"]
            );
            let column = operations
                .iter()
                .position(|op| matches!(op, NirOp::VolatileLoad { .. }))
                .unwrap();
            assert!(calls[0].0 < column && column < calls[1].0);
            assert!(
                operations[..calls[0].0]
                    .iter()
                    .any(|op| matches!(op, NirOp::AddrOf { .. }))
            );
            let read = operations.iter().position(|op| matches!(op,NirOp::Load { place,.. } if matches!(place.kind,NirPlaceKind::Index {..}))).unwrap();
            let write = operations.iter().position(|op| matches!(op,NirOp::Store { place,.. } if matches!(place.kind,NirPlaceKind::Index {..}))).unwrap();
            assert!(
                calls[1].0 < read && read < write,
                "compound must read the captured place after the RHS"
            );
        }
    }
}

#[test]
fn multidimensional_nir_native_counts_dimensions_and_queries_do_not_truncate() {
    let program = lower(
        "BYTE row LONGCARD column BYTE ARRAY big(2,LONGCARD(65537)) SIZE elementCount,bytes \
        PROC Main() big(row,column)=123 elementCount=ELEMENTS(big) bytes=SIZEOF(big) RETURN",
        TargetId::Motorola68000,
    );
    let big = program.globals.iter().find(|g| g.name == "big").unwrap();
    assert_eq!(big.array.as_ref().unwrap().length, Some(131074));
    assert_eq!(big.storage_size.get(), 131074);
    assert!(ops(&program).any(|op| matches!(op,NirOp::Binary { op:NirBinaryOp::Mul, right:NirValue::IntegerConst {bits:65537,ty},.. } if ty.bits == 32)));
    let queries = ops(&program)
        .filter(|op| {
            matches!(
                op,
                NirOp::Store {
                    src: NirValue::IntegerConst { bits: 131074, .. },
                    ..
                }
            )
        })
        .count();
    assert_eq!(queries, 2);
}

#[test]
fn multidimensional_nir_verifier_rejects_corrupt_counts_strides_and_indexes() {
    let original = lower(
        "BYTE row,column CARD ARRAY grid(2,3) PROC Main() grid(row,column)=42 RETURN",
        TargetId::Atari6502,
    );
    for fault in [
        "count",
        "stride",
        "extent",
        "index width",
        "index pointer",
        "arithmetic width",
    ] {
        let mut program = original.clone();
        let grid = program
            .globals
            .iter_mut()
            .find(|g| g.name == "grid")
            .unwrap();
        match fault {
            "count" => grid.array.as_mut().unwrap().length = Some(u32::MAX),
            "stride" => grid.array.as_mut().unwrap().elem_size = ByteSize::ZERO,
            "extent" => grid.array.as_mut().unwrap().length = Some(7),
            "arithmetic width" => {
                let right = program
                    .routines
                    .iter_mut()
                    .flat_map(|r| &mut r.blocks)
                    .flat_map(|b| &mut b.ops)
                    .find_map(|op| match op {
                        NirOp::Binary {
                            op: NirBinaryOp::Mul,
                            right,
                            ..
                        } => Some(right),
                        _ => None,
                    })
                    .unwrap();
                *right = NirValue::ConstU8(3);
            }
            _ => {
                let index = program
                    .routines
                    .iter_mut()
                    .flat_map(|r| &mut r.blocks)
                    .flat_map(|b| &mut b.ops)
                    .find_map(|op| match op {
                        NirOp::Store {
                            place:
                                nir::NirPlace {
                                    kind: NirPlaceKind::Index { index, .. },
                                    ..
                                },
                            ..
                        } => Some(index),
                        _ => None,
                    })
                    .unwrap();
                let NirValue::Temp { ty, .. } = index else {
                    panic!()
                };
                if fault == "index width" {
                    ty.width = Some(ByteSize::ONE);
                } else {
                    ty.kind = NirTypeKind::Pointer {
                        pointee: None,
                        address_space: TargetLayout::DATA_ADDRESS_SPACE,
                    };
                    ty.pointer = true;
                }
            }
        }
        let errors = nir::verify_program(&program).unwrap_err();
        let expected = match fault {
            "count" | "stride" => "invalid count, stride",
            "extent" => "smaller than its declared extent",
            "index width" => "type width mismatch",
            "index pointer" => "index coordinate must be an integer",
            "arithmetic width" => "explicitly widened ADDRESS operands",
            _ => unreachable!(),
        };
        assert!(
            errors.iter().any(|error| error.message.contains(expected)),
            "{fault}: {errors:?}"
        );
    }
}

#[test]
fn multidimensional_nir_matches_raw_and_optimized_snapshots() {
    let source = "BYTE row,column CARD ARRAY grid(3,129) PROC Main() grid(row,column)=42 RETURN";
    for (target, expected) in [
        (
            TargetId::Atari6502,
            include_str!("snapshots/multidimensional_6502.nir"),
        ),
        (
            TargetId::Motorola68000,
            include_str!("snapshots/multidimensional_68k.nir"),
        ),
    ] {
        let raw = lower(source, target);
        let optimized = nir::optimize_program(&raw).unwrap();
        let actual = format!(
            "; raw\n{}\n; optimized\n{}",
            nir::format_program(&raw),
            nir::format_program(&optimized)
        );
        assert_eq!(actual, expected.replace("\r\n", "\n"), "{target:?}");
    }
}

#[test]
fn multidimensional_nir_preserves_inline_and_local_partial_initializer_extents() {
    let source = "TYPE Tile=[BYTE tag CARD ARRAY grid(2,3)] Tile data=[1 2 3] \
        CARD ARRAY values(2,3)=[1] CARD POINTER ref=@data.grid(1,2) \
        PROC Main() CARD ARRAY local(2,3)=[5] \
        local(1,2)=values(1,2) data.grid(1,1)=local(0,0) RETURN";
    for target in [
        TargetId::Atari6502,
        TargetId::Wdc65816Small,
        TargetId::Wdc65816Native,
        TargetId::Motorola68000,
    ] {
        lower(source, target);
    }
}
