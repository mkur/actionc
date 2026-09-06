use actionc::lexer::tokenize;
use actionc::parser::parse;
use actionc::semantic::{self, SemanticOptions};
use actionc::target::TargetId;

fn options(target: TargetId) -> SemanticOptions {
    SemanticOptions {
        enum_types: true,
        ..SemanticOptions::modern().with_target(target)
    }
}

#[test]
fn enum_storage_initializers_and_layouts_preserve_byte_extent_and_nominality() {
    let source = "TYPE E=ENUM [OFF=0 ON=17] CONST Alias=E.ON TYPE Packet=[BYTE ARRAY pad(257) E state E ARRAY states(2) BYTE tail] TYPE Pair=[E first,second] Pair initial=[E.ON E.OFF] E value=[Alias] E ARRAY values(3)=[E.ON E.OFF Alias] E POINTER p Packet data CARD size,align,offset PROC Take(E ARRAY a) a(0)=E.ON RETURN PROC Main() E local p=values local=p^ data.state=local data.states(1)=values(0) p=@data.states Take(data.states) size=SIZEOF(E) align=ALIGNOF(E) offset=OFFSETOF(Packet,state) RETURN";
    for target in [
        TargetId::Atari6502,
        TargetId::Wdc65816Small,
        TargetId::Wdc65816Native,
        TargetId::Motorola68000,
    ] {
        let ast = parse(&tokenize(source).unwrap()).unwrap();
        let model = semantic::analyze_with_options(&ast, options(target))
            .unwrap_or_else(|errors| panic!("{target:?}: {errors:#?}"));
        let packet = model.layout.record_for_name("Packet").unwrap();
        assert_eq!(packet.fields[1].offset, 257);
        assert_eq!(packet.fields[1].size, 1);
        assert_eq!(packet.fields[2].size, 2);
        let ir = semantic::ir::lower_program(&ast, &model);
        let nir = actionc::nir::lower_program(&ir);
        actionc::nir::verify_program(&nir)
            .unwrap_or_else(|errors| panic!("{target:?}: {errors:#?}"));
        let optimized = actionc::nir::optimize_program(&nir).unwrap();
        match target {
            TargetId::Motorola68000 => {
                actionc::mir68k::lower_program(&optimized).unwrap();
            }
            TargetId::Wdc65816Small | TargetId::Wdc65816Native => {
                actionc::mir65816::lower_program(&optimized).unwrap();
            }
            _ => {}
        }
    }
}

#[test]
fn enum_storage_rejects_implicit_byte_mixing_and_numeric_uses() {
    for declaration in [
        "E ARRAY values=[0]",
        "E value=[0]",
        "E ARRAY values=[F.A]",
        "CONST Alias=E.A BYTE ARRAY values=[Alias]",
        "BYTE ARRAY values=[E.A]",
        "TYPE Rec=[E e] Rec value=[0]",
        "E ARRAY values=[-E.A]",
        "E ARRAY values=\"abc\"",
        "BYTE ARRAY values(E.A)",
        "E POINTER ep BYTE POINTER bp PROC Main() ep=bp RETURN",
        "E POINTER ep F POINTER fp PROC Main() ep=fp RETURN",
        "E POINTER ep BYTE ARRAY values(2) PROC Main() ep=values RETURN",
        "PROC Take(E ARRAY a) RETURN BYTE ARRAY values(2) PROC Main() Take(values) RETURN",
        "E value=E.A",
        "E ARRAY data(2) E value PROC Main() value=data RETURN",
        "E ARRAY data(2) E FUNC Read() RETURN(data)",
        "E ARRAY data(2) PROC Take(E value) RETURN PROC Main() Take(data) RETURN",
        "E ARRAY data(2) PROC Main()\nCASE data OF\nWHEN E.A THEN\nESAC\nRETURN",
        "E ARRAY view F ARRAY data(2) PROC Main() view=data RETURN",
        "E ARRAY view BYTE ARRAY data(2) PROC Main() view=data RETURN",
        "E ARRAY view PROC Main() view=E.A RETURN",
        "E ARRAY view PROC Main() view==+E.A RETURN",
        "BYTE ARRAY values(2)=E.A",
        "CONST Alias=E.A PROC Main() [Alias] RETURN",
        "CONST Alias=E.A PROC Main()\nASM\nLDA #Alias\nENDASM\nRETURN",
    ] {
        let source = format!("TYPE E=ENUM [A=3] TYPE F=ENUM [A=3] {declaration}");
        let ast = parse(&tokenize(&source).unwrap())
            .unwrap_or_else(|errors| panic!("{source}\n{errors:#?}"));
        assert!(
            semantic::analyze_with_options(&ast, options(TargetId::Atari6502)).is_err(),
            "{source}"
        );
    }
}

#[test]
fn explicit_numeric_constant_bridges_work_for_sizes_and_addresses() {
    let source = "TYPE E=ENUM [COUNT=3] CONST Count=BYTE(E.COUNT) BYTE ARRAY data(Count)=$5000+CARD(E.COUNT) PROC Main() RETURN";
    let ast = parse(&tokenize(source).unwrap()).unwrap();
    let model = semantic::analyze_with_options(&ast, options(TargetId::Atari6502)).unwrap();
    let ir = semantic::ir::lower_program(&ast, &model);
    actionc::nir::verify_program(&actionc::nir::lower_program(&ir)).unwrap();
}

fn loaded(root_source: &str, lib_source: &str) -> actionc::includes::LoadedCompilation {
    use actionc::includes::{ModuleLoadOptions, load_compilation_from_provider};
    use actionc::source::{InMemorySourceProvider, SourceOrigin};
    let root = SourceOrigin::host("project/main.act");
    let provider = InMemorySourceProvider::default()
        .with_source(root.clone(), root_source.as_bytes().to_vec())
        .with_source(
            SourceOrigin::host("project/lib.act"),
            lib_source.as_bytes().to_vec(),
        );
    load_compilation_from_provider(root, &provider, &ModuleLoadOptions::default()).unwrap()
}

#[test]
fn module_import_forms_and_shadowing_retain_enum_identity() {
    for (import, name) in [("USE Lib AS API", "API.E"), ("USE ALL FROM Lib", "E")] {
        let source = format!(
            "MODULE App\n{import}\n{name} value=[{name}.ON]\n{name} ARRAY values=[{name}.OFF {name}.ON]\nPROC Main()\nTYPE Local=ENUM [ON=9]\nBEGIN\nTYPE E=ENUM [ON=7]\nCONST LocalValue=E.ON\nBYTE result\nresult=BYTE(LocalValue)\nEND\nvalue={name}.ON\nRETURN\nENDMODULE"
        );
        let compilation = loaded(
            &source,
            "MODULE Lib PUBLIC TYPE E=ENUM [OFF ON=17] ENDMODULE",
        );
        let model =
            semantic::analyze_compilation_with_options(&compilation, options(TargetId::Atari6502))
                .unwrap_or_else(|errors| panic!("{source}\n{errors:#?}"));
        assert_eq!(model.enums.types.len(), 3);
        let ids = model
            .enums
            .types
            .values()
            .map(|t| &t.identity.canonical_name)
            .collect::<std::collections::HashSet<_>>();
        assert_eq!(ids.len(), 3);
        let ir = semantic::ir::lower_compilation(&compilation, &model);
        actionc::nir::verify_program(&actionc::nir::lower_program(&ir)).unwrap();
    }
}

#[test]
fn private_enum_names_and_members_cannot_escape_module_visibility() {
    for body in [
        "Lib.Hidden value",
        "CONST value=Lib.Hidden.ON",
        "PROC Main() BYTE value value=BYTE(Lib.Hidden.ON) RETURN",
        "Lib.Hidden FUNC Read() RETURN(Lib.Hidden.ON)",
    ] {
        let compilation = loaded(
            &format!("MODULE App USE Lib {body} ENDMODULE"),
            "MODULE Lib TYPE Hidden=ENUM [ON] ENDMODULE",
        );
        assert!(
            semantic::analyze_compilation_with_options(&compilation, options(TargetId::Atari6502))
                .is_err(),
            "{body}"
        );
    }
}

#[test]
fn named_record_can_use_a_later_enum_type_without_inventing_a_record_layout() {
    let compilation = loaded(
        "MODULE App USE Lib Lib.Packet packet PROC Main() packet.value=Lib.E.ON RETURN ENDMODULE",
        "MODULE Lib PUBLIC TYPE Packet=[E value] PUBLIC TYPE E=ENUM [ON] ENDMODULE",
    );
    let model =
        semantic::analyze_compilation_with_options(&compilation, options(TargetId::Atari6502))
            .unwrap();
    assert_eq!(model.layout.record_for_name("Lib.Packet").unwrap().size, 1);
}
