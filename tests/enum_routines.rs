use actionc::ast::{Item, RoutineKind, TypeBase};
use actionc::lexer::tokenize;
use actionc::parser::parse;
use actionc::semantic::{self, SemanticOptions};

fn options() -> SemanticOptions {
    SemanticOptions {
        enum_types: true,
        ..SemanticOptions::modern()
    }
}

fn checked(source: &str) -> semantic::ir::SemProgram {
    let ast =
        parse(&tokenize(source).unwrap()).unwrap_or_else(|errors| panic!("{source}\n{errors:?}"));
    let model = semantic::analyze_with_options(&ast, options())
        .unwrap_or_else(|errors| panic!("{source}\n{errors:?}"));
    let program = semantic::ir::lower_program(&ast, &model);
    let nir = actionc::nir::lower_program(&program);
    actionc::nir::verify_program(&nir).unwrap();
    actionc::nir::optimize_program(&nir).unwrap();
    program
}

#[test]
fn named_func_headers_and_callable_declarations_have_complete_boundaries() {
    let source = "MODULE API\nTYPE E=ENUM [A]\nE FUNC First(E arg) RETURN(arg)\nPUBLIC E FUNC Second() RETURN(E.A)\nPUBLIC EXTERNAL LIB.E FUNC Third()\nLIB.E FUNC POINTER reader\nPROC Main() RETURN\nENDMODULE";
    let ast = parse(&tokenize(source).unwrap()).unwrap();
    assert_eq!(ast.modules[0].items.len(), 6);
    let Item::Routine(first) = &ast.modules[0].items[1] else {
        panic!("routine")
    };
    assert!(
        matches!(&first.kind, RoutineKind::Func { return_type } if matches!(&return_type.base, TypeBase::Named(name) if name.to_string() == "E"))
    );
    let Item::Declaration(actionc::ast::Decl::Var(reader)) = &ast.modules[0].items[4] else {
        panic!("pointer")
    };
    assert!(
        matches!(&reader.ty.base, TypeBase::Callable(callable) if matches!(&callable.kind, RoutineKind::Func { return_type } if matches!(&return_type.base, TypeBase::Named(name) if name.to_string() == "LIB.E")))
    );
}

#[test]
fn enum_parameters_results_and_callable_results_retain_identity() {
    let ir = checked(
        "TYPE E=ENUM [A B]\nE FUNC Echo(E value) RETURN(value)\nE FUNC Read() RETURN(E(17))\nE FUNC POINTER callback\nE result\nPROC Main() callback=@Read result=Echo(callback()) RETURN",
    );
    let routines = ir.modules[0]
        .items
        .iter()
        .filter_map(|item| match item {
            semantic::ir::SemItem::Routine(r) => Some(r),
            _ => None,
        })
        .collect::<Vec<_>>();
    let result = routines[0].signature.return_type.as_ref().unwrap();
    assert!(result.as_enum().is_some());
    assert_eq!(routines[0].signature.params, [result.clone()]);
    assert_eq!(routines[0].symbol.ty.as_ref(), Some(result));
    assert_eq!(routines[1].callable_type.return_type.as_ref(), Some(result));
}

#[test]
fn invalid_enum_returns_arguments_and_callable_assignments_are_rejected() {
    for tail in [
        "E FUNC Bad() RETURN(0)",
        "E FUNC Bad() RETURN(F.A)",
        "E FUNC Bad() RETURN",
        "E FUNC Bad(BYTE x) IF x THEN RETURN(E.A) FI",
        "E FUNC Bad(E x) RETURN(x) PROC Main() E value value=Bad(0) RETURN",
        "E FUNC Bad(E x) RETURN(x) PROC Main() E value value=Bad(F.A) RETURN",
        "BYTE FUNC Read() RETURN(0) E FUNC POINTER callback PROC Main() callback=@Read RETURN",
        "F FUNC Read() RETURN(F.A) E FUNC POINTER callback PROC Main() callback=@Read RETURN",
        "E FUNC Read() RETURN(E.A) BYTE FUNC POINTER callback PROC Main() callback=@Read RETURN",
        "TYPE Rec=[BYTE x] Rec FUNC Bad() RETURN(0)",
        "E FUNC Bad() TYPE E=ENUM [A] RETURN(E.A)",
    ] {
        let source = format!("TYPE E=ENUM [A]\nTYPE F=ENUM [A]\n{tail}");
        let ast = parse(&tokenize(&source).unwrap())
            .unwrap_or_else(|errors| panic!("{source}\n{errors:?}"));
        assert!(
            semantic::analyze_with_options(&ast, options()).is_err(),
            "{source}"
        );
    }
}

#[test]
fn qualified_module_result_aliases_share_canonical_signatures() {
    use actionc::includes::{ModuleLoadOptions, load_compilation_from_provider};
    use actionc::source::{InMemorySourceProvider, SourceOrigin};
    let root = SourceOrigin::host("project/main.act");
    let provider = InMemorySourceProvider::default()
        .with_source(root.clone(), b"MODULE App USE Lib AS API API.E FUNC Echo(API.E value) RETURN(value) API.E FUNC POINTER callback API.E result PROC Main() callback=@API.Read result=Echo(callback()) RETURN ENDMODULE".to_vec())
        .with_source(SourceOrigin::host("project/lib.act"), b"MODULE Lib PUBLIC TYPE E=ENUM [A=17] PUBLIC E FUNC Read() RETURN(E.A) ENDMODULE".to_vec());
    let loaded =
        load_compilation_from_provider(root, &provider, &ModuleLoadOptions::default()).unwrap();
    let model = semantic::analyze_compilation_with_options(&loaded, options())
        .unwrap_or_else(|errors| panic!("{errors:#?}"));
    let ir = semantic::ir::lower_compilation(&loaded, &model);
    let nir = actionc::nir::lower_program(&ir);
    actionc::nir::verify_program(&nir).unwrap();
    actionc::nir::optimize_program(&nir).unwrap();
    let enums = model.enums.types.values().collect::<Vec<_>>();
    assert_eq!(enums.len(), 1);
    for signature in model.routine_signatures.values().filter(|s| {
        s.return_type
            .as_ref()
            .is_some_and(|ty| ty.as_enum().is_some())
    }) {
        assert_eq!(
            signature.return_type.as_ref().unwrap().as_enum(),
            Some(&enums[0].identity)
        );
    }
}
