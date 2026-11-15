//! The selected core input travels with its materialized result. Speculative
//! branches own their artifacts; diagnostics never select a checkpoint.
use super::{Mir6502Config, ir::*};
use std::{
    ops::{Deref, DerefMut},
    sync::Arc,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct CoreInput {
    pub program: MirProgram,
    pub config: Mir6502Config,
    pub helpers: Vec<MirRuntimeHelper>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Checkpoint {
    pub input: CoreInput,
    /// Exact output after helper resolution. Untracked outer mutations make
    /// this checkpoint unusable instead of silently resuming stale input.
    output: MirProgram,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct PreparedProgram {
    pub program: MirProgram,
    checkpoint: Option<Arc<Checkpoint>>,
}

impl PreparedProgram {
    pub fn new(program: MirProgram, input: Option<CoreInput>) -> Self {
        let checkpoint = input.map(|input| {
            Arc::new(Checkpoint {
                input,
                output: program.clone(),
            })
        });
        Self {
            program,
            checkpoint,
        }
    }
    pub fn checkpoint(&self) -> Option<Arc<Checkpoint>> {
        self.checkpoint
            .as_ref()
            .filter(|c| c.output == self.program)
            .cloned()
    }
    /// An accepted pre-home rewrite with an untracked placement/callee proof
    /// cannot approve a later parameter placement change. Keep its executable.
    pub fn guard_checkpoint(mut self, untracked_proof: bool) -> Self {
        if untracked_proof {
            self.checkpoint = None;
        }
        self
    }
}
impl Deref for PreparedProgram {
    type Target = MirProgram;
    fn deref(&self) -> &MirProgram {
        &self.program
    }
}
impl DerefMut for PreparedProgram {
    fn deref_mut(&mut self) -> &mut MirProgram {
        &mut self.program
    }
}
impl std::borrow::Borrow<MirProgram> for PreparedProgram {
    fn borrow(&self) -> &MirProgram {
        &self.program
    }
}

pub(super) struct SelectedCore {
    pub checkpoint: Arc<Checkpoint>,
    pub staging_routines: std::collections::BTreeSet<RoutineId>,
}

pub(super) struct Resume {
    pub core: SelectedCore,
    pub range_source: Option<MirProgram>,
}

/// The accepted input and executable advance together. A later placement trial
/// must start here, including prior reservations and removed absent-home stores.
pub(super) struct SelectedPlacement {
    pub input: Arc<MirProgram>,
    pub executable: Executable,
}

#[derive(Clone)]
pub(super) struct Executable {
    pub program: MirProgram,
    pub output: crate::codegen::CodegenOutput,
    /// Routine inventory and diagnostic costs, not a per-routine growth veto.
    pub costs: std::collections::BTreeMap<RoutineId, usize>,
    /// Diagnostic identity only; never consulted for acceptance or rollback.
    pub trace_snapshot: Option<u64>,
}

impl Executable {
    pub fn improves(&self, baseline: &Self) -> bool {
        crate::codegen::format_load_file(&self.output).len()
            < crate::codegen::format_load_file(&baseline.output).len()
            && self.costs.keys().eq(baseline.costs.keys())
    }
}

pub(super) fn resume_output(
    nir: &crate::nir::NirProgram,
    resume: &Resume,
    proposal: MirProgram,
    config: &Mir6502Config,
    origin: u16,
    runtime: crate::runtime::Runtime,
    trace: &super::trace::Trace,
) -> Result<Executable, Vec<super::MirDiagnostic>> {
    trace.count("parameter_home_trials", 1);
    let original = &resume.core.checkpoint.input;
    let input = CoreInput {
        program: proposal,
        config: original.config.clone(),
        helpers: original.helpers.clone(),
    };
    let mut program = super::materialize::resume_core(input, origin, trace)?;
    super::verify_program(&program, MirPhase::PostMaterialization)?;
    super::runtime::resolve_helpers(&mut program, runtime)?;
    let mut preservation = super::late_cleanup::preservation::Preservation::new(
        config.enable_peepholes && config.enable_late_private_store_cleanup,
    );
    let program = super::finish_materialization(
        program,
        config,
        origin,
        runtime,
        trace,
        &mut preservation,
        &resume.core.staging_routines,
    )?;
    let program = super::finish_emission_preparation(
        nir,
        program,
        &resume.range_source,
        config,
        origin,
        runtime,
        trace,
        &mut preservation,
    )?;
    super::finalize_output(nir, program, origin, runtime, trace)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn empty() -> MirProgram {
        MirProgram {
            statics: vec![],
            globals: vec![],
            routines: vec![],
            machine_blocks: vec![],
            runtime_helpers: vec![],
        }
    }
    #[test]
    fn selected_result_owns_its_checkpoint_and_mutation_invalidates_it() {
        let p = empty();
        let input = CoreInput {
            program: p.clone(),
            config: Mir6502Config::default(),
            helpers: vec![],
        };
        let selected = PreparedProgram::new(p.clone(), Some(input));
        let mut rejected = selected.clone();
        rejected.globals.push(MirGlobal {
            id: crate::nir::SymbolId(0),
            name: "trial".into(),
            kind: "byte".into(),
            width: Some(MirWidth::Byte),
            storage_size: 1,
            backing: MirGlobalBacking::Absolute(0x600),
            init: None,
        });
        assert!(rejected.checkpoint().is_none());
        assert!(selected.checkpoint().is_some());
        assert!(
            selected
                .clone()
                .guard_checkpoint(true)
                .checkpoint()
                .is_none()
        );
        assert!(
            selected
                .clone()
                .guard_checkpoint(false)
                .checkpoint()
                .is_some()
        );
        assert_eq!(selected.checkpoint().unwrap().input.program, p);
    }

    fn nir(text: &str) -> crate::nir::NirProgram {
        let ast = crate::parser::parse(&crate::lexer::tokenize(text).unwrap()).unwrap();
        let model =
            crate::semantic::analyze_with_options(&ast, crate::semantic::SemanticOptions::modern())
                .unwrap();
        crate::nir::optimize_program(&crate::nir::lower_program(
            &crate::semantic::ir::lower_program(&ast, &model),
        ))
        .unwrap()
    }

    #[test]
    fn whole_image_cost_allows_local_growth_but_requires_saving_and_compatible_routines() {
        let n = nir("BYTE FUNC Value() RETURN(7)");
        let config = Mir6502Config::default();
        let trace = super::super::trace::Trace::disabled();
        let runtime = crate::runtime::Runtime::ActionCart;
        let (program, _) =
            super::super::prepare_program_for_emission(&n, 0x3000, &config, runtime, &trace)
                .unwrap();
        let baseline =
            super::super::finalize_output(&n, program, 0x3000, runtime, &trace).unwrap();
        assert!(!baseline.costs.is_empty());

        // Isolate the cost boundary; transaction tests cover validity and replay.
        let mut candidate = baseline.clone();
        candidate.output.bytes.pop().unwrap();
        *candidate.costs.values_mut().next().unwrap() += 1;
        assert!(candidate.improves(&baseline));

        candidate.costs.clear();
        assert!(!candidate.improves(&baseline), "routine inventory must match");

        candidate = baseline.clone();
        *candidate.costs.values_mut().next().unwrap() -= 1;
        assert!(!candidate.improves(&baseline), "equal image despite local saving");
        candidate.output.bytes.push(0);
        assert!(!candidate.improves(&baseline), "larger image despite local saving");
    }

    #[test]
    fn checkpoint_resume_reproduces_final_output_without_an_upstream_search() {
        let n = nir("CARD FUNC Echo(CARD x) x==+1 RETURN(x)");
        for runtime in [
            crate::runtime::Runtime::ActionCart,
            crate::runtime::Runtime::Standalone,
        ] {
            let config = Mir6502Config {
                enable_parameter_home_selection: true,
                ..Mir6502Config::optimized()
            };
            let session = super::super::trace::Mir6502Trace::new(Default::default());
            let trace = session.context();
            let (program, resume) =
                super::super::prepare_program_for_emission(&n, 0x3000, &config, runtime, &trace)
                    .unwrap();
            let baseline =
                super::super::finalize_output(&n, program, 0x3000, runtime, &trace).unwrap();
            let resume = resume.expect("unmodified selected checkpoint");
            let proposal = resume.core.checkpoint.input.program.clone();
            let before = session.manifest()["work"]["materializations"]
                .as_u64()
                .unwrap();
            let event_start = session.events().len();
            let result =
                resume_output(&n, &resume, proposal, &config, 0x3000, runtime, &trace).unwrap();
            assert_eq!(baseline.output, result.output);
            assert_eq!(baseline.costs, result.costs);
            assert!(!result.improves(&baseline));
            assert_eq!(
                session.manifest()["work"]["materializations"]
                    .as_u64()
                    .unwrap(),
                before + 1
            );
            assert!(
                !session.events()[event_start..]
                    .iter()
                    .any(|e| e.kind == "attempt-start"
                        && matches!(
                            e.data["stage"].as_str(),
                            Some(
                                "inlining"
                                    | "copy_values"
                                    | "scalar_captures"
                                    | "address_values"
                                    | "captured_comparisons"
                            )
                        ))
            );
        }
    }

    #[test]
    fn no_candidate_keeps_output_and_materialization_count_identical() {
        let n = nir("BYTE FUNC Echo(BYTE x) RETURN(x)");
        let mut off = super::super::trace::Mir6502Trace::new(Default::default());
        let config = Mir6502Config {
            enable_parameter_home_selection: false,
            ..Mir6502Config::optimized()
        };
        let expected = super::super::generate_output_with_trace(
            &n,
            0x3000,
            &config,
            crate::runtime::Runtime::ActionCart,
            &mut off,
        )
        .unwrap();
        let mut on = super::super::trace::Mir6502Trace::new(Default::default());
        let config = Mir6502Config {
            enable_parameter_home_selection: true,
            ..config
        };
        let actual = super::super::generate_output_with_trace(
            &n,
            0x3000,
            &config,
            crate::runtime::Runtime::ActionCart,
            &mut on,
        )
        .unwrap();
        assert_eq!(actual, expected);
        assert_eq!(
            off.manifest()["work"]["materializations"],
            on.manifest()["work"]["materializations"]
        );
        assert!(
            !on.events()
                .iter()
                .any(|e| e.kind == "parameter-home-result")
        );
    }
}
