//! Small encoder with typed symbolic fixups. Instruction encodings follow WDC
//! W65C816S tables 5-4/5-5; no assembler or emulator is used by the compiler.
use super::super::{Mir65816DataId, RoutineId, RuntimeSymbolId};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Label(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    Label(Label),
    Routine(RoutineId),
    Runtime(RuntimeSymbolId),
    Data(Mir65816DataId),
    StackOverflow,
}

#[derive(Debug, Clone)]
pub struct Fixup {
    pub offset: usize,
    pub target: Target,
    pub addend: u32,
    /// None writes a complete 24-bit address, Some selects one numeric byte.
    pub byte: Option<u8>,
}

#[derive(Debug, Clone, Default)]
pub struct Code {
    pub bytes: Vec<u8>,
    pub fixups: Vec<Fixup>,
    pub labels: BTreeMap<Label, usize>,
    next_label: u32,
}

impl Code {
    pub(super) fn label(&mut self) -> Label {
        let label = Label(self.next_label);
        self.next_label += 1;
        label
    }
    pub(super) fn mark(&mut self, label: Label) {
        assert!(self.labels.insert(label, self.bytes.len()).is_none());
    }
    pub(super) fn op(&mut self, opcode: u8) {
        self.bytes.push(opcode);
    }
    pub(super) fn byte(&mut self, opcode: u8, value: u8) {
        self.bytes.extend([opcode, value]);
    }
    pub(super) fn word(&mut self, opcode: u8, value: u16) {
        self.op(opcode);
        self.bytes.extend(value.to_le_bytes());
    }
    pub(super) fn long(&mut self, opcode: u8, address: u32) -> Result<(), String> {
        if address >= 0x1000000 {
            return Err("24-bit instruction address overflow".into());
        }
        self.op(opcode);
        self.bytes.extend(&address.to_le_bytes()[..3]);
        Ok(())
    }
    pub(super) fn reference(&mut self, opcode: u8, target: Target, addend: u32, byte: Option<u8>) {
        self.op(opcode);
        self.fixups.push(Fixup {
            offset: self.bytes.len(),
            target,
            addend,
            byte,
        });
        self.bytes
            .extend(std::iter::repeat_n(0, if byte.is_some() { 1 } else { 3 }));
    }
    pub(super) fn jump(&mut self, label: Label) {
        self.reference(0x5c, Target::Label(label), 0, None); // JML
    }
    /// Short inverse branch skips exactly one JML. No range relaxation or
    /// wraparound is assumed for a MIR control-flow edge.
    pub(super) fn branch(&mut self, opcode: u8, label: Label) {
        self.byte(opcode ^ 0x20, 4);
        self.jump(label);
    }
    pub(super) fn a8(&mut self) {
        self.byte(0xe2, 0x20);
    }
    pub(super) fn a16(&mut self) {
        self.byte(0xc2, 0x20);
    }
}
