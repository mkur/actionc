use std::collections::{BTreeMap, BTreeSet};

use super::cfg::NirCfg;
use super::dataflow::{NirDataflowDirection, NirDataflowProblem, solve_dataflow};
use crate::nir::facts::{NirStorageId, root_storage_id};
use crate::nir::{
    BlockId, NirGlobal, NirGlobalBacking, NirLocalBacking, NirMachineAtom, NirMachineItem,
    NirMemoryAccess, NirMemoryRegion, NirMemoryRegionKind, NirOp, NirPlace, NirProgram, NirRoutine,
    NirStorageClass, NirType, NirTypeKind,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum NirStorageBackingClass {
    Ordinary,
    Absolute,
    Alias,
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
    MachineVisibility,
    ReadBeforeDefinition,
    NoDirectAccess,
    AccessTypeMismatch,
}

impl NirPromotionBlocker {
    pub const ALL: [Self; 12] = [
        Self::GlobalStorage,
        Self::NonScalarStorage,
        Self::UnsupportedType,
        Self::AbsoluteBacking,
        Self::AliasBacking,
        Self::AliasedStorage,
        Self::InitializedStorage,
        Self::AddressTaken,
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
    pub width: Option<u16>,
    pub direct_access_ty: Option<NirType>,
    pub storage_class: Option<NirStorageClass>,
    pub backing: NirStorageBackingClass,
    pub load_blocks: BTreeSet<BlockId>,
    pub store_blocks: BTreeSet<BlockId>,
    pub direct_loads: usize,
    pub direct_stores: usize,
    pub address_taken: bool,
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

    /// Whether exact load values may be cached while effect barriers remain in
    /// place. This is intentionally broader than full home promotion:
    /// initialized, persistent, and global storage can still participate in
    /// load forwarding because stores are not removed.
    pub fn is_value_trackable(&self) -> bool {
        let pointer_cell = self.storage_class == Some(NirStorageClass::Array)
            && self.direct_access_ty.as_ref().is_some_and(|ty| {
                matches!(ty.kind, NirTypeKind::Ptr16 { .. }) && ty.width == Some(2)
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
    NirProgramStorageAnalysis {
        routines: program
            .routines
            .iter()
            .map(|routine| analyze_routine_storage(routine, &globals, &global_names))
            .collect(),
    }
}

fn analyze_routine_storage(
    routine: &NirRoutine,
    globals: &BTreeMap<crate::nir::SymbolId, &NirGlobal>,
    global_names: &BTreeMap<String, crate::nir::SymbolId>,
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
                NirStorageBackingClass::Ordinary,
                false,
            ),
        );
    }
    for local in &routine.locals {
        let id = NirStorageId::Local(local.id);
        let backing = match local.backing {
            NirLocalBacking::Ordinary => NirStorageBackingClass::Ordinary,
            NirLocalBacking::Absolute(_) => NirStorageBackingClass::Absolute,
            NirLocalBacking::Alias { .. } | NirLocalBacking::GlobalAlias { .. } => {
                NirStorageBackingClass::Alias
            }
        };
        homes.insert(
            id,
            new_facts(
                id,
                local.name.clone(),
                Some(local.ty.clone()),
                Some(local.storage),
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
        }
    }

    // Globals are routine facts only when the routine names them directly (or
    // a machine item names them). This avoids multiplying every program global
    // by every routine while retaining exact identities for effect analysis.
    let mut referenced_globals = BTreeSet::new();
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
            if let NirOp::MachineBlock { items, .. } = op {
                for name in machine_item_names(items) {
                    if let Some(id) = global_names.get(&name.to_ascii_lowercase()) {
                        referenced_globals.insert(*id);
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
        }
    }
    for global in globals.values() {
        if let NirGlobalBacking::Alias { target, .. } = &global.backing
            && let Some(target) = global_names.get(&target.to_ascii_lowercase())
            && let Some(target) = homes.get_mut(&NirStorageId::Global(*target))
        {
            target.blockers.insert(NirPromotionBlocker::AliasedStorage);
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
                NirOp::Store { place, ty, .. } => {
                    record_direct_access(&mut homes, block.id, place, ty, false);
                }
                NirOp::AddrOf { place, .. } => {
                    if let Some(id) = root_storage_id(place)
                        && let Some(facts) = homes.get_mut(&id)
                    {
                        facts.address_taken = true;
                    }
                }
                NirOp::Call { effects, .. } => {
                    for facts in homes.values_mut() {
                        facts.calls_may_read |=
                            memory_accesses_storage(&effects.memory.reads, facts.id, facts.width);
                        facts.calls_may_write |=
                            memory_accesses_storage(&effects.memory.writes, facts.id, facts.width);
                    }
                }
                NirOp::MachineBlock { items, effects } => {
                    let names_used = machine_item_names(items);
                    let unknown_text = items
                        .iter()
                        .any(|item| matches!(item, NirMachineItem::Raw(_)));
                    if effects.opaque || unknown_text {
                        for facts in homes.values_mut() {
                            facts.machine_visible = true;
                        }
                    } else {
                        for name in names_used {
                            if let Some(id) = names.get(&name.to_ascii_lowercase())
                                && let Some(facts) = homes.get_mut(id)
                            {
                                facts.machine_visible = true;
                            }
                        }
                    }
                }
                NirOp::Define { .. }
                | NirOp::Set { .. }
                | NirOp::Declare { .. }
                | NirOp::Assign { .. }
                | NirOp::CompoundAssign { .. }
                | NirOp::Unary { .. }
                | NirOp::Cast { .. }
                | NirOp::Binary { .. }
                | NirOp::Compare { .. }
                | NirOp::Unsupported { .. }
                | NirOp::Note { .. } => {}
            }
        }
    }

    mark_read_before_definition(routine, &cfg, &mut homes);
    for facts in homes.values_mut() {
        facts.value_needed_at_exit = match facts.id {
            NirStorageId::Local(_) => {
                facts.direct_stores != 0 && facts.possible_read_before_definition
            }
            NirStorageId::Param(_) | NirStorageId::Global(_) => facts.direct_stores != 0,
        };
        if facts.address_taken {
            facts.blockers.insert(NirPromotionBlocker::AddressTaken);
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

fn new_facts(
    id: NirStorageId,
    name: String,
    ty: Option<NirType>,
    storage_class: Option<NirStorageClass>,
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
        backing,
        load_blocks: BTreeSet::new(),
        store_blocks: BTreeSet::new(),
        direct_loads: 0,
        direct_stores: 0,
        address_taken: false,
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
    let mut facts = new_facts(
        NirStorageId::Global(global.id),
        global.name.clone(),
        global.ty.clone(),
        storage_class,
        backing,
        global.init.is_some(),
    );
    facts.blockers.insert(NirPromotionBlocker::GlobalStorage);
    facts
}

fn supported_scalar_type(ty: &NirType) -> bool {
    matches!(
        ty.kind,
        NirTypeKind::Bool
            | NirTypeKind::U8
            | NirTypeKind::I8
            | NirTypeKind::U16
            | NirTypeKind::I16
            | NirTypeKind::Ptr16 { .. }
            | NirTypeKind::Callable { .. }
    ) && matches!(ty.width, Some(1 | 2))
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
        && matches!(access_ty.kind, NirTypeKind::Ptr16 { .. })
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
    width: Option<u16>,
) -> bool {
    match access {
        NirMemoryAccess::None => false,
        NirMemoryAccess::Regions(regions) => {
            let Some(width) = width else {
                return true;
            };
            let storage = NirMemoryRegion {
                kind: NirMemoryRegionKind::Storage(storage),
                offset: 0,
                size: width,
            };
            regions.iter().any(|region| region.overlaps(&storage))
        }
        NirMemoryAccess::Unknown | NirMemoryAccess::All => true,
    }
}

fn for_each_op_place(op: &NirOp, mut visit: impl FnMut(&NirPlace)) {
    match op {
        NirOp::Load { place, .. } | NirOp::AddrOf { place, .. } | NirOp::Store { place, .. } => {
            visit(place)
        }
        NirOp::Define { .. }
        | NirOp::Set { .. }
        | NirOp::Declare { .. }
        | NirOp::Assign { .. }
        | NirOp::CompoundAssign { .. }
        | NirOp::Unary { .. }
        | NirOp::Cast { .. }
        | NirOp::Binary { .. }
        | NirOp::Compare { .. }
        | NirOp::Call { .. }
        | NirOp::MachineBlock { .. }
        | NirOp::Unsupported { .. }
        | NirOp::Note { .. } => {}
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
            | NirMachineItem::Raw(_) => None,
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
            if let NirOp::Store { place, .. } = op
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
                NirOp::Load { place, .. } => {
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
                NirOp::Store { place, .. } => {
                    if let Some(id) = crate::nir::direct_storage_id(place) {
                        defined.insert(id);
                    }
                }
                NirOp::Define { .. }
                | NirOp::Set { .. }
                | NirOp::Declare { .. }
                | NirOp::Assign { .. }
                | NirOp::CompoundAssign { .. }
                | NirOp::AddrOf { .. }
                | NirOp::Unary { .. }
                | NirOp::Cast { .. }
                | NirOp::Binary { .. }
                | NirOp::Compare { .. }
                | NirOp::Call { .. }
                | NirOp::MachineBlock { .. }
                | NirOp::Unsupported { .. }
                | NirOp::Note { .. } => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nir::{
        LocalId, NirBlock, NirLocal, NirMachineEffects, NirMemoryEffects, NirParam, NirPlaceKind,
        NirStorageInit, NirTerminator, NirValue, ParamId, TempId,
    };

    fn byte_type() -> NirType {
        NirType {
            kind: NirTypeKind::U8,
            summary: "Byte".to_string(),
            width: Some(1),
            pointer: false,
        }
    }

    fn local(id: u32, name: &str) -> NirLocal {
        NirLocal {
            id: LocalId(id),
            name: name.to_string(),
            kind: "Byte".to_string(),
            storage: NirStorageClass::Scalar,
            ty: byte_type(),
            backing: NirLocalBacking::Ordinary,
            init: None,
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
            ops,
            terminator,
        }
    }

    fn program(routine: NirRoutine) -> NirProgram {
        NirProgram {
            globals: Vec::new(),
            statics: Vec::new(),
            routines: vec![routine],
        }
    }

    #[test]
    fn classifies_narrow_scalar_candidates_and_exclusion_reasons() {
        let mut initialized = local(2, "initialized");
        initialized.init = Some(NirStorageInit::ZeroFill {
            bytes: 1,
            mutable: true,
            section: "data".to_string(),
        });
        let mut absolute = local(3, "absolute");
        absolute.backing = NirLocalBacking::Absolute(0xD000);
        let mut alias = local(4, "alias");
        alias.backing = NirLocalBacking::Alias {
            target: LocalId(7),
            target_name: "alias_target".to_string(),
            offset: 0,
        };
        let mut array = local(5, "array");
        array.storage = NirStorageClass::Array;
        let routine = NirRoutine {
            name: "Main".to_string(),
            params: vec![
                NirParam {
                    id: ParamId(0),
                    name: "value".to_string(),
                    storage: NirStorageClass::Scalar,
                    ty: byte_type(),
                },
                NirParam {
                    id: ParamId(1),
                    name: "items".to_string(),
                    storage: NirStorageClass::Array,
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
                            kind: NirTypeKind::Ptr16 { pointee: None },
                            summary: "Byte*".to_string(),
                            width: Some(2),
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
                        then_label: "left".to_string(),
                        else_label: "right".to_string(),
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
                    NirTerminator::Goto("join".to_string()),
                ),
                block(
                    2,
                    "right",
                    Vec::new(),
                    NirTerminator::Goto("join".to_string()),
                ),
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
            name: "Machine".to_string(),
            params: Vec::new(),
            locals: vec![local(0, "value")],
            temps: Vec::new(),
            notes: Vec::new(),
            blocks: vec![block(
                0,
                "entry",
                vec![NirOp::MachineBlock {
                    items: vec![NirMachineItem::Byte(0x60)],
                    effects: NirMachineEffects {
                        memory: NirMemoryEffects {
                            reads: NirMemoryAccess::Unknown,
                            writes: NirMemoryAccess::Unknown,
                        },
                        may_call_os: false,
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
                    signature: None,
                    effects: crate::nir::NirCallEffects {
                        memory: NirMemoryEffects {
                            reads: NirMemoryAccess::None,
                            writes: NirMemoryAccess::Regions(vec![NirMemoryRegion {
                                kind: NirMemoryRegionKind::Storage(NirStorageId::Local(LocalId(0))),
                                offset: 0,
                                size: 1,
                            }]),
                        },
                        may_call_os: false,
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
