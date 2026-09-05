//! Development-only semantic/NIR coverage. Public modes remain gated until
//! classic and MIR6502 execution plus aggregate initialization are validated.
use actionc::ast::{FundType, Program};
use actionc::includes::{ModuleLoadOptions, load_compilation_from_provider};
use actionc::lexer::tokenize;
use actionc::nir::{self, NirCallee, NirOp, NirPlaceKind};
use actionc::parser::parse;
use actionc::semantic::{self, RecordFieldStorage, SemanticModel, SemanticOptions, ValueType, ir};
use actionc::source::{InMemorySourceProvider, SourceOrigin};
use actionc::target::TargetId;

fn analyze(source: &str, target: TargetId) -> (Program, SemanticModel) {
    let program = parse(&tokenize(source).unwrap()).unwrap();
    let model = semantic::analyze_with_options(&program, options(target))
        .unwrap_or_else(|errors| panic!("{source}\n{errors:#?}"));
    (program, model)
}

fn options(target: TargetId) -> SemanticOptions {
    SemanticOptions {
        embedded_record_arrays: true,
        ..SemanticOptions::modern().with_target(target)
    }
}

fn lower(source: &str, target: TargetId) -> nir::NirProgram {
    let (program, model) = analyze(source, target);
    let semir = ir::lower_program(&program, &model);
    let nir = nir::lower_program(&semir);
    nir::verify_program(&nir)
        .unwrap_or_else(|errors| panic!("{}\n{errors:#?}", nir::format_program(&nir)));
    nir::optimize_program(&nir).expect("optimized array-place NIR must verify");
    nir
}

#[test]
fn embedded_array_places_lower_direct_pointer_nested_local_and_record_array_accesses() {
    let source = "TYPE Point=[INT x,y] \
        TYPE Buffer=[BYTE tag INT ARRAY x(100),y(100) Point ARRAY points(3)] \
        TYPE Outer=[BYTE lead Buffer inner] \
        Buffer data,absolute=$6000 Outer wrapper Buffer ARRAY rows(3) Buffer POINTER p \
        INT i,j,result INT POINTER ip \
        PROC Take(INT ARRAY a) result=a(0) RETURN \
        PROC Main() Buffer local \
        p=data data.x(i)=data.y(j) p.x(i)=p.y(j) \
        wrapper.inner.x(i)=wrapper.inner.y(j) rows(i).x(j)=42 \
        p.points(i).x=p.points(j).y p.points(i)=p.points(j) \
        local.x(i)=absolute.y(j) ip=rows(i).x Take(p.y) ip=@p.x RETURN";
    for target in [
        TargetId::Atari6502,
        TargetId::Wdc65816Small,
        TargetId::Wdc65816Native,
        TargetId::Motorola68000,
    ] {
        lower(source, target);
    }
}

#[test]
fn embedded_array_places_carry_field_identity_full_extent_and_element_stride_in_semir() {
    let (program, model) = analyze(
        "TYPE Buffer=[INT ARRAY x(100),y(100)] Buffer data INT POINTER p \
         PROC Main() p=data.y RETURN",
        TargetId::Atari6502,
    );
    let semir = ir::lower_program(&program, &model);
    let main = semir.modules[0]
        .items
        .iter()
        .find_map(|item| match item {
            ir::SemItem::Routine(routine) => Some(routine),
            _ => None,
        })
        .unwrap();
    let ir::SemStmt::Assign { value, .. } = &main.body[0] else {
        panic!("assignment");
    };
    let ir::SemExprKind::ArrayDecay(decay) = &value.kind else {
        panic!("inline array decay: {value:?}");
    };
    assert_eq!(decay.origin, ir::SemArrayOrigin::RecordField);
    assert_eq!(decay.element_type, ValueType::fund(FundType::Int));
    let ir::SemLValueKind::Field { field, .. } = &decay.array.kind else {
        panic!("field place");
    };
    assert_eq!((field.offset, field.size), (Some(200), 200));
    let RecordFieldStorage::InlineArray { array_type, stride } = &field.storage else {
        panic!("array shape");
    };
    assert_eq!((array_type.length, *stride), (Some(100), 2));
    assert_eq!(
        model.fields[field.id.unwrap().0].owner,
        field.owner.unwrap()
    );
}

#[test]
fn embedded_array_layout_queries_do_not_execute_indexes() {
    let source = "TYPE Buffer=[INT ARRAY x(100),y(100)] Buffer ARRAY rows(3) CARD a,b,c,d,e \
        CARD FUNC Index() RETURN(1) PROC Main() \
        a=SIZEOF(rows(Index()).x) b=ELEMENTS(rows(Index()).y) \
        c=SIZEOF(rows(Index()).x(0)) d=OFFSETOF(Buffer,y) e=ALIGNOF(rows(0).x) RETURN";
    let nir = lower(source, TargetId::Atari6502);
    let main = nir
        .routines
        .iter()
        .find(|routine| routine.name == "Main")
        .unwrap();
    let values = main
        .blocks
        .iter()
        .flat_map(|block| &block.ops)
        .filter_map(|op| {
            if let NirOp::Store { src, .. } = op {
                Some(src.clone())
            } else {
                None
            }
        })
        .collect::<Vec<_>>();
    assert_eq!(values, [200, 100, 2, 200, 1].map(nir::NirValue::ConstU16));
    assert!(
        !main
            .blocks
            .iter()
            .flat_map(|block| &block.ops)
            .any(|op| matches!(op, NirOp::Call { .. }))
    );
}

#[test]
fn embedded_array_places_capture_destination_before_index_and_rhs_calls() {
    let source = "TYPE Buffer=[INT ARRAY x(100),y(100)] Buffer POINTER p Buffer ARRAY rows(3) \
        BYTE FUNC Index() RETURN(1) INT FUNC Value() RETURN(7) BYTE FUNC Row() RETURN(2) \
        PROC Take(INT ARRAY values) RETURN PROC Main() \
        p.x(Index())=Value() Take(rows(Row()).y) RETURN";
    let nir = lower(source, TargetId::Atari6502);
    let main = nir
        .routines
        .iter()
        .find(|routine| routine.name == "Main")
        .unwrap();
    let ops = main
        .blocks
        .iter()
        .flat_map(|block| &block.ops)
        .collect::<Vec<_>>();
    let calls = ops
        .iter()
        .enumerate()
        .filter_map(|(index, op)| match op {
            NirOp::Call {
                callee: NirCallee::User { name, .. },
                ..
            } => Some((index, name.as_str())),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        calls.iter().map(|(_, name)| *name).collect::<Vec<_>>(),
        ["Index", "Value", "Row", "Take"]
    );
    let captured = ops
        .iter()
        .position(|op| {
            matches!(
                op,
                NirOp::AddrOf {
                    place: nir::NirPlace {
                        kind: NirPlaceKind::Field { .. },
                        ..
                    },
                    ..
                }
            )
        })
        .unwrap();
    assert!(
        captured < calls[0].0,
        "capture pointer field base before Index can change p"
    );
}

#[test]
fn embedded_array_places_keep_volatile_loads_and_stores() {
    let nir = lower(
        "TYPE Buffer=[BYTE ARRAY data(8)] VOLATILE Buffer hw=$D000 BYTE value \
        PROC Main() value=hw.data(1) hw.data(2)=value RETURN",
        TargetId::Atari6502,
    );
    for nir in [nir.clone(), nir::optimize_program(&nir).unwrap()] {
        let ops = nir
            .routines
            .iter()
            .flat_map(|routine| &routine.blocks)
            .flat_map(|block| &block.ops);
        let accesses = ops
            .filter(|op| matches!(op, NirOp::VolatileLoad { .. } | NirOp::VolatileStore { .. }))
            .count();
        assert_eq!(accesses, 2);
    }
}

#[test]
fn embedded_array_places_reject_scalar_use_rebinding_and_wrong_decay_types() {
    for body in [
        "data.x=data.y",
        "data.x=1",
        "data.x==+1",
        "result=data.x",
        "result=data.x+1",
        "TakeByte(data.x)",
        "FOR data.x=0 TO 1 DO OD",
        "IF data.x THEN FI",
        "result=data.x(data.y)",
        "RETURN(data.x)",
        "data.points.x=1",
        "item=data.points",
    ] {
        let source = format!(
            "TYPE Point=[INT x] TYPE Buffer=[INT ARRAY x(4),y(4) Point ARRAY points(2)] \
            Buffer data Point item INT result PROC TakeByte(BYTE POINTER ptr) RETURN \
            INT FUNC Main() {body} RETURN(0)"
        );
        let program = parse(&tokenize(&source).unwrap()).unwrap();
        let errors =
            semantic::analyze_with_options(&program, options(TargetId::Atari6502)).unwrap_err();
        assert!(
            errors
                .iter()
                .any(|error| error.message.contains("embedded array")),
            "{body}: {errors:?}"
        );
    }
}

#[test]
fn embedded_array_places_allow_explicit_address_reinterpretation() {
    lower(
        "TYPE Buffer=[INT ARRAY values(4)] Buffer data BYTE POINTER p \
        PROC Main() p=BYTE POINTER(@data.values) RETURN",
        TargetId::Atari6502,
    );
}

#[test]
fn embedded_array_field_initializers_are_gated_until_static_address_lowering() {
    for initializer in [
        "data.values",
        "@data.values",
        "INT POINTER(@data.values)",
        "@data.values(1)",
    ] {
        let source = format!(
            "TYPE Buffer=[INT ARRAY values(2)] Buffer data INT POINTER p={initializer} \
             PROC Main() RETURN"
        );
        let program = parse(&tokenize(&source).unwrap()).unwrap();
        let errors = semantic::analyze_with_options(&program, options(TargetId::Atari6502))
            .expect_err("inline subobject initializers must not silently become zero storage");
        assert!(
            errors.iter().any(|error| error
                .message
                .contains("embedded array field initializers are not supported yet")),
            "{source}: {errors:#?}"
        );
    }
    lower(
        "TYPE Buffer=[INT ARRAY values(2)] Buffer data \
         CARD count=ELEMENTS(data.values) CARD size=SIZEOF(data.values)+1 \
         PROC Main() RETURN",
        TargetId::Atari6502,
    );
}

#[test]
fn embedded_array_place_verifier_rejects_truncated_strides_and_outside_fields() {
    let source = "TYPE Buffer=[INT ARRAY x(100),y(100)] Buffer data INT result \
        PROC Main() result=data.y(1) RETURN";
    let nir = lower(source, TargetId::Atari6502);
    let mut bad_stride = nir.clone();
    for op in bad_stride
        .routines
        .iter_mut()
        .flat_map(|routine| &mut routine.blocks)
        .flat_map(|block| &mut block.ops)
    {
        if let NirOp::Load {
            place:
                nir::NirPlace {
                    kind: NirPlaceKind::Index { elem_size, .. },
                    ..
                },
            ..
        } = op
        {
            *elem_size = nir::ByteSize::ONE;
        }
    }
    assert!(
        nir::verify_program(&bad_stride)
            .unwrap_err()
            .iter()
            .any(|error| error.message.contains("index stride"))
    );
    let mut bad_field = nir;
    for op in bad_field
        .routines
        .iter_mut()
        .flat_map(|routine| &mut routine.blocks)
        .flat_map(|block| &mut block.ops)
    {
        if let NirOp::AddrOf {
            place:
                nir::NirPlace {
                    kind: NirPlaceKind::Field { offset, .. },
                    ..
                },
            ..
        } = op
        {
            *offset = nir::ByteOffset::from(400u16);
        }
    }
    assert!(
        nir::verify_program(&bad_field)
            .unwrap_err()
            .iter()
            .any(|error| error.message.contains("field extent"))
    );
}

#[test]
fn embedded_array_places_preserve_module_owned_field_shapes_through_nir() {
    let root = SourceOrigin::host("project/main.act");
    let provider = InMemorySourceProvider::default()
        .with_source(
            root.clone(),
            b"MODULE App USE Shapes Shapes.Buffer value INT POINTER p CARD size \
            PROC Main() p=value.y value.x(1)=value.y(2) size=SIZEOF(value.y) RETURN ENDMODULE"
                .to_vec(),
        )
        .with_source(
            SourceOrigin::host("project/shapes.act"),
            b"MODULE Shapes PUBLIC CONST Count=100 \
            PUBLIC TYPE Buffer=[INT ARRAY x(Count),y(Count)] ENDMODULE"
                .to_vec(),
        );
    let loaded =
        load_compilation_from_provider(root, &provider, &ModuleLoadOptions::default()).unwrap();
    let model =
        semantic::analyze_compilation_with_options(&loaded, options(TargetId::Atari6502)).unwrap();
    let semir = ir::lower_compilation(&loaded, &model);
    let nir = nir::lower_program(&semir);
    nir::verify_program(&nir).unwrap();
    nir::optimize_program(&nir).unwrap();
    let record = model.layout.record_for_name("Shapes.Buffer").unwrap();
    assert_eq!(record.size, 400);
    for field in &record.fields {
        assert_eq!(field.size, 200);
    }
}

#[test]
fn embedded_array_places_cover_scalar_widths_and_offsets_above_one_page() {
    for target in [TargetId::Atari6502, TargetId::Motorola68000] {
        for element in ["BYTE", "CHAR", "INT", "CARD", "REAL"] {
            for length in [1, 129, 257] {
                let source = format!(
                    "TYPE Buffer=[BYTE ARRAY prefix(257) {element} ARRAY data({length}) BYTE tail] \
                    Buffer value {element} result BYTE index PROC Main() \
                    value.data(index)=value.data(0) result=value.data(index) RETURN"
                );
                lower(&source, target);
            }
        }
    }
}

#[test]
fn embedded_array_lowering_rejects_a_forged_scalar_load_from_the_array_place() {
    let (program, model) = analyze(
        "TYPE Buffer=[INT ARRAY values(4)] Buffer value INT POINTER p \
        PROC Main() p=value.values RETURN",
        TargetId::Atari6502,
    );
    let mut semir = ir::lower_program(&program, &model);
    let main = semir.modules[0]
        .items
        .iter_mut()
        .find_map(|item| match item {
            ir::SemItem::Routine(routine) => Some(routine),
            _ => None,
        })
        .unwrap();
    let ir::SemStmt::Assign { value, .. } = &mut main.body[0] else {
        panic!("assignment");
    };
    let ir::SemExprKind::ArrayDecay(decay) = &value.kind else {
        panic!("array decay");
    };
    value.kind = ir::SemExprKind::LValue(decay.array.clone());
    value.ty = ValueType::fund(FundType::Int);
    assert!(std::panic::catch_unwind(|| nir::lower_program(&semir)).is_err());
}

#[test]
fn embedded_array_places_match_the_nir_snapshot() {
    let nir = lower(
        "TYPE Buffer=[INT ARRAY x(100),y(100)] Buffer value INT result INT POINTER p \
        PROC Main() value.x(1)=value.y(2) result=value.x(1) p=value.y RETURN",
        TargetId::Atari6502,
    );
    assert_eq!(
        nir::format_program(&nir),
        include_str!("snapshots/embedded_record_array_places.nir")
    );
}
