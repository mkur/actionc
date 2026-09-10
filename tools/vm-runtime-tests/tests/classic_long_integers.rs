use actionc::codegen::{
    CodegenProfile, format_load_file, generate_semir_profile_at_origin_with_runtime,
};
use actionc::compiler::Runtime;
use actionc_vm::{
    CompilerVm, DEFAULT_CART_BASE, ExecutionProfile, ImageKind, OS_ROM_BASE, RunRequest,
    StopReason, VmRunner,
};
use std::path::Path;

fn check(source: &str, inputs: &[(u32, u32)], expected: impl Fn(u32, u32, &mut [u8])) {
    let ast = actionc::parser::parse(&actionc::lexer::tokenize(source).unwrap()).unwrap();
    let model =
        actionc::semantic::analyze_with_options(&ast, actionc::semantic::SemanticOptions::modern())
            .unwrap();
    let semir = actionc::semantic::ir::lower_program(&ast, &model);
    for profile in [CodegenProfile::Compat, CodegenProfile::Modern] {
        for runtime in [Runtime::ActionCart, Runtime::Standalone] {
            let output =
                generate_semir_profile_at_origin_with_runtime(&semir, 0x3000, profile, runtime)
                    .unwrap();
            let image = format_load_file(&output);
            for &(a, b) in inputs {
                let mut vm = CompilerVm::default();
                let execution = if runtime == Runtime::ActionCart {
                    for (kind, name, base) in [
                        (ImageKind::Cartridge, "action.rom", DEFAULT_CART_BASE),
                        (ImageKind::Rom, "altirraos-xl.rom", OS_ROM_BASE),
                    ] {
                        vm.load_image_bytes(
                            kind,
                            name,
                            base,
                            std::fs::read(
                                Path::new(env!("CARGO_MANIFEST_DIR"))
                                    .join("../../roms")
                                    .join(name),
                            )
                            .unwrap(),
                        )
                        .unwrap();
                    }
                    ExecutionProfile::CartridgeObject
                } else {
                    ExecutionProfile::StandaloneObject
                };
                let load = vm
                    .load_atari_object_for_execution(execution, &image)
                    .unwrap();
                assert!(
                    load.segments
                        .iter()
                        .all(|segment| segment.end < 0x600 || segment.start > 0x6FF)
                );
                let mut page = vec![0xCC; 256];
                word(&mut page, 0xE0, a);
                word(&mut page, 0xE4, b);
                for (offset, &value) in page.iter().enumerate() {
                    vm.bus_mut().ram_mut().write(0x600 + offset as u16, value);
                }
                expected(a, b, &mut page);
                page[255] = 0xA5;
                let outcome = VmRunner::new(vm).run(RunRequest {
                    max_steps: 100_000,
                    history_len: 16,
                    ..Default::default()
                });
                assert_eq!(
                    outcome.stop_reason(),
                    StopReason::StepLimit { max_steps: 100_000 }
                );
                let actual: Vec<_> = (0x600..=0x6FF)
                    .map(|address| outcome.memory().read(address))
                    .collect();
                assert_eq!(
                    actual, page,
                    "{profile:?}/{runtime:?} a={a:08X} b={b:08X}; {:?}",
                    outcome.report
                );
            }
        }
    }
}

fn word(page: &mut [u8], offset: usize, value: u32) {
    page[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

#[test]
fn classic_long_arithmetic_casts_and_comparisons_keep_all_bytes() {
    let source = r#"
LONGCARD a=$6E0,b=$6E4
LONGCARD ARRAY out(16)=$601
BYTE ARRAY flags(12)=$650
BYTE done=$6FF
PROC Main()
 BYTE i
 out(0)=a+b out(1)=a-b out(2)=a&b out(3)=a% b out(4)=a XOR b
 out(5)=-LONGINT(a) out(6)=LONGCARD(INT(a))
 out(7)=LONGCARD(CARD(a)*CARD(b)) out(8)=LONGCARD(CARD(a))*CARD(b)
 out(9)=a LSH b out(10)=a RSH b out(11)=a*b
 out(12)=0 out(13)=0 out(14)=0 out(15)=0
 IF b#0 THEN out(12)=a/b out(13)=a MOD b out(14)=LONGINT(a)/LONGINT(b) out(15)=LONGINT(a) MOD LONGINT(b) FI
 FOR i=0 TO 11 DO flags(i)=0 OD
 IF a=b THEN flags(0)=1 FI
 IF a#b THEN flags(1)=1 FI
 IF a<b THEN flags(2)=1 FI
 IF a<=b THEN flags(3)=1 FI
 IF a>b THEN flags(4)=1 FI
 IF a>=b THEN flags(5)=1 FI
 IF LONGINT(a)=LONGINT(b) THEN flags(6)=1 FI
 IF LONGINT(a)#LONGINT(b) THEN flags(7)=1 FI
 IF LONGINT(a)<LONGINT(b) THEN flags(8)=1 FI
 IF LONGINT(a)<=LONGINT(b) THEN flags(9)=1 FI
 IF LONGINT(a)>LONGINT(b) THEN flags(10)=1 FI
 IF LONGINT(a)>=LONGINT(b) THEN flags(11)=1 FI
 done=$A5 DO OD
RETURN
"#;
    let values = [
        0u32, 1, 7, 31, 32, 255, 65535, 65536, 0x12345678, 0x7FFFFFFF, 0x80000000, 0xFFFFFFFF,
    ];
    let inputs: Vec<_> = values
        .into_iter()
        .flat_map(|a| values.into_iter().map(move |b| (a, b)))
        .collect();
    check(source, &inputs, |a, b, page| {
        let (sa, sb) = (a as i32, b as i32);
        let expected = [
            a.wrapping_add(b),
            a.wrapping_sub(b),
            a & b,
            a | b,
            a ^ b,
            a.wrapping_neg(),
            a as i16 as i32 as u32,
            (a as u16).wrapping_mul(b as u16) as i16 as i32 as u32,
            u32::from(a as u16) * u32::from(b as u16),
            if b < 32 { a << b } else { 0 },
            if b < 32 { a >> b } else { 0 },
            a.wrapping_mul(b),
            if b == 0 { 0 } else { a / b },
            if b == 0 { 0 } else { a % b },
            if sb == 0 {
                0
            } else {
                sa.wrapping_div(sb) as u32
            },
            if sb == 0 {
                0
            } else {
                sa.wrapping_rem(sb) as u32
            },
        ];
        for (index, value) in expected.into_iter().enumerate() {
            word(page, 1 + 4 * index, value);
        }
        page[0x50..0x5C].copy_from_slice(
            &[
                a == b,
                a != b,
                a < b,
                a <= b,
                a > b,
                a >= b,
                sa == sb,
                sa != sb,
                sa < sb,
                sa <= sb,
                sa > sb,
                sa >= sb,
            ]
            .map(u8::from),
        );
    });
}

#[test]
fn classic_long_calls_stage_all_bytes_and_preserve_nested_results() {
    let source = r#"
LONGCARD a=$6E0,b=$6E4,result=$601,repeated=$605
BYTE calls=$609,done=$6FF
LONGCARD FUNC Echo(LONGCARD value)
 calls==+1
RETURN(value)
LONGCARD FUNC Combine(LONGCARD left,right BYTE bias)
RETURN(left+right+LONGCARD(bias))
LONGCARD FUNC POINTER callback(LONGCARD value)
PROC Main()
 calls=0 callback=@Echo
 result=Combine(callback(a),Echo(b),7)
 repeated=Combine(Echo(b),callback(a),9)
 done=$A5 DO OD
RETURN
"#;
    check(
        source,
        &[
            (0, 0),
            (0x12345678, 0x87654321),
            (0xFFFFFFFF, 1),
            (0x7FFFFFFF, 0x10002),
        ],
        |a, b, page| {
            word(page, 1, a.wrapping_add(b).wrapping_add(7));
            word(page, 5, a.wrapping_add(b).wrapping_add(9));
            page[9] = 4;
        },
    );
}
