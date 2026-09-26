//! Qualification evidence only. MIR identity + operation spans + checked machine
//! instructions distinguish captured private words from arbitrary STA/LDA runs.
use super::*;
use actionc::{
    mir65816::{emit::MachineProgram, *},
    nir::{NirBinaryOp, NirCompareOp, NirStorageId, RoutineId, TempId},
};
use std::{collections::BTreeMap, ops::Range};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Kind {
    Arithmetic,
    Compare,
    Store,
    Return,
}
#[derive(Clone, Debug)]
pub struct Site {
    pub routine: RoutineId,
    pub temp: TempId,
    pub range: Range<u32>,
    pub producer: u32,
    pub store: u32,
    pub start: u32,
    pub load: Option<u32>,
    pub consumer: u32,
    pub slot: u16,
    pub kind: Kind,
    /// Complete proof window. Relocatable operand bytes are checked by the
    /// linker/relocator and independent source traffic probes, not frozen here.
    pub bytes: Vec<Option<u8>>,
}
/// Independent proof families travel together through existing image/o65 test
/// adapters. Map operations still refer exclusively to word-forwarding sites.
#[derive(Clone, Debug, Default)]
pub struct Index {
    words: BTreeMap<u32, Site>,
    pub shared_return_sites: usize,
    pub control: control_flow::Index,
    pub dispatches: Vec<control_flow::Dispatch>,
    pub multi_words: Vec<multi_word_edge::Site>,
    pub frame_words: Vec<frame_forwarding::Site>,
    pub parameter_words: Vec<parameter_forwarding::Site>,
    pub x_words: Vec<x_residency::Site>,
}
impl std::ops::Deref for Index {
    type Target = BTreeMap<u32, Site>;
    fn deref(&self) -> &Self::Target {
        &self.words
    }
}
impl std::ops::DerefMut for Index {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.words
    }
}
impl Site {
    pub fn forwarded(&self) -> bool {
        self.load.is_none()
    }
    pub fn rebase(&self, base: u32) -> Self {
        let mut s = self.clone();
        let old = self.range.start;
        s.range = base..base + self.range.end - old;
        s.producer = base + self.producer - old;
        s.store = base + self.store - old;
        s.start = base + self.start - old;
        s.load = self.load.map(|pc| base + pc - old);
        s.consumer = base + self.consumer - old;
        s
    }
    pub fn valid(&self, bus: &Bus) -> bool {
        self.range.contains(&self.producer)
            && self.range.contains(&self.consumer)
            && self.producer + self.bytes.len() as u32 <= self.range.end
            && self
                .bytes
                .iter()
                .enumerate()
                .all(|(i, b)| b.is_none_or(|b| bus.ram[self.producer as usize + i] == b))
            && bus.ram[self.store as usize..self.store as usize + 2] == homes::store(self.slot)
    }
}
pub fn reached<'a>(cpu: &Machine, bus: &'a Bus) -> Option<&'a Site> {
    if !cpu.is_instruction_boundary() || cpu.registers().p & 0x30 != 0 {
        return None;
    }
    let s = bus.forwarded_words.get(&cpu.pc())?;
    (s.forwarded() && s.valid(bus)).then_some(s)
}
pub fn resident_compare(bus: &Bus, pc: u32) -> Option<u16> {
    let s = bus.forwarded_words.get(&pc)?;
    (s.forwarded() && s.kind == Kind::Compare && s.valid(bus)).then_some(s.slot)
}
pub fn direct(address: &Mir65816Address) -> bool {
    address.index.is_none()
        && matches!(
            address.base,
            Mir65816AddressBase::AutomaticFrame(_)
                | Mir65816AddressBase::Parameter(_)
                | Mir65816AddressBase::External(_)
                | Mir65816AddressBase::Static(NirStorageId::Global(_))
        )
}
// Independent instruction boundaries, tracking explicit M changes just as the
// standalone disassembler does. Unknown/truncated instructions fail closed.
pub fn instructions(code: &[u8]) -> BTreeMap<usize, (usize, bool)> {
    let mut map = BTreeMap::new();
    let mut at = 0;
    let mut m8 = false;
    while at < code.len() {
        let op = code[at];
        let n = match op {
            0x18 | 0x38 | 0x1b | 0x3b | 0xaa | 0xa8 | 0x98 | 0x8a | 0xeb | 0x4b | 0x48 | 0x3a
            | 0x6b | 0xca | 0xe8 | 0xc8 | 0x0a | 0x4a => 1,
            0xa9 | 0x69 | 0xe9 | 0xc9 | 0x29 | 0x09 | 0x49 => {
                if m8 {
                    2
                } else {
                    3
                }
            }
            0xa0 | 0xa2 | 0xe0 | 0x62 | 0x82 | 0xf4 => 3,
            0xaf | 0xbf | 0x8f | 0x9f | 0x5c | 0x22 => 4,
            0xc2 | 0xe2 | 0xa6 | 0xa5 | 0x85 | 0x65 | 0xe5 | 0xc5 | 0x25 | 0x05 | 0x45 | 0x06
            | 0x26 | 0x46 | 0x66 | 0xa3 | 0x83 | 0x63 | 0xe3 | 0xc3 | 0xa7 | 0x87 | 0xb7 | 0x97
            | 0x23 | 0x03 | 0x43 | 0x80 | 0x10 | 0x30 | 0x50 | 0x70 | 0x90 | 0xb0 | 0xd0 | 0xf0 => {
                2
            }
            _ => panic!("unknown instruction {op:02x} at {at}"),
        };
        assert!(at + n <= code.len());
        map.insert(at, (n, m8));
        if matches!(op, 0xc2 | 0xe2) && code[at + 1] & 0x20 != 0 {
            m8 = op == 0xe2;
        }
        at += n;
    }
    map
}
pub fn index(
    mir: &Mir65816Program,
    machine: &MachineProgram,
    address: impl Fn(RoutineId) -> u32,
) -> Index {
    actionc::mir65816::verify_program(mir).unwrap();
    let mut out = Index {
        shared_return_sites: 0,
        words: BTreeMap::new(),
        control: control_flow::index(mir, machine, &address),
        frame_words: frame_forwarding::index(mir, machine, &address),
        parameter_words: parameter_forwarding::index(mir, machine, &address),
        x_words: x_residency::index(mir, machine, &address),
        multi_words: multi_word_edge::index(mir, machine, &address),
        dispatches: control_flow::dispatches(mir, machine, &address),
    };
    for m in &machine.routines {
        let r = mir.routines.iter().find(|r| r.id == m.id).unwrap();
        let base = address(r.id);
        let code = &m.code.bytes;
        let ins = instructions(code);
        let stack = |id| m.frame.temps.get(&id).and_then(|h| homes::of(*h));
        let word = |v: &Mir65816Value| match v {
            Mir65816Value::U8(_) | Mir65816Value::U16(_) => true,
            Mir65816Value::Temp(id, w) => w.get() == 2 && stack(*id).is_some(),
            Mir65816Value::Param(id) => r.frame.parameters.iter().any(|p| {
                p.param == *id
                    && matches!(p.incoming,Mir65816AbiHome::StackArgument{size,..} if size.get()==2)
            }),
            _ => false,
        };
        for block in &r.blocks {
            for (pi, producer) in block.ops.iter().enumerate() {
                let dest = match producer {
                    Mir65816Op::Load {
                        dest,
                        width,
                        address,
                        volatile: false,
                    } if width.get() == 2 && direct(address) => *dest,
                    Mir65816Op::Binary {
                        dest,
                        width,
                        operation,
                        left,
                        right,
                        ..
                    } if width.get() == 2
                        && matches!(
                            operation,
                            NirBinaryOp::Add
                                | NirBinaryOp::Sub
                                | NirBinaryOp::And
                                | NirBinaryOp::Or
                                | NirBinaryOp::Xor
                        )
                        && word(left)
                        && word(right) =>
                    {
                        *dest
                    }
                    _ => continue,
                };
                let Some(slot) = stack(dest) else { continue };
                let ci = pi + 1;
                let (source, kind) = if let Some(op) = block.ops.get(ci) {
                    match op {
                        Mir65816Op::Binary {
                            width,
                            operation,
                            left,
                            right,
                            ..
                        } if width.get() == 2
                            && matches!(
                                operation,
                                NirBinaryOp::Add
                                    | NirBinaryOp::Sub
                                    | NirBinaryOp::And
                                    | NirBinaryOp::Or
                                    | NirBinaryOp::Xor
                            )
                            && word(left)
                            && word(right) =>
                        {
                            (left, Kind::Arithmetic)
                        }
                        Mir65816Op::Compare {
                            width,
                            operation,
                            signed,
                            left,
                            right,
                            ..
                        } if width.get() == 2
                            && (!signed
                                || matches!(operation, NirCompareOp::Eq | NirCompareOp::Ne))
                            && word(left)
                            && word(right) =>
                        {
                            (
                                if matches!(operation, NirCompareOp::Gt | NirCompareOp::Le) {
                                    right
                                } else {
                                    left
                                },
                                Kind::Compare,
                            )
                        }
                        Mir65816Op::Store {
                            width,
                            value,
                            address,
                            volatile: false,
                        } if width.get() == 2 && direct(address) => (value, Kind::Store),
                        _ => continue,
                    }
                } else {
                    match &block.terminator {
                        Mir65816Terminator::Return { value: Some(v), .. }
                            if r.result_home
                                == Some(Mir65816AbiHome::NativeResult(
                                    abi::ResultLocation::A16,
                                )) =>
                        {
                            (v, Kind::Return)
                        }
                        _ => continue,
                    }
                };
                if !matches!(source,Mir65816Value::Temp(id,w) if *id==dest && w.get()==2) {
                    continue;
                }
                let p = &m.code.mir_spans[&(block.id, pi)];
                let c = &m.code.mir_spans[&(block.id, ci)];
                // The old adjacent-word probe ends at a local instruction. A
                // shared return instead transfers the result through a checked
                // internal join; its separate runtime tests cover those lanes.
                if kind == Kind::Return
                    && actionc::mir65816::emit::proof::selected_actions(&m.code)
                        .unwrap()
                        .iter()
                        .any(|s| {
                            s.source == Some((block.id, ci))
                                && s.request == Some("prepare-return-join")
                        })
                {
                    out.shared_return_sites += 1;
                    continue;
                }
                assert_eq!(p.end, c.start, "nonadjacent MIR spans");
                // A frame/parameter load may consist solely of its retained capture.
                // Its independently checked store/load proof supplies A and N/Z.
                let proof_start = out
                    .frame_words
                    .iter()
                    .find(|s| s.consumer == base + p.start as u32)
                    .map(|s| (s.producer - base) as usize)
                    .or_else(|| {
                        out.parameter_words
                            .iter()
                            .find(|s| s.consumer == base + p.start as u32)
                            .map(|s| (s.producer - base) as usize)
                    })
                    .unwrap_or(p.start);
                assert!(p.end >= proof_start + 4);
                let store = p.end - 2;
                assert_eq!(code[store..p.end], homes::store(slot));
                assert_eq!(ins[&store], (2, false));
                let (&last, &(_, m8)) = ins
                    .range(proof_start..store)
                    .rev()
                    .find(|&(at, _)| !matches!(code[*at], 0x83 | 0x85))
                    .unwrap();
                assert!(!m8);
                assert!(
                    matches!(
                        code[last],
                        0xa3 | 0xa5
                            | 0xaf
                            | 0x63
                            | 0x65
                            | 0x69
                            | 0xe3
                            | 0xe5
                            | 0xe9
                            | 0x23
                            | 0x03
                            | 0x43
                            | 0x25
                            | 0x05
                            | 0x45
                            | 0x29
                            | 0x09
                            | 0x49
                    ),
                    "producer must establish full word and N/Z"
                );
                assert!(
                    !m.code
                        .labels
                        .values()
                        .any(|&at| store < at && at <= c.start),
                    "label breaks adjacency"
                );
                let loaded = code[c.start..].starts_with(&homes::load((true, slot)));
                let consumer = c.start + if loaded { 2 } else { 0 };
                assert_eq!(ins[&consumer].1, false);
                assert!(match kind {
                    Kind::Arithmetic => matches!(
                        code[consumer],
                        0x18 | 0x38 | 0x23 | 0x03 | 0x43 | 0x25 | 0x05 | 0x45 | 0x29 | 0x09 | 0x49
                    ),
                    Kind::Compare =>
                        matches!(code[consumer], 0xc3 | 0xc5 | 0xc9)
                            || matches!(code[consumer], 0xd0 | 0xf0)
                                && matches!(
                                    &block.ops[ci],
                                    Mir65816Op::Compare {
                                        operation: NirCompareOp::Eq | NirCompareOp::Ne,
                                        right: Mir65816Value::U16(0) | Mir65816Value::U8(0),
                                        ..
                                    }
                                ),
                    Kind::Store => matches!(code[consumer], 0x83 | 0x8f),
                    Kind::Return => matches!(code[consumer], 0xa8 | 0x6b),
                });
                let end = consumer + ins[&consumer].0;
                let mut bytes: Vec<_> = code[proof_start..end].iter().copied().map(Some).collect();
                for fix in &m.code.fixups {
                    for at in fix.offset..fix.offset + if fix.byte.is_some() { 1 } else { 3 } {
                        if (proof_start..end).contains(&at) {
                            bytes[at - proof_start] = None;
                        }
                    }
                }
                let s = Site {
                    routine: r.id,
                    temp: dest,
                    range: base..base + code.len() as u32,
                    producer: base + proof_start as u32,
                    store: base + store as u32,
                    start: base + c.start as u32,
                    load: loaded.then_some(base + c.start as u32),
                    consumer: base + consumer as u32,
                    slot,
                    kind,
                    bytes,
                };
                assert!(out.insert(s.start, s).is_none());
            }
        }
    }
    out
}

pub fn compiled(p: &native65816::Prepared, c: &native65816::Compiled) -> Index {
    index(&p.mir, &c.machine, |id| {
        c.image
            .routines
            .iter()
            .find(|r| r.id == id.0)
            .unwrap()
            .address
    })
}
pub fn compile(source: &str, optimize: bool) -> (Image, Index) {
    let p = prepare(source, optimize);
    let c = p.compile(&layout()).unwrap();
    let sites = compiled(&p, &c);
    (
        Image::from_json(&c.image.to_json().unwrap()).unwrap(),
        sites,
    )
}

pub fn o65(source: &str, optimize: bool) -> (Vec<u8>, Index) {
    let p = prepare(source, optimize);
    let m = actionc::mir65816::emit::materialize(&p.mir).unwrap();
    let templates = index(&p.mir, &m, |id| {
        0x10000 * (1 + m.routines.iter().position(|r| r.id == id).unwrap() as u32)
    });
    let bytes = p.compile_o65(&Default::default()).unwrap().bytes;
    (bytes, templates)
}
pub fn relocated(templates: &Index, image: &actionc::mir65816::o65::RelocatedImage) -> Index {
    let words = templates
        .values()
        .map(|s| {
            let r = image
                .profile()
                .routines
                .iter()
                .find(|r| r.id == s.routine.0)
                .unwrap();
            let site = s.rebase(image.routine_address(r));
            (site.start, site)
        })
        .collect();
    Index {
        shared_return_sites: templates.shared_return_sites,
        words,
        control: control_flow::relocated(&templates.control, image),
        x_words: templates
            .x_words
            .iter()
            .map(|s| {
                let r = image
                    .profile()
                    .routines
                    .iter()
                    .find(|r| r.id == s.routine.0)
                    .unwrap();
                s.rebase(image.routine_address(r))
            })
            .collect(),
        parameter_words: templates
            .parameter_words
            .iter()
            .map(|s| {
                let r = image
                    .profile()
                    .routines
                    .iter()
                    .find(|r| r.id == s.routine.0)
                    .unwrap();
                s.rebase(image.routine_address(r))
            })
            .collect(),
        frame_words: templates
            .frame_words
            .iter()
            .map(|s| {
                let r = image
                    .profile()
                    .routines
                    .iter()
                    .find(|r| r.id == s.routine.0)
                    .unwrap();
                s.rebase(image.routine_address(r))
            })
            .collect(),
        multi_words: templates
            .multi_words
            .iter()
            .map(|s| {
                let r = image
                    .profile()
                    .routines
                    .iter()
                    .find(|r| r.id == s.routine.0)
                    .unwrap();
                s.rebase(image.routine_address(r))
            })
            .collect(),
        dispatches: control_flow::relocated_dispatches(&templates.dispatches, image),
    }
}
