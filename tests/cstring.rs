use actionc::{compiler::native65816, includes::ModuleLoadOptions, lexer, parser, semantic, target::TargetId};
use std::{path::PathBuf,sync::atomic::{AtomicU64,Ordering}};
struct Source(PathBuf);
impl Source {
    fn new(text:&str)->Self {
        static NEXT: AtomicU64=AtomicU64::new(0);
        let p=std::env::temp_dir().join(format!("cstring-{}-{}.act",std::process::id(),NEXT.fetch_add(1,Ordering::Relaxed)));
        std::fs::write(&p,text).unwrap(); Self(p)
    }
    fn prepare(&self,opt:bool)->Result<native65816::Prepared,actionc::compiler::CompileError> {
        native65816::prepare_file(&self.0,opt,&ModuleLoadOptions::default())
    }
}
impl Drop for Source { fn drop(&mut self){let _=std::fs::remove_file(&self.0);} }

#[test]
fn decoded_bytes_and_host_newlines_are_independent_of_action_strings() {
    let escapes=r#"c"\n\r\t\b\f\v\a\\\"\'\?\0\x80\xff""#;
    let tokens=lexer::tokenize(escapes).unwrap();
    assert_eq!(tokens[0].kind,lexer::TokenKind::CString(vec![10,13,9,8,12,11,7,92,34,39,63,0,128,255]));
    for invalid in [r#"c"\q""#,r#"c"\x1""#,r#"c"\xZZ""#,r#"c"\{clear}""#,"c\"a\nb\"","c\"a\rb\"","c\"λ\""] {
        assert!(lexer::tokenize(invalid).is_err(),"{invalid}");
    }
    let source=format!("BYTE ARRAY text=c\"{}\"\nSIZE length\nPROC Main()\nlength=SIZEOF(text)\nRETURN\n","x".repeat(300));
    for opt in [false,true] {
        let mut previous=None;
        for newline in ["\n","\r\n"] {
            let p=Source::new(&source.replace('\n',newline)).prepare(opt).unwrap();
            let compiled=p.compile_o65(&Default::default()).unwrap();
            if let Some(bytes)=&previous { assert_eq!(&compiled.bytes,bytes); }
            previous=Some(compiled.bytes);
        }
    }
}

#[test]
fn rejects_read_only_stores_and_implicit_string_conversions() {
    for source in [
        "CSTRING text PROC Main() text=c\"hi\" text(0)=1 RETURN",
        "CSTRING text PROC Main() text=c\"hi\" text^=1 RETURN",
        "CSTRING text PROC Main() text=\"hi\" RETURN",
        "CSTRING text BYTE ARRAY bytes(4) PROC Main() text=bytes RETURN",
        "CSTRING text PROC Main() text=0 RETURN",
        "CSTRING text BYTE POINTER bytes PROC Main() bytes=text RETURN",
        "PROC Accept(CSTRING text) RETURN PROC Main() Accept(\"hi\") RETURN",
        "BYTE ARRAY bytes(3)=c\"abc\" PROC Main() RETURN",
        "CARD ARRAY bytes=c\"abc\" PROC Main() RETURN",
        "CSTRING POINTER text PROC Main() RETURN",
        "CSTRING text=c\"abc\" PROC Main() RETURN",
        "CSTRING text=\"abc\" PROC Main() RETURN",
    ] {
        assert!(Source::new(source).prepare(false).is_err(),"accepted: {source}");
    }
    for source in ["CSTRING text PROC Main() RETURN", "BYTE ARRAY text=c\"hello\" PROC Main() RETURN"] {
        let ast=parser::parse(&lexer::tokenize(source).unwrap()).unwrap();
        let errors=semantic::analyze_with_options(&ast,semantic::SemanticOptions::modern().with_target(TargetId::Motorola68000)).unwrap_err();
        assert!(errors.iter().any(|d|d.message.contains("wdc-65816-native")));
    }
    Source::new("TYPE CSTRING=[BYTE value] CSTRING item PROC Main() item.value=1 RETURN").prepare(false).unwrap();
}

const ALL:&str=r#"MODULE TEST
USE CSTRING AS STR
CSTRING text
SIZE result
INT order
BYTE ARRAY buffer(8)
PROC Main()
 text=c"abc"
 result=STR.strlen(text)
 result=STR.strnlen(buffer,8)
 order=STR.strcmp(text,c"")
 order=STR.strncmp(text,c"a",1)
 text=STR.strchr(text,0)
 result=STR.strlcpy(buffer,text,8)
 result=STR.strlcat(buffer,text,8)
 result=STR.u32toa($ffffffff,buffer,8)
RETURN
ENDMODULE"#;

#[test]
fn external_interface_matches_single_implementation_and_only_used_imports() {
    let externs=Source::new(ALL).prepare(true).unwrap();
    let implementation=Source::new(&ALL.replace("USE CSTRING AS STR","USE CSTRING.IMPL AS STR")).prepare(true).unwrap();
    let signatures:Vec<_>=externs.mir.routines.iter().filter(|r|r.entry.external).collect();
    assert_eq!(signatures.len(),8);
    let contract:serde_json::Value=serde_json::from_str(include_str!("../embedded/modules/cstring/contract.json")).unwrap();
    for declaration in signatures {
        let name=declaration.name.rsplit('.').next().unwrap().to_ascii_uppercase();
        let body=implementation.mir.routines.iter().find(|r|r.name.starts_with(&format!("M_CSTRING_IMPL_{name}_"))).unwrap();
        assert_eq!(body.signature,declaration.signature,"{name}");
        assert_eq!(body.frame.incoming_extent,declaration.frame.incoming_extent,"{name}");
        assert_eq!(body.result_home,declaration.result_home,"{name}");
        assert_eq!(u64::from(body.signature.0),contract["providers"][name.to_ascii_lowercase()]["signature"].as_u64().unwrap(),"{name}");
    }
    let caller=Source::new("MODULE TEST USE CSTRING AS STR SIZE result PROC Main() result=STR.strlen(c\"abc\") RETURN ENDMODULE").prepare(true).unwrap();
    assert_eq!(caller.mir.routines.iter().filter(|r|r.entry.external).count(),1);
    assert_eq!(caller.mir.routines.iter().filter(|r|!r.entry.external).count(),1);
}
