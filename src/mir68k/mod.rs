//! Motorola 68000 backend boundary.
//!
//! This is intentionally separate from MIR6502 and MIR65816. Slice 10 fills
//! the entry point with a small portable-lowering canary.

use crate::backend::{BackendLoweringError, NirBackend, VerifiedNir};
use crate::nir::{NirDiagnostic, NirProgram};
use crate::target::TargetId;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mir68kProgram;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mir68kDiagnostic {
    pub routine: Option<String>,
    pub block: Option<String>,
    pub message: String,
}

#[derive(Debug, Default, Clone, Copy)]
pub struct Mir68kBackend;

impl NirBackend for Mir68kBackend {
    type Output = Mir68kProgram;
    type Diagnostic = Mir68kDiagnostic;

    fn supports_target(&self, target: TargetId) -> bool {
        target == TargetId::Motorola68000
    }

    fn lower(
        &self,
        input: VerifiedNir<'_>,
    ) -> Result<Self::Output, Vec<Self::Diagnostic>> {
        let target = input.target();
        if !self.supports_target(target) {
            return Err(vec![Mir68kDiagnostic {
                routine: None,
                block: None,
                message: format!("MIR68K cannot lower target `{target}`"),
            }]);
        }
        Err(vec![Mir68kDiagnostic {
            routine: None,
            block: None,
            message: "MIR68K lowering canary is not implemented yet".to_string(),
        }])
    }
}

pub fn lower_program(
    program: &NirProgram,
) -> Result<Mir68kProgram, BackendLoweringError<Mir68kDiagnostic>> {
    crate::backend::lower_program(&Mir68kBackend, program)
}

pub fn lower_verified(
    input: VerifiedNir<'_>,
) -> Result<Mir68kProgram, BackendLoweringError<Mir68kDiagnostic>> {
    crate::backend::lower_verified(&Mir68kBackend, input)
}

pub fn verify_nir(program: &NirProgram) -> Result<VerifiedNir<'_>, Vec<NirDiagnostic>> {
    crate::backend::verify_program(program)
}
