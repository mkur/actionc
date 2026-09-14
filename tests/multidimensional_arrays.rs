use actionc::ast::Program;
use actionc::lexer::tokenize;
use actionc::parser::parse;
use actionc::semantic::{self, SemanticModel, SemanticOptions, ir};
use actionc::target::TargetId;

fn options(target: TargetId) -> SemanticOptions {
    SemanticOptions {
        multidimensional_arrays: true,
        ..SemanticOptions::modern().with_target(target)
    }
}

fn analyze(source: &str, target: TargetId) -> (Program, SemanticModel) {
    let ast = parse(&tokenize(source).unwrap()).unwrap();
    let model = semantic::analyze_with_options(&ast, options(target))
        .unwrap_or_else(|errors| panic!("{source}\n{errors:#?}"));
    (ast, model)
}

fn lower(source: &str, target: TargetId) -> ir::SemProgram {
    let (ast, model) = analyze(source, target);
    ir::lower_program(&ast, &model)
}

fn declarations(program: &ir::SemProgram) -> impl Iterator<Item = &ir::SemDeclaration> {
    program
        .modules
        .iter()
        .flat_map(|module| &module.items)
        .filter_map(|item| match item {
            ir::SemItem::Declaration(decl) => Some(decl),
            _ => None,
        })
}

#[test]
fn multidimensional_semir_retains_coordinates_shape_and_evaluation_order() {
    let source = "CARD ARRAY grid(3,129) BYTE calls \
        BYTE FUNC Row() calls==+1 RETURN(2) BYTE FUNC Column() calls==+1 RETURN(128) \
        CARD FUNC Value() calls==+1 RETURN(1234) \
        PROC Main() grid(Row(),Column())=Value() grid(1,2)==+7 RETURN";
    let program = lower(source, TargetId::Atari6502);
    let main = program
        .modules
        .iter()
        .flat_map(|m| &m.items)
        .find_map(|item| match item {
            ir::SemItem::Routine(r) if r.symbol.name == "Main" => Some(r),
            _ => None,
        })
        .unwrap();
    let ir::SemStmt::Assign { target, value, .. } = &main.body[0] else {
        panic!()
    };
    let ir::SemLValueKind::MultiIndex(index) = &target.kind else {
        panic!("{target:?}")
    };
    assert_eq!(index.shape.dimensions(), [3, 129]);
    assert_eq!(index.coordinates.len(), 2);
    assert!(index.base.eval_order < index.coordinates[0].eval_order);
    assert!(index.coordinates[0].eval_order < index.coordinates[1].eval_order);
    assert!(index.coordinates[1].eval_order < value.eval_order);
    assert!(
        index
            .coordinates
            .iter()
            .all(|expr| matches!(expr.kind, ir::SemExprKind::Call(_)))
    );
    let ir::SemStmt::CompoundAssign { target, .. } = &main.body[1] else {
        panic!()
    };
    assert!(matches!(target.kind, ir::SemLValueKind::MultiIndex(_)));
    let grid = declarations(&program)
        .find(|decl| decl.symbol.name == "grid")
        .unwrap();
    let ir::SemDeclarationStorage::Array {
        array_type,
        length: Some(length),
        ..
    } = &grid.storage
    else {
        panic!()
    };
    assert_eq!(array_type.length(), Some(387));
    assert!(
        matches!(&length.kind, ir::SemExprKind::Literal(ir::SemLiteral::Constant(value)) if value.bits == 387)
    );
    assert!(ir::format_program(&program).contains("ARRAY(3,129)"));
}

#[test]
fn multidimensional_places_cover_records_locals_decay_and_unevaluated_queries() {
    let source = "TYPE Point=[INT x,y] TYPE Tile=[BYTE tag Point ARRAY pixels(2,3)] \
        TYPE Box=[BYTE lead Tile inner] Tile data,absolute=$6000 Tile ARRAY rows(2,3) \
        Tile POINTER p Box wrapper INT result BYTE i,j \
        Point POINTER pixel INT POINTER word CARD ARRAY grid(2,3) SIZE count,bytes \
        PROC Take(CARD ARRAY flat) result=flat(5) RETURN \
        BYTE FUNC Index() RETURN(1) \
        PROC Main() CARD ARRAY local(2,3) p=data \
        data.pixels(i,j).x=p.pixels(i,j).y \
        wrapper.inner.pixels(i,j)=absolute.pixels(i,j) \
        rows(i,j).pixels(i,j).x=42 local(i,j)=grid(i,j) \
        pixel=@data.pixels(i,j) word=@pixel.x Take(grid) \
        count=ELEMENTS(rows(Index(),Index()).pixels) bytes=SIZEOF(data.pixels) RETURN";
    for target in [
        TargetId::Atari6502,
        TargetId::Wdc65816Small,
        TargetId::Wdc65816Native,
        TargetId::Motorola68000,
    ] {
        let program = lower(source, target);
        let text = ir::format_program(&program);
        assert!(text.contains("ARRAY(2,3)"));
        // Layout operands disappear without emitting either Index() call.
        let main = program
            .modules
            .iter()
            .flat_map(|m| &m.items)
            .find_map(|item| match item {
                ir::SemItem::Routine(r) if r.symbol.name == "Main" => Some(r),
                _ => None,
            })
            .unwrap();
        let queries = main.body.iter().filter_map(|stmt| match stmt {
            ir::SemStmt::Assign { target, value, .. } if matches!(&target.kind,
                ir::SemLValueKind::Symbol(symbol) if ["count", "bytes"].contains(&symbol.name.as_str())) => Some(value),
            _ => None,
        }).collect::<Vec<_>>();
        assert_eq!(queries.len(), 2);
        assert!(
            queries
                .iter()
                .all(|value| matches!(value.kind, ir::SemExprKind::Literal(_)))
        );
    }
}

#[test]
fn multidimensional_initializers_preserve_leaf_order_full_extent_and_relocations() {
    let source = "TYPE Point=[BYTE tag CARD word] \
        CARD ARRAY values(2,3)=[1 2 3 4] Point ARRAY points(2,3)=[1 2 3 4] \
        CARD POINTER ref=@values(1,2) CARD POINTER ARRAY refs(2,2)=[@values(1,0) @points(1,2).word] \
        TYPE Tile=[CARD ARRAY pixels(2,3)] Tile tileData=[1 2 3 4] PROC Main() RETURN";
    for (target, stride, word_offset, pointer_width) in [
        (TargetId::Atari6502, 3, 1, 2),
        (TargetId::Motorola68000, 4, 2, 4),
    ] {
        let program = lower(source, target);
        let get = |name: &str| {
            declarations(&program)
                .find(|d| d.symbol.name == name)
                .unwrap()
        };
        for (name, extent, offsets) in [
            ("values", 12, vec![0, 2, 4, 6]),
            (
                "points",
                6 * stride,
                vec![0, word_offset, stride, stride + word_offset],
            ),
            ("tileData", 12, vec![0, 2, 4, 6]),
        ] {
            let plan = get(name).static_initializer.as_ref().unwrap();
            assert_eq!(plan.initialized_extent, extent, "{target:?}/{name}");
            assert_eq!(
                plan.writes.iter().map(|w| w.offset).collect::<Vec<_>>(),
                offsets
            );
        }
        let plan = get("refs").static_initializer.as_ref().unwrap();
        assert_eq!(plan.initialized_extent, 4 * pointer_width);
        for (write, target, addend) in [
            (&plan.writes[0], get("values").symbol.id, 6),
            (
                &plan.writes[1],
                get("points").symbol.id,
                (5 * stride + word_offset) as i32,
            ),
        ] {
            assert!(
                matches!(write.value, ir::SemStaticInitializerValue::Address { target: ref symbol, addend: offset, .. } if symbol.id == target && offset == addend)
            );
        }
    }
}

#[test]
fn multidimensional_indexes_reject_wrong_rank_types_bounds_and_excess_initializers() {
    for (source, message) in [
        (
            "CARD ARRAY a(2,3) CARD x PROC Main() x=a(1) RETURN",
            "rank 2",
        ),
        (
            "CARD ARRAY a(2,3) CARD x PROC Main() a(1,2,0)=1 RETURN",
            "rank 2",
        ),
        (
            "CARD ARRAY a(2,3) CARD x PROC Main() x=a(0,3) RETURN",
            "dimension 2",
        ),
        (
            "CARD ARRAY a(2,3) CARD x PROC Main() x=a(-1,0) RETURN",
            "dimension 1",
        ),
        (
            "CARD ARRAY a(2,3) CARD x PROC Main() x=a(0,1.5) RETURN",
            "requires an integer",
        ),
        (
            "CARD ARRAY a(2,3) CARD POINTER p CARD x PROC Main() x=a(p,0) RETURN",
            "requires an integer",
        ),
        ("CARD ARRAY a(2,3)=[1 2 3 4 5 6 7]", "too many initializer"),
        (
            "TYPE Axis=ENUM [First Second] CARD ARRAY a(2,3) CARD x PROC Main() x=a(Axis.First,0) RETURN",
            "convert enum indexes explicitly",
        ),
        (
            "TYPE T=[CARD ARRAY a(2,3)] T data CARD POINTER p PROC Main() data.a=p RETURN",
            "cannot be assigned or rebound",
        ),
        (
            "PROC Take(CARD ARRAY a(2,3)) RETURN",
            "parameters are not supported",
        ),
        (
            "CARD ARRAY a(3) CARD x PROC Main() x=a(1,2) RETURN",
            "rank 1",
        ),
    ] {
        let ast = parse(&tokenize(source).unwrap()).unwrap();
        let errors =
            semantic::analyze_with_options(&ast, options(TargetId::Atari6502)).unwrap_err();
        assert!(
            errors.iter().any(|e| e.message.contains(message)),
            "{source}: {errors:?}"
        );
    }
    // Real function calls and element-pointer decay retain their original arity.
    lower(
        "CARD ARRAY a(2,3) CARD POINTER p CARD x \
        CARD FUNC Add(CARD left,right) RETURN(left+right) \
        PROC Main() p=a a=p x=Add(1,2) a(1,2)=x RETURN",
        TargetId::Atari6502,
    );
}

#[test]
fn multidimensional_callable_elements_and_explicit_enum_coordinates_remain_typed() {
    lower(
        "TYPE Axis=ENUM [First Second] BYTE value CARD ARRAY grid(2,3) \
        PROC Handler(BYTE x) value=x RETURN \
        PROC POINTER ARRAY handlers(2,3)(BYTE x)=[@Handler] \
        PROC Main() handlers(0,0)(5) grid(BYTE(Axis.First),2)=value RETURN",
        TargetId::Atari6502,
    );
}
