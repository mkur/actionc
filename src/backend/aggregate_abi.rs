//! Physical by-value aggregate ABI expansion, shared by the target planners.
//!
//! Verified logical NIR becomes verified physical NIR: a hidden first result
//! pointer and pointers to caller-owned argument captures. Callees copy arguments
//! into independent mutable homes. Signature IDs retain logical nominal identity.
//! Typing, tag validation and ordered evaluation are already explicit in input.

use super::VerifiedNir;
use crate::nir::*;
use crate::target::TargetLayout;
use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet};

fn aggregate(ty: &NirType) -> bool {
    matches!(ty.kind, NirTypeKind::Record { .. })
}

pub(crate) fn expand_aggregate_abi(
    input: VerifiedNir<'_>,
) -> Result<Cow<'_, NirProgram>, Vec<NirDiagnostic>> {
    let program = input.program();
    let signatures = program
        .routines
        .iter()
        .flat_map(|routine| {
            std::iter::once(&routine.signature).chain(routine.blocks.iter().flat_map(|block| {
                block.ops.iter().filter_map(|op| match op {
                    NirOp::Call { signature, .. } => signature.as_ref(),
                    _ => None,
                })
            }))
        })
        .collect::<Vec<_>>();
    if !signatures
        .iter()
        .any(|sig| sig.params.iter().chain(sig.result.iter()).any(aggregate))
    {
        return Ok(Cow::Borrowed(program));
    }
    let returns = signatures
        .iter()
        .filter(|sig| sig.result.as_ref().is_some_and(aggregate))
        .map(|sig| sig.id)
        .collect();
    let mut expansion = Expansion {
        layout: program.target_layout,
        returns,
        homes: BTreeMap::new(),
        next_temp: 0,
    };
    let mut physical = program.clone();
    for global in &mut physical.globals {
        if let Some(ty) = &mut global.ty {
            expansion.ty(ty);
        }
    }
    for data in &mut physical.statics {
        expansion.ty(&mut data.ty);
    }
    for routine in &mut physical.routines {
        expansion.routine(routine);
    }
    crate::nir::verify_program(&physical)?;
    Ok(Cow::Owned(physical))
}

struct Expansion {
    layout: TargetLayout,
    returns: BTreeSet<SignatureId>,
    homes: BTreeMap<ParamId, (LocalId, String)>,
    next_temp: u32,
}

impl Expansion {
    fn pointer(&self, ty: &NirType) -> NirType {
        NirType {
            kind: NirTypeKind::Pointer {
                pointee: Some(Box::new(ty.kind.clone())),
                address_space: self.layout.data_pointer.address_space,
            },
            summary: format!("{} POINTER", ty.summary),
            width: Some(self.layout.data_pointer.size_bytes),
            pointer: true,
        }
    }
    fn ty(&self, ty: &mut NirType) {
        self.kind(&mut ty.kind);
    }
    fn kind(&self, kind: &mut NirTypeKind) {
        match kind {
            NirTypeKind::Callable {
                kind, signature, ..
            } if self.returns.contains(signature) => *kind = NirCallableKind::Proc,
            NirTypeKind::Pointer {
                pointee: Some(pointee),
                ..
            } => self.kind(pointee),
            _ => {}
        }
    }
    fn signature(&self, sig: &mut NirCallableSignature) {
        for ty in &mut sig.params {
            if aggregate(ty) {
                *ty = self.pointer(ty);
            }
            self.ty(ty);
        }
        if sig.result.as_ref().is_some_and(aggregate) {
            let result = sig.result.take().unwrap();
            sig.params.insert(0, self.pointer(&result));
            sig.kind = NirCallableKind::Proc;
        }
        if let Some(ty) = &mut sig.result {
            self.ty(ty);
        }
        if let Some(ty) = &mut sig.variadic {
            self.ty(ty);
        }
    }
    fn temp(&mut self, ty: NirType) -> NirValue {
        let id = TempId(self.next_temp);
        self.next_temp += 1;
        NirValue::Temp { id, ty }
    }
    fn address(&mut self, place: NirPlace, ops: &mut Vec<NirOp>) -> NirValue {
        let ty = self.pointer(place.ty.as_ref().expect("verified aggregate place type"));
        let value = self.temp(ty.clone());
        ops.push(NirOp::AddrOf {
            dest: value.temp().unwrap(),
            ty,
            place,
        });
        value
    }
    fn incoming(&mut self, param: &NirParam, ops: &mut Vec<NirOp>) -> NirPlace {
        let value = self.temp(param.ty.clone());
        let pointee = match &param.ty.kind {
            NirTypeKind::Pointer {
                pointee: Some(ty), ..
            } => ty.as_ref().clone(),
            _ => unreachable!(),
        };
        let size = match pointee {
            NirTypeKind::Record {
                size: Some(size), ..
            } => size,
            _ => unreachable!(),
        };
        ops.push(NirOp::Load {
            dest: value.temp().unwrap(),
            ty: param.ty.clone(),
            place: NirPlace {
                kind: NirPlaceKind::Param {
                    id: param.id,
                    name: param.name.clone(),
                },
                ty: Some(param.ty.clone()),
            },
        });
        NirPlace {
            kind: NirPlaceKind::Deref { addr: value },
            ty: Some(NirType {
                summary: param.ty.summary.trim_end_matches(" POINTER").to_owned(),
                kind: pointee,
                width: Some(size),
                pointer: false,
            }),
        }
    }
    fn copy(destination: NirPlace, source: NirPlace) -> NirOp {
        let size = source
            .ty
            .as_ref()
            .and_then(|ty| ty.width)
            .expect("complete aggregate extent");
        NirOp::CopyBytes {
            destination,
            source,
            size,
            destination_volatile: false,
            source_volatile: false,
        }
    }
    fn routine(&mut self, routine: &mut NirRoutine) {
        self.homes.clear();
        self.next_temp = routine
            .temps
            .iter()
            .map(|temp| temp.id.0)
            .max()
            .map_or(0, |id| id + 1);
        let mut next_local = routine
            .locals
            .iter()
            .map(|local| local.id.0)
            .max()
            .map_or(0, |id| id + 1);
        let pointer_layout = NirObjectLayout::new(
            self.layout.data_pointer.size_bytes,
            self.layout.data_pointer.alignment_bytes,
        );
        let mut prologue = Vec::new();
        for param in &mut routine.params {
            if aggregate(&param.ty) {
                let id = LocalId(next_local);
                next_local += 1;
                let name = format!("{}.__value", param.name);
                self.homes.insert(param.id, (id, name.clone()));
                routine.locals.push(NirLocal {
                    id,
                    name: name.clone(),
                    kind: "aggregate parameter value".into(),
                    purpose: NirLocalPurpose::Storage,
                    storage: NirStorageClass::Record,
                    duration: param.duration,
                    layout: param.layout,
                    ty: param.ty.clone(),
                    backing: NirLocalBacking::Ordinary,
                    init: None,
                });
                let destination = NirPlace {
                    kind: NirPlaceKind::Local { id, name },
                    ty: Some(param.ty.clone()),
                };
                param.ty = self.pointer(&param.ty);
                param.layout = pointer_layout;
                param.storage = NirStorageClass::Scalar;
                let source = self.incoming(param, &mut prologue);
                prologue.push(Self::copy(destination, source));
            }
            self.ty(&mut param.ty);
        }
        let result_param = routine
            .signature
            .result
            .as_ref()
            .filter(|ty| aggregate(ty))
            .map(|ty| NirParam {
                id: ParamId(
                    routine
                        .params
                        .iter()
                        .map(|param| param.id.0)
                        .max()
                        .map_or(0, |id| id + 1),
                ),
                name: "__aggregate_result".into(),
                storage: NirStorageClass::Scalar,
                duration: match routine.activation {
                    NirActivationModel::ClassicStatic => NirStorageDuration::RoutineStatic,
                    NirActivationModel::NativeReentrant => NirStorageDuration::Automatic,
                },
                layout: pointer_layout,
                ty: self.pointer(ty),
            });
        if let Some(param) = &result_param {
            routine.params.insert(0, param.clone());
        }
        self.signature(&mut routine.signature);
        for local in &mut routine.locals {
            self.ty(&mut local.ty);
            if let Some(init) = &mut local.init {
                match init {
                    NirStorageInit::Bytes { image, .. } => self.image(image),
                    NirStorageInit::Descriptor { backing, .. } => self.image(&mut backing.image),
                    NirStorageInit::ZeroFill { .. } => {}
                }
            }
        }
        for block in &mut routine.blocks {
            for param in &mut block.params {
                self.ty(&mut param.ty);
            }
            let mut ops = Vec::new();
            for mut op in std::mem::take(&mut block.ops) {
                self.op(&mut op);
                if let NirOp::Call {
                    args,
                    aggregate_result,
                    ..
                } = &mut op
                {
                    if let Some(place) = aggregate_result.take() {
                        let address = self.address(place, &mut ops);
                        args.insert(0, address);
                    }
                    for arg in args {
                        if let NirValue::Aggregate { place } = arg {
                            *arg = self.address(*place.clone(), &mut ops);
                        }
                    }
                }
                ops.push(op);
            }
            self.terminator(&mut block.terminator);
            if let NirTerminator::Return(Some(NirValue::Aggregate { place })) = &block.terminator {
                let destination = self.incoming(
                    result_param
                        .as_ref()
                        .expect("verified aggregate return signature"),
                    &mut ops,
                );
                ops.push(Self::copy(destination, *place.clone()));
                block.terminator = NirTerminator::Return(None);
            }
            block.ops = ops;
        }
        if let Some(entry) = routine.blocks.first_mut() {
            prologue.append(&mut entry.ops);
            entry.ops = prologue;
        }
        rebuild_temps(routine);
    }
    fn value(&mut self, value: &mut NirValue) {
        match value {
            NirValue::Aggregate { place } => self.place(place),
            NirValue::Temp { ty, .. }
            | NirValue::Null { ty }
            | NirValue::AddressConst { ty, .. }
            | NirValue::StaticAddr { ty, .. }
            | NirValue::RoutineAddr { ty, .. } => self.ty(ty),
            NirValue::IntegerConst { .. } | NirValue::Param(_) | NirValue::GlobalAddr(_) => {}
        }
    }
    fn place(&mut self, place: &mut NirPlace) {
        if let Some(ty) = &mut place.ty {
            self.ty(ty);
        }
        match &mut place.kind {
            NirPlaceKind::Param { id, .. } => {
                if let Some((id, name)) = self.homes.get(id) {
                    place.kind = NirPlaceKind::Local {
                        id: *id,
                        name: name.clone(),
                    };
                }
            }
            NirPlaceKind::Field { base, ty, .. } => {
                self.place(base);
                self.ty(ty);
            }
            NirPlaceKind::Deref { addr } => self.value(addr),
            NirPlaceKind::Index {
                base_addr,
                index,
                elem_ty,
                ..
            } => {
                self.value(base_addr);
                self.value(index);
                self.ty(elem_ty);
            }
            _ => {}
        }
    }
    fn storage(&self, storage: &mut NirStorageId) {
        if let NirStorageId::Param(id) = storage
            && let Some((id, _)) = self.homes.get(id)
        {
            *storage = NirStorageId::Local(*id);
        }
    }
    fn image(&self, image: &mut NirDataImage) {
        for fragment in &mut image.fragments {
            if let NirDataFragment::Address { target: NirDataAddressTarget::Storage(storage), .. } = fragment {
                self.storage(storage);
            }
        }
    }

    fn effects(&self, effects: &mut NirMemoryEffects) {
        for access in [&mut effects.reads, &mut effects.writes] {
            if let NirMemoryAccess::Regions(regions) = access {
                for region in regions {
                    if let NirMemoryRegionKind::Storage(storage) = &mut region.kind {
                        self.storage(storage);
                    }
                }
            }
        }
    }
    fn real_source(&mut self, source: &mut NirRealSource) {
        if let NirRealSource::Place(place) = source {
            self.place(place);
        }
    }
    fn op(&mut self, op: &mut NirOp) {
        match op {
            NirOp::Load { ty, place, .. }
            | NirOp::VolatileLoad { ty, place, .. }
            | NirOp::AddrOf { ty, place, .. } => {
                self.ty(ty);
                self.place(place);
            }
            NirOp::Store { ty, place, src } | NirOp::VolatileStore { ty, place, src } => {
                self.ty(ty);
                self.place(place);
                self.value(src);
            }
            NirOp::CopyBytes {
                destination,
                source,
                ..
            } => {
                self.place(destination);
                self.place(source);
            }
            NirOp::Unary { ty, src, .. } => {
                self.ty(ty);
                self.value(src);
            }
            NirOp::Cast { src, from, to, .. } => {
                self.value(src);
                self.ty(from);
                self.ty(to);
            }
            NirOp::Binary {
                ty, left, right, ..
            } => {
                self.ty(ty);
                self.value(left);
                self.value(right);
            }
            NirOp::Compare {
                ty,
                operand_ty,
                left,
                right,
                ..
            } => {
                self.ty(ty);
                self.ty(operand_ty);
                self.value(left);
                self.value(right);
            }
            NirOp::PointerOffset {
                ty, base, offset, ..
            } => {
                self.ty(ty);
                self.value(base);
                self.value(offset);
            }
            NirOp::Call {
                callee,
                args,
                result,
                aggregate_result,
                signature,
                effects,
            } => {
                if let NirCallee::Indirect { target, ty } = callee {
                    self.value(target);
                    self.ty(ty);
                }
                for arg in args {
                    self.value(arg);
                }
                if let Some(place) = aggregate_result {
                    self.place(place);
                }
                if let Some(result) = result {
                    self.ty(&mut result.ty);
                }
                if let Some(sig) = signature {
                    self.signature(sig);
                }
                self.effects(&mut effects.memory);
            }
            NirOp::Real(op) => match op {
                NirRealOp::Copy {
                    destination,
                    source,
                } => {
                    self.place(destination);
                    self.real_source(source);
                }
                NirRealOp::Unary {
                    destination,
                    operand,
                    ..
                } => {
                    self.place(destination);
                    self.real_source(operand);
                }
                NirRealOp::Binary {
                    destination,
                    left,
                    right,
                    ..
                } => {
                    self.place(destination);
                    self.real_source(left);
                    self.real_source(right);
                }
                NirRealOp::Compare {
                    result_type,
                    left,
                    right,
                    ..
                } => {
                    self.ty(result_type);
                    self.real_source(left);
                    self.real_source(right);
                }
                NirRealOp::IntegerToReal {
                    destination,
                    source,
                    source_type,
                } => {
                    self.place(destination);
                    self.value(source);
                    self.ty(source_type);
                }
                NirRealOp::RealToInteger {
                    source,
                    result_type,
                    ..
                } => {
                    self.place(source);
                    self.ty(result_type);
                }
            },
            NirOp::ForeignCode { code, effects } => {
                let target = |target: &mut NirForeignCodeTarget| {
                    if let NirForeignCodeTarget::Storage(storage) = target {
                        self.storage(storage);
                    }
                };
                match &mut code.payload {
                    NirForeignCodePayload::Structured(items) => {
                        for item in items {
                            if let NirMachineItem::Relocation { target: value, .. } = item {
                                target(value);
                            }
                        }
                    }
                    NirForeignCodePayload::Bytes { relocations, .. } => {
                        for relocation in relocations {
                            target(&mut relocation.target);
                        }
                    }
                }
                self.effects(&mut effects.memory);
            }
            NirOp::Unsupported { .. } => {}
        }
    }
    fn terminator(&mut self, term: &mut NirTerminator) {
        match term {
            NirTerminator::Goto(edge) => {
                for arg in &mut edge.args {
                    self.value(arg);
                }
            }
            NirTerminator::Branch {
                condition,
                then_edge,
                else_edge,
            } => {
                self.value(condition);
                for arg in then_edge.args.iter_mut().chain(&mut else_edge.args) {
                    self.value(arg);
                }
            }
            NirTerminator::Return(Some(value)) => self.value(value),
            _ => {}
        }
    }
}

fn rebuild_temps(routine: &mut NirRoutine) {
    routine.temps.clear();
    for block in &routine.blocks {
        for param in &block.params {
            routine.temps.push(NirTemp {
                id: param.dest,
                ty: param.ty.clone(),
                def: NirTempDef {
                    block: block.id,
                    op_index: None,
                },
            });
        }
        for (index, op) in block.ops.iter().enumerate() {
            let definition = match op {
                NirOp::Load { dest, ty, .. }
                | NirOp::VolatileLoad { dest, ty, .. }
                | NirOp::AddrOf { dest, ty, .. }
                | NirOp::Unary { dest, ty, .. }
                | NirOp::Binary { dest, ty, .. }
                | NirOp::Compare { dest, ty, .. }
                | NirOp::PointerOffset { dest, ty, .. }
                | NirOp::Cast { dest, to: ty, .. } => Some((*dest, ty)),
                NirOp::Call {
                    result: Some(result),
                    ..
                } => Some((result.dest, &result.ty)),
                NirOp::Real(
                    NirRealOp::Compare {
                        result,
                        result_type,
                        ..
                    }
                    | NirRealOp::RealToInteger {
                        result,
                        result_type,
                        ..
                    },
                ) => Some((*result, result_type)),
                _ => None,
            };
            if let Some((id, ty)) = definition {
                routine.temps.push(NirTemp {
                    id,
                    ty: ty.clone(),
                    def: NirTempDef {
                        block: block.id,
                        op_index: Some(index),
                    },
                });
            }
        }
    }
}
