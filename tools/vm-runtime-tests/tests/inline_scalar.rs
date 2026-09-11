use actionc::compiler::Runtime;
use actionc::mir6502::{self, Mir6502Config};
use actionc_vm::{RunRequest, StopReason, VmRunner};
#[path = "support/fixed_point.rs"]
#[allow(dead_code)]
mod support;

fn compile(source: &str, inline: bool, runtime: Runtime) -> Vec<u8> {
    let ast = actionc::parser::parse(&actionc::lexer::tokenize(source).unwrap()).unwrap();
    let model =
        actionc::semantic::analyze_with_options(&ast, actionc::semantic::SemanticOptions::modern())
            .unwrap();
    let sem = actionc::semantic::ir::lower_program(&ast, &model);
    let nir = actionc::nir::optimize_program(&actionc::nir::lower_program(&sem)).unwrap();
    let mut config = Mir6502Config::optimized();
    config.enable_small_leaf_inlining = inline;
    let output =
        mir6502::generate_output_with_config_and_runtime(&nir, 0x3000, &config, runtime).unwrap();
    actionc::codegen::format_load_file(&output)
}

#[test]
fn complete_long_calls_preserve_both_results_across_repeated_calls_in_both_runtimes() {
    for (ty, operator) in [("LONGCARD", "+"), ("LONGINT", "-")] {
        let source = format!(
            "{ty} a=$600,b=$604,output=$610 BYTE done=$620 INLINE {ty} FUNC Combine({ty} left,right) RETURN(left{operator}right) PROC Main() output=Combine(a,b)+Combine(b,a) done=$A5 RETURN"
        );
        for runtime in [Runtime::ActionCart, Runtime::Standalone] {
            for inline in [false, true] {
                let image = compile(&source, inline, runtime);
                for a in [0u32, 1, 65535, 65536, 0x7fff_ffff, 0x8000_0000, u32::MAX] {
                    for b in [0u32, 1, 0x8000_0001, u32::MAX] {
                        let (mut vm, profile) = support::vm_for(runtime);
                        vm.load_atari_object_for_execution(profile, &image).unwrap();
                        vm.bus_mut().ram_mut().map(0x600, &a.to_le_bytes()).unwrap();
                        vm.bus_mut().ram_mut().map(0x604, &b.to_le_bytes()).unwrap();
                        let outcome = VmRunner::new(vm).run(RunRequest {
                            max_steps: 30_000,
                            history_len: 8,
                            ..Default::default()
                        });
                        assert_eq!(
                            outcome.stop_reason(),
                            StopReason::StepLimit { max_steps: 30_000 }
                        );
                        assert_eq!(
                            outcome.memory().read(0x620),
                            0xA5,
                            "{runtime:?}, inline={inline}: {:?}",
                            outcome.report
                        );
                        let result = u32::from_le_bytes(std::array::from_fn(|i| {
                            outcome.memory().read(0x610 + i as u16)
                        }));
                        let expected = if operator == "+" {
                            a.wrapping_add(b).wrapping_mul(2)
                        } else {
                            0
                        };
                        assert_eq!(
                            result, expected,
                            "{runtime:?}, inline={inline}, {operator}: {a},{b}"
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn retained_wide_helpers_preserve_results_order_and_nonreturning_faults() {
    use actionc::compiler::CompileMode;
    for (expression, kind) in [
        ("LONGINT(left)*LONGINT(right)", 0),
        ("LONGINT(left)/LONGINT(right)", 1),
        ("LONGINT(left) MOD LONGINT(right)", 2),
        (
            "(LONGINT(left)*LONGINT(right))+(LONGINT(right)*LONGINT(left+1))",
            3,
        ),
    ] {
        let source = format!(
            "INT a=$6E0,b=$6E2 LONGINT output=$600 BYTE done=$6DF INLINE LONGINT FUNC Map(INT left,right) RETURN({expression}) PROC Main() output=Map(a,b) done=$A5 RETURN"
        );
        for runtime in [Runtime::ActionCart, Runtime::Standalone] {
            for inline in [false, true] {
                let image = compile(&source, inline, runtime);
                for a in [i16::MIN, -4097, -1, 0, 1, 4095, i16::MAX] {
                    for b in [i16::MIN, -4095, -1, 0, 1, 4096, i16::MAX] {
                        let fault = (kind == 1 || kind == 2) && b == 0;
                        support::execute(
                            &image,
                            CompileMode::Mir6502,
                            runtime,
                            (a, b),
                            fault,
                            |page| {
                                if fault {
                                    return;
                                }
                                let (left, right) = (i32::from(a), i32::from(b));
                                let value = match kind {
                                    0 => left.wrapping_mul(right),
                                    1 => left.wrapping_div(right),
                                    2 => left.wrapping_rem(right),
                                    _ => left.wrapping_mul(right).wrapping_add(
                                        right.wrapping_mul(i32::from(a.wrapping_add(1))),
                                    ),
                                };
                                page[..4].copy_from_slice(&value.to_le_bytes());
                            },
                        );
                    }
                }
            }
        }
    }
}
