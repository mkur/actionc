//! The verified-NIR handoff shared by independent target backends.
//!
//! Constructing [`VerifiedNir`] runs the complete NIR verifier. Backends receive
//! this token instead of Semantic IR or an unchecked [`NirProgram`], so target
//! lowering cannot silently recover missing language facts from an earlier IR.

use crate::nir::{NirDiagnostic, NirProgram, NirRuntimeBinding};
use crate::target::{TargetId, TargetLayout};

#[derive(Debug, Clone, Copy)]
pub struct VerifiedNir<'a> {
    program: &'a NirProgram,
}

impl<'a> VerifiedNir<'a> {
    pub fn program(self) -> &'a NirProgram {
        self.program
    }

    pub fn target(self) -> TargetId {
        self.program.target_layout.target
    }

    pub fn target_layout(self) -> &'a TargetLayout {
        &self.program.target_layout
    }

    pub fn runtime_bindings(self) -> &'a [NirRuntimeBinding] {
        &self.program.runtime_bindings
    }
}

pub fn verify_program(program: &NirProgram) -> Result<VerifiedNir<'_>, Vec<NirDiagnostic>> {
    crate::nir::verify_program(program)?;
    Ok(VerifiedNir { program })
}

/// A target-owned lowering entry point. Its only compiler IR input is verified
/// NIR; object formats, linker policy, physical ABIs, and instruction choices
/// remain in the implementing backend.
pub trait NirBackend {
    type Output;
    type Diagnostic;

    fn supports_target(&self, target: TargetId) -> bool;

    fn lower(&self, input: VerifiedNir<'_>) -> Result<Self::Output, Vec<Self::Diagnostic>>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BackendLoweringError<D> {
    InvalidNir(Vec<NirDiagnostic>),
    UnsupportedTarget(TargetId),
    Backend(Vec<D>),
}

pub fn lower_program<B: NirBackend>(
    backend: &B,
    program: &NirProgram,
) -> Result<B::Output, BackendLoweringError<B::Diagnostic>> {
    let input = verify_program(program).map_err(BackendLoweringError::InvalidNir)?;
    lower_verified(backend, input)
}

pub fn lower_verified<B: NirBackend>(
    backend: &B,
    input: VerifiedNir<'_>,
) -> Result<B::Output, BackendLoweringError<B::Diagnostic>> {
    if !backend.supports_target(input.target()) {
        return Err(BackendLoweringError::UnsupportedTarget(input.target()));
    }
    backend.lower(input).map_err(BackendLoweringError::Backend)
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use super::*;

    fn empty_program(target: TargetId) -> NirProgram {
        NirProgram {
            target_layout: TargetLayout::for_target(target),
            runtime_bindings: Vec::new(),
            globals: Vec::new(),
            statics: Vec::new(),
            routines: Vec::new(),
        }
    }

    struct ProbeBackend<'a> {
        called: &'a Cell<bool>,
    }

    impl NirBackend for ProbeBackend<'_> {
        type Output = (TargetId, u8, usize);
        type Diagnostic = ();

        fn supports_target(&self, _target: TargetId) -> bool {
            true
        }

        fn lower(&self, input: VerifiedNir<'_>) -> Result<Self::Output, Vec<Self::Diagnostic>> {
            self.called.set(true);
            Ok((
                input.target(),
                input.target_layout().address_bits,
                input.runtime_bindings().len(),
            ))
        }
    }

    #[test]
    fn backend_receives_the_verified_layout_and_runtime_view() {
        let called = Cell::new(false);
        let program = empty_program(TargetId::Wdc65816Native);
        let facts = lower_program(&ProbeBackend { called: &called }, &program)
            .expect("valid NIR reaches backend");
        assert!(called.get());
        assert_eq!(facts, (TargetId::Wdc65816Native, 24, 0));
    }

    #[test]
    fn invalid_nir_never_reaches_a_backend() {
        let called = Cell::new(false);
        let mut program = empty_program(TargetId::Atari6502);
        program.target_layout.address_bits = 0;
        let error = lower_program(&ProbeBackend { called: &called }, &program)
            .expect_err("invalid NIR must stop at the boundary");
        assert!(matches!(error, BackendLoweringError::InvalidNir(_)));
        assert!(!called.get());
    }
}
