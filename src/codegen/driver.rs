use super::*;

pub fn generate(program: &Program) -> Result<CodegenOutput, Vec<Diagnostic>> {
    generate_with_origin(program, CODE_ORIGIN)
}

pub fn generate_with_origin(
    program: &Program,
    origin: u16,
) -> Result<CodegenOutput, Vec<Diagnostic>> {
    generate_with_options(program, origin, false, CodegenProfile::Compat)
}

pub fn generate_compatible_with_origin(
    program: &Program,
    origin: u16,
) -> Result<CodegenOutput, Vec<Diagnostic>> {
    generate_profile_with_origin(program, origin, CodegenProfile::Compat)
}

pub fn generate_profile_with_origin(
    program: &Program,
    origin: u16,
    profile: CodegenProfile,
) -> Result<CodegenOutput, Vec<Diagnostic>> {
    let origin = if origin == CODE_ORIGIN {
        program_code_origin(program).unwrap_or(origin)
    } else {
        origin
    };
    generate_with_options(program, origin, true, profile)
}

pub fn generate_semir_profile_with_origin(
    program: &crate::semantic::ir::SemProgram,
    origin: u16,
    profile: CodegenProfile,
) -> Result<CodegenOutput, Vec<Diagnostic>> {
    let program = super::semir::semir_to_ast(program)?;
    generate_profile_with_origin(&program, origin, profile)
}

pub fn generate_semir_native_profile_with_origin(
    program: &crate::semantic::ir::SemProgram,
    origin: u16,
    profile: CodegenProfile,
) -> Result<CodegenOutput, Vec<Diagnostic>> {
    super::semir_native::generate_native_profile_with_origin(program, origin, profile)
}

fn generate_with_options(
    program: &Program,
    origin: u16,
    segment_storage: bool,
    profile: CodegenProfile,
) -> Result<CodegenOutput, Vec<Diagnostic>> {
    let runtime_target = if segment_storage {
        RuntimeTarget::Cartridge
    } else {
        RuntimeTarget::StandaloneSlots
    };
    let storage_base = if segment_storage { origin } else { DATA_BASE };
    let record_layouts = collect_record_layouts(program);
    let routines = collect_routine_info(program, &record_layouts);
    let routine_assignment_targets = collect_routine_assignment_targets(program, &routines);
    let numeric_defines = collect_numeric_defines(program);
    match profile {
        CodegenProfile::Compat => validate_compatible_source_surface(program, &routines)?,
        CodegenProfile::Modern => validate_modern_source_surface(program, &routines)?,
    }
    let layout = if segment_storage {
        StorageLayout::empty(storage_base)
    } else {
        StorageLayout::from_program(
            program,
            storage_base,
            segment_storage,
            &record_layouts,
            &numeric_defines,
        )
    };
    let callable_pointers = collect_global_callable_pointers(program);
    let machine_defines = collect_machine_defines(program);
    let mut generator = Generator {
        emitter: Emitter::with_origin(origin),
        layout,
        record_layouts,
        routines,
        callable_pointers,
        numeric_defines,
        machine_defines,
        runtime_helpers: RuntimeHelperTargets::default_for_target(runtime_target),
        routine_assignment_targets,
        local_symbols: HashMap::new(),
        local_callable_pointers: HashMap::new(),
        storage_symbols: Vec::new(),
        source_ranges: Vec::new(),
        current_return_slot: None,
        diagnostics: Vec::new(),
        label_counter: 0,
        exit_labels: Vec::new(),
        profile,
        segment_storage,
        processor: ProcessorState::default(),
        straight_line_store_y: None,
        y_constant_store_lookahead: None,
        label_store_y_hints: HashMap::new(),
        label_byte_values: HashMap::new(),
        last_label_position: None,
        compatible_cursor: segment_storage.then_some(origin),
        skipped_ranges: Vec::new(),
        last_routine_label: None,
        last_routine_ended_with_rts: false,
        routine_addresses: Vec::new(),
        routine_ranges: Vec::new(),
        routine_signatures: Vec::new(),
        current_routine_effects: None,
        current_routine_has_effect_contract: false,
        current_inferred_routine_facts: None,
        current_modern_routine_layout: ModernRoutineLayout::default(),
        preserve_modern_routine_layout: false,
        machine_blocks: Vec::new(),
        optimizations: Vec::new(),
        proofs: Vec::new(),
        proof_attempts: Vec::new(),
        branch_inversion_candidates: Vec::new(),
        deferred_output_cursor: origin,
        suppress_implicit_rts_once: false,
        inline_byte_constant_shift: false,
    };
    generator.generate_program(program);
    generator.finish()
}

fn program_code_origin(program: &Program) -> Option<u16> {
    let mut appmhi_low = None;
    let mut appmhi_high = None;
    let mut codebase_low = None;
    let mut codebase_high = None;

    for module in &program.modules {
        for item in &module.items {
            let Item::Set(set) = item else {
                continue;
            };
            let Some(address) = constant_u16(&set.address) else {
                continue;
            };
            let Some(value) = constant_u16(&set.value) else {
                continue;
            };
            match address {
                0x000E => {
                    appmhi_low = Some(value & 0x00FF);
                    appmhi_high = Some(value >> 8);
                }
                0x000F => appmhi_high = Some(value & 0x00FF),
                0x0491 => {
                    codebase_low = Some(value & 0x00FF);
                    codebase_high = Some(value >> 8);
                }
                0x0492 => codebase_high = Some(value & 0x00FF),
                _ => {}
            }
        }
    }

    let appmhi = appmhi_low
        .zip(appmhi_high)
        .map(|(low, high)| low | (high << 8));
    let codebase = codebase_low
        .zip(codebase_high)
        .map(|(low, high)| low | (high << 8));

    match (appmhi, codebase) {
        (Some(appmhi), Some(codebase)) if appmhi == codebase => Some(appmhi),
        (Some(appmhi), None) => Some(appmhi),
        (None, Some(codebase)) => Some(codebase),
        _ => None,
    }
}
