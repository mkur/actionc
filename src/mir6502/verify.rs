use std::collections::BTreeSet;

use super::abi::{action_arg_home, action_arg_width_bytes};
use super::analysis::effects::{MirHomeByte, classify_op};
use super::diagnostics::MirDiagnostic;
use super::ir::{
    MirAddr, MirAddressConsumer, MirBinaryOp, MirBlockId, MirCondDest, MirDef, MirEdge, MirFrame,
    MirGlobal, MirGlobalInit, MirMachineBlockId, MirMem, MirOp, MirPhase, MirPointerPair,
    MirProgram, MirReg, MirRoutine, MirRuntimeHelperTarget, MirStorageBase, MirStorageInit,
    MirTerminator, MirValue, RoutineId,
};
use crate::nir::SymbolId;

pub(super) fn verify_program(
    program: &MirProgram,
    phase: MirPhase,
) -> Result<(), Vec<MirDiagnostic>> {
    let mut verifier = MirVerifier {
        diagnostics: Vec::new(),
        phase,
    };
    verifier.verify_program(program);
    if verifier.diagnostics.is_empty() {
        Ok(())
    } else {
        Err(verifier.diagnostics)
    }
}

struct MirVerifier {
    diagnostics: Vec<MirDiagnostic>,
    phase: MirPhase,
}

impl MirVerifier {
    fn physical_homes_required(&self) -> bool {
        matches!(self.phase, MirPhase::PostHome | MirPhase::PreEmission)
    }

    fn abstract_conditions_forbidden(&self) -> bool {
        matches!(self.phase, MirPhase::PreEmission)
    }

    fn physical_home_phase_name(&self) -> &'static str {
        match self.phase {
            MirPhase::PostHome => "post-home",
            MirPhase::PreEmission => "pre-emission",
            MirPhase::PreMaterialization | MirPhase::PostMaterialization => "materialized",
        }
    }

    fn verify_program(&mut self, program: &MirProgram) {
        let static_ids = program
            .statics
            .iter()
            .map(|static_data| static_data.id)
            .collect::<BTreeSet<_>>();
        let global_ids = program
            .globals
            .iter()
            .map(|global| global.id)
            .collect::<BTreeSet<_>>();
        for global in &program.globals {
            if global.name.is_empty() {
                self.diagnostics.push(MirDiagnostic::routine(
                    "globals",
                    "global name must not be empty",
                ));
            }
            if let Some(init) = &global.init {
                self.verify_global_init(global, init);
            }
        }
        for static_data in &program.statics {
            if static_data.name.is_empty() {
                self.diagnostics.push(MirDiagnostic::routine(
                    "statics",
                    "static name must not be empty",
                ));
            }
            if static_data.alignment == 0 {
                self.diagnostics.push(MirDiagnostic::routine(
                    "statics",
                    format!("static `s{}` has zero alignment", static_data.id.0),
                ));
            }
        }
        let all_routine_ids = program
            .routines
            .iter()
            .map(|routine| routine.id)
            .collect::<BTreeSet<_>>();
        let mut machine_ids = BTreeSet::new();
        for machine in &program.machine_blocks {
            if !machine_ids.insert(machine.id) {
                self.diagnostics.push(MirDiagnostic::routine(
                    "machine_blocks",
                    format!("duplicate machine block id `m{}`", machine.id.0),
                ));
            }
            if machine.items.is_empty() {
                self.diagnostics.push(MirDiagnostic::routine(
                    "machine_blocks",
                    format!("machine block `m{}` has no payload items", machine.id.0),
                ));
            }
        }
        let mut routine_ids = BTreeSet::new();
        if matches!(self.phase, MirPhase::PreEmission) {
            for helper in &program.runtime_helpers {
                if matches!(helper.target, MirRuntimeHelperTarget::Deferred) {
                    self.diagnostics.push(MirDiagnostic::routine(
                        "runtime_helpers",
                        "pre-emission MIR cannot contain deferred runtime helper targets",
                    ));
                }
            }
        }
        for routine in &program.routines {
            if !routine_ids.insert(routine.id) {
                self.diagnostics.push(MirDiagnostic::routine(
                    &routine.name,
                    format!("duplicate routine id `r{}`", routine.id.0),
                ));
            }
            self.verify_routine(
                routine,
                &static_ids,
                &global_ids,
                &all_routine_ids,
                &machine_ids,
            );
        }
    }

    fn verify_global_init(&mut self, global: &MirGlobal, init: &MirGlobalInit) {
        match init {
            MirGlobalInit::Bytes {
                bytes,
                zero_fill,
                section,
                ..
            } => {
                if section.is_empty() {
                    self.diagnostics.push(MirDiagnostic::routine(
                        "globals",
                        format!("global `g{}` init section must not be empty", global.id.0),
                    ));
                }
                if (bytes.len() as u16).saturating_add(*zero_fill) < global.storage_size {
                    self.diagnostics.push(MirDiagnostic::routine(
                        "globals",
                        format!(
                            "global `g{}` init payload is smaller than storage size",
                            global.id.0
                        ),
                    ));
                }
            }
            MirGlobalInit::Descriptor {
                backing,
                descriptor_size,
                section,
                ..
            } => {
                if !matches!(*descriptor_size, 2 | 4) {
                    self.diagnostics.push(MirDiagnostic::routine(
                        "globals",
                        format!(
                            "global `g{}` descriptor init has unsupported size {}",
                            global.id.0, descriptor_size
                        ),
                    ));
                }
                if backing.owner != global.id {
                    self.diagnostics.push(MirDiagnostic::routine(
                        "globals",
                        format!(
                            "global `g{}` descriptor backing owner does not match global id",
                            global.id.0
                        ),
                    ));
                }
                if backing.bytes.is_empty() && backing.zero_fill == 0 {
                    self.diagnostics.push(MirDiagnostic::routine(
                        "globals",
                        format!("global `g{}` descriptor backing is empty", global.id.0),
                    ));
                }
                if backing.section.is_empty() || section.is_empty() {
                    self.diagnostics.push(MirDiagnostic::routine(
                        "globals",
                        format!(
                            "global `g{}` descriptor sections must not be empty",
                            global.id.0
                        ),
                    ));
                }
            }
            MirGlobalInit::ZeroFill { bytes, section, .. } => {
                if *bytes < global.storage_size {
                    self.diagnostics.push(MirDiagnostic::routine(
                        "globals",
                        format!(
                            "global `g{}` zero-fill is smaller than storage size",
                            global.id.0
                        ),
                    ));
                }
                if section.is_empty() {
                    self.diagnostics.push(MirDiagnostic::routine(
                        "globals",
                        format!(
                            "global `g{}` zero-fill section must not be empty",
                            global.id.0
                        ),
                    ));
                }
            }
            MirGlobalInit::ProgramEndWord { section, .. } => {
                if global.storage_size < 2 {
                    self.diagnostics.push(MirDiagnostic::routine(
                        "globals",
                        format!(
                            "global `g{}` program-end word init needs at least 2 bytes of storage",
                            global.id.0
                        ),
                    ));
                }
                if section.is_empty() {
                    self.diagnostics.push(MirDiagnostic::routine(
                        "globals",
                        format!(
                            "global `g{}` program-end word section must not be empty",
                            global.id.0
                        ),
                    ));
                }
            }
            MirGlobalInit::RoutineAddress {
                descriptor_size,
                section,
                ..
            } => {
                if !matches!(*descriptor_size, 2 | 4) {
                    self.diagnostics.push(MirDiagnostic::routine(
                        "globals",
                        format!(
                            "global `g{}` routine-address init has unsupported size {}",
                            global.id.0, descriptor_size
                        ),
                    ));
                }
                if section.is_empty() {
                    self.diagnostics.push(MirDiagnostic::routine(
                        "globals",
                        format!(
                            "global `g{}` routine-address section must not be empty",
                            global.id.0
                        ),
                    ));
                }
            }
        }
    }

    fn verify_routine(
        &mut self,
        routine: &MirRoutine,
        static_ids: &BTreeSet<SymbolId>,
        global_ids: &BTreeSet<SymbolId>,
        routine_ids: &BTreeSet<RoutineId>,
        machine_ids: &BTreeSet<MirMachineBlockId>,
    ) {
        self.verify_frame_inits(routine);
        let mut block_ids = BTreeSet::new();
        for block in &routine.blocks {
            if !block_ids.insert(block.id) {
                self.diagnostics.push(MirDiagnostic::block(
                    &routine.name,
                    &block.label,
                    format!("duplicate block id `b{}`", block.id.0),
                ));
            }
        }

        let mut predecessor_counts = routine
            .blocks
            .iter()
            .map(|block| (block.id, 0usize))
            .collect::<std::collections::BTreeMap<_, _>>();
        for block in &routine.blocks {
            let edges: [Option<&MirEdge>; 2] = match &block.terminator {
                MirTerminator::Jump(edge) => [Some(edge), None],
                MirTerminator::Branch {
                    then_edge,
                    else_edge,
                    ..
                } => [Some(then_edge), Some(else_edge)],
                MirTerminator::Return | MirTerminator::Exit | MirTerminator::Unreachable => {
                    [None, None]
                }
            };
            for edge in edges.into_iter().flatten() {
                if let Some(count) = predecessor_counts.get_mut(&edge.target) {
                    *count = count.saturating_add(1);
                }
            }
        }

        let mut block_param_defs = BTreeSet::new();
        for block in &routine.blocks {
            let mut param_temps = BTreeSet::new();
            for param in &block.params {
                if !param_temps.insert(param.dest) {
                    self.diagnostics.push(MirDiagnostic::block(
                        &routine.name,
                        &block.label,
                        format!("duplicate block parameter `v{}`", param.dest.0),
                    ));
                }
                if !block_param_defs.insert(param.dest) {
                    self.diagnostics.push(MirDiagnostic::block(
                        &routine.name,
                        &block.label,
                        format!(
                            "block parameter temp `v{}` is defined by more than one block",
                            param.dest.0
                        ),
                    ));
                }
                self.verify_def(routine, block.label.as_str(), &MirDef::VTemp(param.dest));
            }
            if !block.params.is_empty()
                && predecessor_counts.get(&block.id).copied().unwrap_or(0) == 0
            {
                self.diagnostics.push(MirDiagnostic::block(
                    &routine.name,
                    &block.label,
                    "block parameters require at least one predecessor contribution",
                ));
            }
            if !matches!(self.phase, MirPhase::PreMaterialization) && !block.params.is_empty() {
                self.diagnostics.push(MirDiagnostic::block(
                    &routine.name,
                    &block.label,
                    "materialized MIR cannot contain block parameters",
                ));
            }
            for op in &block.ops {
                self.verify_op(
                    routine,
                    block.label.as_str(),
                    op,
                    static_ids,
                    global_ids,
                    routine_ids,
                    machine_ids,
                );
            }
            self.verify_scaled_y_protocol(routine, block.label.as_str(), &block.ops);

            match &block.terminator {
                MirTerminator::Jump(edge) => self.verify_edge(
                    routine,
                    block.label.as_str(),
                    &block_ids,
                    edge,
                    static_ids,
                    global_ids,
                    routine_ids,
                    "jump",
                ),
                MirTerminator::Branch {
                    cond,
                    then_edge,
                    else_edge,
                } => {
                    self.verify_cond(
                        routine,
                        block.label.as_str(),
                        cond,
                        static_ids,
                        global_ids,
                        routine_ids,
                    );
                    self.verify_edge(
                        routine,
                        block.label.as_str(),
                        &block_ids,
                        then_edge,
                        static_ids,
                        global_ids,
                        routine_ids,
                        "branch then",
                    );
                    self.verify_edge(
                        routine,
                        block.label.as_str(),
                        &block_ids,
                        else_edge,
                        static_ids,
                        global_ids,
                        routine_ids,
                        "branch else",
                    );
                }
                MirTerminator::Return | MirTerminator::Exit | MirTerminator::Unreachable => {}
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn verify_edge(
        &mut self,
        routine: &MirRoutine,
        block: &str,
        block_ids: &BTreeSet<MirBlockId>,
        edge: &MirEdge,
        static_ids: &BTreeSet<SymbolId>,
        global_ids: &BTreeSet<SymbolId>,
        routine_ids: &BTreeSet<RoutineId>,
        label: &str,
    ) {
        self.require_named_block_target(routine, block, block_ids, edge.target, label);
        let Some(target) = routine
            .blocks
            .iter()
            .find(|candidate| candidate.id == edge.target)
        else {
            return;
        };
        if edge.args.len() != target.params.len() {
            self.diagnostics.push(MirDiagnostic::block(
                &routine.name,
                block,
                format!(
                    "{label} edge supplies {} argument(s), expected {}",
                    edge.args.len(),
                    target.params.len()
                ),
            ));
        }
        for (index, arg) in edge.args.iter().enumerate() {
            self.verify_value(
                routine,
                block,
                &arg.value,
                static_ids,
                global_ids,
                routine_ids,
            );
            if let Some(param) = target.params.get(index)
                && arg.width != param.width
            {
                self.diagnostics.push(MirDiagnostic::block(
                    &routine.name,
                    block,
                    format!("{label} edge argument {index} width does not match target parameter"),
                ));
            }
        }
        if !matches!(self.phase, MirPhase::PreMaterialization) && !edge.args.is_empty() {
            self.diagnostics.push(MirDiagnostic::block(
                &routine.name,
                block,
                "materialized MIR cannot contain edge arguments",
            ));
        }
    }

    fn verify_frame_inits(&mut self, routine: &MirRoutine) {
        let virtual_slots = routine
            .frame
            .virtual_zero_page
            .iter()
            .copied()
            .collect::<BTreeSet<_>>();
        let mut allocated_slots = BTreeSet::new();
        let mut used_bytes = BTreeSet::new();
        for fixed in &routine.frame.fixed_zero_page {
            used_bytes.insert(fixed.0);
        }
        for allocation in &routine.frame.zero_page_allocations {
            if allocation.size == 0 {
                self.diagnostics.push(MirDiagnostic::routine(
                    &routine.name,
                    format!(
                        "zero-page allocation `zp{}` has zero size",
                        allocation.slot.0
                    ),
                ));
                continue;
            }
            if !virtual_slots.contains(&allocation.slot) {
                self.diagnostics.push(MirDiagnostic::routine(
                    &routine.name,
                    format!(
                        "zero-page allocation `zp{}` does not name a virtual slot",
                        allocation.slot.0
                    ),
                ));
            }
            if !allocated_slots.insert(allocation.slot) {
                self.diagnostics.push(MirDiagnostic::routine(
                    &routine.name,
                    format!(
                        "duplicate zero-page allocation for `zp{}`",
                        allocation.slot.0
                    ),
                ));
            }
            let Some(end) = allocation.start.0.checked_add(allocation.size - 1) else {
                self.diagnostics.push(MirDiagnostic::routine(
                    &routine.name,
                    format!("zero-page allocation `zp{}` wraps", allocation.slot.0),
                ));
                continue;
            };
            for byte in allocation.start.0..=end {
                if !used_bytes.insert(byte) {
                    self.diagnostics.push(MirDiagnostic::routine(
                        &routine.name,
                        format!(
                            "zero-page allocation `zp{}` overlaps byte ${byte:02X}",
                            allocation.slot.0
                        ),
                    ));
                }
            }
        }
        if matches!(self.phase, MirPhase::PreEmission) {
            for slot in virtual_slots {
                if !allocated_slots.contains(&slot) {
                    self.diagnostics.push(MirDiagnostic::routine(
                        &routine.name,
                        format!("virtual zero-page slot `zp{}` is not allocated", slot.0),
                    ));
                }
            }
        }
        for local in &routine.frame.locals {
            let Some(init) = &local.init else {
                continue;
            };
            self.verify_storage_init(&routine.name, &format!("l{}", local.id.0), init);
        }
    }

    fn verify_storage_init(&mut self, routine: &str, slot: &str, init: &MirStorageInit) {
        match init {
            MirStorageInit::Bytes { section, .. } | MirStorageInit::ZeroFill { section, .. } => {
                if section.is_empty() {
                    self.diagnostics.push(MirDiagnostic::routine(
                        routine,
                        format!("storage `{slot}` init section must not be empty"),
                    ));
                }
            }
            MirStorageInit::Descriptor {
                backing,
                descriptor_size,
                section,
                ..
            } => {
                if !matches!(*descriptor_size, 2 | 4) {
                    self.diagnostics.push(MirDiagnostic::routine(
                        routine,
                        format!(
                            "storage `{slot}` descriptor init has unsupported size {descriptor_size}"
                        ),
                    ));
                }
                if backing.bytes.is_empty() && backing.zero_fill == 0 {
                    self.diagnostics.push(MirDiagnostic::routine(
                        routine,
                        format!("storage `{slot}` descriptor backing is empty"),
                    ));
                }
                if backing.section.is_empty() || section.is_empty() {
                    self.diagnostics.push(MirDiagnostic::routine(
                        routine,
                        format!("storage `{slot}` descriptor sections must not be empty"),
                    ));
                }
            }
            MirStorageInit::RoutineAddress {
                routine: _target,
                descriptor_size,
                section,
                ..
            } => {
                if !matches!(*descriptor_size, 2 | 4) {
                    self.diagnostics.push(MirDiagnostic::routine(
                        routine,
                        format!(
                            "storage `{slot}` routine address init has unsupported size {descriptor_size}"
                        ),
                    ));
                }
                if section.is_empty() {
                    self.diagnostics.push(MirDiagnostic::routine(
                        routine,
                        format!("storage `{slot}` routine address init section must not be empty"),
                    ));
                }
            }
        }
    }

    fn verify_cond(
        &mut self,
        routine: &MirRoutine,
        block: &str,
        cond: &super::ir::MirCond,
        static_ids: &BTreeSet<SymbolId>,
        global_ids: &BTreeSet<SymbolId>,
        routine_ids: &BTreeSet<RoutineId>,
    ) {
        match cond {
            super::ir::MirCond::Deferred => {
                if self.abstract_conditions_forbidden() {
                    self.diagnostics.push(MirDiagnostic::block(
                        &routine.name,
                        block,
                        format!(
                            "{} MIR cannot contain deferred branch conditions",
                            self.physical_home_phase_name()
                        ),
                    ));
                }
            }
            super::ir::MirCond::BoolValue(value) => {
                if matches!(self.phase, MirPhase::PostHome)
                    && let MirValue::Def(def @ (MirDef::VTemp(_) | MirDef::VTempByte { .. })) =
                        value
                {
                    self.verify_condition_temp_use(routine, block, def);
                } else {
                    self.verify_value(routine, block, value, static_ids, global_ids, routine_ids);
                }
                if self.abstract_conditions_forbidden() {
                    self.diagnostics.push(MirDiagnostic::block(
                        &routine.name,
                        block,
                        format!(
                            "{} MIR cannot contain abstract bool branch conditions",
                            self.physical_home_phase_name()
                        ),
                    ));
                }
            }
            super::ir::MirCond::FlagTest(_) | super::ir::MirCond::AnyFlagTest(_) => {}
            super::ir::MirCond::FusedCompare { producer, .. } => {
                if !routine
                    .blocks
                    .iter()
                    .any(|block| block.id == producer.block)
                {
                    self.diagnostics.push(MirDiagnostic::block(
                        &routine.name,
                        block,
                        format!("fused compare block `b{}` does not exist", producer.block.0),
                    ));
                }
            }
        }
    }

    fn verify_op(
        &mut self,
        routine: &MirRoutine,
        block: &str,
        op: &MirOp,
        static_ids: &BTreeSet<SymbolId>,
        global_ids: &BTreeSet<SymbolId>,
        routine_ids: &BTreeSet<RoutineId>,
        machine_ids: &BTreeSet<MirMachineBlockId>,
    ) {
        match op {
            MirOp::LoadImm { dst, width, .. } => {
                self.verify_pre_emission_width(routine, block, *width);
                self.verify_def(routine, block, dst);
            }
            MirOp::Load { dst, src, width } => {
                self.verify_pre_emission_width(routine, block, *width);
                self.verify_def(routine, block, dst);
                self.verify_addr(routine, block, src, static_ids, global_ids, routine_ids);
            }
            MirOp::Store { dst, src, width } => {
                self.verify_pre_emission_width(routine, block, *width);
                self.verify_addr(routine, block, dst, static_ids, global_ids, routine_ids);
                self.verify_value(routine, block, src, static_ids, global_ids, routine_ids);
            }
            MirOp::UpdateMem { op, mem, width } => {
                if !matches!(
                    (op, width),
                    (super::ir::MirUpdateOp::Inc, super::ir::MirWidth::Word)
                        | (super::ir::MirUpdateOp::Dec, super::ir::MirWidth::Word)
                ) {
                    self.verify_pre_emission_width(routine, block, *width);
                }
                self.verify_mem(routine, block, &routine.frame, mem, static_ids, global_ids);
                if !matches!(width, super::ir::MirWidth::Byte | super::ir::MirWidth::Word) {
                    self.diagnostics.push(MirDiagnostic::block(
                        &routine.name,
                        block,
                        "memory update must be byte or word width",
                    ));
                }
                if matches!(width, super::ir::MirWidth::Word)
                    && !matches!(
                        op,
                        super::ir::MirUpdateOp::Inc | super::ir::MirUpdateOp::Dec
                    )
                {
                    self.diagnostics.push(MirDiagnostic::block(
                        &routine.name,
                        block,
                        "word memory update only supports increment or decrement",
                    ));
                }
            }
            MirOp::UpdateIndexedMem { base, .. } => {
                self.verify_mem(routine, block, &routine.frame, base, static_ids, global_ids);
                if matches!(base, MirMem::ZeroPage(_) | MirMem::FixedZeroPage(_)) {
                    self.diagnostics.push(MirDiagnostic::block(
                        &routine.name,
                        block,
                        "indexed memory update requires an absolute-addressable base",
                    ));
                }
            }
            MirOp::AddByteToWordMem { mem, value } | MirOp::SubByteFromWordMem { mem, value } => {
                self.verify_mem(routine, block, &routine.frame, mem, static_ids, global_ids);
                if let super::ir::MirValue::PointerCell(value_mem) = value {
                    self.verify_mem(
                        routine,
                        block,
                        &routine.frame,
                        value_mem,
                        static_ids,
                        global_ids,
                    );
                } else {
                    self.verify_value(routine, block, value, static_ids, global_ids, routine_ids);
                }
            }
            MirOp::Move { dst, src, width } => {
                self.verify_pre_emission_width(routine, block, *width);
                self.verify_def(routine, block, dst);
                self.verify_value(routine, block, src, static_ids, global_ids, routine_ids);
            }
            MirOp::LeaAddr { dst, target, width } => {
                self.verify_def(routine, block, dst);
                self.verify_mem(
                    routine,
                    block,
                    &routine.frame,
                    target,
                    static_ids,
                    global_ids,
                );
                if !matches!(width, super::ir::MirWidth::Word) {
                    self.diagnostics.push(MirDiagnostic::block(
                        &routine.name,
                        block,
                        "address materialization must produce word width",
                    ));
                }
            }
            MirOp::AdvanceAddress {
                consumer,
                index,
                scale,
            } => {
                self.verify_address_consumer(routine, block, consumer);
                self.reject_scaled_y_consumer(routine, block, consumer, "address advance");
                self.verify_value(routine, block, index, static_ids, global_ids, routine_ids);
                if *scale == 0 {
                    self.diagnostics.push(MirDiagnostic::block(
                        &routine.name,
                        block,
                        "address advance scale must be nonzero",
                    ));
                }
            }
            MirOp::Extend { dst, src, .. } | MirOp::Truncate { dst, src, .. } => {
                self.verify_def(routine, block, dst);
                self.verify_value(routine, block, src, static_ids, global_ids, routine_ids);
            }
            MirOp::Unary { dst, src, .. } => {
                self.verify_def(routine, block, dst);
                self.verify_value(routine, block, src, static_ids, global_ids, routine_ids);
            }
            MirOp::Binary {
                op,
                dst,
                left,
                right,
                carry_in,
                width,
                ..
            } => {
                self.verify_pre_emission_width(routine, block, *width);
                self.verify_def(routine, block, dst);
                self.verify_value(routine, block, left, static_ids, global_ids, routine_ids);
                self.verify_rhs_value(routine, block, right, static_ids, global_ids, routine_ids);
                if matches!(
                    self.phase,
                    MirPhase::PostHome | MirPhase::PostMaterialization | MirPhase::PreEmission
                ) && matches!(op, MirBinaryOp::Add | MirBinaryOp::Sub)
                    && matches!(width, super::ir::MirWidth::Byte)
                    && carry_in.is_none()
                {
                    self.diagnostics.push(MirDiagnostic::block(
                        &routine.name,
                        block,
                        "pre-emission add/sub cannot have unspecified carry_in",
                    ));
                }
            }
            MirOp::Compare {
                dst,
                left,
                right,
                width,
                ..
            } => {
                self.verify_pre_emission_width(routine, block, *width);
                self.verify_cond_dest(routine, block, dst);
                self.verify_value(routine, block, left, static_ids, global_ids, routine_ids);
                self.verify_rhs_value(routine, block, right, static_ids, global_ids, routine_ids);
            }
            MirOp::Call {
                target,
                abi,
                args,
                result,
                effects: _,
            } => {
                match target {
                    super::ir::MirCallTarget::Routine(id) if !routine_ids.contains(id) => {
                        self.diagnostics.push(MirDiagnostic::block(
                            &routine.name,
                            block,
                            format!("call target `r{}` does not exist", id.0),
                        ));
                    }
                    super::ir::MirCallTarget::Indirect { target, width } => {
                        if !matches!(width, super::ir::MirWidth::Word) {
                            self.diagnostics.push(MirDiagnostic::block(
                                &routine.name,
                                block,
                                "indirect call target must be word-width",
                            ));
                        }
                        self.verify_indirect_call_target(
                            routine,
                            block,
                            target,
                            static_ids,
                            global_ids,
                            routine_ids,
                        );
                    }
                    _ => {}
                }
                if args.len() != abi.params.len() {
                    self.diagnostics.push(MirDiagnostic::block(
                        &routine.name,
                        block,
                        "call ABI parameter count does not match argument bindings",
                    ));
                }
                let mut arg_offset = 0u16;
                for (index, arg) in args.iter().enumerate() {
                    if abi.params.get(index) != Some(&arg.home) {
                        self.diagnostics.push(MirDiagnostic::block(
                            &routine.name,
                            block,
                            format!("call argument {index} home does not match ABI"),
                        ));
                    }
                    let expected_home = action_arg_home(arg_offset, arg.width);
                    if arg.home != expected_home {
                        self.diagnostics.push(MirDiagnostic::block(
                            &routine.name,
                            block,
                            format!(
                                "call argument {index} does not use the canonical Action ABI home at byte offset {arg_offset}"
                            ),
                        ));
                    }
                    arg_offset = arg_offset.saturating_add(action_arg_width_bytes(arg.width));
                    self.verify_value(
                        routine,
                        block,
                        &arg.value,
                        static_ids,
                        global_ids,
                        routine_ids,
                    );
                    self.verify_pre_emission_width(routine, block, arg.width);
                }
                match (result, &abi.result) {
                    (Some(result), Some(home)) if &result.home == home => {
                        self.verify_def(routine, block, &result.dst);
                        self.verify_pre_emission_width(routine, block, result.width);
                    }
                    (None, None) => {}
                    _ => {
                        self.diagnostics.push(MirDiagnostic::block(
                            &routine.name,
                            block,
                            "call result home does not match ABI",
                        ));
                    }
                }
            }
            MirOp::MaterializeAddress { consumer, value } => {
                self.verify_address_consumer(routine, block, consumer);
                self.reject_scaled_y_consumer(routine, block, consumer, "materialize address");
                self.verify_value(routine, block, value, static_ids, global_ids, routine_ids);
            }
            MirOp::MaterializeIndexedAddress {
                consumer,
                base,
                index,
                scale,
            } => {
                self.verify_address_consumer(routine, block, consumer);
                if consumer.uses_scaled_y() && *scale != 2 {
                    self.diagnostics.push(MirDiagnostic::block(
                        &routine.name,
                        block,
                        "scaled-Y address materialization requires scale two",
                    ));
                }
                self.verify_value_allow_pointer_cell(
                    routine,
                    block,
                    base,
                    static_ids,
                    global_ids,
                    routine_ids,
                );
                self.verify_value_allow_pointer_cell(
                    routine,
                    block,
                    index,
                    static_ids,
                    global_ids,
                    routine_ids,
                );
            }
            MirOp::LoadIndirect {
                consumer,
                dst,
                offset,
            } => {
                self.verify_address_consumer(routine, block, consumer);
                self.verify_scaled_y_offset(routine, block, consumer, *offset);
                self.verify_def(routine, block, dst);
            }
            MirOp::StoreIndirect {
                consumer,
                src,
                offset,
            } => {
                self.verify_address_consumer(routine, block, consumer);
                self.verify_scaled_y_offset(routine, block, consumer, *offset);
                self.verify_value(routine, block, src, static_ids, global_ids, routine_ids);
            }
            MirOp::IndirectByteCompound {
                op, target, source, ..
            } => {
                self.verify_address_consumer(routine, block, target);
                self.verify_address_consumer(routine, block, source);
                self.reject_scaled_y_consumer(routine, block, target, "indirect compound target");
                self.reject_scaled_y_consumer(routine, block, source, "indirect compound source");
                if !matches!(
                    op,
                    super::ir::MirBinaryOp::Add | super::ir::MirBinaryOp::Sub
                ) {
                    self.diagnostics.push(MirDiagnostic::block(
                        &routine.name,
                        block,
                        "indirect byte compound only supports add and subtract",
                    ));
                }
            }
            MirOp::RuntimeHelper { .. } | MirOp::Barrier { .. } => {}
            MirOp::MachineBlock { id, .. } => {
                if !machine_ids.contains(id) {
                    self.diagnostics.push(MirDiagnostic::block(
                        &routine.name,
                        block,
                        format!("machine block `m{}` does not exist", id.0),
                    ));
                }
            }
        }
    }

    fn verify_cond_dest(&mut self, routine: &MirRoutine, block: &str, dst: &MirCondDest) {
        match dst {
            MirCondDest::Temp(id) if !routine.temps.iter().any(|temp| temp.id == *id) => {
                self.diagnostics.push(MirDiagnostic::block(
                    &routine.name,
                    block,
                    format!("condition temp `v{}` does not exist", id.0),
                ));
            }
            MirCondDest::Temp(_) | MirCondDest::Flags => {}
        }
    }

    fn verify_condition_temp_use(&mut self, routine: &MirRoutine, block: &str, def: &MirDef) {
        match def {
            MirDef::VTemp(id) => {
                if !routine.temps.iter().any(|temp| temp.id == *id) {
                    self.diagnostics.push(MirDiagnostic::block(
                        &routine.name,
                        block,
                        format!("condition temp `v{}` does not exist", id.0),
                    ));
                }
            }
            MirDef::VTempByte { id, byte } => {
                if !routine.temps.iter().any(|temp| temp.id == *id) {
                    self.diagnostics.push(MirDiagnostic::block(
                        &routine.name,
                        block,
                        format!("condition temp `v{}` does not exist", id.0),
                    ));
                }
                if *byte > 1 {
                    self.diagnostics.push(MirDiagnostic::block(
                        &routine.name,
                        block,
                        format!("condition temp byte `v{}.b{}` is out of range", id.0, byte),
                    ));
                }
            }
            MirDef::Reg(_) => unreachable!("condition temp helper called with a register"),
        }
    }

    fn verify_address_consumer(
        &mut self,
        routine: &MirRoutine,
        block: &str,
        consumer: &MirAddressConsumer,
    ) {
        match consumer.pointer_pair() {
            MirPointerPair::Fixed { .. } => {}
            MirPointerPair::Virtual(slot) => {
                let message = format!(
                    "virtual address pair `zp{}` is not supported before emission",
                    slot.0
                );
                if matches!(self.phase, MirPhase::PreEmission) {
                    self.diagnostics
                        .push(MirDiagnostic::block(&routine.name, block, message));
                }
            }
        }
    }

    fn verify_scaled_y_protocol(&mut self, routine: &MirRoutine, block: &str, ops: &[MirOp]) {
        let mut prepared = Vec::<(MirPointerPair, MirValue)>::new();
        let mut active_index = None::<MirValue>;
        let mut active_offset = 0u16;

        for (op_index, op) in ops.iter().enumerate() {
            match op {
                MirOp::MaterializeIndexedAddress {
                    consumer: MirAddressConsumer::ScaledIndirectIndexedY(pair),
                    index,
                    ..
                } => {
                    prepared.retain(|(candidate, _)| candidate != pair);
                    prepared.push((*pair, index.clone()));
                    active_index = Some(index.clone());
                    active_offset = 0;
                    continue;
                }
                MirOp::LoadIndirect {
                    consumer: MirAddressConsumer::ScaledIndirectIndexedY(pair),
                    offset,
                    ..
                }
                | MirOp::StoreIndirect {
                    consumer: MirAddressConsumer::ScaledIndirectIndexedY(pair),
                    offset,
                    ..
                } => {
                    let prepared_index = prepared
                        .iter()
                        .find_map(|(candidate, index)| (candidate == pair).then_some(index));
                    if prepared_index.is_none() || prepared_index != active_index.as_ref() {
                        self.diagnostics.push(MirDiagnostic::block(
                            &routine.name,
                            block,
                            format!(
                                "scaled-Y access at op #{op_index} has no active matching index"
                            ),
                        ));
                    }
                    if *offset < active_offset {
                        self.diagnostics.push(MirDiagnostic::block(
                            &routine.name,
                            block,
                            format!(
                                "scaled-Y access at op #{op_index} moves backward from offset {active_offset} to {offset}"
                            ),
                        ));
                    } else {
                        active_offset = *offset;
                    }
                    if matches!(
                        op,
                        MirOp::StoreIndirect {
                            src: MirValue::Def(MirDef::Reg(MirReg::Y)),
                            ..
                        }
                    ) {
                        self.diagnostics.push(MirDiagnostic::block(
                            &routine.name,
                            block,
                            format!("scaled-Y store at op #{op_index} cannot source Y"),
                        ));
                    }
                    continue;
                }
                _ => {}
            }

            let effects = classify_op(op);
            prepared.retain(|(pair, _)| {
                pointer_pair_homes(*pair).iter().all(|home| {
                    !effects.homes.writes.contains(home)
                        && !effects.addresses.pair_writes.contains(home)
                })
            });
            if effects.may_clobber_reg_compat(MirReg::Y) {
                active_index = None;
                active_offset = 0;
            }
        }
    }

    fn reject_scaled_y_consumer(
        &mut self,
        routine: &MirRoutine,
        block: &str,
        consumer: &MirAddressConsumer,
        operation: &str,
    ) {
        if consumer.uses_scaled_y() {
            self.diagnostics.push(MirDiagnostic::block(
                &routine.name,
                block,
                format!("{operation} cannot use a scaled-Y address consumer"),
            ));
        }
    }

    fn verify_scaled_y_offset(
        &mut self,
        routine: &MirRoutine,
        block: &str,
        consumer: &MirAddressConsumer,
        offset: u16,
    ) {
        if consumer.uses_scaled_y() && offset > 1 {
            self.diagnostics.push(MirDiagnostic::block(
                &routine.name,
                block,
                "scaled-Y indirect access only supports offsets zero and one",
            ));
        }
    }

    fn verify_def(&mut self, routine: &MirRoutine, block: &str, def: &MirDef) {
        match def {
            MirDef::VTemp(id) => {
                if !routine.temps.iter().any(|temp| temp.id == *id) {
                    self.diagnostics.push(MirDiagnostic::block(
                        &routine.name,
                        block,
                        format!("temp definition `v{}` does not exist", id.0),
                    ));
                }
                if self.physical_homes_required() {
                    self.diagnostics.push(MirDiagnostic::block(
                        &routine.name,
                        block,
                        format!(
                            "{} MIR cannot contain virtual temp `v{}`",
                            self.physical_home_phase_name(),
                            id.0
                        ),
                    ));
                }
            }
            MirDef::VTempByte { id, byte } => {
                if !routine.temps.iter().any(|temp| temp.id == *id) {
                    self.diagnostics.push(MirDiagnostic::block(
                        &routine.name,
                        block,
                        format!("temp byte definition `v{}` does not exist", id.0),
                    ));
                }
                if *byte > 1 {
                    self.diagnostics.push(MirDiagnostic::block(
                        &routine.name,
                        block,
                        format!("temp byte definition `v{}.b{}` is out of range", id.0, byte),
                    ));
                }
                if self.physical_homes_required() {
                    self.diagnostics.push(MirDiagnostic::block(
                        &routine.name,
                        block,
                        format!(
                            "{} MIR cannot contain virtual temp byte `v{}.b{}`",
                            self.physical_home_phase_name(),
                            id.0,
                            byte
                        ),
                    ));
                }
            }
            MirDef::Reg(_) => {}
        }
    }

    fn verify_pre_emission_width(
        &mut self,
        routine: &MirRoutine,
        block: &str,
        width: super::ir::MirWidth,
    ) {
        if matches!(self.phase, MirPhase::PreEmission) && matches!(width, super::ir::MirWidth::Word)
        {
            self.diagnostics.push(MirDiagnostic::block(
                &routine.name,
                block,
                "pre-emission MIR cannot contain word-width pseudo ops",
            ));
        }
    }

    fn verify_value(
        &mut self,
        routine: &MirRoutine,
        block: &str,
        value: &MirValue,
        static_ids: &BTreeSet<SymbolId>,
        global_ids: &BTreeSet<SymbolId>,
        routine_ids: &BTreeSet<RoutineId>,
    ) {
        match value {
            MirValue::ConstU8(_) | MirValue::ConstU16(_) => {}
            MirValue::Def(def) => self.verify_def(routine, block, def),
            MirValue::Word { lo, hi } => {
                self.verify_value(routine, block, lo, static_ids, global_ids, routine_ids);
                self.verify_value(routine, block, hi, static_ids, global_ids, routine_ids);
            }
            MirValue::StaticAddr(id) => {
                self.require_symbol(routine, block, static_ids, *id, "static")
            }
            MirValue::GlobalAddr(id) => {
                self.require_symbol(routine, block, global_ids, *id, "global")
            }
            MirValue::RoutineAddr(id) if !routine_ids.contains(id) => {
                self.diagnostics.push(MirDiagnostic::block(
                    &routine.name,
                    block,
                    format!("routine address `r{}` does not exist", id.0),
                ));
            }
            MirValue::RoutineAddr(_) => {}
            MirValue::RoutineAddrByte { id, byte } => {
                if !routine_ids.contains(id) {
                    self.diagnostics.push(MirDiagnostic::block(
                        &routine.name,
                        block,
                        format!("routine address `r{}` does not exist", id.0),
                    ));
                }
                if *byte > 1 {
                    self.diagnostics.push(MirDiagnostic::block(
                        &routine.name,
                        block,
                        format!("routine address byte `{byte}` is invalid"),
                    ));
                }
            }
            MirValue::StorageAddrByte { mem, byte } => {
                self.verify_mem(routine, block, &routine.frame, mem, static_ids, global_ids);
                if *byte > 1 {
                    self.diagnostics.push(MirDiagnostic::block(
                        &routine.name,
                        block,
                        format!("storage address byte `{byte}` is invalid"),
                    ));
                }
            }
            MirValue::PointerCell(mem) => {
                self.verify_mem(routine, block, &routine.frame, mem, static_ids, global_ids);
                if matches!(self.phase, MirPhase::PreEmission) {
                    self.diagnostics.push(MirDiagnostic::block(
                        &routine.name,
                        block,
                        format!("pointer-cell value {mem:?} must be materialized before emission"),
                    ));
                }
            }
        }
    }

    fn verify_value_allow_pointer_cell(
        &mut self,
        routine: &MirRoutine,
        block: &str,
        value: &MirValue,
        static_ids: &BTreeSet<SymbolId>,
        global_ids: &BTreeSet<SymbolId>,
        routine_ids: &BTreeSet<RoutineId>,
    ) {
        match value {
            MirValue::Word { lo, hi } => {
                self.verify_value_allow_pointer_cell(
                    routine,
                    block,
                    lo,
                    static_ids,
                    global_ids,
                    routine_ids,
                );
                self.verify_value_allow_pointer_cell(
                    routine,
                    block,
                    hi,
                    static_ids,
                    global_ids,
                    routine_ids,
                );
            }
            MirValue::PointerCell(mem) => {
                self.verify_mem(routine, block, &routine.frame, mem, static_ids, global_ids);
            }
            _ => self.verify_value(routine, block, value, static_ids, global_ids, routine_ids),
        }
    }

    fn verify_indirect_call_target(
        &mut self,
        routine: &MirRoutine,
        block: &str,
        value: &MirValue,
        static_ids: &BTreeSet<SymbolId>,
        global_ids: &BTreeSet<SymbolId>,
        routine_ids: &BTreeSet<RoutineId>,
    ) {
        match value {
            MirValue::Word { lo, hi } => {
                self.verify_indirect_call_target_byte(
                    routine,
                    block,
                    lo,
                    static_ids,
                    global_ids,
                    routine_ids,
                );
                self.verify_indirect_call_target_byte(
                    routine,
                    block,
                    hi,
                    static_ids,
                    global_ids,
                    routine_ids,
                );
            }
            MirValue::ConstU16(_) | MirValue::RoutineAddr(_) => {
                if matches!(self.phase, MirPhase::PreEmission) {
                    self.diagnostics.push(MirDiagnostic::block(
                        &routine.name,
                        block,
                        "indirect call target must be materialized into a callable home",
                    ));
                }
            }
            other => self.verify_value(routine, block, other, static_ids, global_ids, routine_ids),
        }
    }

    fn verify_indirect_call_target_byte(
        &mut self,
        routine: &MirRoutine,
        block: &str,
        value: &MirValue,
        static_ids: &BTreeSet<SymbolId>,
        global_ids: &BTreeSet<SymbolId>,
        routine_ids: &BTreeSet<RoutineId>,
    ) {
        match value {
            MirValue::PointerCell(mem) => {
                self.verify_mem(routine, block, &routine.frame, mem, static_ids, global_ids);
            }
            MirValue::ConstU8(_) => {}
            other => self.verify_value(routine, block, other, static_ids, global_ids, routine_ids),
        }
    }

    fn verify_rhs_value(
        &mut self,
        routine: &MirRoutine,
        block: &str,
        value: &MirValue,
        static_ids: &BTreeSet<SymbolId>,
        global_ids: &BTreeSet<SymbolId>,
        routine_ids: &BTreeSet<RoutineId>,
    ) {
        if matches!(self.phase, MirPhase::PreEmission)
            && let MirValue::PointerCell(mem) = value
        {
            self.verify_mem(routine, block, &routine.frame, mem, static_ids, global_ids);
            return;
        }
        self.verify_value(routine, block, value, static_ids, global_ids, routine_ids);
    }

    fn verify_addr(
        &mut self,
        routine: &MirRoutine,
        block: &str,
        addr: &MirAddr,
        static_ids: &BTreeSet<SymbolId>,
        global_ids: &BTreeSet<SymbolId>,
        routine_ids: &BTreeSet<RoutineId>,
    ) {
        match addr {
            MirAddr::Direct(mem) => {
                self.verify_mem(routine, block, &routine.frame, mem, static_ids, global_ids)
            }
            MirAddr::Label(_) => self.diagnostics.push(MirDiagnostic::block(
                &routine.name,
                block,
                "label addresses are not supported before materialization",
            )),
            MirAddr::ZeroPageIndexedX { base } => {
                if !routine.frame.virtual_zero_page.contains(base) {
                    self.diagnostics.push(MirDiagnostic::block(
                        &routine.name,
                        block,
                        format!("zero-page slot `zp{}` does not exist", base.0),
                    ));
                }
                if matches!(self.phase, MirPhase::PreMaterialization) {
                    self.diagnostics.push(MirDiagnostic::block(
                        &routine.name,
                        block,
                        "indexed addresses are not supported before materialization",
                    ));
                }
            }
            MirAddr::AbsoluteIndexedX { base } | MirAddr::AbsoluteIndexedY { base } => {
                self.verify_mem(routine, block, &routine.frame, base, static_ids, global_ids);
                if matches!(self.phase, MirPhase::PreMaterialization) {
                    self.diagnostics.push(MirDiagnostic::block(
                        &routine.name,
                        block,
                        "indexed addresses are not supported before materialization",
                    ));
                }
            }
            MirAddr::IndirectIndexedY { zp } => {
                if !routine.frame.virtual_zero_page.contains(zp) {
                    self.diagnostics.push(MirDiagnostic::block(
                        &routine.name,
                        block,
                        format!("zero-page slot `zp{}` does not exist", zp.0),
                    ));
                }
                if matches!(self.phase, MirPhase::PreMaterialization) {
                    self.diagnostics.push(MirDiagnostic::block(
                        &routine.name,
                        block,
                        "indexed addresses are not supported before materialization",
                    ));
                }
            }
            MirAddr::FixedIndirectIndexedY { zp } => {
                if !routine.frame.fixed_zero_page.contains(zp) {
                    self.diagnostics.push(MirDiagnostic::block(
                        &routine.name,
                        block,
                        format!("fixed zero-page slot `${:02X}` does not exist", zp.0),
                    ));
                }
                if matches!(self.phase, MirPhase::PreMaterialization) {
                    self.diagnostics.push(MirDiagnostic::block(
                        &routine.name,
                        block,
                        "indexed addresses are not supported before materialization",
                    ));
                }
            }
            MirAddr::ComputedIndex {
                base,
                index,
                elem_size,
                offset: _,
            } => {
                if *elem_size == 0 {
                    self.diagnostics.push(MirDiagnostic::block(
                        &routine.name,
                        block,
                        "computed index address has zero element size",
                    ));
                }
                self.verify_value(routine, block, base, static_ids, global_ids, routine_ids);
                self.verify_value(routine, block, index, static_ids, global_ids, routine_ids);
                if matches!(self.phase, MirPhase::PreEmission) {
                    let message = if *elem_size > 1 {
                        "dynamic word index addresses must be materialized before emission"
                    } else {
                        "computed index addresses must be materialized before emission"
                    };
                    self.diagnostics
                        .push(MirDiagnostic::block(&routine.name, block, message));
                }
            }
            MirAddr::PointerCell { ptr, .. } => {
                self.verify_mem(routine, block, &routine.frame, ptr, static_ids, global_ids);
                if matches!(self.phase, MirPhase::PreEmission) {
                    self.diagnostics.push(MirDiagnostic::block(
                        &routine.name,
                        block,
                        "pointer-cell addresses must be materialized before emission",
                    ));
                }
            }
            MirAddr::PointerIndex {
                ptr,
                index,
                elem_size,
                offset: _,
            } => {
                if *elem_size == 0 {
                    self.diagnostics.push(MirDiagnostic::block(
                        &routine.name,
                        block,
                        "pointer index address has zero element size",
                    ));
                }
                self.verify_mem(routine, block, &routine.frame, ptr, static_ids, global_ids);
                self.verify_value(routine, block, index, static_ids, global_ids, routine_ids);
                if matches!(self.phase, MirPhase::PreEmission) {
                    let message = if *elem_size > 1 {
                        "dynamic pointer word index addresses must be materialized before emission"
                    } else {
                        "pointer index addresses must be materialized before emission"
                    };
                    self.diagnostics
                        .push(MirDiagnostic::block(&routine.name, block, message));
                }
            }
            MirAddr::Deref { ptr, .. } => {
                self.verify_value(routine, block, ptr, static_ids, global_ids, routine_ids);
                if matches!(self.phase, MirPhase::PreEmission) {
                    self.diagnostics.push(MirDiagnostic::block(
                        &routine.name,
                        block,
                        "dereference addresses must be materialized before emission",
                    ));
                }
            }
        }
    }

    fn verify_mem(
        &mut self,
        routine: &MirRoutine,
        block: &str,
        frame: &MirFrame,
        mem: &MirMem,
        static_ids: &BTreeSet<SymbolId>,
        global_ids: &BTreeSet<SymbolId>,
    ) {
        match mem {
            MirMem::Absolute(_) => {}
            MirMem::Static { id, .. } => {
                self.require_symbol(routine, block, static_ids, *id, "static")
            }
            MirMem::Global { id, .. } => {
                self.require_symbol(routine, block, global_ids, *id, "global")
            }
            MirMem::Local { id, .. } if !has_local_slot(frame, *id) => {
                self.diagnostics.push(MirDiagnostic::block(
                    &routine.name,
                    block,
                    format!("local storage `l{}` does not exist", id.0),
                ));
            }
            MirMem::Param { id, .. } if !has_param_slot(frame, *id) => {
                self.diagnostics.push(MirDiagnostic::block(
                    &routine.name,
                    block,
                    format!("param storage `p{}` does not exist", id.0),
                ));
            }
            MirMem::Spill { id, .. } if !frame.spills.contains(id) => {
                self.diagnostics.push(MirDiagnostic::block(
                    &routine.name,
                    block,
                    format!("spill storage `sp{}` does not exist", id.0),
                ));
            }
            MirMem::ZeroPage(id) if !frame.virtual_zero_page.contains(id) => {
                self.diagnostics.push(MirDiagnostic::block(
                    &routine.name,
                    block,
                    format!("zero-page slot `zp{}` does not exist", id.0),
                ));
            }
            MirMem::FixedZeroPage(id) if !frame.fixed_zero_page.contains(id) => {
                self.diagnostics.push(MirDiagnostic::block(
                    &routine.name,
                    block,
                    format!("fixed zero-page slot `${:02X}` does not exist", id.0),
                ));
            }
            MirMem::Local { .. }
            | MirMem::Param { .. }
            | MirMem::Spill { .. }
            | MirMem::ZeroPage(_)
            | MirMem::FixedZeroPage(_) => {}
        }
    }

    fn require_symbol(
        &mut self,
        routine: &MirRoutine,
        block: &str,
        symbols: &BTreeSet<SymbolId>,
        id: SymbolId,
        kind: &str,
    ) {
        if !symbols.contains(&id) {
            self.diagnostics.push(MirDiagnostic::block(
                &routine.name,
                block,
                format!(
                    "{kind} storage `{}{}` does not exist",
                    symbol_prefix(kind),
                    id.0
                ),
            ));
        }
    }

    fn require_named_block_target(
        &mut self,
        routine: &MirRoutine,
        block: &str,
        block_ids: &BTreeSet<MirBlockId>,
        target: MirBlockId,
        edge: &str,
    ) {
        if !block_ids.contains(&target) {
            self.diagnostics.push(MirDiagnostic::block(
                &routine.name,
                block,
                format!("{edge} target `b{}` does not exist", target.0),
            ));
        }
    }
}

fn pointer_pair_homes(pair: MirPointerPair) -> Vec<MirHomeByte> {
    match pair {
        MirPointerPair::Fixed { lo } => vec![
            MirHomeByte::FixedZeroPage(lo),
            MirHomeByte::FixedZeroPage(super::ir::MirFixedZpSlot(lo.0.saturating_add(1))),
        ],
        MirPointerPair::Virtual(slot) => vec![MirHomeByte::VirtualZeroPage(slot)],
    }
}

fn symbol_prefix(kind: &str) -> &'static str {
    match kind {
        "static" => "s",
        "global" => "g",
        _ => "",
    }
}

fn has_param_slot(frame: &MirFrame, id: crate::nir::ParamId) -> bool {
    frame
        .params
        .iter()
        .any(|slot| matches!(slot.base, MirStorageBase::Param(param) if param == id))
}

fn has_local_slot(frame: &MirFrame, id: crate::nir::LocalId) -> bool {
    frame.locals.iter().any(|slot| match slot.base {
        MirStorageBase::Local(local) => local == id,
        MirStorageBase::LocalAlias { id: alias, .. } => alias == id,
        _ => false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mir6502::{
        MirArgHome, MirBlock, MirCallAbi, MirCallArg, MirCallTarget, MirEffects, MirFixedZpSlot,
        MirFrame, MirGlobal, MirProgram, MirRegisterSet, MirRoutine, MirRoutineAbi,
        MirRuntimeHelper, MirRuntimeHelperDecl, MirRuntimeHelperTarget, MirStatic, MirTemp,
        MirTempId, MirWidth, RoutineId,
    };
    use crate::nir::SymbolId;

    #[test]
    fn accepts_valid_shell_program() {
        let program = program_with_routines(vec![routine(
            RoutineId(0),
            "Main",
            vec![block(MirBlockId(0), "bb0", MirTerminator::Return)],
        )]);

        assert!(verify_program(&program, MirPhase::PreMaterialization).is_ok());
    }

    #[test]
    fn rejects_duplicate_routine_ids() {
        let program = program_with_routines(vec![
            routine(
                RoutineId(0),
                "One",
                vec![block(MirBlockId(0), "bb0", MirTerminator::Return)],
            ),
            routine(
                RoutineId(0),
                "Two",
                vec![block(MirBlockId(0), "bb0", MirTerminator::Return)],
            ),
        ]);

        let diagnostics = verify_program(&program, MirPhase::PreMaterialization)
            .expect_err("duplicate routine id rejected");
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.message.contains("duplicate routine id `r0`"))
        );
    }

    #[test]
    fn rejects_duplicate_block_ids() {
        let program = program_with_routines(vec![routine(
            RoutineId(0),
            "Main",
            vec![
                block(MirBlockId(0), "bb0", MirTerminator::Return),
                block(MirBlockId(0), "bb1", MirTerminator::Return),
            ],
        )]);

        let diagnostics = verify_program(&program, MirPhase::PreMaterialization)
            .expect_err("duplicate block id rejected");
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.message.contains("duplicate block id `b0`"))
        );
    }

    #[test]
    fn rejects_missing_jump_target() {
        let program = program_with_routines(vec![routine(
            RoutineId(0),
            "Main",
            vec![block(
                MirBlockId(0),
                "bb0",
                MirTerminator::Jump(MirEdge::plain(MirBlockId(99))),
            )],
        )]);

        let diagnostics = verify_program(&program, MirPhase::PreMaterialization)
            .expect_err("missing jump target rejected");
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.message.contains("jump target `b99`"))
        );
    }

    #[test]
    fn rejects_missing_branch_target() {
        let program = program_with_routines(vec![routine(
            RoutineId(0),
            "Main",
            vec![block(
                MirBlockId(0),
                "bb0",
                MirTerminator::Branch {
                    cond: crate::mir6502::MirCond::Deferred,
                    then_edge: MirEdge::plain(MirBlockId(1)),
                    else_edge: MirEdge::plain(MirBlockId(2)),
                },
            )],
        )]);

        let diagnostics = verify_program(&program, MirPhase::PreMaterialization)
            .expect_err("missing branch target rejected");
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.message.contains("branch then target `b1`"))
        );
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.message.contains("branch else target `b2`"))
        );
    }

    #[test]
    fn rejects_block_argument_arity_mismatch() {
        let mut main = routine(
            RoutineId(0),
            "Main",
            vec![
                block(
                    MirBlockId(0),
                    "entry",
                    MirTerminator::Jump(MirEdge::plain(MirBlockId(1))),
                ),
                block(MirBlockId(1), "join", MirTerminator::Return),
            ],
        );
        main.temps.push(MirTemp { id: MirTempId(0) });
        main.blocks[1].params.push(crate::mir6502::MirBlockParam {
            dest: MirTempId(0),
            width: MirWidth::Byte,
        });

        let diagnostics = verify_program(
            &program_with_routines(vec![main]),
            MirPhase::PreMaterialization,
        )
        .expect_err("missing block argument rejected");
        assert!(diagnostics.iter().any(|diagnostic| {
            diagnostic
                .message
                .contains("supplies 0 argument(s), expected 1")
        }));
    }

    #[test]
    fn rejects_block_argument_width_mismatch() {
        let mut main = routine(
            RoutineId(0),
            "Main",
            vec![
                block(
                    MirBlockId(0),
                    "entry",
                    MirTerminator::Jump(MirEdge {
                        target: MirBlockId(1),
                        args: vec![crate::mir6502::MirEdgeArg {
                            value: MirValue::ConstU16(1),
                            width: MirWidth::Word,
                        }],
                    }),
                ),
                block(MirBlockId(1), "join", MirTerminator::Return),
            ],
        );
        main.temps.push(MirTemp { id: MirTempId(0) });
        main.blocks[1].params.push(crate::mir6502::MirBlockParam {
            dest: MirTempId(0),
            width: MirWidth::Byte,
        });

        let diagnostics = verify_program(
            &program_with_routines(vec![main]),
            MirPhase::PreMaterialization,
        )
        .expect_err("mismatched block argument width rejected");
        assert!(diagnostics.iter().any(|diagnostic| {
            diagnostic
                .message
                .contains("argument 0 width does not match target parameter")
        }));
    }

    #[test]
    fn rejects_block_parameters_without_predecessor_contribution() {
        let mut main = routine(
            RoutineId(0),
            "Main",
            vec![block(MirBlockId(0), "entry", MirTerminator::Return)],
        );
        main.temps.push(MirTemp { id: MirTempId(0) });
        main.blocks[0].params.push(crate::mir6502::MirBlockParam {
            dest: MirTempId(0),
            width: MirWidth::Byte,
        });

        let diagnostics = verify_program(
            &program_with_routines(vec![main]),
            MirPhase::PreMaterialization,
        )
        .expect_err("orphan block parameter rejected");
        assert!(diagnostics.iter().any(|diagnostic| {
            diagnostic
                .message
                .contains("require at least one predecessor contribution")
        }));
    }

    #[test]
    fn rejects_unknown_static_address() {
        let program = program_with_routines(vec![routine(
            RoutineId(0),
            "Main",
            vec![block_with_ops(
                MirBlockId(0),
                "bb0",
                vec![MirOp::Load {
                    dst: MirDef::Reg(crate::mir6502::MirReg::A),
                    src: MirAddr::Direct(MirMem::Static {
                        id: SymbolId(99),
                        offset: 0,
                    }),
                    width: MirWidth::Byte,
                }],
                MirTerminator::Return,
            )],
        )]);

        let diagnostics = verify_program(&program, MirPhase::PreMaterialization)
            .expect_err("unknown static rejected");
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.message.contains("static storage `s99`"))
        );
    }

    #[test]
    fn accepts_direct_absolute_address() {
        let mut main = routine(
            RoutineId(0),
            "Main",
            vec![block_with_ops(
                MirBlockId(0),
                "bb0",
                vec![
                    MirOp::Load {
                        dst: MirDef::Reg(crate::mir6502::MirReg::A),
                        src: MirAddr::Direct(MirMem::Absolute(0x3000)),
                        width: MirWidth::Byte,
                    },
                    MirOp::Move {
                        dst: MirDef::VTemp(MirTempId(0)),
                        src: MirValue::Def(MirDef::Reg(crate::mir6502::MirReg::A)),
                        width: MirWidth::Byte,
                    },
                ],
                MirTerminator::Return,
            )],
        );
        main.temps.push(MirTemp { id: MirTempId(0) });
        let program = program_with_routines(vec![main]);

        assert!(verify_program(&program, MirPhase::PreMaterialization).is_ok());
    }

    #[test]
    fn post_home_phase_rejects_surviving_virtual_temp() {
        let mut main = routine(
            RoutineId(0),
            "Main",
            vec![block_with_ops(
                MirBlockId(0),
                "bb0",
                vec![MirOp::LoadImm {
                    dst: MirDef::VTemp(MirTempId(0)),
                    value: 1,
                    width: MirWidth::Byte,
                }],
                MirTerminator::Return,
            )],
        );
        main.temps.push(MirTemp { id: MirTempId(0) });
        let program = program_with_routines(vec![main]);

        assert!(verify_program(&program, MirPhase::PostMaterialization).is_ok());
        let diagnostics = verify_program(&program, MirPhase::PostHome)
            .expect_err("post-home phase rejects virtual temps");
        assert!(diagnostics.iter().any(|diagnostic| {
            diagnostic
                .message
                .contains("post-home MIR cannot contain virtual temp `v0`")
        }));
    }

    #[test]
    fn rejects_deferred_runtime_helper_target_before_emission() {
        let mut program = program_with_routines(Vec::new());
        program.runtime_helpers.push(MirRuntimeHelperDecl {
            helper: MirRuntimeHelper::Mul,
            target: MirRuntimeHelperTarget::Deferred,
            abi: MirCallAbi {
                params: Vec::new(),
                result: None,
                clobbers: MirRegisterSet::default(),
                preserves: MirRegisterSet::default(),
            },
            effects: MirEffects::default(),
        });

        assert!(verify_program(&program, MirPhase::PreMaterialization).is_ok());
        let diagnostics = verify_program(&program, MirPhase::PreEmission)
            .expect_err("deferred helper target rejected before emission");
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.message.contains("deferred runtime helper"))
        );
    }

    #[test]
    fn rejects_unspecified_add_sub_carry_before_emission() {
        let program = program_with_routines(vec![routine(
            RoutineId(0),
            "Main",
            vec![block_with_ops(
                MirBlockId(0),
                "bb0",
                vec![MirOp::Binary {
                    op: MirBinaryOp::Add,
                    dst: MirDef::Reg(crate::mir6502::MirReg::A),
                    left: MirValue::Def(MirDef::Reg(crate::mir6502::MirReg::A)),
                    right: MirValue::ConstU8(1),
                    width: MirWidth::Byte,
                    carry_in: None,
                    carry_out: crate::mir6502::MirCarryOut::Ignore,
                }],
                MirTerminator::Return,
            )],
        )]);

        assert!(verify_program(&program, MirPhase::PreMaterialization).is_ok());
        let diagnostics = verify_program(&program, MirPhase::PreEmission)
            .expect_err("unspecified add/sub carry rejected before emission");
        assert!(diagnostics.iter().any(|diagnostic| {
            diagnostic
                .message
                .contains("pre-emission add/sub cannot have unspecified carry_in")
        }));
    }

    #[test]
    fn rejects_noncanonical_action_call_shadow_home() {
        let shadow = MirArgHome::FixedZeroPage(MirFixedZpSlot(0xa0));
        let primary = MirArgHome::Reg(MirReg::A);
        let call = MirOp::Call {
            target: MirCallTarget::Routine(RoutineId(1)),
            abi: MirCallAbi {
                params: vec![shadow.clone(), primary.clone()],
                result: None,
                clobbers: MirRegisterSet::default(),
                preserves: MirRegisterSet::default(),
            },
            args: vec![
                MirCallArg {
                    value: MirValue::ConstU8(0x12),
                    width: MirWidth::Byte,
                    home: shadow,
                },
                MirCallArg {
                    value: MirValue::ConstU8(0x12),
                    width: MirWidth::Byte,
                    home: primary,
                },
            ],
            result: None,
            effects: MirEffects::default(),
        };
        let program = program_with_routines(vec![
            routine(
                RoutineId(0),
                "Main",
                vec![block_with_ops(
                    MirBlockId(0),
                    "bb0",
                    vec![call],
                    MirTerminator::Return,
                )],
            ),
            routine(
                RoutineId(1),
                "Capture",
                vec![block(MirBlockId(0), "bb0", MirTerminator::Return)],
            ),
        ]);

        let diagnostics = verify_program(&program, MirPhase::PreMaterialization)
            .expect_err("caller-side A0 shadow rejected");
        assert!(diagnostics.iter().any(|diagnostic| {
            diagnostic.message.contains(
                "call argument 0 does not use the canonical Action ABI home at byte offset 0",
            )
        }));
    }

    #[test]
    fn rejects_scaled_y_access_without_matching_materialization() {
        let pair = MirPointerPair::Fixed {
            lo: crate::mir6502::MirFixedZpSlot(0xac),
        };
        let program = program_with_routines(vec![routine(
            RoutineId(0),
            "Main",
            vec![block_with_ops(
                MirBlockId(0),
                "bb0",
                vec![MirOp::LoadIndirect {
                    dst: MirDef::Reg(MirReg::A),
                    consumer: MirAddressConsumer::ScaledIndirectIndexedY(pair),
                    offset: 0,
                }],
                MirTerminator::Return,
            )],
        )]);

        let diagnostics = verify_program(&program, MirPhase::PreEmission)
            .expect_err("unprepared scaled-Y access rejected");
        assert!(diagnostics.iter().any(|diagnostic| {
            diagnostic
                .message
                .contains("scaled-Y access at op #0 has no active matching index")
        }));
    }

    #[test]
    fn rejects_scaled_y_access_that_moves_offset_backward() {
        let pair = MirPointerPair::Fixed {
            lo: crate::mir6502::MirFixedZpSlot(0xac),
        };
        let consumer = MirAddressConsumer::ScaledIndirectIndexedY(pair);
        let program = program_with_routines(vec![routine(
            RoutineId(0),
            "Main",
            vec![block_with_ops(
                MirBlockId(0),
                "bb0",
                vec![
                    MirOp::MaterializeIndexedAddress {
                        consumer,
                        base: MirValue::ConstU16(0x4000),
                        index: MirValue::ConstU8(3),
                        scale: 2,
                    },
                    MirOp::LoadIndirect {
                        dst: MirDef::Reg(MirReg::A),
                        consumer,
                        offset: 1,
                    },
                    MirOp::LoadIndirect {
                        dst: MirDef::Reg(MirReg::A),
                        consumer,
                        offset: 0,
                    },
                ],
                MirTerminator::Return,
            )],
        )]);

        let diagnostics = verify_program(&program, MirPhase::PreEmission)
            .expect_err("backward scaled-Y offset rejected");
        assert!(diagnostics.iter().any(|diagnostic| {
            diagnostic
                .message
                .contains("scaled-Y access at op #2 moves backward from offset 1 to 0")
        }));
    }

    fn program_with_routines(routines: Vec<MirRoutine>) -> MirProgram {
        MirProgram {
            statics: Vec::<MirStatic>::new(),
            globals: Vec::<MirGlobal>::new(),
            routines,
            machine_blocks: Vec::new(),
            runtime_helpers: Vec::<MirRuntimeHelperDecl>::new(),
        }
    }

    fn routine(id: RoutineId, name: &str, blocks: Vec<MirBlock>) -> MirRoutine {
        MirRoutine {
            id,
            name: name.to_string(),
            abi: MirRoutineAbi::Action,
            frame: MirFrame::default(),
            temps: Vec::new(),
            blocks,
            effects: MirEffects::default(),
        }
    }

    fn block(id: MirBlockId, label: &str, terminator: MirTerminator) -> MirBlock {
        block_with_ops(id, label, Vec::new(), terminator)
    }

    fn block_with_ops(
        id: MirBlockId,
        label: &str,
        ops: Vec<MirOp>,
        terminator: MirTerminator,
    ) -> MirBlock {
        MirBlock {
            id,
            label: label.to_string(),
            params: Vec::new(),
            ops,
            terminator,
        }
    }
}
