//! Same source corpus for NIR proof baselines and VM cost/oracle checks.

pub const SHAPES: [&str; 3] = ["record", "union", "variant"];
pub const CASES: [&str; 4] = [
    "fresh_call",
    "capture_chain",
    "snapshot_mutation",
    "ordered_arguments",
];

pub fn source(shape: &str, case: &str) -> String {
    let declaration = match shape {
        "record" => "TYPE Value=[BYTE ARRAY bytes(3)]",
        "union" => "TYPE Value=UNION [CARD word BYTE ARRAY bytes(3)]",
        "variant" => "TYPE Value=VARIANT [NONE SOME [BYTE first,second,tail]]",
        _ => unreachable!(),
    };
    let make = if shape == "variant" {
        "Value FUNC Make(BYTE n) RETURN(Value.SOME(n,n+1,$A5))"
    } else {
        "Value FUNC Make(BYTE n) Value made made.bytes(0)=n made.bytes(1)=n+1 made.bytes(2)=$A5 RETURN(made)"
    };
    let (helpers, body) = match case {
        "fresh_call" => ("", "LET saved=Make(seed)"),
        "capture_chain" => ("", "original=Make(seed) LET first=original LET saved=first"),
        "snapshot_mutation" => (
            "",
            "original=Make(seed) LET saved=original original=Make(99)",
        ),
        "ordered_arguments" => (
            "BYTE FUNC Mutate() original=Make(99) RETURN(0) Value FUNC Choose(Value item BYTE later) RETURN(item)",
            "original=Make(seed) LET saved=Choose(original,Mutate())",
        ),
        _ => unreachable!(),
    };
    let read = if shape == "variant" {
        "CASE saved OF\nWHEN Value.SOME(a,b,c) THEN\noutput(0)=a output(1)=b output(2)=c\nELSE\noutput(0)=255\nESAC"
    } else {
        "output(0)=saved.bytes(0) output(1)=saved.bytes(1) output(2)=saved.bytes(2)"
    };
    let changed = matches!(case, "snapshot_mutation" | "ordered_arguments");
    let after = if !changed {
        "output(3)=0"
    } else if shape == "variant" {
        "CASE original OF\nWHEN Value.SOME(a,_,_) THEN\noutput(3)=a\nELSE\noutput(3)=255\nESAC"
    } else {
        "output(3)=original.bytes(0)"
    };
    format!(
        "{declaration}\nValue original\nBYTE ARRAY output=$600\nBYTE seed=$690,done=$67F\n{make}\n{helpers}\nPROC Main()\n{body}\n{read}\n{after}\ndone=$A5 DO OD RETURN\n"
    )
}

pub fn expected(case: &str, seed: u8) -> [u8; 4] {
    [
        seed,
        seed.wrapping_add(1),
        0xA5,
        if matches!(case, "snapshot_mutation" | "ordered_arguments") {
            99
        } else {
            0
        },
    ]
}
