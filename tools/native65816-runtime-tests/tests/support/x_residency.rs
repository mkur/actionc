//! Independent MIR identities plus final bytes. No production LoopXPlan access.
use super::*;
use actionc::mir65816::{emit::*, *};
use std::ops::Range;

#[derive(Clone, Debug)]
pub struct Site {
    pub routine: actionc::nir::RoutineId,
    pub range: Range<u32>,
    pub compare: u32,
    pub load: u32,
    pub home: u16,
    pub threshold: u16,
    pub refresh: Vec<u32>,
    pub load_bytes: Vec<u8>,
}
fn immediate(v: &Mir65816Value) -> Option<u16> {
    match v {
        Mir65816Value::U8(v) => Some((*v).into()),
        Mir65816Value::U16(v) => Some(*v),
        _ => None,
    }
}
pub fn index(
    p: &Mir65816Program,
    machine: &MachineProgram,
    address: impl Fn(actionc::nir::RoutineId) -> u32,
) -> Vec<Site> {
    verify_program(p).unwrap();
    let mut out = vec![];
    for m in &machine.routines {
        let r = p.routines.iter().find(|r| r.id == m.id).unwrap();
        let base = address(m.id);
        for (bi, b) in r.blocks.iter().enumerate() {
            for (i, op) in b.ops.iter().enumerate() {
                let span = &m.code.mir_spans[&(b.id, i)];
                let mut at = span.start;
                if m.code.bytes.get(at..at + 2) == Some(&[0xc2, 0x20]) {
                    at += 2;
                }
                if m.code.bytes.get(at) != Some(&0xe0) {
                    continue;
                }
                let Mir65816Op::Compare {
                    dest,
                    width,
                    signed: false,
                    operation,
                    left: Mir65816Value::Temp(param, w),
                    right,
                } = op
                else {
                    panic!("CPX without unsigned typed comparison")
                };
                assert_eq!((width.get(), w.get()), (2, 2));
                let threshold = match operation {
                    actionc::nir::NirCompareOp::Lt => immediate(right).unwrap(),
                    actionc::nir::NirCompareOp::Le => {
                        immediate(right).unwrap().checked_add(1).unwrap()
                    }
                    _ => panic!("CPX predicate"),
                };
                assert_eq!(
                    m.code.bytes[at..at + 3],
                    [0xe0, threshold as u8, (threshold >> 8) as u8]
                );
                assert!(
                    matches!(&b.terminator,Mir65816Terminator::Branch{condition:Mir65816Value::Temp(id,_),..} if id==dest)
                );
                assert_eq!(b.params.last().unwrap().0, *param);
                let home = homes::of(m.frame.temps[param]).unwrap();
                assert!((288..=318).contains(&home));
                let mut updates = vec![];
                for block in &r.blocks {
                    for (j, op) in block.ops.iter().enumerate() {
                        if let Mir65816Op::Binary {
                            dest,
                            width,
                            operation: actionc::nir::NirBinaryOp::Add,
                            left: Mir65816Value::Temp(id, _),
                            right,
                            signed: false,
                        } = op
                        {
                            if id != param {
                                continue;
                            }
                            assert_eq!((width.get(), immediate(right)), (2, Some(1)));
                            let s = &m.code.mir_spans[&(block.id, j)];
                            let mut lo = s.start;
                            if m.code.bytes.get(lo..lo + 2) == Some(&[0xc2, 0x20]) {
                                lo += 2;
                            }
                            let mut expected = vec![0x8a, 0x18, 0x69, 1, 0];
                            expected.extend(homes::store(homes::of(m.frame.temps[dest]).unwrap()));
                            assert_eq!(m.code.bytes[lo..s.end], expected);
                            assert!(
                                matches!(&block.terminator,Mir65816Terminator::Goto(e) if e.target==b.id && matches!(e.args.last(),Some(Mir65816Value::Temp(id,_)) if id==dest))
                            );
                            updates.push((base + lo as u32, expected));
                        }
                    }
                }
                assert_eq!(updates.len(), 1);
                let (load, load_bytes) = updates.pop().unwrap();
                let mut refresh = vec![];
                for t in &m.code.mir_transfers {
                    if t.target == Label(bi as u32) {
                        assert_eq!(m.code.bytes[t.offset - 1], 0xaa);
                        assert!(
                            m.code.bytes[t.offset - 3..t.offset - 1] == homes::store(home)
                                || m.code.bytes[t.offset - 3..t.offset - 1]
                                    == homes::load((true, home))
                        );
                        refresh.push(base + t.offset as u32 - 1);
                    }
                }
                assert_eq!(refresh.len(), 2);
                out.push(Site {
                    routine: m.id,
                    range: base..base + m.code.bytes.len() as u32,
                    compare: base + at as u32,
                    load,
                    home,
                    threshold,
                    refresh,
                    load_bytes,
                });
            }
        }
    }
    out
}
impl Site {
    pub fn rebase(&self, base: u32) -> Self {
        let mut s = self.clone();
        let old = s.range.start;
        s.range = base..base + s.range.end - old;
        s.compare = base + s.compare - old;
        s.load = base + s.load - old;
        s.refresh = s.refresh.iter().map(|p| base + p - old).collect();
        s
    }
    pub fn valid(&self, bus: &Bus) -> bool {
        let at = self.compare as usize;
        bus.ram[at..at + 3] == [0xe0, self.threshold as u8, (self.threshold >> 8) as u8]
            && bus.ram[self.load as usize..self.load as usize + self.load_bytes.len()]
                == self.load_bytes
            && self.refresh.iter().all(|&p| {
                bus.ram[p as usize] == 0xaa
                    && (bus.ram[p as usize - 2..p as usize] == homes::store(self.home)
                        || bus.ram[p as usize - 2..p as usize] == homes::load((true, self.home)))
            })
    }
    pub fn assert_live(&self, cpu: &Machine, bus: &Bus) {
        assert!(self.valid(bus));
        let r = cpu.registers();
        assert_eq!(r.p & 0x30, 0);
        assert_eq!(
            u32::from(r.x),
            bus.value(homes::address(r.s, r.d, self.home), 2),
            "X/home relation"
        );
    }
}
pub fn reached<'a>(cpu: &Machine, bus: &'a Bus) -> Option<&'a Site> {
    let s = bus
        .forwarded_words
        .x_words
        .iter()
        .find(|s| s.load == cpu.pc())?;
    s.valid(bus).then_some(s)
}
pub fn compare<'a>(cpu: &Machine, bus: &'a Bus) -> Option<&'a Site> {
    let s = bus
        .forwarded_words
        .x_words
        .iter()
        .find(|s| s.compare == cpu.pc())?;
    if !s.valid(bus) {
        return None;
    }
    s.assert_live(cpu, bus);
    Some(s)
}
