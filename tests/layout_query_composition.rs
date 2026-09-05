use std::fs;
use std::path::PathBuf;

use actionc::compiler::{CompileMode, CompileOptions, Runtime, compile_file};

struct TestDir(PathBuf);

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn composed_layout_queries_match_literals_without_folding_runtime_calls() {
    let temp = TestDir(std::env::temp_dir().join(format!(
        "actionc-layout-query-composition-{}",
        std::process::id()
    )));
    fs::create_dir(&temp.0).unwrap();
    let declarations = "TYPE Pair=[BYTE tag CARD word] TYPE Single=[BYTE tag] \
        BYTE ARRAY values(7) CARD a,b,c,d,e,f,g \
        CARD FUNC Live(CARD value) RETURN(value+2)";
    let queries = temp.0.join("queries.act");
    let literals = temp.0.join("literals.act");
    fs::write(
        &queries,
        format!(
            "{declarations} PROC Main() \
         CONST Width=SIZEOF(Pair)+SIZEOF(Single) \
         CONST Shape=SIZEOF(Pair)+OFFSETOF(Pair,word)+ALIGNOF(Pair) \
         CONST Entries=ELEMENTS(values)+SIZEOF(Pair) \
         a=Width b=SIZEOF(Pair)+Live(5) c=Live(5)+SIZEOF(Pair) \
         d=SIZEOF(Pair)+SIZEOF(Single) e=Shape f=Entries \
         g=BYTE(SIZEOF(Pair))+BYTE(SIZEOF(Single)) RETURN"
        ),
    )
    .unwrap();
    fs::write(
        &literals,
        format!(
            "{declarations} PROC Main() a=4 b=3+Live(5) c=Live(5)+3 \
         d=3+1 e=5 f=10 g=BYTE(3)+BYTE(1) RETURN"
        ),
    )
    .unwrap();
    for mode in [
        CompileMode::Compatibility,
        CompileMode::Optimized,
        CompileMode::Mir6502,
    ] {
        for runtime in [Runtime::ActionCart, Runtime::Standalone] {
            let options = CompileOptions::for_mode(mode).with_runtime(runtime);
            let actual = compile_file(&queries, &options)
                .unwrap_or_else(|error| panic!("{mode:?}/{runtime:?}: {error}"));
            let expected = compile_file(&literals, &options).unwrap();
            assert_eq!(
                actual.object_bytes(),
                expected.object_bytes(),
                "{mode:?}/{runtime:?}"
            );
        }
    }
}
