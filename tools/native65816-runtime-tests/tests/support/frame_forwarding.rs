//! Independent MIR/address + final-byte evidence for frame store/load forwarding.
//! No selector witness or state-tracker claims are consumed here.
use super::*;
use actionc::{
    mir65816::{
        emit::{Location, MachineProgram},
        *,
    },
    nir::RoutineId,
};
use std::ops::Range;

#[derive(Clone, Debug)]
pub struct Site {
    pub routine: RoutineId,
    pub range: Range<u32>,
    pub producer: u32,
    pub store: u32,
    pub consumer: u32,
    pub source: u8,
    pub destination: u8,
    pub bytes: Vec<Option<u8>>,
}
impl Site {
    pub fn valid(&self, bus: &Bus) -> bool {
        self.range.contains(&self.producer)
            && self.range.contains(&(self.consumer + 1))
            && self
                .bytes
                .iter()
                .enumerate()
                .all(|(i, b)| b.is_none_or(|b| bus.ram[self.producer as usize + i] == b))
            && bus.ram[self.store as usize..self.store as usize + 2] == [0x83, self.source]
            && bus.ram[self.consumer as usize..self.consumer as usize + 2]
                == [0x83, self.destination]
    }
    pub fn rebase(&self, base: u32) -> Self {
        let mut s = self.clone();
        let old = self.range.start;
        s.range = base..base + self.range.end - old;
        s.producer = base + self.producer - old;
        s.store = base + self.store - old;
        s.consumer = base + self.consumer - old;
        s
    }
    pub fn assert_live(&self, cpu: &Machine, bus: &Bus) {
        assert!(self.valid(bus));
        let r = cpu.registers();
        assert_eq!(cpu.pc(), self.consumer);
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
                .frame_words
                .iter()
                .find(|s| s.consumer == cpu.pc())
        })
        .flatten()
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
            for (i, ops) in b.ops.windows(2).enumerate() {
                let (
                    Mir65816Op::Store {
                        address: a,
                        value: Mir65816Value::Temp(_, w),
                        width: sw,
                        volatile: false,
                    },
                    Mir65816Op::Load {
                        address: l,
                        dest,
                        width: lw,
                        volatile: false,
                    },
                ) = (&ops[0], &ops[1])
                else {
                    continue;
                };
                if a != l || a.index.is_some() || w.get() != 2 || sw.get() != 2 || lw.get() != 2 {
                    continue;
                }
                let Mir65816AddressBase::AutomaticFrame(id) = a.base else {
                    continue;
                };
                let object = r.frame.objects.iter().find(|o| o.id == id).unwrap();
                if object.addressable {
                    continue;
                }
                assert!(a.displacement.get().checked_add(2).unwrap() <= object.size.get());
                let source =
                    u8::try_from(object.stack_offset.get() + a.displacement.get()).unwrap();
                assert!((1..=254).contains(&source));
                let Location::Stack(home) = m.frame.temps[dest] else {
                    continue;
                };
                assert_eq!(home.width, 2);
                let destination = u8::try_from(home.offset).unwrap();
                let p = &m.code.mir_spans[&(b.id, i)];
                let c = &m.code.mir_spans[&(b.id, i + 1)];
                assert_eq!(p.end, c.start);
                assert_eq!(
                    &code[c.clone()],
                    &[0x83, destination],
                    "eligible frame load must retain only its capture"
                );
                let store = p.end - 2;
                assert_eq!(&code[store..p.end], &[0x83, source]);
                // Walk only consecutive word STA instructions back to a word
                // LDA/ADC/SBC. Every intervening operation preserves A and N/Z.
                let mut producer = store;
                loop {
                    assert_eq!(ins[&producer].1, false);
                    if code[producer] != 0x83 {
                        break;
                    }
                    producer = *ins.range(..producer).next_back().unwrap().0;
                }
                assert!(matches!(
                    code[producer],
                    0xa3 | 0xaf | 0x69 | 0x63 | 0xe9 | 0xe3
                ));
                assert!(
                    !m.code
                        .labels
                        .values()
                        .any(|&at| producer < at && at <= c.start)
                );
                let mut bytes: Vec<_> = code[producer..c.end].iter().copied().map(Some).collect();
                for fix in &m.code.fixups {
                    for at in fix.offset..fix.offset + if fix.byte.is_some() { 1 } else { 3 } {
                        if (producer..c.end).contains(&at) {
                            bytes[at - producer] = None
                        }
                    }
                }
                out.push(Site {
                    routine: r.id,
                    range: base..base + code.len() as u32,
                    producer: base + producer as u32,
                    store: base + store as u32,
                    consumer: base + c.start as u32,
                    source,
                    destination,
                    bytes,
                });
            }
        }
    }
    out
}
