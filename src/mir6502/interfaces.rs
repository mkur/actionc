use std::collections::{BTreeMap, BTreeSet};

use crate::runtime::Runtime;
use crate::runtime_bindings::{BindingTarget, binding_key, parse_bindings};
use crate::runtime_source::{RuntimeImage, RuntimeUnit, resolve_runtime_unit};

use super::diagnostics::MirDiagnostic;
use super::ir::{
    MirCallTarget, MirDataImage, MirDataRelocationTarget, MirGlobalInit, MirInlineAsmTarget,
    MirMachineItem, MirOp, MirProgram, MirRoutine, MirRoutineAbi, MirStorageInit, MirValue,
    RoutineId,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ResolvedTarget {
    Absolute(u16),
    Routine(RoutineId),
}

const RESIDENT_MODULE: &str = "ACTION.RUNTIME.RESIDENT";
const RESIDENT_LINK_MODULE: &str = "ACTION_RUNTIME_RESIDENT";

pub(super) fn resolve_interfaces(
    program: &mut MirProgram,
    runtime: Runtime,
) -> Result<(), Vec<MirDiagnostic>> {
    let external = program
        .routines
        .iter()
        .filter(|routine| routine.abi == MirRoutineAbi::ExternalInterface)
        .map(|routine| (routine.id, routine.name.clone()))
        .collect::<BTreeMap<_, _>>();
    if external.is_empty() {
        return Ok(());
    }

    let referenced = referenced_external_routines(program, &external);
    let bindings = parse_bindings(runtime).map_err(frontend_diagnostics)?;
    let mut resolved = BTreeMap::new();

    match runtime {
        Runtime::ActionCart => {
            for id in &referenced {
                let name = &external[id];
                match bindings.get(&binding_key(name)) {
                    Some(BindingTarget::Absolute(address)) => {
                        resolved.insert(*id, ResolvedTarget::Absolute(*address));
                    }
                    Some(BindingTarget::RuntimeRoutine { .. }) => {
                        return Err(diagnostic(format!(
                            "cart binding for external `{name}` is not an absolute address"
                        )));
                    }
                    None => return Err(missing_binding(name, runtime)),
                }
            }
        }
        Runtime::Standalone => {
            let interface_signatures = sys_interface_signatures()?;
            let mut external_roots = BTreeMap::<RoutineId, (RuntimeUnit, String)>::new();
            for id in &referenced {
                let name = &external[id];
                let Some(binding) = bindings.get(&binding_key(name)) else {
                    return Err(missing_binding(name, runtime));
                };
                let BindingTarget::RuntimeRoutine { unit, routine } = binding else {
                    return Err(diagnostic(format!(
                        "standalone binding for external `{name}` is an absolute address"
                    )));
                };
                let unit = resolve_runtime_unit(unit).map_err(frontend_diagnostics)?;
                external_roots.insert(*id, (unit, routine.clone()));
            }

            let (runtime_image, runtime) = super::standalone::compile_runtime_image_with_semir()?;
            let mut implementation_roots = BTreeMap::new();
            for (external_id, (unit, routine)) in external_roots {
                let interface_name = &external[&external_id];
                validate_semantic_abi(
                    interface_name,
                    &interface_signatures,
                    &runtime_image,
                    &unit,
                    &routine,
                )?;
                let implementation =
                    find_runtime_routine(&runtime, RESIDENT_LINK_MODULE, &routine)?;
                validate_abi(
                    program
                        .routines
                        .iter()
                        .find(|candidate| candidate.id == external_id)
                        .expect("external routine exists"),
                    runtime
                        .routines
                        .iter()
                        .find(|candidate| candidate.id == implementation)
                        .expect("runtime routine exists"),
                )?;
                implementation_roots.insert(external_id, implementation);
            }
            let selected = super::standalone::dependency_closure(
                &runtime,
                implementation_roots.values().copied().collect(),
            )?;
            let rebase = super::standalone::append_runtime_closure(
                program,
                &runtime,
                &selected,
                RESIDENT_MODULE,
                RESIDENT_LINK_MODULE,
            )?;
            super::standalone::append_runtime_helper_requirements(program, &runtime, &selected)?;
            for (external_id, implementation) in implementation_roots {
                resolved.insert(
                    external_id,
                    ResolvedTarget::Routine(rebase[&implementation]),
                );
            }
        }
    }

    rewrite_external_references(program, &resolved, &external)?;
    program
        .routines
        .retain(|routine| routine.abi != MirRoutineAbi::ExternalInterface);
    Ok(())
}

fn sys_interface_signatures()
-> Result<BTreeMap<String, crate::semantic::ir::SemRoutineSignature>, Vec<MirDiagnostic>> {
    let program =
        crate::runtime_source::compile_embedded_module("sys.act").map_err(frontend_diagnostics)?;
    Ok(program
        .modules
        .iter()
        .flat_map(|module| &module.items)
        .filter_map(|item| match item {
            crate::semantic::ir::SemItem::Routine(routine) if routine.is_external => Some((
                binding_key(&routine.symbol.qualified_name),
                routine.signature.clone(),
            )),
            _ => None,
        })
        .collect())
}

fn validate_semantic_abi(
    interface_name: &str,
    interfaces: &BTreeMap<String, crate::semantic::ir::SemRoutineSignature>,
    runtime: &RuntimeImage,
    unit: &RuntimeUnit,
    expected: &str,
) -> Result<(), Vec<MirDiagnostic>> {
    let Some(interface) = interfaces.get(&binding_key(interface_name)) else {
        return Err(diagnostic(format!(
            "authoritative SYS interface has no external `{interface_name}`"
        )));
    };
    let implementation_unit = runtime.routine_units.get(&expected.to_ascii_uppercase());
    if implementation_unit != Some(unit) {
        return Err(diagnostic(format!(
            "embedded {} has no implementation routine `{expected}`",
            unit.name
        )));
    }
    let implementation = runtime
        .semir
        .modules
        .iter()
        .flat_map(|module| &module.items)
        .find_map(|item| match item {
            crate::semantic::ir::SemItem::Routine(routine)
                if routine
                    .symbol
                    .qualified_name
                    .rsplit('.')
                    .next()
                    .is_some_and(|name| name.eq_ignore_ascii_case(expected)) =>
            {
                Some(routine)
            }
            _ => None,
        });
    let Some(implementation) = implementation else {
        return Err(diagnostic(format!(
            "embedded {} has no implementation routine `{expected}`",
            unit.name
        )));
    };
    if interface != &implementation.signature {
        return Err(diagnostic(format!(
            "ABI mismatch between external `{interface_name}` and runtime implementation `{}.{expected}`",
            unit.name
        )));
    }
    Ok(())
}

fn find_runtime_routine(
    runtime: &MirProgram,
    link_module: &str,
    expected: &str,
) -> Result<RoutineId, Vec<MirDiagnostic>> {
    let matches = runtime
        .routines
        .iter()
        .filter(|routine| {
            super::standalone::runtime_routine_name(&routine.name, link_module)
                .eq_ignore_ascii_case(expected)
        })
        .map(|routine| routine.id)
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [id] => Ok(*id),
        [] => Err(diagnostic(format!(
            "embedded runtime has no implementation routine `{expected}`"
        ))),
        _ => Err(diagnostic(format!(
            "embedded runtime has multiple implementation routines named `{expected}`"
        ))),
    }
}

fn validate_abi(
    interface: &MirRoutine,
    implementation: &MirRoutine,
) -> Result<(), Vec<MirDiagnostic>> {
    let interface_params = interface
        .frame
        .params
        .iter()
        .map(|param| (param.storage, param.width))
        .collect::<Vec<_>>();
    let implementation_params = implementation
        .frame
        .params
        .iter()
        .map(|param| (param.storage, param.width))
        .collect::<Vec<_>>();
    if interface_params != implementation_params {
        return Err(diagnostic(format!(
            "ABI mismatch between external `{}` and runtime implementation `{}`",
            interface.name, implementation.name
        )));
    }
    Ok(())
}

fn referenced_external_routines(
    program: &MirProgram,
    external: &BTreeMap<RoutineId, String>,
) -> BTreeSet<RoutineId> {
    let mut referenced = BTreeSet::new();
    let mut record = |id| {
        if external.contains_key(&id) {
            referenced.insert(id);
        }
    };
    for static_data in &program.statics {
        record_image_references(&static_data.image, &mut record);
    }
    for global in &program.globals {
        if let Some(init) = &global.init {
            record_global_init_references(init, &mut record);
        }
    }
    for routine in &program.routines {
        for slot in routine.frame.params.iter().chain(&routine.frame.locals) {
            if let Some(init) = &slot.init {
                record_storage_init_references(init, &mut record);
            }
        }
        for block in &routine.blocks {
            for op in &block.ops {
                match op {
                    MirOp::Call {
                        target: MirCallTarget::Routine(id),
                        ..
                    } => record(*id),
                    MirOp::Move {
                        src: MirValue::RoutineAddr(id),
                        ..
                    } => record(*id),
                    _ => {}
                }
            }
        }
    }
    for machine in &program.machine_blocks {
        for item in &machine.items {
            if let MirMachineItem::Relocation {
                target: MirInlineAsmTarget::Routine(id),
                ..
            } = item
            {
                record(*id);
            }
        }
    }
    referenced
}

fn record_global_init_references(init: &MirGlobalInit, record: &mut impl FnMut(RoutineId)) {
    match init {
        MirGlobalInit::Bytes { image, .. } => record_image_references(image, record),
        MirGlobalInit::Descriptor { backing, .. } => {
            record_image_references(&backing.image, record)
        }
        MirGlobalInit::RoutineAddress { routine, .. } => record(*routine),
        MirGlobalInit::ZeroFill { .. } | MirGlobalInit::ProgramEndWord { .. } => {}
    }
}

fn record_storage_init_references(init: &MirStorageInit, record: &mut impl FnMut(RoutineId)) {
    match init {
        MirStorageInit::Bytes { image, .. } => record_image_references(image, record),
        MirStorageInit::Descriptor { backing, .. } => {
            record_image_references(&backing.image, record)
        }
        MirStorageInit::RoutineAddress { routine, .. } => record(*routine),
        MirStorageInit::ZeroFill { .. } => {}
    }
}

fn record_image_references(image: &MirDataImage, record: &mut impl FnMut(RoutineId)) {
    for relocation in &image.relocations {
        if let MirDataRelocationTarget::Routine(id) = relocation.target {
            record(id);
        }
    }
}

fn rewrite_external_references(
    program: &mut MirProgram,
    resolved: &BTreeMap<RoutineId, ResolvedTarget>,
    external_names: &BTreeMap<RoutineId, String>,
) -> Result<(), Vec<MirDiagnostic>> {
    for static_data in &mut program.statics {
        rewrite_image(&mut static_data.image, resolved)?;
    }
    for global in &mut program.globals {
        if let Some(init) = &mut global.init {
            rewrite_global_init(init, resolved)?;
        }
    }
    for routine in &mut program.routines {
        for slot in routine
            .frame
            .params
            .iter_mut()
            .chain(&mut routine.frame.locals)
        {
            if let Some(init) = &mut slot.init {
                rewrite_storage_init(init, resolved)?;
            }
        }
        for block in &mut routine.blocks {
            for op in &mut block.ops {
                match op {
                    MirOp::Call { target, .. } => {
                        let MirCallTarget::Routine(id) = target else {
                            continue;
                        };
                        if let Some(binding) = resolved.get(id) {
                            *target = match binding {
                                ResolvedTarget::Absolute(address) => MirCallTarget::Runtime {
                                    name: external_names
                                        .get(id)
                                        .map(|name| external_display_name(name))
                                        .unwrap_or_else(|| "SYS external".to_string()),
                                    address: Some(*address),
                                },
                                ResolvedTarget::Routine(id) => MirCallTarget::Routine(*id),
                            };
                        }
                    }
                    MirOp::Move { src, .. } => {
                        let MirValue::RoutineAddr(id) = src else {
                            continue;
                        };
                        if let Some(binding) = resolved.get(id) {
                            *src = match binding {
                                ResolvedTarget::Absolute(address) => MirValue::ConstU16(*address),
                                ResolvedTarget::Routine(id) => MirValue::RoutineAddr(*id),
                            };
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    for machine in &mut program.machine_blocks {
        for item in &mut machine.items {
            if let MirMachineItem::Relocation { target, .. } = item
                && let MirInlineAsmTarget::Routine(id) = target
                && let Some(binding) = resolved.get(id)
            {
                *target = match binding {
                    ResolvedTarget::Absolute(address) => MirInlineAsmTarget::Absolute(*address),
                    ResolvedTarget::Routine(id) => MirInlineAsmTarget::Routine(*id),
                };
            }
        }
    }
    Ok(())
}

fn external_display_name(name: &str) -> String {
    name.rsplit(['.', ':'])
        .find(|component| !component.is_empty())
        .unwrap_or(name)
        .to_string()
}

fn rewrite_global_init(
    init: &mut MirGlobalInit,
    resolved: &BTreeMap<RoutineId, ResolvedTarget>,
) -> Result<(), Vec<MirDiagnostic>> {
    match init {
        MirGlobalInit::Bytes { image, .. } => rewrite_image(image, resolved),
        MirGlobalInit::Descriptor { backing, .. } => rewrite_image(&mut backing.image, resolved),
        MirGlobalInit::RoutineAddress {
            routine,
            descriptor_size,
            size_word,
            mutable,
            section,
        } => match resolved.get(routine) {
            Some(ResolvedTarget::Routine(id)) => {
                *routine = *id;
                Ok(())
            }
            Some(ResolvedTarget::Absolute(address)) => {
                *init = MirGlobalInit::Bytes {
                    image: absolute_routine_descriptor(*address, *descriptor_size, *size_word),
                    zero_fill: 0,
                    mutable: *mutable,
                    section: section.clone(),
                    array: None,
                };
                Ok(())
            }
            None => Ok(()),
        },
        MirGlobalInit::ZeroFill { .. } | MirGlobalInit::ProgramEndWord { .. } => Ok(()),
    }
}

fn rewrite_storage_init(
    init: &mut MirStorageInit,
    resolved: &BTreeMap<RoutineId, ResolvedTarget>,
) -> Result<(), Vec<MirDiagnostic>> {
    match init {
        MirStorageInit::Bytes { image, .. } => rewrite_image(image, resolved),
        MirStorageInit::Descriptor { backing, .. } => rewrite_image(&mut backing.image, resolved),
        MirStorageInit::RoutineAddress {
            routine,
            descriptor_size,
            size_word,
            mutable,
            section,
        } => match resolved.get(routine) {
            Some(ResolvedTarget::Routine(id)) => {
                *routine = *id;
                Ok(())
            }
            Some(ResolvedTarget::Absolute(address)) => {
                *init = MirStorageInit::Bytes {
                    image: absolute_routine_descriptor(*address, *descriptor_size, *size_word),
                    zero_fill: 0,
                    mutable: *mutable,
                    section: section.clone(),
                };
                Ok(())
            }
            None => Ok(()),
        },
        MirStorageInit::ZeroFill { .. } => Ok(()),
    }
}

fn absolute_routine_descriptor(
    address: u16,
    descriptor_size: u16,
    size_word: Option<u16>,
) -> MirDataImage {
    let mut bytes = address.to_le_bytes().to_vec();
    if descriptor_size >= 4 {
        bytes.extend_from_slice(&size_word.unwrap_or(address).to_le_bytes());
    }
    bytes.resize(usize::from(descriptor_size), 0);
    MirDataImage {
        bytes,
        relocations: Vec::new(),
    }
}

fn rewrite_image(
    image: &mut MirDataImage,
    resolved: &BTreeMap<RoutineId, ResolvedTarget>,
) -> Result<(), Vec<MirDiagnostic>> {
    for relocation in &mut image.relocations {
        let MirDataRelocationTarget::Routine(id) = relocation.target else {
            continue;
        };
        if let Some(binding) = resolved.get(&id) {
            relocation.target = match binding {
                ResolvedTarget::Absolute(address) => MirDataRelocationTarget::Absolute(*address),
                ResolvedTarget::Routine(id) => MirDataRelocationTarget::Routine(*id),
            };
        }
    }
    Ok(())
}

fn missing_binding(name: &str, runtime: Runtime) -> Vec<MirDiagnostic> {
    diagnostic(format!(
        "E-BINDING-MISSING-FOR-RUNTIME: external `{name}` has no `{runtime}` binding"
    ))
}

fn frontend_diagnostics(diagnostics: Vec<crate::diagnostic::Diagnostic>) -> Vec<MirDiagnostic> {
    diagnostics
        .into_iter()
        .map(|diagnostic| MirDiagnostic {
            routine: None,
            block: None,
            message: format!("embedded runtime binding frontend: {}", diagnostic.message),
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

    #[test]
    fn embedded_sys_bindings_are_unique_and_runtime_specific() {
        let cart = parse_bindings(Runtime::ActionCart).expect("cart bindings");
        let standalone = parse_bindings(Runtime::Standalone).expect("standalone bindings");
        assert_eq!(cart.len(), 71);
        assert_eq!(standalone.len(), 71);
        assert_eq!(cart["SYS.ZERO"], BindingTarget::Absolute(0xA78A));
        assert_eq!(
            standalone["SYS.ZERO"],
            BindingTarget::RuntimeRoutine {
                unit: "SYSBLK".to_string(),
                routine: "Zero".to_string(),
            }
        );
        assert_eq!(cart["SYS.SCOMPARE"], BindingTarget::Absolute(0xA864));
        assert_eq!(
            standalone["SYS.SCOMPARE"],
            BindingTarget::RuntimeRoutine {
                unit: "SYSSTR".to_string(),
                routine: "SCompare".to_string(),
            }
        );
        assert_eq!(cart["SYS.PRINTF"], BindingTarget::Absolute(0xA3CC));
        assert_eq!(
            standalone["SYS.PRINTF"],
            BindingTarget::RuntimeRoutine {
                unit: "SYSIO".to_string(),
                routine: "PrintF".to_string(),
            }
        );
        assert_eq!(cart["SYS.PRINTH"], BindingTarget::Absolute(0xB8C2));
        assert_eq!(
            standalone["SYS.PRINTH"],
            BindingTarget::RuntimeRoutine {
                unit: "SYSIO".to_string(),
                routine: "PrintH".to_string(),
            }
        );
        assert_eq!(cart["SYS.ERROR"], BindingTarget::Absolute(0x04CB));
        assert_eq!(
            standalone["SYS.ERROR"],
            BindingTarget::RuntimeRoutine {
                unit: "SYSLIB".to_string(),
                routine: "Error".to_string(),
            }
        );
        assert_eq!(cart["SYS.RAND"], BindingTarget::Absolute(0xA6F1));
        assert_eq!(
            standalone["SYS.RAND"],
            BindingTarget::RuntimeRoutine {
                unit: "SYSMISC".to_string(),
                routine: "Rand".to_string(),
            }
        );
        assert_eq!(cart["SYS.GRAPHICS"], BindingTarget::Absolute(0xA654));
        assert_eq!(
            standalone["SYS.GRAPHICS"],
            BindingTarget::RuntimeRoutine {
                unit: "SYSGR".to_string(),
                routine: "Graphics".to_string(),
            }
        );
        assert_eq!(cart["SYS.OPEN"], BindingTarget::Absolute(0xA444));
        assert_eq!(
            standalone["SYS.OPEN"],
            BindingTarget::RuntimeRoutine {
                unit: "SYSIO".to_string(),
                routine: "Open".to_string(),
            }
        );
        assert_eq!(cart["SYS.INPUTD"], BindingTarget::Absolute(0xA4A7));
        assert_eq!(
            standalone["SYS.INPUTD"],
            BindingTarget::RuntimeRoutine {
                unit: "SYSIO".to_string(),
                routine: "InputD".to_string(),
            }
        );
        assert_eq!(cart["SYS.PRINTBDE"], BindingTarget::Absolute(0xA508));
        assert_eq!(
            standalone["SYS.PRINTBDE"],
            BindingTarget::RuntimeRoutine {
                unit: "SYSIO".to_string(),
                routine: "PrintBDE".to_string(),
            }
        );
    }

    #[test]
    fn embedded_binding_unit_compiles_without_unit_specific_resolver_code() {
        let unit = resolve_runtime_unit("SYSBLK").expect("SYSBLK unit");
        let runtime =
            super::super::standalone::compile_runtime_unit(&unit.file_name, &unit.module_name)
                .expect("compile SYSBLK");
        for expected in ["Zero", "SetBlock", "MoveBlock"] {
            find_runtime_routine(&runtime, &unit.link_module, expected)
                .unwrap_or_else(|diagnostics| panic!("{expected}: {diagnostics:?}"));
        }
        let zero =
            find_runtime_routine(&runtime, &unit.link_module, "Zero").expect("Zero implementation");
        let set_block = find_runtime_routine(&runtime, &unit.link_module, "SetBlock")
            .expect("SetBlock implementation");
        let closure =
            super::super::standalone::dependency_closure(&runtime, [zero].into_iter().collect())
                .expect("Zero dependency closure");
        assert_eq!(closure, [zero, set_block].into_iter().collect());
    }

    #[test]
    fn a_second_embedded_binding_unit_uses_the_same_resolver_path() {
        let unit = resolve_runtime_unit("SYSSTR").expect("SYSSTR unit");
        let runtime =
            super::super::standalone::compile_runtime_unit(&unit.file_name, &unit.module_name)
                .expect("compile SYSSTR");
        for expected in ["SCompare", "SCopy", "SCopyS", "SAssign"] {
            find_runtime_routine(&runtime, &unit.link_module, expected)
                .unwrap_or_else(|diagnostics| panic!("{expected}: {diagnostics:?}"));
        }
        let scompare = find_runtime_routine(&runtime, &unit.link_module, "SCompare")
            .expect("SCompare implementation");
        let scompare_closure = super::super::standalone::dependency_closure(
            &runtime,
            [scompare].into_iter().collect(),
        )
        .expect("SCompare dependency closure");
        assert_eq!(scompare_closure, [scompare].into_iter().collect());

        let scopy = find_runtime_routine(&runtime, &unit.link_module, "SCopy")
            .expect("SCopy implementation");
        let scopys = find_runtime_routine(&runtime, &unit.link_module, "SCopyS")
            .expect("SCopyS implementation");
        let scopys_closure =
            super::super::standalone::dependency_closure(&runtime, [scopys].into_iter().collect())
                .expect("SCopyS dependency closure");
        assert_eq!(scopys_closure, [scopy, scopys].into_iter().collect());
    }
}
