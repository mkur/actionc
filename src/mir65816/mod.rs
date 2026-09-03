//! WDC 65816 backend boundary.
//!
//! Slice 9 reserves an independent entry point. The portable lowering canary
//! is implemented in the following slice; this module deliberately does not
//! reuse MIR6502.

use crate::backend::{BackendLoweringError, NirBackend, VerifiedNir};
use crate::nir::{NirDiagnostic, NirProgram};
use crate::target::TargetId;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mir65816Program;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mir65816Diagnostic {
    pub routine: Option<String>,
    pub block: Option<String>,
    pub message: String,
}

#[derive(Debug, Default, Clone, Copy)]
pub struct Mir65816Backend;

impl NirBackend for Mir65816Backend {
    type Output = Mir65816Program;
    type Diagnostic = Mir65816Diagnostic;

    fn supports_target(&self, target: TargetId) -> bool {
        matches!(target, TargetId::Wdc65816Native | TargetId::Wdc65816Small)
    }

    fn lower(
        &self,
        input: VerifiedNir<'_>,
    ) -> Result<Self::Output, Vec<Self::Diagnostic>> {
        let target = input.target();
        if !self.supports_target(target) {
            return Err(vec![Mir65816Diagnostic {
                routine: None,
                block: None,
                message: format!("MIR65816 cannot lower target `{target}`"),
            }]);
        }
        Err(vec![Mir65816Diagnostic {
            routine: None,
            block: None,
            message: "MIR65816 lowering canary is not implemented yet".to_string(),
        }])
    }
}

pub fn lower_program(
    program: &NirProgram,
) -> Result<Mir65816Program, BackendLoweringError<Mir65816Diagnostic>> {
    crate::backend::lower_program(&Mir65816Backend, program)
}

pub fn lower_verified(
    input: VerifiedNir<'_>,
) -> Result<Mir65816Program, BackendLoweringError<Mir65816Diagnostic>> {
    crate::backend::lower_verified(&Mir65816Backend, input)
}

pub fn verify_nir(program: &NirProgram) -> Result<VerifiedNir<'_>, Vec<NirDiagnostic>> {
    crate::backend::verify_program(program)
}
