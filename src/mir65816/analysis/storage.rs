//! Byte-granular logical contents of invocation storage. These are not physical
//! temp homes, nor a pointee alias analysis. Unknown versions are never equality.
use super::{cfg::Cfg, *};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum StorageId {
    Frame(Mir65816FrameObjectId),
    Input(ParamId),
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ownership {
    ImmutableInput,
    InvocationPrivate,
    /// Includes all address-required objects, even without a proved escape.
    Addressable,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StorageFacts {
    pub size: ByteSize,
    pub ownership: Ownership,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StorageVersion {
    Incoming {
        storage: Storage,
        byte: u32,
    },
    /// A write outside every CFG cycle. A cyclic write has unknown version:
    /// its static site cannot distinguish successive dynamic instances.
    Write {
        site: Point,
        storage: Storage,
        byte: u32,
    },
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ByteContents {
    pub definitely_initialized: bool,
    pub version: Option<StorageVersion>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StorageContents {
    pub bytes: Vec<ByteContents>,
}
impl StorageContents {
    pub fn definitely_initialized(&self) -> bool {
        !self.bytes.is_empty() && self.bytes.iter().all(|b| b.definitely_initialized)
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Version {
    Incoming(StorageId),
    Write(ProgramPoint),
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ByteFact {
    initialized: bool,
    version: Option<Version>,
}
type State = BTreeMap<StorageId, Vec<ByteFact>>;

#[derive(Debug, Clone)]
enum Effect {
    None,
    /// Calls preserve only unexposed invocation storage and immutable inputs.
    Call,
    /// Unknown writes have no extent/disjointness proof, including numerically
    /// formed pointers and dynamic frame indices. Invalidate every version.
    UnknownWrite,
    Write {
        storage: StorageId,
        start: usize,
        bytes: usize,
        volatile: bool,
    },
}

#[derive(Debug)]
pub(super) struct StorageAnalysis {
    pub facts: BTreeMap<StorageId, StorageFacts>,
    pub parameters: BTreeMap<ParamId, StorageId>,
    result: DataflowResult<BlockId, Option<State>>,
    effects: BTreeMap<BlockId, Vec<Effect>>,
}

impl StorageAnalysis {
    pub fn new(routine: &Mir65816Routine, cfg: &Cfg) -> Result<Self, String> {
        let mut facts = BTreeMap::new();
        let mut initialized = BTreeSet::new();
        let mut owners = BTreeSet::new();
        for o in &routine.frame.objects {
            let id = StorageId::Frame(o.id);
            if o.size == ByteSize::ZERO
                || facts
                    .insert(
                        id,
                        StorageFacts {
                            size: o.size,
                            ownership: if o.addressable {
                                Ownership::Addressable
                            } else {
                                Ownership::InvocationPrivate
                            },
                        },
                    )
                    .is_some()
            {
                return Err("duplicate or empty frame object".into());
            }
            let owner = match o.owner {
                Mir65816FrameObjectOwner::Param(p) => (true, p.0),
                Mir65816FrameObjectOwner::Local(l) => (false, l.0),
            };
            if !owners.insert(owner) {
                return Err("duplicate frame owner".into());
            }
        }
        let mut parameters = BTreeMap::new();
        for p in &routine.frame.parameters {
            let Mir65816AbiHome::StackArgument { size, .. } = p.incoming else {
                return Err("unsized parameter home".into());
            };
            if size == ByteSize::ZERO {
                return Err("empty parameter".into());
            }
            let id = if let Some(object) = p.frame_object {
                let o = routine
                    .frame
                    .objects
                    .iter()
                    .find(|o| o.id == object)
                    .ok_or("missing parameter frame object")?;
                if o.owner != Mir65816FrameObjectOwner::Param(p.param) || o.size != size {
                    return Err("parameter frame ownership/extent mismatch".into());
                }
                StorageId::Frame(object)
            } else {
                let id = StorageId::Input(p.param);
                facts.insert(
                    id,
                    StorageFacts {
                        size,
                        ownership: Ownership::ImmutableInput,
                    },
                );
                id
            };
            if parameters.insert(p.param, id).is_some() {
                return Err("duplicate parameter identity".into());
            }
            initialized.insert(id);
        }
        for o in &routine.frame.objects {
            if let Mir65816FrameObjectOwner::Param(p) = o.owner {
                if parameters.get(&p) != Some(&StorageId::Frame(o.id)) {
                    return Err("unbound parameter frame object".into());
                }
            }
        }
        // Reuse the projected address-required flag, and conservatively notice
        // address formation in all blocks, including unreachable ones. Never
        // trust a forged stronger flag to establish non-escape.
        for b in &routine.blocks {
            for op in &b.ops {
                if let Mir65816Op::AddressOf { address, .. } = op {
                    if let Some(id) = storage_base(address, &parameters) {
                        facts
                            .get_mut(&id)
                            .ok_or("address of unknown frame object")?
                            .ownership = Ownership::Addressable;
                    }
                }
            }
        }
        let mut effects = BTreeMap::new();
        for b in &routine.blocks {
            let mut ops = Vec::new();
            for op in &b.ops {
                let effect = match op {
                    Mir65816Op::Load {
                        address,
                        width,
                        volatile,
                        ..
                    } => {
                        check_range(address, width.get(), &parameters, &facts)?;
                        if *volatile {
                            Effect::UnknownWrite
                        } else {
                            Effect::None
                        }
                    }
                    Mir65816Op::AddressOf { address, .. } => {
                        check_range(address, 0, &parameters, &facts)?;
                        Effect::None
                    }
                    Mir65816Op::Store {
                        address,
                        width,
                        volatile,
                        ..
                    } => write_effect(address, width.get(), *volatile, &parameters, &facts)?,
                    Mir65816Op::Copy {
                        destination,
                        source,
                        bytes,
                        destination_volatile,
                        source_volatile,
                        ..
                    } => {
                        check_range(source, bytes.get(), &parameters, &facts)?;
                        if bytes.is_zero() {
                            check_range(destination, 0, &parameters, &facts)?;
                            Effect::None
                        } else {
                            write_effect(
                                destination,
                                bytes.get(),
                                *destination_volatile || *source_volatile,
                                &parameters,
                                &facts,
                            )?
                        }
                    }
                    Mir65816Op::Call { .. } => Effect::Call,
                    Mir65816Op::Unary { .. }
                    | Mir65816Op::Cast { .. }
                    | Mir65816Op::PointerOffset { .. }
                    | Mir65816Op::Binary { .. }
                    | Mir65816Op::Compare { .. } => Effect::None,
                };
                if let Effect::Write {
                    storage: StorageId::Input(_),
                    ..
                } = effect
                {
                    return Err("write to parameter without a mutable frame home".into());
                }
                ops.push(effect);
            }
            effects.insert(b.id, ops);
        }
        let initial: State = facts
            .iter()
            .map(|(&id, fact)| {
                (
                    id,
                    vec![
                        ByteFact {
                            initialized: initialized.contains(&id),
                            version: initialized.contains(&id).then_some(Version::Incoming(id)),
                        };
                        fact.size.get() as usize
                    ],
                )
            })
            .collect();
        let problem = ContentsProblem {
            facts: &facts,
            effects: &effects,
            cfg,
            initial,
        };
        let result = solve_dataflow(cfg, &problem);
        Ok(Self {
            facts,
            parameters,
            result,
            effects,
        })
    }
    pub fn evaluations(&self) -> usize {
        self.result.evaluations()
    }
    pub fn contents(
        &self,
        routine: &Mir65816Routine,
        cfg: &Cfg,
        point: ProgramPoint,
        storage: StorageId,
        offset: u32,
        bytes: u32,
        point_handle: impl Fn(ProgramPoint) -> Point,
        storage_handle: impl Fn(StorageId) -> Storage,
    ) -> Result<StorageContents, QueryError> {
        let end = offset.checked_add(bytes).ok_or(QueryError::InvalidRange)?;
        if bytes == 0 || end > self.facts[&storage].size.get() {
            return Err(QueryError::InvalidRange);
        }
        let mut state = self
            .result
            .in_state(point.block)
            .and_then(Option::as_ref)
            .ok_or(QueryError::Unreachable)?
            .clone();
        debug_assert!(point.index <= routine.blocks[cfg.indices[&point.block]].ops.len());
        for (index, effect) in self.effects[&point.block][..point.index].iter().enumerate() {
            apply(
                &self.facts,
                &mut state,
                effect,
                ProgramPoint {
                    block: point.block,
                    index,
                },
                cfg.cyclic.contains(&point.block),
            );
        }
        Ok(StorageContents {
            bytes: state[&storage][offset as usize..end as usize]
                .iter()
                .enumerate()
                .map(|(index, b)| ByteContents {
                    definitely_initialized: b.initialized,
                    version: b.version.map(|v| match v {
                        Version::Incoming(s) => StorageVersion::Incoming {
                            storage: storage_handle(s),
                            byte: offset + index as u32,
                        },
                        Version::Write(p) => StorageVersion::Write {
                            site: point_handle(p),
                            storage: storage_handle(storage),
                            byte: offset + index as u32,
                        },
                    }),
                })
                .collect(),
        })
    }
}

fn storage_base(
    address: &Mir65816Address,
    params: &BTreeMap<ParamId, StorageId>,
) -> Option<StorageId> {
    match address.base {
        Mir65816AddressBase::AutomaticFrame(o) => Some(StorageId::Frame(o)),
        Mir65816AddressBase::Parameter(p) => {
            Some(params.get(&p).copied().unwrap_or(StorageId::Input(p)))
        }
        Mir65816AddressBase::Static(_)
        | Mir65816AddressBase::External(_)
        | Mir65816AddressBase::Indirect(_) => None,
    }
}

fn check_range(
    address: &Mir65816Address,
    bytes: u32,
    params: &BTreeMap<ParamId, StorageId>,
    facts: &BTreeMap<StorageId, StorageFacts>,
) -> Result<Option<(StorageId, usize, usize)>, String> {
    let Some(id) = storage_base(address, params) else {
        return Ok(None);
    };
    let fact = facts.get(&id).ok_or("unknown automatic storage identity")?;
    let index = match &address.index {
        None => 0,
        Some(index) => {
            let value = match index.value {
                Mir65816Value::U8(v) => u32::from(v),
                Mir65816Value::U16(v) => u32::from(v),
                Mir65816Value::U24(v) | Mir65816Value::U32(v) => v,
                Mir65816Value::Null(_) => 0,
                _ => return Ok(None),
            };
            value
                .checked_mul(index.stride.get())
                .ok_or("automatic storage index overflows")?
        }
    };
    let offset = address
        .displacement
        .get()
        .checked_add(index)
        .ok_or("automatic storage displacement overflows")?;
    let end = offset
        .checked_add(bytes)
        .ok_or("automatic storage extent overflows")?;
    if end > fact.size.get() {
        return Err("automatic storage access exceeds its declared extent".into());
    }
    Ok(Some((id, offset as usize, bytes as usize)))
}
fn write_effect(
    a: &Mir65816Address,
    bytes: u32,
    volatile: bool,
    p: &BTreeMap<ParamId, StorageId>,
    f: &BTreeMap<StorageId, StorageFacts>,
) -> Result<Effect, String> {
    if bytes == 0 {
        return Err("empty memory write".into());
    }
    Ok(match check_range(a, bytes, p, f)? {
        Some((storage, start, bytes)) => Effect::Write {
            storage,
            start,
            bytes,
            volatile,
        },
        None => Effect::UnknownWrite,
    })
}
fn apply(
    facts: &BTreeMap<StorageId, StorageFacts>,
    state: &mut State,
    effect: &Effect,
    point: ProgramPoint,
    cyclic: bool,
) {
    let all = matches!(
        effect,
        Effect::UnknownWrite | Effect::Write { volatile: true, .. }
    );
    if all || matches!(effect, Effect::Call) {
        for (id, bytes) in state.iter_mut() {
            if all || facts[id].ownership == Ownership::Addressable {
                for byte in bytes {
                    byte.version = None;
                }
            }
        }
    }
    if let Effect::Write {
        storage,
        start,
        bytes,
        volatile,
    } = *effect
    {
        for byte in &mut state.get_mut(&storage).unwrap()[start..start + bytes] {
            byte.initialized = true;
            byte.version = (!cyclic && !volatile).then_some(Version::Write(point));
        }
    }
}

struct ContentsProblem<'a> {
    facts: &'a BTreeMap<StorageId, StorageFacts>,
    effects: &'a BTreeMap<BlockId, Vec<Effect>>,
    cfg: &'a Cfg,
    initial: State,
}
impl DataflowProblem<Cfg> for ContentsProblem<'_> {
    type State = Option<State>;
    fn direction(&self) -> DataflowDirection {
        DataflowDirection::Forward
    }
    fn bottom(&self) -> Self::State {
        None
    }
    fn boundary(&self, b: BlockId) -> Option<Self::State> {
        (Some(b) == self.cfg.entry()).then(|| Some(self.initial.clone()))
    }
    fn join(&self, into: &mut Self::State, other: &Self::State) {
        let Some(other) = other else {
            return;
        };
        let Some(into) = into else {
            *into = Some(other.clone());
            return;
        };
        for (id, bytes) in into {
            for (a, b) in bytes.iter_mut().zip(&other[id]) {
                a.initialized &= b.initialized;
                if a.version != b.version {
                    a.version = None;
                }
            }
        }
    }
    fn transfer(&self, b: BlockId, state: &Self::State) -> Self::State {
        let mut state = state.clone()?;
        for (index, effect) in self.effects[&b].iter().enumerate() {
            apply(
                self.facts,
                &mut state,
                effect,
                ProgramPoint { block: b, index },
                self.cfg.cyclic.contains(&b),
            );
        }
        Some(state)
    }
}
