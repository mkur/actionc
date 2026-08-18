use super::diagnostics::MirDiagnostic;
use super::ir::{MirProgram, MirRuntimeHelper, MirRuntimeHelperTarget};
use crate::runtime::Runtime;

pub(super) fn resolve_helpers(
    program: &mut MirProgram,
    runtime: Runtime,
) -> Result<(), Vec<MirDiagnostic>> {
    program
        .runtime_helpers
        .sort_by_key(|declaration| declaration.helper);

    for declaration in &mut program.runtime_helpers {
        if !matches!(declaration.target, MirRuntimeHelperTarget::Deferred) {
            continue;
        }
        declaration.target = match runtime {
            Runtime::ActionCart => {
                MirRuntimeHelperTarget::KnownAbsolute(cartridge_address(declaration.helper))
            }
            Runtime::Standalone => {
                return Err(vec![MirDiagnostic {
                    routine: None,
                    block: None,
                    message: format!(
                        "standalone implementation for runtime helper `{}` is not linked yet",
                        helper_name(declaration.helper)
                    ),
                }]);
            }
        };
    }
    Ok(())
}

pub(super) const fn helper_name(helper: MirRuntimeHelper) -> &'static str {
    match helper {
        MirRuntimeHelper::Mul => "MultI",
        MirRuntimeHelper::Div => "DivI",
        MirRuntimeHelper::Mod => "RemI",
        MirRuntimeHelper::Lsh => "LShift",
        MirRuntimeHelper::Rsh => "RShift",
        MirRuntimeHelper::SArgs => "SArgs",
    }
}

fn cartridge_address(helper: MirRuntimeHelper) -> u16 {
    use crate::codegen::runtime_helper;

    match helper {
        MirRuntimeHelper::Mul => runtime_helper::CARTRIDGE_MUL.address(),
        MirRuntimeHelper::Div => runtime_helper::CARTRIDGE_DIV.address(),
        MirRuntimeHelper::Mod => runtime_helper::CARTRIDGE_MOD.address(),
        MirRuntimeHelper::Lsh => runtime_helper::CARTRIDGE_LSH.address(),
        MirRuntimeHelper::Rsh => runtime_helper::CARTRIDGE_RSH.address(),
        MirRuntimeHelper::SArgs => runtime_helper::CARTRIDGE_SARGS.address(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mir6502::ir::MirRuntimeHelperDecl;

    #[test]
    fn cart_resolution_preserves_established_addresses() {
        let mut program = MirProgram {
            statics: Vec::new(),
            globals: Vec::new(),
            routines: Vec::new(),
            machine_blocks: Vec::new(),
            runtime_helpers: [
                MirRuntimeHelper::Mul,
                MirRuntimeHelper::Div,
                MirRuntimeHelper::Mod,
                MirRuntimeHelper::Lsh,
                MirRuntimeHelper::Rsh,
                MirRuntimeHelper::SArgs,
            ]
            .into_iter()
            .map(|helper| MirRuntimeHelperDecl {
                helper,
                target: MirRuntimeHelperTarget::Deferred,
                abi: crate::mir6502::materialize::helper_abi(),
                effects: crate::mir6502::materialize::helper_effects(&helper),
            })
            .collect(),
        };

        resolve_helpers(&mut program, Runtime::ActionCart).unwrap();

        let addresses = program
            .runtime_helpers
            .iter()
            .map(|decl| match decl.target {
                MirRuntimeHelperTarget::KnownAbsolute(address) => address,
                _ => panic!("cart helper was not resolved to an absolute address"),
            })
            .collect::<Vec<_>>();
        assert_eq!(
            addresses,
            vec![0xA000, 0xA090, 0xA0DE, 0xB5C0, 0xA0E6, 0xA0F5]
        );
    }
}
