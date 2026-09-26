//! Independent typed MIR + final-byte proofs. Never consult emitter witnesses.
use super::*;
use actionc::{
    mir65816::{emit::MachineProgram, *},
    nir::{ParamId, RoutineId, TempId},
};
use std::ops::Range;

#[derive(Clone, Debug)]
pub struct Site {
    pub routine: RoutineId,
    pub parameter: ParamId,
    pub range: Range<u32>,
    pub producer: u32,
    pub stores: Vec<u32>,
    pub consumer: u32,
    pub source: u8,
    pub destination: u16,
    pub bridge: bool,
    pub bytes: Vec<u8>,
}
impl Site {
    pub fn valid(&self, bus: &Bus) -> bool {
        self.range.contains(&self.producer)
            && self.range.contains(&(self.consumer + 1))
            && bus.ram[self.producer as usize..self.producer as usize + self.bytes.len()]
                == self.bytes
            && bus.ram[self.producer as usize..self.producer as usize + 2] == [0xa3, self.source]
            && bus.ram[self.consumer as usize..self.consumer as usize + 2]
                == homes::store(self.destination)
    }
    pub fn rebase(&self, base: u32) -> Self {
        let mut s = self.clone();
        let old = self.range.start;
        s.range = base..base + self.range.end - old;
        s.producer = base + self.producer - old;
        s.consumer = base + self.consumer - old;
        s.stores = self.stores.iter().map(|&pc| base + pc - old).collect();
        s
    }
    pub fn assert_live(&self, cpu: &Machine, bus: &Bus) {
        assert!(self.valid(bus));
        assert_eq!(cpu.pc(), self.consumer);
        let r = cpu.registers();
        assert_eq!(r.p & 0x30, 0);
        assert_eq!(
            u32::from(r.a),
            bus.value(u32::from(r.s) + u32::from(self.source), 2)
        );
        assert_eq!(
            r.p & 0x82,
            if r.a == 0 { 2 } else { 0 } | if r.a & 0x8000 != 0 { 0x80 } else { 0 }
        );
    }
}
pub fn reached<'a>(cpu: &Machine, bus: &'a Bus) -> Option<&'a Site> {
    cpu.is_instruction_boundary()
        .then(|| {
            bus.forwarded_words
                .parameter_words
                .iter()
                .find(|s| s.consumer == cpu.pc())
        })
        .flatten()
}
fn load(
    op: &Mir65816Op,
    r: &Mir65816Routine,
    m: &actionc::mir65816::emit::MachineRoutine,
) -> Option<(ParamId, u8, TempId, u16)> {
    let Mir65816Op::Load {
        dest,
        address,
        width,
        volatile: false,
    } = op
    else {
        return None;
    };
    let Mir65816AddressBase::Parameter(id) = address.base else {
        return None;
    };
    if width.get() != 2 || address.index.is_some() || address.displacement.get() != 0 {
        return None;
    }
    let p = r.frame.parameters.iter().find(|p| p.param == id).unwrap();
    let Mir65816AbiHome::StackArgument { offset, size, .. } = p.incoming else {
        return None;
    };
    if p.frame_object.is_some()
        || size.get() != 2
        || r.frame
            .objects
            .iter()
            .any(|o| o.owner == Mir65816FrameObjectOwner::Param(id))
    {
        return None;
    }
    for op in r.blocks.iter().flat_map(|b| &b.ops) {
        let refers = |a: &Mir65816Address| a.base == Mir65816AddressBase::Parameter(id);
        let unsafe_use = match op {
            Mir65816Op::Load {
                address,
                width,
                volatile,
                ..
            } => {
                refers(address)
                    && (*volatile
                        || width.get() != 2
                        || address.index.is_some()
                        || address.displacement.get() != 0)
            }
            Mir65816Op::Store { address, .. } | Mir65816Op::AddressOf { address, .. } => {
                refers(address)
            }
            Mir65816Op::Copy {
                source,
                destination,
                ..
            } => refers(source) || refers(destination),
            _ => false,
        };
        if unsafe_use {
            return None;
        }
    }
    // Native argument starts one byte above the three-byte return address.
    let source = u8::try_from(u32::from(m.frame.extent) + 4 + offset.get()).unwrap();
    let capture = homes::of(m.frame.temps[dest])?;
    assert!((1..=254).contains(&source));
    assert!(u16::from(source).abs_diff(capture) >= 2);
    Some((id, source, *dest, capture))
}
pub fn index(
    mir: &Mir65816Program,
    machine: &MachineProgram,
    address: impl Fn(RoutineId) -> u32,
) -> Vec<Site> {
    let mut out = vec![];
    for m in &machine.routines {
        let r = mir.routines.iter().find(|r| r.id == m.id).unwrap();
        let base = address(r.id);
        let code = &m.code.bytes;
        let ins = forwarding::instructions(code);
        for b in &r.blocks {
            for (i, op) in b.ops.iter().enumerate() {
                let Some((id, source, temp, capture)) = load(op, r, m) else {
                    continue;
                };
                let p = &m.code.mir_spans[&(b.id, i)];
                if p.is_empty() {
                    // Adjacent incoming comparisons have no capture to track.
                    assert!(
                        matches!(b.ops.get(i+1),Some(Mir65816Op::Compare{left:Mir65816Value::Temp(t,_),..}) if *t==temp)
                            || matches!(b.ops.get(i+1),Some(Mir65816Op::Compare{right:Mir65816Value::Temp(t,_),..}) if *t==temp)
                    );
                    continue;
                }
                if code[p.clone()] == homes::store(capture) {
                    continue;
                } // Omitted consumer cannot rearm.
                let start = p.start + usize::from(code[p.clone()].starts_with(&[0xc2, 0x20])) * 2;
                let mut expected = vec![0xa3, source];
                expected.extend(homes::store(capture));
                assert_eq!(code[start..p.end], expected);
                let mut stores = vec![base + start as u32 + 2];
                let mut j = i + 1;
                let mut bridge = false;
                if let Some(Mir65816Op::Store {
                    address,
                    value: Mir65816Value::Temp(t, w),
                    width,
                    volatile: false,
                }) = b.ops.get(j)
                {
                    if *t != temp || w.get() != 2 || width.get() != 2 || address.index.is_some() {
                        continue;
                    }
                    let Mir65816AddressBase::AutomaticFrame(object) = address.base else {
                        continue;
                    };
                    let o = r.frame.objects.iter().find(|o| o.id == object).unwrap();
                    if o.addressable {
                        continue;
                    }
                    assert!(address.displacement.get() + 2 <= o.size.get());
                    let dest =
                        u8::try_from(o.stack_offset.get() + address.displacement.get()).unwrap();
                    if dest.abs_diff(source) < 2 || u16::from(dest).abs_diff(capture) < 2 {
                        continue;
                    }
                    let s = &m.code.mir_spans[&(b.id, j)];
                    assert_eq!(s.start, p.end);
                    assert_eq!(code[s.clone()], [0x83, dest]);
                    expected.extend([0x83, dest]);
                    stores.push(base + s.start as u32);
                    j += 1;
                    bridge = true;
                }
                let Some((other, home, _, destination)) =
                    b.ops.get(j).and_then(|op| load(op, r, m))
                else {
                    continue;
                };
                if other != id || home != source {
                    continue;
                }
                let c = &m.code.mir_spans[&(b.id, j)];
                assert_eq!(c.start, start + expected.len());
                expected.extend(homes::store(destination));
                assert_eq!(
                    code[start..c.end],
                    expected,
                    "parameter forwarding must retain exactly its stores"
                );
                assert!(
                    !m.code
                        .labels
                        .values()
                        .any(|&pc| start < pc && pc <= c.start)
                );
                for pc in (start..c.end).step_by(2) {
                    assert_eq!(ins[&pc], (2, false));
                }
                out.push(Site {
                    routine: r.id,
                    parameter: id,
                    range: base..base + code.len() as u32,
                    producer: base + start as u32,
                    stores,
                    consumer: base + c.start as u32,
                    source,
                    destination,
                    bridge,
                    bytes: expected,
                });
            }
        }
    }
    out
}

/// Keep the raw typed worker in an otherwise independently optimized program.
/// This deliberately probes target emission when source optimization removes the
/// repeated loads. IDs, signature and parameter plans come from the same source.
pub fn prepared(source: &str, optimize: bool) -> native65816::Prepared {
    let raw = prepare(source, false);
    let mut p = prepare(source, optimize);
    for r in &mut p.mir.routines {
        if r.name.to_ascii_lowercase().contains("parampair")
            || r.name.to_ascii_lowercase().contains("parambridge")
        {
            let original = raw.mir.routines.iter().find(|s| s.id == r.id).unwrap();
            assert_eq!(r.name, original.name);
            *r = original.clone();
        }
    }
    actionc::mir65816::verify_program(&p.mir).unwrap();
    p
}
