mod common;
use actionc::compiler::native::{NativeCompileOptions, compile_file};
use actionc_vm68k_tests::Machine;

#[test]
fn runtime_integer_operations_match_host_oracle() {
    for (kind, bits, signed) in [
        ("BYTE", 8, false),
        ("CARD", 16, false),
        ("INT", 16, true),
        ("LONGCARD", 32, false),
        ("LONGINT", 32, true),
    ] {
        let source = common::Source::new(&format!(
            "{kind} a,b,sum,difference,band,bor,bxor,negative,shl,shr\nBYTE count,eq,ne,lt,le,gt,ge\nLONGINT widened\nPROC Entry()\nsum=a+b difference=a-b band=a AND b bor=a OR b bxor=a XOR b negative=-a\nshl=a LSH count shr=a RSH count\neq=a=b ne=a#b lt=a<b le=a<=b gt=a>b ge=a>=b widened=LONGINT(a)\nRETURN"
        ));
        let mask = u32::MAX >> (32 - bits);
        let sign = 1u32 << (bits - 1);
        let values = [0, 1, 2, sign - 1, sign, mask - 1, mask, 0x12345678 & mask];
        for optimize in [false, true] {
            let compiled = compile_file(
                &source.0,
                &NativeCompileOptions {
                    optimize,
                    ..Default::default()
                },
            )
            .unwrap();
            let image = &compiled.image;
            for (index, &a) in values.iter().enumerate() {
                for &b in &values {
                    for count in [0, 1, bits - 1, bits, bits + 1, 63, 64, 255] {
                        let mut vm = Machine::from_image(image).unwrap();
                        for (name, value) in [("a", a), ("b", b), ("count", count)] {
                            vm.write_scalar(image.symbol(name).unwrap(), value).unwrap();
                        }
                        vm.run(2000).assert_completed();
                        let number = |v: u32| {
                            if signed && v & sign != 0 {
                                i64::from(v) - (1i64 << bits)
                            } else {
                                i64::from(v)
                            }
                        };
                        for (name, expected) in [
                            ("sum", a.wrapping_add(b) & mask),
                            ("difference", a.wrapping_sub(b) & mask),
                            ("band", a & b),
                            ("bor", a | b),
                            ("bxor", a ^ b),
                            ("negative", a.wrapping_neg() & mask),
                            (
                                "shl",
                                if count >= bits {
                                    0
                                } else {
                                    a.wrapping_shl(count) & mask
                                },
                            ),
                            ("shr", if count >= bits { 0 } else { a >> count }),
                            ("eq", (a == b) as u32),
                            ("ne", (a != b) as u32),
                            ("lt", (number(a) < number(b)) as u32),
                            ("le", (number(a) <= number(b)) as u32),
                            ("gt", (number(a) > number(b)) as u32),
                            ("ge", (number(a) >= number(b)) as u32),
                            ("widened", number(a) as u32),
                        ] {
                            assert_eq!(
                                vm.read_scalar(image.symbol(name).unwrap()).unwrap(),
                                expected,
                                "{kind}/{optimize}/{index}/{a:x}/{b:x}/{count}/{name}"
                            );
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn runtime_loops_if_and_case_preserve_control_flow() {
    let source = common::Source::new(
        "CARD limit,total,selected\nPROC Entry()\nCARD i\ntotal=0\nFOR i=0 TO limit DO\nIF i AND 1 THEN total==+i ELSE total==+2 FI\nOD\nCASE limit OF\nWHEN 0 THEN\nselected=10\nWHEN 1 TO 4 THEN\nselected=20\nWHEN 100 THEN\nselected=30\nELSE\nselected=40\nESAC\nRETURN",
    );
    for optimize in [false, true] {
        let image = compile_file(
            &source.0,
            &NativeCompileOptions {
                optimize,
                ..Default::default()
            },
        )
        .unwrap()
        .image;
        for limit in [0, 1, 4, 10, 100] {
            let mut vm = Machine::from_image(&image).unwrap();
            vm.write_scalar(image.symbol("limit").unwrap(), limit)
                .unwrap();
            vm.run(50000).assert_completed();
            let expected: u32 = (0..=limit).map(|i| if i & 1 != 0 { i } else { 2 }).sum();
            assert_eq!(
                vm.read_scalar(image.symbol("total").unwrap()).unwrap(),
                expected
            );
            assert_eq!(
                vm.read_scalar(image.symbol("selected").unwrap()).unwrap(),
                if limit == 0 {
                    10
                } else if limit <= 4 {
                    20
                } else if limit == 100 {
                    30
                } else {
                    40
                }
            );
        }
    }
}

#[test]
fn cyclic_parallel_edge_copies_execute_on_loop_backedges() {
    use actionc::{
        mir68k::{self, *},
        nir::*,
        target::ByteSize,
    };
    let ast = actionc::parser::parse(
        &actionc::lexer::tokenize("CARD result PROC Entry() RETURN").unwrap(),
    )
    .unwrap();
    let model = actionc::semantic::analyze_with_options(
        &ast,
        actionc::semantic::SemanticOptions::modern()
            .with_target(actionc::target::TargetId::Motorola68000),
    )
    .unwrap();
    let mut mir = mir68k::lower_program(&actionc::nir::lower_program(
        &actionc::semantic::ir::lower_program(&ast, &model),
    ))
    .unwrap();
    let ty = mir.data[0].ty.clone().unwrap();
    let boolean = NirType {
        kind: NirTypeKind::Bool,
        summary: "bool".into(),
        width: Some(ByteSize::ONE),
        pointer: false,
    };
    let word = ByteSize::new(2);
    let t = |i| Mir68kValue::Temp(TempId(i), word);
    let edge = |id, args| Mir68kEdge {
        target: BlockId(id),
        args,
    };
    let r = &mut mir.routines[0];
    r.temps = (0..6)
        .map(|i| (TempId(i), if i == 3 { boolean.clone() } else { ty.clone() }))
        .collect();
    r.blocks = vec![
        Mir68kBlock {
            id: BlockId(0),
            params: vec![],
            ops: vec![],
            terminator: Mir68kTerminator::Goto(edge(
                1,
                vec![
                    Mir68kValue::U16(1),
                    Mir68kValue::U16(2),
                    Mir68kValue::U16(3),
                ],
            )),
        },
        Mir68kBlock {
            id: BlockId(1),
            params: (0..3).map(|i| (TempId(i), ty.clone())).collect(),
            ops: vec![Mir68kOp::Compare {
                dest: TempId(3),
                width: word,
                signed: false,
                operation: NirCompareOp::Eq,
                left: t(2),
                right: Mir68kValue::U16(0),
            }],
            terminator: Mir68kTerminator::Branch {
                condition: Mir68kValue::Temp(TempId(3), ByteSize::ONE),
                then_edge: edge(2, vec![t(0)]),
                else_edge: edge(3, vec![]),
            },
        },
        Mir68kBlock {
            id: BlockId(3),
            params: vec![],
            ops: vec![Mir68kOp::Binary {
                dest: TempId(4),
                width: word,
                signed: false,
                operation: NirBinaryOp::Sub,
                left: t(2),
                right: Mir68kValue::U16(1),
            }],
            terminator: Mir68kTerminator::Goto(edge(1, vec![t(1), t(0), t(4)])),
        },
        Mir68kBlock {
            id: BlockId(2),
            params: vec![(TempId(5), ty)],
            ops: vec![Mir68kOp::Store {
                address: Mir68kAddress {
                    base: Mir68kAddressBase::Static(NirStorageId::Global(match mir.data[0].id {
                        Mir68kDataId::Global(id) => id,
                        _ => panic!(),
                    })),
                    base_alignment: Some(word),
                    alignment_proof: None,
                    displacement: actionc::target::ByteOffset::ZERO,
                    index: None,
                    mode: Mir68kAddressMode::Static,
                },
                value: t(5),
                width: word,
                access: Mir68kAccess::NativeAlignedWord,
                volatile: false,
            }],
            terminator: Mir68kTerminator::Return {
                value: None,
                restore_frame_bytes: r.frame.extent,
            },
        },
    ];
    let machine = mir68k::materialize::materialize(&mir).unwrap();
    let image = mir68k::image::link(&mir, &machine, 0x10000).unwrap();
    let mut vm = Machine::from_image(&image).unwrap();
    vm.run(1000).assert_completed();
    assert_eq!(vm.read_scalar(image.symbol("result").unwrap()).unwrap(), 2);
}
