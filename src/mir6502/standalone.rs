use std::collections::{BTreeMap, BTreeSet};
use std::sync::OnceLock;

use super::diagnostics::MirDiagnostic;
use super::ir::{
    MirCallTarget, MirInlineAsmTarget, MirMachineBlockId, MirMachineItem, MirOp, MirProgram,
    MirRuntimeHelper, MirRuntimeHelperTarget, RoutineId,
};
use crate::embedded_vfs::EmbeddedSourceProvider;
use crate::includes::{ModuleLoadOptions, load_compilation_from_provider};
use crate::semantic::{analyze_compilation, ir};
use crate::source::{InMemorySourceProvider, SourceOrigin};

static SYSLIB_MIR: OnceLock<Result<MirProgram, Vec<MirDiagnostic>>> = OnceLock::new();

pub(super) fn syslib_mir() -> Result<MirProgram, Vec<MirDiagnostic>> {
    SYSLIB_MIR.get_or_init(compile_syslib).clone()
}

pub(super) fn link_helpers(program: &mut MirProgram) -> Result<(), Vec<MirDiagnostic>> {
    for declaration in &program.runtime_helpers {
        match declaration.target {
            MirRuntimeHelperTarget::KnownAbsolute(address) => {
                return Err(diagnostic(format!(
                    "standalone runtime rejects absolute override ${address:04X} for `{}`",
                    super::runtime::helper_name(declaration.helper)
                )));
            }
            MirRuntimeHelperTarget::Routine(id) => {
                if !program.routines.iter().any(|routine| routine.id == id) {
                    return Err(diagnostic(format!(
                        "local override for `{}` refers to missing routine r{}",
                        super::runtime::helper_name(declaration.helper),
                        id.0
                    )));
                }
            }
            MirRuntimeHelperTarget::Deferred => {}
        }
    }

    let required = program
        .runtime_helpers
        .iter()
        .filter(|declaration| matches!(declaration.target, MirRuntimeHelperTarget::Deferred))
        .map(|declaration| declaration.helper)
        .collect::<BTreeSet<_>>();
    if required.is_empty() {
        return Ok(());
    }

    let runtime = syslib_mir()?;
    let roots = required
        .iter()
        .map(|helper| {
            validate_helper_contract(program, &runtime, *helper)?;
            find_runtime_helper(&runtime, *helper).map(|routine| (*helper, routine))
        })
        .collect::<Result<BTreeMap<_, _>, _>>()?;
    let selected = dependency_closure(&runtime, roots.values().copied().collect::<BTreeSet<_>>())?;
    let routine_rebase = selected
        .iter()
        .copied()
        .enumerate()
        .map(|(index, old)| {
            (
                old,
                RoutineId(next_routine_id(program).wrapping_add(index as u32)),
            )
        })
        .collect::<BTreeMap<_, _>>();

    let selected_machine_ids = runtime
        .routines
        .iter()
        .filter(|routine| selected.contains(&routine.id))
        .flat_map(|routine| routine.blocks.iter())
        .flat_map(|block| block.ops.iter())
        .filter_map(|op| match op {
            MirOp::MachineBlock { id, .. } => Some(*id),
            _ => None,
        })
        .collect::<BTreeSet<_>>();
    let machine_base = next_machine_id(program);
    let machine_rebase = selected_machine_ids
        .iter()
        .copied()
        .enumerate()
        .map(|(index, old)| {
            (
                old,
                MirMachineBlockId(machine_base.wrapping_add(index as u32)),
            )
        })
        .collect::<BTreeMap<_, _>>();

    for old_id in &selected {
        let mut routine = runtime
            .routines
            .iter()
            .find(|routine| routine.id == *old_id)
            .expect("selected runtime routine exists")
            .clone();
        routine.id = routine_rebase[old_id];
        routine.name = format!(
            "ACTION.RUNTIME.SYSLIB::{}",
            runtime_routine_name(&routine.name)
        );
        for block in &mut routine.blocks {
            for op in &mut block.ops {
                match op {
                    MirOp::Call {
                        target: MirCallTarget::Routine(id),
                        ..
                    } => *id = rebased_routine(*id, &routine_rebase)?,
                    MirOp::MachineBlock { id, .. } => {
                        *id = *machine_rebase.get(id).ok_or_else(|| {
                            diagnostic(format!(
                                "runtime routine `{}` references unselected machine block m{}",
                                routine.name, id.0
                            ))
                        })?
                    }
                    _ => {}
                }
            }
        }
        program.routines.push(routine);
    }

    for old_id in &selected_machine_ids {
        let mut machine = runtime
            .machine_blocks
            .iter()
            .find(|machine| machine.id == *old_id)
            .expect("selected runtime machine block exists")
            .clone();
        machine.id = machine_rebase[old_id];
        for item in &mut machine.items {
            if let MirMachineItem::Relocation {
                target: MirInlineAsmTarget::Routine(id),
                ..
            } = item
            {
                *id = rebased_routine(*id, &routine_rebase)?;
            }
        }
        program.machine_blocks.push(machine);
    }

    for declaration in &mut program.runtime_helpers {
        if matches!(declaration.target, MirRuntimeHelperTarget::Deferred) {
            declaration.target =
                MirRuntimeHelperTarget::Routine(routine_rebase[&roots[&declaration.helper]]);
        }
    }
    Ok(())
}

fn validate_helper_contract(
    application: &MirProgram,
    runtime: &MirProgram,
    helper: MirRuntimeHelper,
) -> Result<(), Vec<MirDiagnostic>> {
    let application = application
        .runtime_helpers
        .iter()
        .find(|declaration| declaration.helper == helper)
        .ok_or_else(|| {
            diagnostic(format!(
                "application has no logical declaration for runtime helper `{}`",
                super::runtime::helper_name(helper)
            ))
        })?;
    let implementation = runtime
        .runtime_helpers
        .iter()
        .find(|declaration| declaration.helper == helper)
        .ok_or_else(|| {
            diagnostic(format!(
                "embedded SYSLIB has no contract for runtime helper `{}`",
                super::runtime::helper_name(helper)
            ))
        })?;
    if application.abi != implementation.abi {
        return Err(diagnostic(format!(
            "ABI mismatch for standalone runtime helper `{}`",
            super::runtime::helper_name(helper)
        )));
    }
    if application.effects != implementation.effects {
        return Err(diagnostic(format!(
            "effect mismatch for standalone runtime helper `{}`",
            super::runtime::helper_name(helper)
        )));
    }
    Ok(())
}

fn find_runtime_helper(
    program: &MirProgram,
    helper: MirRuntimeHelper,
) -> Result<RoutineId, Vec<MirDiagnostic>> {
    let matches = program
        .runtime_helpers
        .iter()
        .filter(|declaration| declaration.helper == helper)
        .filter_map(|declaration| match declaration.target {
            MirRuntimeHelperTarget::Routine(id) => Some(id),
            _ => None,
        })
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [id] => Ok(*id),
        [] => Err(diagnostic(format!(
            "embedded SYSLIB has no implementation for `{}`",
            super::runtime::helper_name(helper)
        ))),
        _ => Err(diagnostic(format!(
            "embedded SYSLIB has more than one implementation for `{}`",
            super::runtime::helper_name(helper)
        ))),
    }
}

fn runtime_routine_name(name: &str) -> &str {
    let key = name
        .strip_prefix("M_ACTION_RUNTIME_SYSLIB_")
        .and_then(|name| name.rsplit_once('_').map(|(name, _)| name))
        .unwrap_or(name);
    match key {
        "ERROR" => "Error",
        "BREAK" => "Break",
        "LSHIFT" => "LShift",
        "RSHIFT" => "RShift",
        "SETSIGN" => "SetSign",
        "SS1" => "SS1",
        "SMOPS" => "SMOps",
        "MULTB" => "MultB",
        "MULTI" => "MultI",
        "DIVI" => "DivI",
        "REMI" => "RemI",
        "SARGS" => "SArgs",
        _ => key,
    }
}

fn dependency_closure(
    program: &MirProgram,
    roots: BTreeSet<RoutineId>,
) -> Result<BTreeSet<RoutineId>, Vec<MirDiagnostic>> {
    let all_ids = program
        .routines
        .iter()
        .map(|routine| routine.id)
        .collect::<BTreeSet<_>>();
    let machine_blocks = program
        .machine_blocks
        .iter()
        .map(|machine| (machine.id, machine))
        .collect::<BTreeMap<_, _>>();
    let mut selected = BTreeSet::new();
    let mut pending = roots;
    while let Some(id) = pending.pop_first() {
        if !all_ids.contains(&id) {
            return Err(diagnostic(format!(
                "embedded runtime dependency r{} is missing",
                id.0
            )));
        }
        if !selected.insert(id) {
            continue;
        }
        let routine = program
            .routines
            .iter()
            .find(|routine| routine.id == id)
            .expect("validated runtime routine exists");
        for block in &routine.blocks {
            for op in &block.ops {
                match op {
                    MirOp::Call {
                        target: MirCallTarget::Routine(target),
                        ..
                    } => {
                        pending.insert(*target);
                    }
                    MirOp::MachineBlock { id, .. } => {
                        let machine = machine_blocks.get(id).ok_or_else(|| {
                            diagnostic(format!(
                                "runtime routine `{}` refers to missing machine block m{}",
                                routine.name, id.0
                            ))
                        })?;
                        for item in &machine.items {
                            if let MirMachineItem::Relocation {
                                target: MirInlineAsmTarget::Routine(target),
                                ..
                            } = item
                            {
                                pending.insert(*target);
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    Ok(selected)
}

fn rebased_routine(
    old: RoutineId,
    rebase: &BTreeMap<RoutineId, RoutineId>,
) -> Result<RoutineId, Vec<MirDiagnostic>> {
    rebase.get(&old).copied().ok_or_else(|| {
        diagnostic(format!(
            "runtime dependency r{} was not included in the selected closure",
            old.0
        ))
    })
}

fn next_routine_id(program: &MirProgram) -> u32 {
    program
        .routines
        .iter()
        .map(|routine| routine.id.0)
        .max()
        .map_or(0, |id| id.wrapping_add(1))
}

fn next_machine_id(program: &MirProgram) -> u32 {
    program
        .machine_blocks
        .iter()
        .map(|machine| machine.id.0)
        .max()
        .map_or(0, |id| id.wrapping_add(1))
}

fn compile_syslib() -> Result<MirProgram, Vec<MirDiagnostic>> {
    let source = EmbeddedSourceProvider
        .runtime_source("syslib.act")
        .ok_or_else(|| diagnostic("embedded runtime source `SYSLIB.ACT` is missing"))?;
    let text = crate::source::decode_source(source.bytes);
    let text = make_internal_named_module(&text)?;
    let origin = SourceOrigin::embedded("runtime/internal/syslib.act", "<runtime:SYSLIB.ACT>");
    let provider = InMemorySourceProvider::default().with_source(origin.clone(), text);
    let loaded = load_compilation_from_provider(origin, &provider, &ModuleLoadOptions::default())
        .map_err(frontend_diagnostics)?;
    let model = analyze_compilation(&loaded).map_err(frontend_diagnostics)?;
    let semir = ir::lower_compilation(&loaded, &model);
    let nir = crate::nir::lower_program(&semir);
    crate::nir::verify_program(&nir).map_err(|diagnostics| {
        diagnostics
            .into_iter()
            .map(|diagnostic| MirDiagnostic {
                routine: diagnostic.routine,
                block: diagnostic.block,
                message: format!("embedded SYSLIB NIR: {}", diagnostic.message),
            })
            .collect::<Vec<_>>()
    })?;
    super::lower_program(&nir).map_err(|diagnostics| {
        diagnostics
            .into_iter()
            .map(|mut diagnostic| {
                diagnostic.message = format!("embedded SYSLIB MIR: {}", diagnostic.message);
                diagnostic
            })
            .collect()
    })
}

fn make_internal_named_module(source: &str) -> Result<String, Vec<MirDiagnostic>> {
    let mut converted_first_marker = false;
    let mut output = String::with_capacity(source.len() + 32);
    for line in source.lines() {
        let trimmed = line.trim_start();
        if trimmed.to_ascii_uppercase().starts_with("MODULE ;") {
            if !converted_first_marker {
                output.push_str("MODULE ACTION.RUNTIME.SYSLIB");
                converted_first_marker = true;
            } else {
                output.push_str("ENDMODULE");
            }
        } else {
            output.push_str(line);
        }
        output.push('\n');
    }
    if !converted_first_marker {
        return Err(diagnostic(
            "embedded runtime source `SYSLIB.ACT` has no legacy MODULE marker",
        ));
    }
    Ok(output)
}

fn frontend_diagnostics(diagnostics: Vec<crate::diagnostic::Diagnostic>) -> Vec<MirDiagnostic> {
    diagnostics
        .into_iter()
        .map(|diagnostic| MirDiagnostic {
            routine: None,
            block: None,
            message: format!("embedded SYSLIB frontend: {}", diagnostic.message),
        })
        .collect()
}

fn diagnostic(message: impl Into<String>) -> Vec<MirDiagnostic> {
    vec![MirDiagnostic {
        routine: None,
        block: None,
        message: message.into(),
    }]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::asm6502::InlineAsmRelocationKind;
    use crate::mir6502::MirOp;
    use crate::mir6502::ir::{MirInlineAsmTarget, MirMachineItem, MirRuntimeHelperDecl};
    use crate::source::Span;

    #[test]
    fn embedded_syslib_is_lowered_with_resolved_local_machine_references() {
        let program = syslib_mir().expect("compile embedded SYSLIB");
        let sargs = program
            .routines
            .iter()
            .find(|routine| routine.name.to_ascii_uppercase().contains("SARGS"))
            .unwrap_or_else(|| {
                panic!(
                    "SArgs routine; found {:?}",
                    program
                        .routines
                        .iter()
                        .map(|routine| &routine.name)
                        .collect::<Vec<_>>()
                )
            });
        let machine_id = sargs.blocks.iter().find_map(|block| {
            block.ops.iter().find_map(|op| match op {
                MirOp::MachineBlock { id, .. } => Some(*id),
                _ => None,
            })
        });
        let machine = program
            .machine_blocks
            .iter()
            .find(|machine| Some(machine.id) == machine_id)
            .expect("SArgs machine block");
        assert!(machine.items.iter().any(|item| {
            matches!(
                item,
                MirMachineItem::Relocation {
                    target: MirInlineAsmTarget::Routine(_),
                    ..
                }
            )
        }));
    }

    #[test]
    fn dependency_closure_terminates_and_is_stable_for_a_recursive_group() {
        let mut program = syslib_mir().expect("compile embedded SYSLIB");
        let multi = find_runtime_helper(&program, MirRuntimeHelper::Mul).expect("MultI root");
        let set_sign = program
            .routines
            .iter()
            .find(|routine| runtime_routine_name(&routine.name) == "SetSign")
            .expect("SetSign dependency");
        let machine_id = set_sign
            .blocks
            .iter()
            .flat_map(|block| &block.ops)
            .find_map(|op| match op {
                MirOp::MachineBlock { id, .. } => Some(*id),
                _ => None,
            })
            .expect("SetSign machine block");
        program
            .machine_blocks
            .iter_mut()
            .find(|machine| machine.id == machine_id)
            .expect("SetSign machine payload")
            .items
            .push(MirMachineItem::Relocation {
                kind: InlineAsmRelocationKind::Absolute16,
                target: MirInlineAsmTarget::Routine(multi),
                addend: 0,
                requires_zero_page: false,
                span: Span::new(0, 0),
            });

        let first = dependency_closure(&program, BTreeSet::from([multi])).unwrap();
        let second = dependency_closure(&program, BTreeSet::from([multi])).unwrap();
        assert_eq!(first, second);
        assert!(first.contains(&multi));
        assert!(first.contains(&set_sign.id));
    }

    #[test]
    fn standalone_linking_rejects_a_logical_helper_contract_mismatch() {
        let runtime = syslib_mir().expect("compile embedded SYSLIB");
        let mut declaration = runtime
            .runtime_helpers
            .iter()
            .find(|declaration| declaration.helper == MirRuntimeHelper::Mul)
            .expect("MultI declaration")
            .clone();
        declaration.target = MirRuntimeHelperTarget::Deferred;
        declaration.effects.opaque = !declaration.effects.opaque;
        let mut application = MirProgram {
            statics: Vec::new(),
            globals: Vec::new(),
            routines: Vec::new(),
            machine_blocks: Vec::new(),
            runtime_helpers: vec![MirRuntimeHelperDecl { ..declaration }],
        };

        let diagnostics = link_helpers(&mut application).expect_err("reject effect mismatch");
        assert!(diagnostics[0].message.contains("effect mismatch"));
    }
}
