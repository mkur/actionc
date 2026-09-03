use std::collections::{BTreeMap, BTreeSet};

use super::cfg::NirCfg;
use super::dataflow::{NirDataflowDirection, NirDataflowProblem, solve_dataflow};
use crate::nir::facts::{NirStorageId, root_storage_id};
use crate::nir::{
    BlockId, NirGlobal, NirGlobalBacking, NirLocalBacking, NirMachineAtom, NirMachineItem,
    NirMemoryAccess, NirMemoryRegion, NirMemoryRegionKind, NirOp, NirPlace, NirProgram, NirRealOp,
    NirRealSource, NirRoutine, NirStorageClass, NirStorageDuration, NirStorageInit, NirType,
    NirTypeKind, RoutineId,
};
use crate::target::{ByteOffset, ByteSize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum NirStorageBackingClass {
    Ordinary,
    Absolute,
    Alias,
}

/// Domain in which a `NirStorageId` denotes one concrete object.
///
/// In particular, an invocation-relative local ID denotes a different object
/// in every live activation. This remains an abstract identity rule: it does
/// not select a stack, frame pointer, register, or static address.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum NirStorageIdentityDomain {
    Program,
    Routine(RoutineId),
    Invocation(RoutineId),
    External,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum NirPromotionBlocker {
    GlobalStorage,
    NonScalarStorage,
    UnsupportedType,
    AbsoluteBacking,
    AliasBacking,
    AliasedStorage,
    InitializedStorage,
    AddressTaken,
    AddressRequired,
    MachineVisibility,
    ReadBeforeDefinition,
    NoDirectAccess,
    AccessTypeMismatch,
}

impl NirPromotionBlocker {
    pub const ALL: [Self; 13] = [
        Self::GlobalStorage,
        Self::NonScalarStorage,
        Self::UnsupportedType,
        Self::AbsoluteBacking,
        Self::AliasBacking,
        Self::AliasedStorage,
        Self::InitializedStorage,
        Self::AddressTaken,
        Self::AddressRequired,
        Self::MachineVisibility,
        Self::ReadBeforeDefinition,
        Self::NoDirectAccess,
        Self::AccessTypeMismatch,
    ];

    pub const fn code(self) -> &'static str {
        match self {
            Self::GlobalStorage => "global_storage",
            Self::NonScalarStorage => "non_scalar_storage",
            Self::UnsupportedType => "unsupported_type",
            Self::AbsoluteBacking => "absolute_backing",
            Self::AliasBacking => "alias_backing",
            Self::AliasedStorage => "aliased_storage",
            Self::InitializedStorage => "initialized_storage",
            Self::AddressTaken => "address_taken",
            Self::AddressRequired => "address_required",
            Self::MachineVisibility => "machine_visibility",
            Self::ReadBeforeDefinition => "read_before_definition",
            Self::NoDirectAccess => "no_direct_access",
            Self::AccessTypeMismatch => "access_type_mismatch",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NirStorageFacts {
    pub id: NirStorageId,
    pub name: String,
    pub ty: Option<NirType>,
    pub width: Option<ByteSize>,
    pub direct_access_ty: Option<NirType>,
    pub storage_class: Option<NirStorageClass>,
    pub duration: Option<crate::nir::NirStorageDuration>,
    pub layout: Option<crate::nir::NirObjectLayout>,
    pub identity_domain: NirStorageIdentityDomain,
    /// Aliases share this target's object identity rather than creating a new
    /// object in `identity_domain`.
    pub alias_target: Option<NirStorageId>,
    pub backing: NirStorageBackingClass,
    pub load_blocks: BTreeSet<BlockId>,
    pub store_blocks: BTreeSet<BlockId>,
    pub direct_loads: usize,
    pub direct_stores: usize,
    pub address_taken: bool,
    pub address_required: bool,
    pub possible_read_before_definition: bool,
    pub value_needed_at_exit: bool,
    pub machine_visible: bool,
    pub calls_may_read: bool,
    pub calls_may_write: bool,
    pub blockers: BTreeSet<NirPromotionBlocker>,
}

impl NirStorageFacts {
    pub fn is_promotable(&self) -> bool {
        self.blockers.is_empty()
    }

    pub fn is_invocation_relative(&self) -> bool {
        matches!(
            self.identity_domain,
            NirStorageIdentityDomain::Invocation(_)
        )
    }

    /// Whether calls cannot name the current dynamic instance through any
    /// address-forming operation visible in verified NIR.
    ///
    /// This is deliberately a conservative escape proof. An automatic home
    /// stops being private as soon as its address is required, even when a
    /// more aggressive flow-sensitive analysis might prove that the address
    /// never leaves the routine.
    pub fn is_proven_private_to_invocation(&self) -> bool {
        self.is_invocation_relative() && !self.address_required
    }

    pub fn requires_addressable_home(&self) -> bool {
        self.address_required
    }

    /// Whether exact load values may be cached while effect barriers remain in
    /// place. This is intentionally broader than full home promotion:
    /// initialized, persistent, and global storage can still participate in
    /// load forwarding because stores are not removed.
    pub fn is_value_trackable(&self) -> bool {
        let pointer_cell = self.storage_class == Some(NirStorageClass::Array)
            && self.direct_access_ty.as_ref().is_some_and(|ty| {
                matches!(ty.kind, NirTypeKind::Pointer { .. }) && ty.width == Some(ByteSize::new(2))
            });
        self.blockers.iter().all(|blocker| {
            matches!(
                blocker,
                NirPromotionBlocker::GlobalStorage
                    | NirPromotionBlocker::InitializedStorage
                    | NirPromotionBlocker::ReadBeforeDefinition
                    | NirPromotionBlocker::NoDirectAccess
            ) || (*blocker == NirPromotionBlocker::NonScalarStorage && pointer_cell)
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NirRoutineStorageAnalysis {
    pub routine: String,
    pub homes: BTreeMap<NirStorageId, NirStorageFacts>,
}

impl NirRoutineStorageAnalysis {
    pub fn storage_by_name(&self, name: &str) -> Option<&NirStorageFacts> {
        self.homes.values().find(|facts| facts.name == name)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NirProgramStorageAnalysis {
    pub routines: Vec<NirRoutineStorageAnalysis>,
}

impl NirProgramStorageAnalysis {
    pub fn routine(&self, name: &str) -> Option<&NirRoutineStorageAnalysis> {
        self.routines.iter().find(|routine| routine.routine == name)
    }
}

pub fn analyze_program_storage(program: &NirProgram) -> NirProgramStorageAnalysis {
    let globals = program
        .globals
        .iter()
        .map(|global| (global.id, global))
        .collect::<BTreeMap<_, _>>();
    let global_names = program
        .globals
        .iter()
        .map(|global| (global.name.to_ascii_lowercase(), global.id))
        .collect::<BTreeMap<_, _>>();
    let global_data_address_taken = program_data_relocation_storage_targets(program)
        .into_iter()
        .filter(|id| matches!(id, NirStorageId::Global(_)))
        .collect::<BTreeSet<_>>();
    NirProgramStorageAnalysis {
        routines: program
            .routines
            .iter()
            .map(|routine| {
                let mut address_taken = global_data_address_taken.clone();
                for local in &routine.locals {
                    if let Some(init) = &local.init {
                        storage_init_relocation_targets(init, &mut address_taken);
                    }
                }
                analyze_routine_storage(routine, &globals, &global_names, &address_taken)
            })
            .collect(),
    }
}

fn analyze_routine_storage(
    routine: &NirRoutine,
    globals: &BTreeMap<crate::nir::SymbolId, &NirGlobal>,
    global_names: &BTreeMap<String, crate::nir::SymbolId>,
    data_address_taken: &BTreeSet<NirStorageId>,
) -> NirRoutineStorageAnalysis {
    let cfg = NirCfg::from_routine(routine);
    let mut homes = BTreeMap::new();

    for param in &routine.params {
        let id = NirStorageId::Param(param.id);
        homes.insert(
            id,
            new_facts(
                id,
                param.name.clone(),
                Some(param.ty.clone()),
                Some(param.storage),
                Some(param.duration),
                Some(param.layout),
                duration_identity_domain(param.duration, routine.id),
                None,
                NirStorageBackingClass::Ordinary,
                false,
            ),
        );
    }
    for local in &routine.locals {
        let id = NirStorageId::Local(local.id);
        let (backing, identity_domain, alias_target) = match local.backing {
            NirLocalBacking::Ordinary => (
                NirStorageBackingClass::Ordinary,
                duration_identity_domain(local.duration, routine.id),
                None,
            ),
            NirLocalBacking::Absolute(_) => (
                NirStorageBackingClass::Absolute,
                NirStorageIdentityDomain::External,
                None,
            ),
            NirLocalBacking::Alias { target, .. } => (
                NirStorageBackingClass::Alias,
                duration_identity_domain(local.duration, routine.id),
                Some(NirStorageId::Local(target)),
            ),
            NirLocalBacking::GlobalAlias { target, .. } => (
                NirStorageBackingClass::Alias,
                NirStorageIdentityDomain::External,
                Some(NirStorageId::Global(target)),
            ),
        };
        homes.insert(
            id,
            new_facts(
                id,
                local.name.clone(),
                Some(local.ty.clone()),
                Some(local.storage),
                Some(local.duration),
                Some(local.layout),
                identity_domain,
                alias_target,
                backing,
                local.init.is_some(),
            ),
        );
    }
    for local in &routine.locals {
        if let NirLocalBacking::Alias { target, .. } = local.backing
            && let Some(target) = homes.get_mut(&NirStorageId::Local(target))
        {
            target.blockers.insert(NirPromotionBlocker::AliasedStorage);
            target.address_required = true;
        }
    }

    // Globals are routine facts only when the routine names them directly (or
    // a machine item names them). This avoids multiplying every program global
    // by every routine while retaining exact identities for effect analysis.
    let mut referenced_globals = BTreeSet::new();
    referenced_globals.extend(data_address_taken.iter().filter_map(|id| match id {
        NirStorageId::Global(id) => Some(*id),
        NirStorageId::Local(_) | NirStorageId::Param(_) => None,
    }));
    let mut has_conservative_escape_barrier = false;
    for block in &routine.blocks {
        if !cfg.reachable().contains(&block.id) {
            continue;
        }
        for op in &block.ops {
            for_each_op_place(op, |place| {
                if let Some(NirStorageId::Global(id)) = root_storage_id(place) {
                    referenced_globals.insert(id);
                }
            });
            if let NirOp::ForeignCode { code, .. } = op {
                match &code.payload {
                    crate::nir::NirForeignCodePayload::Structured(items) => {
                        for name in machine_item_names(items) {
                            if let Some(id) = global_names.get(&name.to_ascii_lowercase()) {
                                referenced_globals.insert(*id);
                            }
                        }
                        for item in items {
                            if let NirMachineItem::Relocation {
                                target:
                                    crate::nir::NirForeignCodeTarget::Storage(NirStorageId::Global(id)),
                                ..
                            } = item
                            {
                                referenced_globals.insert(*id);
                            }
                        }
                    }
                    crate::nir::NirForeignCodePayload::Bytes { relocations, .. } => {
                        for relocation in relocations {
                            if let crate::nir::NirForeignCodeTarget::Storage(
                                NirStorageId::Global(id),
                            ) = relocation.target
                            {
                                referenced_globals.insert(id);
                            }
                        }
                    }
                }
            }
        }
    }
    for id in referenced_globals {
        if let Some(global) = globals.get(&id) {
            homes.insert(NirStorageId::Global(id), global_facts(global));
        }
    }
    for local in &routine.locals {
        if let NirLocalBacking::GlobalAlias { target, .. } = local.backing
            && let Some(target) = homes.get_mut(&NirStorageId::Global(target))
        {
            target.blockers.insert(NirPromotionBlocker::AliasedStorage);
            target.address_required = true;
        }
    }
    for global in globals.values() {
        if let NirGlobalBacking::Alias { target, .. } = &global.backing
            && let Some(target) = homes.get_mut(&NirStorageId::Global(*target))
        {
            target.blockers.insert(NirPromotionBlocker::AliasedStorage);
            target.address_required = true;
        }
    }
    for id in data_address_taken {
        if let Some(facts) = homes.get_mut(id) {
            facts.address_taken = true;
            facts.address_required = true;
        }
    }

    let names = homes
        .values()
        .map(|facts| (facts.name.to_ascii_lowercase(), facts.id))
        .collect::<BTreeMap<_, _>>();
    for block in &routine.blocks {
        if !cfg.reachable().contains(&block.id) {
            continue;
        }
        for op in &block.ops {
            match op {
                NirOp::Load { ty, place, .. } => {
                    record_direct_access(&mut homes, block.id, place, ty, true);
                }
                NirOp::VolatileLoad { ty, place, .. } => {
                    record_direct_access(&mut homes, block.id, place, ty, true);
                    mark_volatile_access(&mut homes, place);
                }
                NirOp::Store { place, ty, .. } => {
                    record_direct_access(&mut homes, block.id, place, ty, false);
                }
                NirOp::VolatileStore { place, ty, .. } => {
                    record_direct_access(&mut homes, block.id, place, ty, false);
                    mark_volatile_access(&mut homes, place);
                }
                NirOp::CopyBytes {
                    destination,
                    source,
                    destination_volatile,
                    source_volatile,
                    ..
                } => {
                    if let Some(ty) = &source.ty {
                        record_direct_access(&mut homes, block.id, source, ty, true);
                    }
                    if let Some(ty) = &destination.ty {
                        record_direct_access(&mut homes, block.id, destination, ty, false);
                    }
                    mark_address_required(&mut homes, source, false);
                    mark_address_required(&mut homes, destination, false);
                    if *source_volatile {
                        mark_volatile_access(&mut homes, source);
                    }
                    if *destination_volatile {
                        mark_volatile_access(&mut homes, destination);
                    }
                }
                NirOp::AddrOf { place, .. } => {
                    mark_address_required(&mut homes, place, true);
                }
                NirOp::Call {
                    callee, effects, ..
                } => {
                    has_conservative_escape_barrier |= effects.opaque
                        || effects.may_call_external
                        || matches!(callee, crate::nir::NirCallee::Indirect { .. });
                    for facts in homes.values_mut() {
                        facts.calls_may_read |=
                            call_memory_accesses_storage(&effects.memory.reads, facts);
                        facts.calls_may_write |=
                            call_memory_accesses_storage(&effects.memory.writes, facts);
                    }
                }
                NirOp::ForeignCode { code, effects } => {
                    has_conservative_escape_barrier |= effects.opaque || effects.may_call_external;
                    if effects.opaque {
                        for facts in homes.values_mut() {
                            facts.machine_visible = true;
                            facts.address_required = true;
                        }
                    } else {
                        match &code.payload {
                            crate::nir::NirForeignCodePayload::Structured(items) => {
                                for name in machine_item_names(items) {
                                    if let Some(id) = names.get(&name.to_ascii_lowercase())
                                        && let Some(facts) = homes.get_mut(id)
                                    {
                                        facts.machine_visible = true;
                                        facts.address_required = true;
                                    }
                                }
                                for item in items {
                                    if let NirMachineItem::Relocation {
                                        target: crate::nir::NirForeignCodeTarget::Storage(id),
                                        ..
                                    } = item
                                        && let Some(facts) = homes.get_mut(id)
                                    {
                                        facts.machine_visible = true;
                                        facts.address_required = true;
                                    }
                                }
                            }
                            crate::nir::NirForeignCodePayload::Bytes { relocations, .. } => {
                                for relocation in relocations {
                                    if let crate::nir::NirForeignCodeTarget::Storage(id) =
                                        relocation.target
                                        && let Some(facts) = homes.get_mut(&id)
                                    {
                                        facts.machine_visible = true;
                                        facts.address_required = true;
                                        facts.calls_may_read |= memory_accesses_storage(
                                            &effects.memory.reads,
                                            facts.id,
                                            facts.width,
                                        );
                                        facts.calls_may_write |= memory_accesses_storage(
                                            &effects.memory.writes,
                                            facts.id,
                                            facts.width,
                                        );
                                    }
                                }
                            }
                        }
                    }
                }
                NirOp::Real(_) => {
                    for facts in homes.values_mut() {
                        facts.machine_visible = true;
                        facts.address_required = true;
                        facts.calls_may_read = true;
                        facts.calls_may_write = true;
                    }
                }
                NirOp::Unary { .. }
                | NirOp::Cast { .. }
                | NirOp::PointerOffset { .. }
                | NirOp::Binary { .. }
                | NirOp::Compare { .. }
                | NirOp::Unsupported { .. } => {}
            }
        }
    }

    mark_read_before_definition(routine, &cfg, &mut homes);
    if has_conservative_escape_barrier {
        for facts in homes.values_mut() {
            if facts.is_invocation_relative() && facts.address_required {
                facts.calls_may_read = true;
                facts.calls_may_write = true;
            }
        }
    }
    for facts in homes.values_mut() {
        facts.value_needed_at_exit = match (facts.id, facts.identity_domain) {
            // An automatic object's lifetime ends with this activation. Its
            // final value cannot seed a later recursive or repeated call.
            (NirStorageId::Local(_), NirStorageIdentityDomain::Invocation(_)) => false,
            (NirStorageId::Local(_), _) => {
                facts.direct_stores != 0 && facts.possible_read_before_definition
            }
            (NirStorageId::Param(_), NirStorageIdentityDomain::Invocation(_)) => false,
            (NirStorageId::Param(_) | NirStorageId::Global(_), _) => facts.direct_stores != 0,
        };
        if facts.address_taken {
            facts.blockers.insert(NirPromotionBlocker::AddressTaken);
        }
        if facts.address_required {
            facts.blockers.insert(NirPromotionBlocker::AddressRequired);
        }
        if facts.machine_visible {
            facts
                .blockers
                .insert(NirPromotionBlocker::MachineVisibility);
        }
        if facts.possible_read_before_definition {
            facts
                .blockers
                .insert(NirPromotionBlocker::ReadBeforeDefinition);
        }
        if facts.direct_loads == 0 && facts.direct_stores == 0 {
            facts.blockers.insert(NirPromotionBlocker::NoDirectAccess);
        }
    }

    NirRoutineStorageAnalysis {
        routine: routine.name.clone(),
        homes,
    }
}

fn program_data_relocation_storage_targets(program: &NirProgram) -> BTreeSet<NirStorageId> {
    let mut targets = BTreeSet::new();
    for global in &program.globals {
        if let Some(init) = &global.init {
            match init {
                crate::nir::NirGlobalInit::Bytes { image, .. } => {
                    data_image_storage_targets(image, &mut targets)
                }
                crate::nir::NirGlobalInit::Descriptor { backing, .. } => {
                    data_image_storage_targets(&backing.image, &mut targets)
                }
                crate::nir::NirGlobalInit::ZeroFill { .. }
                | crate::nir::NirGlobalInit::LinkValue { .. }
                | crate::nir::NirGlobalInit::RoutineAddress { .. } => {}
            }
        }
    }
    for static_data in &program.statics {
        data_image_storage_targets(&static_data.image, &mut targets);
    }
    for routine in &program.routines {
        for local in &routine.locals {
            if let Some(init) = &local.init {
                storage_init_relocation_targets(init, &mut targets);
            }
        }
    }
    targets
}

fn storage_init_relocation_targets(init: &NirStorageInit, targets: &mut BTreeSet<NirStorageId>) {
    match init {
        NirStorageInit::Bytes { image, .. } => data_image_storage_targets(image, targets),
        NirStorageInit::Descriptor { backing, .. } => {
            data_image_storage_targets(&backing.image, targets)
        }
        NirStorageInit::ZeroFill { .. } => {}
    }
}

fn data_image_storage_targets(
    image: &crate::nir::NirDataImage,
    targets: &mut BTreeSet<NirStorageId>,
) {
    targets.extend(
        image
            .fragments
            .iter()
            .filter_map(|fragment| match fragment {
                crate::nir::NirDataFragment::Address {
                    target: crate::nir::NirDataAddressTarget::Storage(id),
                    ..
                } => Some(*id),
                _ => None,
            }),
    );
}

fn new_facts(
    id: NirStorageId,
    name: String,
    ty: Option<NirType>,
    storage_class: Option<NirStorageClass>,
    duration: Option<crate::nir::NirStorageDuration>,
    layout: Option<crate::nir::NirObjectLayout>,
    identity_domain: NirStorageIdentityDomain,
    alias_target: Option<NirStorageId>,
    backing: NirStorageBackingClass,
    initialized: bool,
) -> NirStorageFacts {
    let width = ty.as_ref().and_then(|ty| ty.width);
    let mut blockers = BTreeSet::new();
    if storage_class != Some(NirStorageClass::Scalar) {
        blockers.insert(NirPromotionBlocker::NonScalarStorage);
    }
    if !ty.as_ref().is_some_and(supported_scalar_type) {
        blockers.insert(NirPromotionBlocker::UnsupportedType);
    }
    match backing {
        NirStorageBackingClass::Ordinary => {}
        NirStorageBackingClass::Absolute => {
            blockers.insert(NirPromotionBlocker::AbsoluteBacking);
        }
        NirStorageBackingClass::Alias => {
            blockers.insert(NirPromotionBlocker::AliasBacking);
        }
    }
    if initialized {
        blockers.insert(NirPromotionBlocker::InitializedStorage);
    }
    NirStorageFacts {
        id,
        name,
        ty,
        width,
        direct_access_ty: None,
        storage_class,
        duration,
        layout,
        identity_domain,
        alias_target,
        backing,
        load_blocks: BTreeSet::new(),
        store_blocks: BTreeSet::new(),
        direct_loads: 0,
        direct_stores: 0,
        address_taken: false,
        address_required: false,
        possible_read_before_definition: false,
        value_needed_at_exit: false,
        machine_visible: false,
        calls_may_read: false,
        calls_may_write: false,
        blockers,
    }
}

fn global_facts(global: &NirGlobal) -> NirStorageFacts {
    let storage_class = if global.array.is_some() {
        Some(NirStorageClass::Array)
    } else if global
        .ty
        .as_ref()
        .is_some_and(|ty| matches!(ty.kind, NirTypeKind::Record { .. }))
    {
        Some(NirStorageClass::Record)
    } else {
        global.ty.as_ref().map(|_| NirStorageClass::Scalar)
    };
    let backing = match global.backing {
        NirGlobalBacking::Ordinary => NirStorageBackingClass::Ordinary,
        NirGlobalBacking::Absolute(_) => NirStorageBackingClass::Absolute,
        NirGlobalBacking::Alias { .. } => NirStorageBackingClass::Alias,
    };
    let (identity_domain, alias_target) = match global.backing {
        NirGlobalBacking::Ordinary => (NirStorageIdentityDomain::Program, None),
        NirGlobalBacking::Absolute(_) => (NirStorageIdentityDomain::External, None),
        NirGlobalBacking::Alias { target, .. } => (
            NirStorageIdentityDomain::Program,
            Some(NirStorageId::Global(target)),
        ),
    };
    let mut facts = new_facts(
        NirStorageId::Global(global.id),
        global.name.clone(),
        global.ty.clone(),
        storage_class,
        None,
        None,
        identity_domain,
        alias_target,
        backing,
        global.init.is_some(),
    );
    facts.blockers.insert(NirPromotionBlocker::GlobalStorage);
    facts
}

fn duration_identity_domain(
    duration: NirStorageDuration,
    routine: RoutineId,
) -> NirStorageIdentityDomain {
    match duration {
        NirStorageDuration::Automatic => NirStorageIdentityDomain::Invocation(routine),
        NirStorageDuration::RoutineStatic => NirStorageIdentityDomain::Routine(routine),
        NirStorageDuration::External => NirStorageIdentityDomain::External,
    }
}

fn supported_scalar_type(ty: &NirType) -> bool {
    matches!(
        ty.kind,
        NirTypeKind::Bool
            | NirTypeKind::U8
            | NirTypeKind::I8
            | NirTypeKind::U16
            | NirTypeKind::I16
            | NirTypeKind::Pointer { .. }
            | NirTypeKind::Callable { .. }
    ) && matches!(ty.width.map(ByteSize::get), Some(1 | 2))
}

fn record_direct_access(
    homes: &mut BTreeMap<NirStorageId, NirStorageFacts>,
    block: BlockId,
    place: &NirPlace,
    access_ty: &NirType,
    load: bool,
) {
    let Some(id) = crate::nir::direct_storage_id(place) else {
        return;
    };
    let Some(facts) = homes.get_mut(&id) else {
        return;
    };
    if load {
        facts.direct_loads = facts.direct_loads.saturating_add(1);
        facts.load_blocks.insert(block);
    } else {
        facts.direct_stores = facts.direct_stores.saturating_add(1);
        facts.store_blocks.insert(block);
    }
    if let Some(direct_ty) = &facts.direct_access_ty {
        if !same_type(direct_ty, access_ty) {
            facts
                .blockers
                .insert(NirPromotionBlocker::AccessTypeMismatch);
        }
    } else {
        facts.direct_access_ty = Some(access_ty.clone());
    }
    let place_matches = place.ty.as_ref().is_some_and(|ty| same_type(ty, access_ty));
    let home_matches = facts.storage_class == Some(NirStorageClass::Array)
        && matches!(access_ty.kind, NirTypeKind::Pointer { .. })
        || facts.ty.as_ref().is_some_and(|ty| same_type(ty, access_ty));
    if !place_matches || !home_matches {
        facts
            .blockers
            .insert(NirPromotionBlocker::AccessTypeMismatch);
    }
}

fn same_type(left: &NirType, right: &NirType) -> bool {
    left.kind == right.kind && left.width == right.width
}

fn memory_accesses_storage(
    access: &NirMemoryAccess,
    storage: NirStorageId,
    width: Option<ByteSize>,
) -> bool {
    match access {
        NirMemoryAccess::None => false,
        NirMemoryAccess::Regions(regions) => {
            let Some(width) = width else {
                return true;
            };
            let storage = NirMemoryRegion {
                kind: NirMemoryRegionKind::Storage(storage),
                offset: ByteOffset::ZERO,
                size: width,
            };
            regions.iter().any(|region| region.overlaps(&storage))
        }
        NirMemoryAccess::Unknown | NirMemoryAccess::All => true,
    }
}

fn call_memory_accesses_storage(access: &NirMemoryAccess, facts: &NirStorageFacts) -> bool {
    match access {
        // Unknown callees cannot name a private instance in their caller's
        // activation. A required address removes this exemption, and explicit
        // regions always remain authoritative.
        NirMemoryAccess::Unknown | NirMemoryAccess::All
            if facts.is_invocation_relative() && !facts.address_required =>
        {
            false
        }
        _ => memory_accesses_storage(access, facts.id, facts.width),
    }
}

fn mark_volatile_access(homes: &mut BTreeMap<NirStorageId, NirStorageFacts>, place: &NirPlace) {
    if let Some(id) = root_storage_id(place)
        && let Some(facts) = homes.get_mut(&id)
    {
        // Reuse the existing conservative promotion boundary: volatile homes
        // must remain observable memory just like machine-visible storage.
        facts.machine_visible = true;
        facts.address_required = true;
    }
}

fn mark_address_required(
    homes: &mut BTreeMap<NirStorageId, NirStorageFacts>,
    place: &NirPlace,
    address_taken: bool,
) {
    if let Some(id) = root_storage_id(place)
        && let Some(facts) = homes.get_mut(&id)
    {
        facts.address_required = true;
        facts.address_taken |= address_taken;
    }
}

fn for_each_op_place(op: &NirOp, mut visit: impl FnMut(&NirPlace)) {
    match op {
        NirOp::Load { place, .. }
        | NirOp::VolatileLoad { place, .. }
        | NirOp::AddrOf { place, .. }
        | NirOp::Store { place, .. }
        | NirOp::VolatileStore { place, .. } => visit(place),
        NirOp::CopyBytes {
            destination,
            source,
            ..
        } => {
            visit(destination);
            visit(source);
        }
        NirOp::Real(real) => for_each_real_place(real, visit),
        NirOp::Unary { .. }
        | NirOp::Cast { .. }
        | NirOp::PointerOffset { .. }
        | NirOp::Binary { .. }
        | NirOp::Compare { .. }
        | NirOp::Call { .. }
        | NirOp::ForeignCode { .. }
        | NirOp::Unsupported { .. } => {}
    }
}

fn machine_item_names(items: &[NirMachineItem]) -> BTreeSet<String> {
    items
        .iter()
        .filter_map(|item| match item {
            NirMachineItem::Name(name) | NirMachineItem::AddressByte { name, .. } => {
                Some(name.clone())
            }
            NirMachineItem::AddressExpr {
                atom: NirMachineAtom::Name(name),
                ..
            } => Some(name.clone()),
            NirMachineItem::Byte(_)
            | NirMachineItem::Word(_)
            | NirMachineItem::StringLiteral(_)
            | NirMachineItem::CharLiteral(_)
            | NirMachineItem::AddressExpr { .. }
            | NirMachineItem::Relocation { .. } => None,
        })
        .collect()
}

struct DefinitelyDefined<'a> {
    routine: &'a NirRoutine,
    entry: Option<BlockId>,
    boundary: BTreeSet<NirStorageId>,
}

impl NirDataflowProblem for DefinitelyDefined<'_> {
    type State = Option<BTreeSet<NirStorageId>>;

    fn direction(&self) -> NirDataflowDirection {
        NirDataflowDirection::Forward
    }

    fn bottom(&self) -> Self::State {
        None
    }

    fn boundary(&self, block: BlockId) -> Option<Self::State> {
        (Some(block) == self.entry).then(|| Some(self.boundary.clone()))
    }

    fn join(&self, into: &mut Self::State, other: &Self::State) {
        let Some(other) = other else {
            return;
        };
        if let Some(into) = into {
            into.retain(|id| other.contains(id));
        } else {
            *into = Some(other.clone());
        }
    }

    fn transfer(&self, block: BlockId, state: &Self::State) -> Self::State {
        let mut state = state.clone()?;
        let Some(block) = self.routine.blocks.iter().find(|item| item.id == block) else {
            return Some(state);
        };
        for op in &block.ops {
            if let NirOp::Store { place, .. } | NirOp::VolatileStore { place, .. } = op
                && let Some(id) = crate::nir::direct_storage_id(place)
            {
                state.insert(id);
            }
        }
        Some(state)
    }
}

fn mark_read_before_definition(
    routine: &NirRoutine,
    cfg: &NirCfg,
    homes: &mut BTreeMap<NirStorageId, NirStorageFacts>,
) {
    let boundary = homes
        .keys()
        .copied()
        .filter(|id| matches!(id, NirStorageId::Param(_) | NirStorageId::Global(_)))
        .collect();
    let result = solve_dataflow(
        cfg,
        &DefinitelyDefined {
            routine,
            entry: cfg.entry(),
            boundary,
        },
    );
    for block in &routine.blocks {
        let Some(Some(mut defined)) = result.in_state(block.id).cloned() else {
            continue;
        };
        for op in &block.ops {
            match op {
                NirOp::Load { place, .. } | NirOp::VolatileLoad { place, .. } => {
                    let Some(id @ NirStorageId::Local(_)) = crate::nir::direct_storage_id(place)
                    else {
                        continue;
                    };
                    if !defined.contains(&id)
                        && let Some(facts) = homes.get_mut(&id)
                    {
                        facts.possible_read_before_definition = true;
                    }
                }
                NirOp::Store { place, .. } | NirOp::VolatileStore { place, .. } => {
                    if let Some(id) = crate::nir::direct_storage_id(place) {
                        defined.insert(id);
                    }
                }
                NirOp::CopyBytes {
                    destination,
                    source,
                    ..
                } => {
                    if let Some(id @ NirStorageId::Local(_)) = crate::nir::direct_storage_id(source)
                        && !defined.contains(&id)
                        && let Some(facts) = homes.get_mut(&id)
                    {
                        facts.possible_read_before_definition = true;
                    }
                    if let Some(id) = crate::nir::direct_storage_id(destination) {
                        defined.insert(id);
                    }
                }
                NirOp::Real(real) => {
                    for destination in real_destinations(real) {
                        if let Some(id) = crate::nir::direct_storage_id(destination) {
                            defined.insert(id);
                        }
                    }
                }
                NirOp::AddrOf { .. }
                | NirOp::Unary { .. }
                | NirOp::Cast { .. }
                | NirOp::PointerOffset { .. }
                | NirOp::Binary { .. }
                | NirOp::Compare { .. }
                | NirOp::Call { .. }
                | NirOp::ForeignCode { .. }
                | NirOp::Unsupported { .. } => {}
            }
        }
    }
}

fn for_each_real_place(op: &NirRealOp, mut visit: impl FnMut(&NirPlace)) {
    match op {
        NirRealOp::Copy {
            destination,
            source,
        } => {
            visit(destination);
            if let NirRealSource::Place(source) = source {
                visit(source);
            }
        }
        NirRealOp::Unary {
            destination,
            operand,
            ..
        } => {
            visit(destination);
            if let NirRealSource::Place(operand) = operand {
                visit(operand);
            }
        }
        NirRealOp::Binary {
            destination,
            left,
            right,
            ..
        } => {
            visit(destination);
            if let NirRealSource::Place(left) = left {
                visit(left);
            }
            if let NirRealSource::Place(right) = right {
                visit(right);
            }
        }
        NirRealOp::Compare { left, right, .. } => {
            if let NirRealSource::Place(left) = left {
                visit(left);
            }
            if let NirRealSource::Place(right) = right {
                visit(right);
            }
        }
        NirRealOp::IntegerToReal { destination, .. } => visit(destination),
        NirRealOp::RealToInteger { source, .. } => visit(source),
    }
}

fn real_destinations(op: &NirRealOp) -> impl Iterator<Item = &NirPlace> {
    match op {
        NirRealOp::Copy { destination, .. }
        | NirRealOp::Unary { destination, .. }
        | NirRealOp::Binary { destination, .. }
        | NirRealOp::IntegerToReal { destination, .. } => Some(destination),
        NirRealOp::Compare { .. } | NirRealOp::RealToInteger { .. } => None,
    }
    .into_iter()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::foreign::{ForeignRelocationEncoding, ForeignSymbolUse};
    use crate::nir::{
        LocalId, NirBlock, NirCallConvention, NirCallEffects, NirCallee, NirForeignCode,
        NirForeignCodeKind, NirForeignCodePayload, NirForeignRelocation, NirLocal, NirLocalPurpose,
        NirMachineEffects, NirMemoryEffects, NirParam, NirPlaceKind, NirStorageDuration,
        NirStorageInit, NirTerminator, NirValue, ParamId, TempId,
    };

    fn byte_type() -> NirType {
        NirType {
            kind: NirTypeKind::U8,
            summary: "Byte".to_string(),
            width: Some(crate::target::ByteSize::ONE),
            pointer: false,
        }
    }

    fn local(id: u32, name: &str) -> NirLocal {
        NirLocal {
            id: LocalId(id),
            name: name.to_string(),
            kind: "Byte".to_string(),
            purpose: NirLocalPurpose::Storage,
            storage: NirStorageClass::Scalar,
            duration: crate::nir::NirStorageDuration::RoutineStatic,
            layout: crate::nir::NirObjectLayout::byte(),
            ty: byte_type(),
            backing: NirLocalBacking::Ordinary,
            init: None,
        }
    }

    fn automatic_local(id: u32, name: &str) -> NirLocal {
        NirLocal {
            duration: NirStorageDuration::Automatic,
            ..local(id, name)
        }
    }

    fn local_place(id: u32, name: &str) -> NirPlace {
        NirPlace {
            kind: NirPlaceKind::Local {
                id: LocalId(id),
                name: name.to_string(),
            },
            ty: Some(byte_type()),
        }
    }

    fn param_place(id: u32, name: &str) -> NirPlace {
        NirPlace {
            kind: NirPlaceKind::Param {
                id: ParamId(id),
                name: name.to_string(),
            },
            ty: Some(byte_type()),
        }
    }

    fn block(id: u32, label: &str, ops: Vec<NirOp>, terminator: NirTerminator) -> NirBlock {
        NirBlock {
            id: BlockId(id),
            label: label.to_string(),
            params: Vec::new(),
            ops,
            terminator,
        }
    }

    fn edge(target: u32) -> crate::nir::NirEdge {
        crate::nir::NirEdge {
            target: BlockId(target),
            args: Vec::new(),
        }
    }

    fn program(routine: NirRoutine) -> NirProgram {
        NirProgram {
            target_layout: crate::target::TargetLayout::atari_6502(),
            runtime_bindings: Vec::new(),
            globals: Vec::new(),
            statics: Vec::new(),
            routines: vec![routine],
        }
    }

    fn native_program(routine: NirRoutine) -> NirProgram {
        NirProgram {
            target_layout: crate::target::TargetLayout::motorola_68000(),
            runtime_bindings: Vec::new(),
            globals: Vec::new(),
            statics: Vec::new(),
            routines: vec![routine],
        }
    }

    #[test]
    fn automatic_storage_identity_is_scoped_to_an_invocation() {
        let mut alias = automatic_local(1, "alias");
        alias.backing = NirLocalBacking::Alias {
            target: LocalId(0),
            target_name: "value".to_string(),
            offset: ByteOffset::ZERO,
        };
        let routine = NirRoutine {
            id: RoutineId(7),
            signature: crate::nir::NirCallableSignature::default(),
            convention: NirCallConvention::TargetPublic,
            activation: crate::nir::NirActivationModel::NativeReentrant,
            entry: crate::nir::NirRoutineEntry::default(),
            name: "Recursive".to_string(),
            params: Vec::new(),
            locals: vec![
                automatic_local(0, "value"),
                alias,
                automatic_local(2, "private"),
            ],
            temps: Vec::new(),
            notes: Vec::new(),
            blocks: vec![block(0, "entry", Vec::new(), NirTerminator::Return(None))],
        };

        let analysis = analyze_program_storage(&native_program(routine));
        let routine = analysis.routine("Recursive").expect("routine storage");
        let value = routine.storage_by_name("value").expect("value facts");
        let alias = routine.storage_by_name("alias").expect("alias facts");
        let private = routine.storage_by_name("private").expect("private facts");
        assert_eq!(
            value.identity_domain,
            NirStorageIdentityDomain::Invocation(RoutineId(7))
        );
        assert!(value.is_invocation_relative());
        assert_eq!(alias.identity_domain, value.identity_domain);
        assert_eq!(alias.alias_target, Some(NirStorageId::Local(LocalId(0))));
        assert!(!value.is_proven_private_to_invocation());
        assert!(alias.is_proven_private_to_invocation());
        assert!(private.is_proven_private_to_invocation());
    }

    #[test]
    fn address_operations_and_foreign_metadata_keep_automatic_homes_addressable() {
        let pointer_ty = NirType {
            kind: NirTypeKind::Pointer {
                pointee: Some(Box::new(NirTypeKind::U8)),
                address_space: crate::target::TargetLayout::DATA_ADDRESS_SPACE,
            },
            summary: "Byte*".to_string(),
            width: Some(ByteSize::new(4)),
            pointer: true,
        };
        let code = NirForeignCode {
            target: crate::target::TargetId::Motorola68000,
            kind: NirForeignCodeKind::InlineAssembly,
            payload: NirForeignCodePayload::Bytes {
                bytes: vec![0; 4],
                relocations: vec![NirForeignRelocation {
                    offset: ByteOffset::ZERO,
                    encoding: ForeignRelocationEncoding::Address {
                        width: ByteSize::new(4),
                    },
                    target: crate::nir::NirForeignCodeTarget::Storage(NirStorageId::Local(
                        LocalId(2),
                    )),
                    addend: 0,
                    required_address_bits: Some(32),
                    symbol_use: ForeignSymbolUse::Address,
                    span: crate::source::Span::new(0, 0),
                }],
            },
            source: String::new(),
            span: crate::source::Span::new(0, 0),
        };
        let routine = NirRoutine {
            id: RoutineId(0),
            signature: crate::nir::NirCallableSignature::default(),
            convention: NirCallConvention::TargetPublic,
            activation: crate::nir::NirActivationModel::NativeReentrant,
            entry: crate::nir::NirRoutineEntry::default(),
            name: "Addressable".to_string(),
            params: Vec::new(),
            locals: (0..5)
                .map(|id| automatic_local(id, &format!("v{id}")))
                .collect(),
            temps: Vec::new(),
            notes: Vec::new(),
            blocks: vec![block(
                0,
                "entry",
                vec![
                    NirOp::CopyBytes {
                        destination: local_place(4, "v4"),
                        source: local_place(0, "v0"),
                        size: ByteSize::ONE,
                        destination_volatile: false,
                        source_volatile: false,
                    },
                    NirOp::VolatileLoad {
                        dest: TempId(0),
                        ty: byte_type(),
                        place: local_place(1, "v1"),
                    },
                    NirOp::ForeignCode {
                        code,
                        effects: NirMachineEffects {
                            memory: NirMemoryEffects {
                                reads: NirMemoryAccess::None,
                                writes: NirMemoryAccess::None,
                            },
                            may_call_external: true,
                            opaque: false,
                        },
                    },
                    NirOp::AddrOf {
                        dest: TempId(1),
                        ty: pointer_ty,
                        place: local_place(3, "v3"),
                    },
                    NirOp::Call {
                        callee: NirCallee::Builtin("External".to_string()),
                        args: Vec::new(),
                        result: None,
                        signature: Some(crate::nir::NirCallableSignature::empty_proc(
                            NirCallConvention::Runtime,
                        )),
                        effects: NirCallEffects {
                            memory: NirMemoryEffects {
                                reads: NirMemoryAccess::None,
                                writes: NirMemoryAccess::None,
                            },
                            may_call_external: true,
                            opaque: false,
                        },
                    },
                ],
                NirTerminator::Return(None),
            )],
        };

        let analysis = analyze_program_storage(&native_program(routine));
        let routine = analysis.routine("Addressable").expect("routine storage");
        for name in ["v0", "v1", "v2", "v3", "v4"] {
            let facts = routine.storage_by_name(name).expect("local facts");
            assert!(facts.requires_addressable_home(), "{name}");
            assert!(
                facts
                    .blockers
                    .contains(&NirPromotionBlocker::AddressRequired),
                "{name}"
            );
            assert!(facts.calls_may_read && facts.calls_may_write, "{name}");
        }
        assert!(routine.storage_by_name("v2").unwrap().machine_visible);
        assert!(routine.storage_by_name("v3").unwrap().address_taken);
    }

    #[test]
    fn classifies_narrow_scalar_candidates_and_exclusion_reasons() {
        let mut initialized = local(2, "initialized");
        initialized.init = Some(NirStorageInit::ZeroFill {
            bytes: ByteSize::ONE,
            mutable: true,
            section: "data".to_string(),
        });
        let mut absolute = local(3, "absolute");
        absolute.backing = NirLocalBacking::Absolute(crate::target::AddressValue::data(0xD000));
        let mut alias = local(4, "alias");
        alias.backing = NirLocalBacking::Alias {
            target: LocalId(7),
            target_name: "alias_target".to_string(),
            offset: ByteOffset::ZERO,
        };
        let mut array = local(5, "array");
        array.storage = NirStorageClass::Array;
        let routine = NirRoutine {
            id: crate::nir::RoutineId(0),
            signature: crate::nir::NirCallableSignature::default(),
            convention: crate::nir::NirCallConvention::TargetPublic,
            activation: crate::nir::NirActivationModel::ClassicStatic,
            entry: crate::nir::NirRoutineEntry::default(),
            name: "Main".to_string(),
            params: vec![
                NirParam {
                    id: ParamId(0),
                    name: "value".to_string(),
                    storage: NirStorageClass::Scalar,
                    duration: crate::nir::NirStorageDuration::RoutineStatic,
                    layout: crate::nir::NirObjectLayout::byte(),
                    ty: byte_type(),
                },
                NirParam {
                    id: ParamId(1),
                    name: "items".to_string(),
                    storage: NirStorageClass::Array,
                    duration: crate::nir::NirStorageDuration::RoutineStatic,
                    layout: crate::nir::NirObjectLayout::byte(),
                    ty: byte_type(),
                },
            ],
            locals: vec![
                local(0, "good"),
                local(1, "read_first"),
                initialized,
                absolute,
                alias,
                array,
                local(6, "escaped"),
                local(7, "alias_target"),
            ],
            temps: Vec::new(),
            notes: Vec::new(),
            blocks: vec![block(
                0,
                "entry",
                vec![
                    NirOp::Store {
                        place: local_place(0, "good"),
                        src: NirValue::ConstU8(1),
                        ty: byte_type(),
                    },
                    NirOp::Load {
                        dest: TempId(0),
                        ty: byte_type(),
                        place: local_place(0, "good"),
                    },
                    NirOp::Load {
                        dest: TempId(1),
                        ty: byte_type(),
                        place: local_place(1, "read_first"),
                    },
                    NirOp::Load {
                        dest: TempId(2),
                        ty: byte_type(),
                        place: param_place(0, "value"),
                    },
                    NirOp::Load {
                        dest: TempId(3),
                        ty: byte_type(),
                        place: param_place(1, "items"),
                    },
                    NirOp::AddrOf {
                        dest: TempId(4),
                        ty: NirType {
                            kind: NirTypeKind::Pointer {
                                pointee: None,
                                address_space: crate::target::TargetLayout::DATA_ADDRESS_SPACE,
                            },
                            summary: "Byte*".to_string(),
                            width: Some(crate::target::ByteSize::new(2)),
                            pointer: true,
                        },
                        place: local_place(6, "escaped"),
                    },
                ],
                NirTerminator::Return(None),
            )],
        };

        let analysis = analyze_program_storage(&program(routine));
        let routine = analysis.routine("Main").unwrap();
        assert!(routine.storage_by_name("good").unwrap().is_promotable());
        assert!(routine.storage_by_name("value").unwrap().is_promotable());
        assert!(
            routine
                .storage_by_name("read_first")
                .unwrap()
                .blockers
                .contains(&NirPromotionBlocker::ReadBeforeDefinition)
        );
        assert!(
            routine
                .storage_by_name("initialized")
                .unwrap()
                .blockers
                .contains(&NirPromotionBlocker::InitializedStorage)
        );
        assert!(
            routine
                .storage_by_name("absolute")
                .unwrap()
                .blockers
                .contains(&NirPromotionBlocker::AbsoluteBacking)
        );
        assert!(
            routine
                .storage_by_name("alias")
                .unwrap()
                .blockers
                .contains(&NirPromotionBlocker::AliasBacking)
        );
        assert!(
            routine
                .storage_by_name("alias_target")
                .unwrap()
                .blockers
                .contains(&NirPromotionBlocker::AliasedStorage)
        );
        assert!(
            routine
                .storage_by_name("array")
                .unwrap()
                .blockers
                .contains(&NirPromotionBlocker::NonScalarStorage)
        );
        assert!(
            routine
                .storage_by_name("items")
                .unwrap()
                .blockers
                .contains(&NirPromotionBlocker::NonScalarStorage)
        );
        assert!(
            routine
                .storage_by_name("escaped")
                .unwrap()
                .blockers
                .contains(&NirPromotionBlocker::AddressTaken)
        );
    }

    #[test]
    fn definite_assignment_intersects_diamond_predecessors() {
        let routine = NirRoutine {
            id: crate::nir::RoutineId(0),
            signature: crate::nir::NirCallableSignature::default(),
            convention: crate::nir::NirCallConvention::TargetPublic,
            activation: crate::nir::NirActivationModel::ClassicStatic,
            entry: crate::nir::NirRoutineEntry::default(),
            name: "Diamond".to_string(),
            params: Vec::new(),
            locals: vec![local(0, "value")],
            temps: Vec::new(),
            notes: Vec::new(),
            blocks: vec![
                block(
                    0,
                    "entry",
                    Vec::new(),
                    NirTerminator::Branch {
                        condition: NirValue::ConstU8(1),
                        then_edge: edge(1),
                        else_edge: edge(2),
                    },
                ),
                block(
                    1,
                    "left",
                    vec![NirOp::Store {
                        place: local_place(0, "value"),
                        src: NirValue::ConstU8(1),
                        ty: byte_type(),
                    }],
                    NirTerminator::Goto(edge(3)),
                ),
                block(2, "right", Vec::new(), NirTerminator::Goto(edge(3))),
                block(
                    3,
                    "join",
                    vec![NirOp::Load {
                        dest: TempId(0),
                        ty: byte_type(),
                        place: local_place(0, "value"),
                    }],
                    NirTerminator::Return(None),
                ),
            ],
        };

        let analysis = analyze_program_storage(&program(routine));
        let facts = analysis
            .routine("Diamond")
            .unwrap()
            .storage_by_name("value")
            .unwrap();
        assert!(facts.possible_read_before_definition);
        assert!(facts.value_needed_at_exit);
    }

    #[test]
    fn opaque_machine_blocks_make_storage_visible() {
        let routine = NirRoutine {
            id: crate::nir::RoutineId(0),
            signature: crate::nir::NirCallableSignature::default(),
            convention: crate::nir::NirCallConvention::TargetPublic,
            activation: crate::nir::NirActivationModel::ClassicStatic,
            entry: crate::nir::NirRoutineEntry::default(),
            name: "Machine".to_string(),
            params: Vec::new(),
            locals: vec![local(0, "value")],
            temps: Vec::new(),
            notes: Vec::new(),
            blocks: vec![block(
                0,
                "entry",
                vec![NirOp::ForeignCode {
                    code: NirForeignCode {
                        target: crate::target::TargetId::Atari6502,
                        kind: NirForeignCodeKind::LegacyMachineBlock,
                        payload: NirForeignCodePayload::Structured(vec![NirMachineItem::Byte(
                            0x60,
                        )]),
                        source: String::new(),
                        span: crate::source::Span::new(0, 0),
                    },
                    effects: NirMachineEffects {
                        memory: NirMemoryEffects {
                            reads: NirMemoryAccess::Unknown,
                            writes: NirMemoryAccess::Unknown,
                        },
                        may_call_external: false,
                        opaque: true,
                    },
                }],
                NirTerminator::Return(None),
            )],
        };

        let analysis = analyze_program_storage(&program(routine));
        let facts = analysis
            .routine("Machine")
            .unwrap()
            .storage_by_name("value")
            .unwrap();
        assert!(facts.machine_visible);
        assert!(
            facts
                .blockers
                .contains(&NirPromotionBlocker::MachineVisibility)
        );
    }

    #[test]
    fn structured_call_regions_mark_only_overlapping_storage() {
        let routine = NirRoutine {
            id: crate::nir::RoutineId(0),
            signature: crate::nir::NirCallableSignature::default(),
            convention: crate::nir::NirCallConvention::TargetPublic,
            activation: crate::nir::NirActivationModel::ClassicStatic,
            entry: crate::nir::NirRoutineEntry::default(),
            name: "Effects".to_string(),
            params: Vec::new(),
            locals: vec![local(0, "x"), local(1, "y")],
            temps: Vec::new(),
            notes: Vec::new(),
            blocks: vec![block(
                0,
                "entry",
                vec![NirOp::Call {
                    callee: crate::nir::NirCallee::Builtin("TouchX".to_string()),
                    args: Vec::new(),
                    result: None,
                    signature: Some(crate::nir::NirCallableSignature::empty_proc(
                        crate::nir::NirCallConvention::Runtime,
                    )),
                    effects: crate::nir::NirCallEffects {
                        memory: NirMemoryEffects {
                            reads: NirMemoryAccess::None,
                            writes: NirMemoryAccess::Regions(vec![NirMemoryRegion {
                                kind: NirMemoryRegionKind::Storage(NirStorageId::Local(LocalId(0))),
                                offset: ByteOffset::ZERO,
                                size: ByteSize::ONE,
                            }]),
                        },
                        may_call_external: false,
                        opaque: false,
                    },
                }],
                NirTerminator::Return(None),
            )],
        };

        let analysis = analyze_program_storage(&program(routine));
        let routine = analysis.routine("Effects").unwrap();
        assert!(routine.storage_by_name("x").unwrap().calls_may_write);
        assert!(!routine.storage_by_name("y").unwrap().calls_may_write);
    }
}
