#[path = "support/unions.rs"]
mod unions;
#[path = "support/variant_values.rs"]
mod values;

#[test]
fn generic_union_variants_capture_payloads_before_guard_mutation() {
    let source = r#"
TYPE Overlay<T,U>=UNION [T first U second]
TYPE Bytes=[BYTE ARRAY data(3)]
TYPE Event<T>=VARIANT [NONE DATA [T value]]
Overlay<CARD,Bytes> original
Event<Overlay<CARD,Bytes>> current
BYTE ARRAY output=$600
BYTE calls
BYTE FUNC Reject()
 calls==+1 original.first=$EEEE current=Event<Overlay<CARD,Bytes>>.NONE
RETURN(0)
PROC Main()
 calls=0 original.first=$1234 original.second.data(2)=7
 current=Event<Overlay<CARD,Bytes>>.DATA(original)
 original.first=$9999
 CASE current OF
 WHEN Event<Overlay<CARD,Bytes>>.NONE THEN
   output(0)=99
 WHEN Event<Overlay<CARD,Bytes>>.DATA(saved) IF Reject() THEN
   output(0)=98
 WHEN Event<Overlay<CARD,Bytes>>.DATA(saved) THEN
   output(0)=saved.second.data(0) output(1)=saved.second.data(1)
   output(2)=saved.second.data(2)
 ESAC
 output(3)=calls
 CASE current OF
 WHEN Event<Overlay<CARD,Bytes>>.NONE THEN
   output(4)=1
 ELSE
   output(4)=0
 ESAC
 DO OD
RETURN
"#;
    let mut expected = vec![0xCC; 0x500];
    expected[..5].copy_from_slice(&[0x34, 0x12, 7, 1, 1]);
    unions::check(source, &expected);
}

#[test]
fn module_only_generic_unions_preserve_identity_through_aliases_and_public_callbacks() {
    use actionc::includes::{ModuleLoadOptions, load_compilation_from_provider};
    use actionc::source::{InMemorySourceProvider, SourceOrigin};
    let root = SourceOrigin::host("project/main.act");
    let provider = InMemorySourceProvider::default()
        .with_source(
            root.clone(),
            br#"
MODULE App
USE Lib AS A USE Lib AS B
A.View<CARD> original
B.View<CARD> FUNC POINTER callback(B.View<CARD> input)
BYTE ARRAY output=$600
PROC Main()
 original.first=$1234 callback=A.Copy
 LET saved=callback(original)
 original.first=0
 output(0)=saved.bytes(0) output(1)=saved.bytes(1)
 output(2)=A.Size()
 DO OD
RETURN
ENDMODULE
"#
            .to_vec(),
        )
        .with_source(
            SourceOrigin::host("project/lib.act"),
            br#"
MODULE Lib
PUBLIC TYPE View<T>=UNION [T first BYTE ARRAY bytes(2)]
PUBLIC View<CARD> FUNC Copy(View<CARD> input)
 input.bytes(0)==+1
RETURN(input)
PUBLIC BYTE FUNC Size() RETURN(BYTE(SIZEOF(View<CARD>)))
ENDMODULE
"#
            .to_vec(),
        );
    let loaded =
        load_compilation_from_provider(root, &provider, &ModuleLoadOptions::default()).unwrap();
    let mut options = actionc::semantic::SemanticOptions::modern();
    options.algebraic_types.unions = true;
    let model = actionc::semantic::analyze_compilation_with_options(&loaded, options).unwrap();
    assert_eq!(model.generics.instances.len(), 1);
    let mut expected = vec![0xCC; 0x500];
    expected[..3].copy_from_slice(&[0x35, 0x12, 2]);
    values::check_semir(
        &actionc::semantic::ir::lower_compilation(&loaded, &model),
        &expected,
        false,
    );
}

#[test]
fn union_payload_does_not_remove_the_outer_variant_error_check() {
    let source = r#"
TYPE View=UNION [CARD word]
TYPE Event=VARIANT [NONE DATA [View value]]
View original
Event current
BYTE result=$600
PROC Main()
 result=41 original.word=$1234 current=Event.DATA(original)
 [ $A9 $FF $8D current ]
 CASE current OF
 WHEN Event.NONE THEN
   result=99
 WHEN Event.DATA(saved) THEN
   result=saved.word
 ESAC
 DO OD
RETURN
"#;
    let ast = actionc::parser::parse(&actionc::lexer::tokenize(source).unwrap()).unwrap();
    let mut options = actionc::semantic::SemanticOptions::modern();
    options.algebraic_types.unions = true;
    let model = actionc::semantic::analyze_with_options(&ast, options).unwrap();
    let mut expected = vec![0xCC; 0x500];
    expected[0] = 41;
    values::check_semir(
        &actionc::semantic::ir::lower_program(&ast, &model),
        &expected,
        true,
    );
}
