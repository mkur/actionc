use actionc::{
    ast::{Item, Stmt},
    lexer::tokenize,
    parser::parse,
    semantic::{self, SemanticOptions},
};

const TYPES: &str = "TYPE MaybeByte=VARIANT [NONE SOME [BYTE value]]\nTYPE Other=VARIANT [NONE SOME [BYTE value]]\nMaybeByte item\nBYTE result\n";

fn check(source: &str) {
    let ast = parse(&tokenize(source).unwrap()).unwrap();
    let model = semantic::analyze_with_options(&ast, SemanticOptions::modern()).unwrap();
    let semir = semantic::ir::lower_program(&ast, &model);
    let nir = actionc::nir::lower_program(&semir);
    actionc::nir::verify_program(&nir).unwrap();
    let optimized = actionc::nir::optimize_program(&nir).unwrap();
    actionc::mir6502::lower_program(&optimized).unwrap();
    actionc::codegen::generate_semir_profile_with_origin(
        &semir,
        0x3000,
        actionc::codegen::CodegenProfile::Modern,
    )
    .unwrap();
}

fn rejects(source: &str, needle: &str) {
    let ast = parse(&tokenize(source).unwrap()).unwrap();
    let errors = semantic::analyze_with_options(&ast, SemanticOptions::modern()).expect_err(source);
    assert!(
        errors.iter().any(|error| error.message.contains(needle)),
        "{source}: {errors:?}"
    );
}

#[test]
fn local_use_parses_contextually_with_statement_boundaries() {
    let source = format!(
        "{TYPES}PROC Main()\nresult=1 USE ALL FROM MaybeByte LET saved=SOME(42)\nitem=NONE\nRETURN"
    );
    let ast = parse(&tokenize(&source).unwrap()).unwrap();
    let Item::Routine(routine) = ast.modules[0].items.last().unwrap() else {
        panic!()
    };
    assert!(matches!(&routine.body[1], Stmt::UseVariant { target, .. } if target == "MaybeByte"));
    check(&source);
    check("BYTE use,all,from,as PROC Main() use=1 all=2 from=3 as=4 RETURN");
    check("TYPE USE=[BYTE value]\nPROC Main()\nUSE ALL\nALL.value=1\nRETURN");
    for body in [
        "USE ALL FROM",
        "USE ALL FROM MaybeByte AS M",
        "USE ALL FROM MaybeByte.*",
        "USE ALL FROM MaybeByte<BYTE>",
    ] {
        let source = format!("{TYPES}PROC Main() {body}\nRETURN");
        assert!(parse(&tokenize(&source).unwrap()).is_err(), "{source}");
    }
}

#[test]
fn constructors_and_patterns_share_the_qualified_identity() {
    check(&format!(
        r#"{TYPES}
PROC Main()
  USE ALL FROM MaybeByte
  USE ALL FROM MaybeByte
  LET saved=some(42)
  item=MaybeByte.NONE
  CASE saved OF
  WHEN none THEN
    result=0
  WHEN SOME(n) THEN
    BEGIN
      USE ALL FROM MaybeByte
      item=SOME(n)
      CASE item OF
      WHEN MaybeByte.NONE THEN
        result=1
      WHEN MaybeByte.SOME(m) THEN
        result=m
      ESAC
    END
  ESAC
RETURN
"#
    ));
}

#[test]
fn local_openings_end_at_block_and_routine_boundaries() {
    check(&format!(
        r#"{TYPES}
PROC First()
  USE ALL FROM MaybeByte
  item=SOME(1)
RETURN
PROC Main()
  BEGIN
    USE ALL FROM MaybeByte
    item=NONE
  END
  BEGIN
    USE ALL FROM Other
    LET saved=SOME(2)
  END
RETURN
"#
    ));
    for body in [
        "item=NONE USE ALL FROM MaybeByte",
        "BEGIN\nUSE ALL FROM MaybeByte\nitem=NONE\nEND\nitem=NONE",
        "BEGIN\nUSE ALL FROM MaybeByte\nitem=NONE\nEND\nBEGIN\nitem=NONE\nEND",
        "USE ALL FROM MaybeByte item=NONE RETURN PROC Next() item=NONE",
    ] {
        rejects(&format!("{TYPES}PROC Main()\n{body}\nRETURN"), "undefined");
    }
}

#[test]
fn local_opening_requires_an_explicit_lexical_statement_list() {
    for body in [
        "IF 1 THEN USE ALL FROM MaybeByte FI",
        "DO USE ALL FROM MaybeByte EXIT OD",
        "CASE item OF\nWHEN MaybeByte.NONE THEN\nUSE ALL FROM MaybeByte\nELSE\nresult=0\nESAC",
    ] {
        rejects(&format!("{TYPES}PROC Main()\n{body}\nRETURN"), "BEGIN/END");
    }
    rejects(&format!("{TYPES}USE ALL FROM MaybeByte"), "inside routines");
    let ast =
        parse(&tokenize(&format!("{TYPES}PROC Main() USE ALL FROM MaybeByte RETURN")).unwrap())
            .unwrap();
    for options in [SemanticOptions::default(), {
        let mut options = SemanticOptions::modern();
        options.algebraic_types.variants = false;
        options
    }] {
        let errors = semantic::analyze_with_options(&ast, options).unwrap_err();
        assert!(
            errors
                .iter()
                .any(|e| e.message.contains("requires modern variants"))
        );
    }
}

#[test]
fn opening_rejects_collisions_without_changing_nominal_rules() {
    for body in [
        "USE ALL FROM MaybeByte USE ALL FROM Other",
        "USE ALL FROM MaybeByte\nBEGIN\nUSE ALL FROM Other\nEND",
        "LET some=1 USE ALL FROM MaybeByte",
        "BEGIN\nBYTE none\nUSE ALL FROM MaybeByte\nEND",
        "USE ALL FROM MaybeByte\nDEFINE NONE=\"1\"",
    ] {
        rejects(&format!("{TYPES}PROC Main()\n{body}\nRETURN"), "collision");
    }
    for declaration in ["BYTE NONE", "BYTE FUNC SOME() RETURN(0)"] {
        rejects(
            &format!("{TYPES}{declaration} PROC Main() USE ALL FROM MaybeByte RETURN"),
            "collision",
        );
    }
    for (body, needle) in [
        ("item=SOME", "payload"),
        ("item=NONE()", "payload"),
        ("item=SOME(1,2)", "payload"),
        ("LET saved=Other.SOME(1) item=saved", "cannot assign record"),
        (
            "CASE Other.NONE OF\nWHEN NONE THEN\nresult=0\nELSE\nresult=1\nESAC",
            "exact selector",
        ),
        (
            "CASE item OF\nWHEN SOME(n) THEN\nn=2\nELSE\nresult=0\nESAC",
            "immutable",
        ),
    ] {
        rejects(
            &format!("{TYPES}PROC Main()\nUSE ALL FROM MaybeByte\n{body}\nRETURN"),
            needle,
        );
    }
}

#[test]
fn ordinary_inner_bindings_shadow_openings_and_qualified_names_still_work() {
    check(&format!(
        r#"{TYPES}
PROC Main()
  USE ALL FROM MaybeByte
  BEGIN
    BYTE SOME
    SOME=7
    item=MaybeByte.SOME(SOME)
  END
  item=SOME(8)
  LET NONE=9
  result=NONE
  item=MaybeByte.NONE
RETURN
"#
    ));
}

#[test]
fn nested_nullary_patterns_are_constructors_instead_of_binders() {
    check(
        r#"
TYPE Inner=VARIANT [NONE SOME [BYTE value]]
TYPE Outer=VARIANT [BOX [Inner value]]
PROC Main()
  USE ALL FROM Inner
  USE ALL FROM Outer
  LET item=BOX(NONE)
  CASE item OF
  WHEN BOX(NONE) THEN
    PrintBE(0)
  WHEN BOX(SOME(n)) THEN
    PrintBE(n)
  ESAC
RETURN
"#,
    );
    rejects(
        r#"
TYPE Inner=VARIANT [NONE SOME [BYTE value]]
TYPE Outer=VARIANT [BOX [Inner value]]
PROC Main()
  USE ALL FROM Inner
  USE ALL FROM Outer
  CASE BOX(NONE) OF
  WHEN BOX(NONE) THEN
    PrintBE(0)
  ESAC
RETURN
"#,
        "non-exhaustive",
    );
}

#[test]
fn opening_accepts_local_types_and_rejects_non_variant_targets() {
    check(
        "PROC Main() TYPE Local=VARIANT [EMPTY FULL [BYTE value]] USE ALL FROM Local LET saved=FULL(7) RETURN",
    );
    check(
        "PROC Main()\nBEGIN\nTYPE Local=VARIANT [EMPTY FULL [BYTE value]]\nUSE ALL FROM Local\nLET saved=EMPTY\nEND\nRETURN",
    );
    for target in ["Missing", "result", "PrintBE", "Color", "RecordType"] {
        rejects(
            &format!(
                "{TYPES}TYPE Color=ENUM [RED BLUE] TYPE RecordType=[BYTE value] PROC Main() USE ALL FROM {target} RETURN"
            ),
            "visible VARIANT type",
        );
    }
}

#[test]
fn module_aliases_preserve_visibility_and_constructor_identity() {
    use actionc::includes::{ModuleLoadOptions, load_compilation_from_provider};
    use actionc::source::{InMemorySourceProvider, SourceOrigin};
    for (body, error) in [
        (
            "USE ALL FROM A.Event\nUSE ALL FROM B.Event\nLET saved=SOME(7)\nCASE saved OF\nWHEN B.Event.SOME(n) THEN\nPrintBE(n)\nELSE\nPrintBE(0)\nESAC",
            None,
        ),
        (
            "USE ALL FROM A.Event\nUSE ALL FROM B.Event\nLET saved=A.Make(IF 1 THEN 7 ELSE 0 FI)\nLET value=CASE saved OF\nWHEN B.Event.SOME(n) IF n>0 THEN\nIF n=7 THEN n ELSE 0 FI\nWHEN SOME(_) THEN\n0\nWHEN NONE THEN\n0\nESAC\nPrintBE(value)",
            None,
        ),
        ("USE ALL FROM A.Hidden", Some("visible VARIANT type")),
        ("USE ALL FROM A.Missing", Some("visible VARIANT type")),
        ("USE ALL FROM A", Some("visible VARIANT type")),
        ("USE ALL FROM A.AliasCollision", Some("collision")),
    ] {
        let root = SourceOrigin::host("project/main.act");
        let provider = InMemorySourceProvider::default()
            .with_source(root.clone(), format!(
                "MODULE App USE Lib AS A USE Lib AS B\nPROC Main()\n{body}\nRETURN ENDMODULE"
            ).into_bytes())
            .with_source(SourceOrigin::host("project/lib.act"), b"MODULE Lib PUBLIC TYPE Event=VARIANT [NONE SOME [BYTE value]] TYPE Hidden=VARIANT [SECRET] PUBLIC TYPE AliasCollision=VARIANT [A] PUBLIC Event FUNC Make(BYTE n) RETURN(Event.SOME(n)) ENDMODULE".to_vec());
        let loaded =
            load_compilation_from_provider(root, &provider, &ModuleLoadOptions::default()).unwrap();
        let result = semantic::analyze_compilation_with_options(&loaded, SemanticOptions::modern());
        if let Some(needle) = error {
            assert!(
                result
                    .unwrap_err()
                    .iter()
                    .any(|e| e.message.contains(needle)),
                "{body}"
            );
        } else {
            let model = result.unwrap();
            let semir = semantic::ir::lower_compilation(&loaded, &model);
            let nir = actionc::nir::lower_program(&semir);
            actionc::nir::verify_program(&nir).unwrap();
            actionc::nir::optimize_program(&nir).unwrap();
        }
    }
}
