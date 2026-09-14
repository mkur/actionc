//! Runtime selection resolves interface IDs before target materialization.
use crate::{
    mir68k::{self, amiga::ConsoleService},
    nir::{RoutineId, runtime_symbol_id},
};
use std::collections::BTreeMap;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum NativeRuntime {
    #[default]
    Bare,
    AmigaDos,
}

/// Only bindings that passed interface signature validation can be constructed.
#[derive(Debug, Clone, Default)]
pub struct RuntimeBindings {
    console: BTreeMap<RoutineId, ConsoleService>,
}
impl RuntimeBindings {
    pub fn console_bindings(&self) -> &BTreeMap<RoutineId, ConsoleService> {
        &self.console
    }
    pub fn console(&self, routine: RoutineId) -> Option<ConsoleService> {
        self.console.get(&routine).copied()
    }
}

pub fn bind(
    program: &mir68k::Mir68kProgram,
    runtime: NativeRuntime,
) -> Result<RuntimeBindings, String> {
    mir68k::verify::verify_contract(program).map_err(|e| format!("invalid MIR68K: {e:?}"))?;
    let mut result = RuntimeBindings::default();
    for routine in &program.routines {
        let Some(symbol) = routine.entry.external_symbol else {
            continue;
        };
        if runtime == NativeRuntime::Bare {
            return Err(format!(
                "{}: bare runtime has no external service adapter",
                routine.name
            ));
        }
        let service = SERVICES
            .iter()
            .find(|(name, _)| runtime_symbol_id(name) == symbol)
            .map(|(_, service)| *service)
            .ok_or_else(|| {
                format!(
                    "{}: Amiga runtime does not support this external service",
                    routine.name
                )
            })?;
        mir68k::amiga::validate_console_signature(service, &routine.signature)
            .map_err(|e| format!("{}: {e}", routine.name))?;
        if !matches!(
            routine.entry.placement,
            crate::nir::NirRoutinePlacement::Relocatable
        ) {
            return Err(format!(
                "{}: Amiga runtime service must be relocatable",
                routine.name
            ));
        }
        result.console.insert(routine.id, service);
    }
    Ok(result)
}

const SERVICES: &[(&str, ConsoleService)] = &[
    ("SYS.Put", ConsoleService::Put),
    ("SYS.PutE", ConsoleService::PutE),
    ("SYS.Print", ConsoleService::Print),
    ("SYS.PrintE", ConsoleService::PrintE),
    ("SYS.PrintB", ConsoleService::PrintB),
    ("SYS.PrintBE", ConsoleService::PrintBE),
    ("SYS.PrintC", ConsoleService::PrintC),
    ("SYS.PrintCE", ConsoleService::PrintCE),
    ("SYS.PrintI", ConsoleService::PrintI),
    ("SYS.PrintIE", ConsoleService::PrintIE),
];
