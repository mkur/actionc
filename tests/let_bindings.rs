use actionc::lexer::tokenize;
use actionc::parser::parse;
use actionc::semantic::ir::{self, SemExprKind, SemItem, SemStmt};
use actionc::semantic::{self, ScalarType, SemanticOptions};
use actionc::target::TargetId;

fn checked(source: &str) -> ir::SemProgram {
    checked_target(source, TargetId::Atari6502)
}

fn checked_target(source: &str, target: TargetId) -> ir::SemProgram {
    let ast = parse(&tokenize(source).unwrap()).unwrap_or_else(|e| panic!("{source}\n{e:?}"));
    let model = semantic::analyze_with_options(&ast, SemanticOptions::modern().with_target(target))
        .unwrap_or_else(|e| panic!("{source}\n{e:?}"));
    let semir = ir::lower_program(&ast, &model);
    let nir = actionc::nir::lower_program(&semir);
    actionc::nir::verify_program(&nir).unwrap_or_else(|e| panic!("{source}\n{e:?}\n{nir:#?}"));
    actionc::nir::optimize_program(&nir).unwrap();
    semir
}

fn rejected(source: &str, message: &str) {
    let ast = parse(&tokenize(source).unwrap()).unwrap_or_else(|e| panic!("{source}\n{e:?}"));
    let errors = semantic::analyze_with_options(&ast, SemanticOptions::modern()).unwrap_err();
    assert!(
        errors.iter().any(|e| e.message.contains(message)),
        "{source}\n{errors:?}"
    );
}

fn main_routine(program: &ir::SemProgram) -> &ir::SemRoutine {
    program
        .modules
        .iter()
        .flat_map(|m| &m.items)
        .find_map(|item| match item {
            SemItem::Routine(r) if r.symbol.name == "Main" => Some(r),
            _ => None,
        })
        .unwrap()
}

#[test]
fn let_sequential_shadowing_has_distinct_storage_and_parent_initializers() {
    let program = checked(
        "BYTE value, result\nPROC Main()\nresult=value\nLET value=value\nLET value=value+1\nresult=value\nRETURN",
    );
    let routine = main_routine(&program);
    let SemStmt::Assign { value: before, .. } = &routine.body[0] else {
        panic!()
    };
    let SemStmt::LexicalBlock {
        declarations, body, ..
    } = &routine.body[1]
    else {
        panic!()
    };
    assert!(declarations[0].symbol.is_immutable);
    assert!(declarations[0].initializer.is_none());
    assert!(declarations[0].static_initializer.is_none());
    let SemStmt::Assign { value: initial, .. } = &body[0] else {
        panic!()
    };
    let SemExprKind::Symbol(before) = &before.kind else {
        panic!()
    };
    let SemExprKind::Symbol(initial) = &initial.kind else {
        panic!()
    };
    assert_eq!(before.id, initial.id);
    let SemStmt::LexicalBlock {
        declarations: second,
        body,
        ..
    } = &body[1]
    else {
        panic!()
    };
    assert_ne!(declarations[0].symbol.id, second[0].symbol.id);
    let SemStmt::Assign { value, .. } = &body[0] else {
        panic!()
    };
    let SemExprKind::Binary { left, .. } = &value.kind else {
        panic!("{value:#?}")
    };
    let SemExprKind::Symbol(symbol) = &left.kind else {
        panic!()
    };
    assert_eq!(symbol.id, declarations[0].symbol.id);
    assert!(symbol.is_immutable);
}

#[test]
fn let_preserves_scalar_enum_pointer_and_callable_types() {
    let program = checked(
        "TYPE E=ENUM [A B]\nBYTE x\nBYTE FUNC Read() RETURN(7)\nPROC Main()\nLET tiny=1\nLET CARD word=1\nLET signed=-1\nLET wide=65536\nLET real=1.25\nLET state=E.B\nLET ptr=@x\nLET callback=@Read\nLET result=callback()\nRETURN",
    );
    let mut types = Vec::new();
    let mut statements = main_routine(&program).body.as_slice();
    while let Some(SemStmt::LexicalBlock {
        declarations, body, ..
    }) = statements.first()
    {
        types.push(declarations[0].ty.value.clone());
        statements = body;
        statements = &statements[1..];
    }
    assert_eq!(types.len(), 9);
    assert_eq!(types[0].as_scalar(), Some(ScalarType::Byte));
    assert_eq!(types[1].as_scalar(), Some(ScalarType::Card));
    assert_eq!(types[2].as_scalar(), Some(ScalarType::Int));
    assert_eq!(types[3].as_scalar().unwrap().width_bytes(), 4);
    assert!(types[4].is_real());
    assert!(types[5].as_enum().is_some());
    assert!(types[6].is_pointer());
    assert!(types[7].as_callable_pointer().is_some());
    assert_eq!(types[8].as_scalar(), Some(ScalarType::Byte));
}

#[test]
fn let_rejects_every_source_write_to_the_binding() {
    for tail in ["x=2", "x==+1", "FOR x=0 TO 1 DO OD"] {
        rejected(
            &format!("PROC Main()\nLET x=1\n{tail}\nRETURN"),
            "immutable LET",
        );
    }
}

#[test]
fn let_rejects_address_escape_and_machine_access() {
    for tail in [
        "p=@x",
        "BEGIN\nBYTE alias=x\nEND",
        "BEGIN\nCARD address=[@x]\nEND",
        "BEGIN\nBYTE POINTER ptr=x\nEND",
        "BEGIN\nBYTE ARRAY bytes=x\nEND",
        "BEGIN\nBYTE ARRAY address=[<@x >@x]\nEND",
        "[ $AD x ]",
        "ASM\n lda x\nENDASM",
        "ASM OPAQUE\n sta x\nENDASM",
    ] {
        rejected(
            &format!("BYTE POINTER p\nPROC Main()\nLET x=1\n{tail}\nRETURN"),
            "immutable LET",
        );
    }
}

#[test]
fn let_pointer_pointees_remain_assignable() {
    checked(
        "TYPE R=[BYTE field BYTE ARRAY data(3)]\nR object\nBYTE value\nPROC Main()\nLET p=@value\np^=2\nLET q=@object\nq.field=3\nq.data(1)=4\nLET element=@q.field\nelement^=5\nRETURN",
    );
}

#[test]
fn let_requires_explicit_blocks_in_control_flow_bodies() {
    for body in [
        "IF 1 THEN LET x=1 FI",
        "WHILE 1 DO LET x=1 OD",
        "DO LET x=1 UNTIL 1 OD",
        "FOR i=0 TO 1 DO LET x=1 OD",
        "CASE i OF\nWHEN 0 THEN\nLET x=1\nESAC",
    ] {
        rejected(&format!("PROC Main()\nBYTE i\n{body}\nRETURN"), "BEGIN/END");
    }
    checked(
        "PROC Main()\nBYTE i\nFOR i=0 TO 1 DO\nBEGIN\nLET x=i\nIF x THEN\nBEGIN\nLET y=x+1\nEXIT\nEND\nFI\nEND\nOD\nRETURN",
    );
}

#[test]
fn let_scope_visibility_and_constant_contexts_are_enforced() {
    rejected("PROC Main()\nLET x=x\nRETURN", "undefined symbol");
    rejected(
        "BYTE y\nPROC Main()\ny=x\nLET x=1\nRETURN",
        "undefined symbol",
    );
    rejected(
        "BYTE y\nPROC Main()\nBEGIN\nLET x=1\nEND\ny=x\nRETURN",
        "undefined symbol",
    );
    rejected(
        "PROC Main()\nLET x=1\nBEGIN\nCONST y=x\nEND\nRETURN",
        "runtime value",
    );
    rejected(
        "PROC Main()\nLET x=1\nCASE 1 OF\nWHEN x THEN\nRETURN\nESAC\nRETURN",
        "constant",
    );
    rejected("LET x=1\nPROC Main() RETURN", "inside routines");
    rejected(
        "PROC Main()\nLET x=1\nBEGIN\nBYTE ARRAY bytes(x+1)\nEND\nRETURN",
        "array bound",
    );
    checked(
        "PROC Main()\nLET x=1\nBEGIN\nCONST count=SIZEOF(x)\nBYTE ARRAY bytes(count)\nbytes(0)=2\nEND\nRETURN",
    );
}

#[test]
fn let_keeps_nominal_checks_and_excludes_owned_aggregates() {
    rejected(
        "TYPE E=ENUM [A]\nPROC Main() LET E x=0 RETURN",
        "exact enum type",
    );
    rejected(
        "TYPE R=[BYTE x]\nR value\nPROC Main() LET r=value RETURN",
        "owned aggregates",
    );
    rejected("PROC Main() LET text=\"hello\" RETURN", "owned aggregates");
    checked("TYPE E=ENUM [A]\nPROC Main()\nLET E=E.A\nLET value=E\nRETURN");
    rejected(
        "BYTE ARRAY data(3)\nPROC Main() LET values=data RETURN",
        "owned arrays",
    );
    rejected(
        "PROC Main() LET BYTE FUNC POINTER reader()=1 RETURN",
        "typed callable value",
    );
    checked("BYTE ARRAY data(3)\nPROC Main()\nLET BYTE POINTER ptr=data\nptr(1)=2\nRETURN");
}

#[test]
fn let_is_modern_only_but_ordinary_identifier_uses_remain_compatible() {
    let ast = parse(&tokenize("PROC Main() LET x=1 RETURN").unwrap()).unwrap();
    let errors = semantic::analyze(&ast).unwrap_err();
    assert!(
        errors
            .iter()
            .any(|e| e.message.contains("LET requires the modern profile"))
    );
    let ast = parse(&tokenize("BYTE let\nPROC Main() let=1 RETURN").unwrap()).unwrap();
    semantic::analyze(&ast).unwrap();
}

#[test]
fn let_native_lowering_reuses_target_types_and_activation() {
    for target in [
        TargetId::Motorola68000,
        TargetId::Wdc65816Native,
        TargetId::Wdc65816Small,
    ] {
        checked_target(
            "CARD result\nPROC Main()\nLET CARD n=1\nLET LONGCARD wide=LONGCARD(n)+65536\nresult=CARD(wide)\nRETURN",
            target,
        );
    }
}
