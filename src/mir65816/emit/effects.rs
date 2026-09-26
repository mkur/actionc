//! Physical effects of admitted native forms, independent of forward-value
//! precision. No effect is supplied by a selector or inferred from raw bytes.
//! Later foundation slices consume these conservative facts for liveness.
#![cfg_attr(not(any(test, feature = "native65816-state-proof")), allow(dead_code))]
use super::super::abi::{self, FarTransfer, ResultLocation};
use super::selected::*;
use super::state::{Environment, Width};
use super::{Label, Mir65816AbiHome, Mir65816CallPlan, Target};

pub const C: u8 = 0x01;
pub const Z: u8 = 0x02;
pub const V: u8 = 0x40;
pub const N: u8 = 0x80;
pub const NZ: u8 = N | Z;
pub const NZCV: u8 = NZ | C | V;

/// Masks describe physical bits, including the hidden accumulator byte.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Registers {
    pub a: u16,
    pub x: u16,
    pub y: u16,
}
impl Registers {
    pub const ALL: Self = Self {
        a: 0xffff,
        x: 0xffff,
        y: 0xffff,
    };
}

/// Protected execution environment. These effects never grant rewrite permission.
pub mod env {
    pub const S: u16 = 1;
    pub const D: u16 = 2;
    pub const DBR: u16 = 4;
    pub const PBR: u16 = 8;
    pub const E: u16 = 16;
    pub const M: u16 = 32;
    pub const X: u16 = 64;
    pub const DECIMAL: u16 = 128;
    pub const I: u16 = 256;
    pub const PC: u16 = 512;
    pub const ALL: u16 = 1023;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Access {
    Read,
    /// A known destination is written, independently of the value written.
    Write,
    /// Unknown destination or callee clobber; cannot kill a reaching definition.
    MayWrite,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Memory {
    /// Relative to S at this instruction's entry, before any push/pop.
    Stack {
        displacement: i32,
        bytes: u16,
    },
    /// Native bank-zero D-relative addressing; no page-wrap assumption.
    DirectPage {
        offset: u16,
        bytes: u16,
    },
    Long {
        address: u32,
        bytes: u8,
    },
    Symbol {
        target: Target,
        addend: u32,
        bytes: u8,
    },
    IndirectLong {
        pointer: u8,
        indexed_y: bool,
        bytes: u8,
    },
    Unknown,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MemoryEffect {
    pub access: Access,
    pub memory: Memory,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Control {
    #[default]
    Next,
    Branch {
        predicate: u8,
        target: Label,
    },
    Jump(Target),
    Call {
        target: Option<Target>,
    },
    Return,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct InstructionEffects {
    pub reads: Registers,
    pub writes: Registers,
    /// Defined result bits are distinguished from unspecified call-clobbered bits.
    pub clobbers: Registers,
    pub flag_reads: u8,
    pub flag_writes: u8,
    pub flag_clobbers: u8,
    pub environment_reads: u16,
    pub environment_writes: u16,
    /// Ordered accesses. A read preceding a write cannot be killed by that write.
    pub memory: Vec<MemoryEffect>,
    pub control: Control,
    /// These effects do not prove that observable/unknown accesses can be removed.
    pub barrier: bool,
}
#[cfg(feature = "native65816-state-proof")]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EffectRecord {
    pub start: usize,
    pub end: usize,
    pub effects: InstructionEffects,
}

/// Constructed only from the verified MIR call plan; no caller-provided masks.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct CallContract {
    pub(super) outgoing: u16,
    arguments: Vec<(u16, u16)>,
    result: Option<ResultLocation>,
}
impl CallContract {
    pub fn result(home: Option<Mir65816AbiHome>) -> Result<Option<ResultLocation>, String> {
        match home {
            None => Ok(None),
            Some(Mir65816AbiHome::NativeResult(result)) => Ok(Some(result)),
            _ => Err("native effect summary requires a native result home".into()),
        }
    }
    pub fn from_plan(plan: &Mir65816CallPlan, transfer: FarTransfer) -> Result<Self, String> {
        let native = plan.native.ok_or("missing native call effect contract")?;
        if native.transfer != transfer
            || native.boundary != abi::BOUNDARY
            || native.argument_bytes.get() > plan.outgoing_bytes.get()
            || native.caller_cleanup_bytes != plan.outgoing_bytes
        {
            return Err("inconsistent native call effect contract".into());
        }
        let mut arguments = Vec::new();
        for home in &plan.arguments {
            let Mir65816AbiHome::StackArgument { offset, size, .. } = *home else {
                return Err("non-stack native argument in effect contract".into());
            };
            if size.get() == 0
                || offset
                    .get()
                    .checked_add(size.get())
                    .is_none_or(|end| end > native.argument_bytes.get())
            {
                return Err("native argument exceeds outgoing effect range".into());
            }
            arguments.push((
                u16::try_from(offset.get()).map_err(|_| "argument effect offset overflow")?,
                u16::try_from(size.get()).map_err(|_| "argument effect size overflow")?,
            ));
        }
        Ok(Self {
            outgoing: u16::try_from(plan.outgoing_bytes.get())
                .map_err(|_| "outgoing effect extent overflow")?,
            arguments,
            result: Self::result(plan.result)?,
        })
    }
}
fn mask(width: Width) -> u16 {
    if width == Width::Byte { 0xff } else { 0xffff }
}
fn result_registers(result: Option<ResultLocation>) -> Registers {
    match result {
        None => Registers::default(),
        Some(ResultLocation::A8ZeroExtended | ResultLocation::A16) => Registers {
            a: 0xffff,
            ..Registers::default()
        },
        // Zero extension of the high X byte is part of the three-byte ABI.
        Some(ResultLocation::A16X8ZeroExtended | ResultLocation::A16X16) => Registers {
            a: 0xffff,
            x: 0xffff,
            y: 0,
        },
    }
}
impl InstructionEffects {
    fn memory(&mut self, access: Access, memory: Memory) {
        self.memory.push(MemoryEffect { access, memory });
    }
    fn stack(&mut self, access: Access, displacement: i32, bytes: u16) {
        self.environment_reads |= env::S;
        self.memory(
            access,
            Memory::Stack {
                displacement,
                bytes,
            },
        );
    }
    fn dp(&mut self, access: Access, offset: u16, bytes: u16) {
        self.environment_reads |= env::D;
        self.memory(access, Memory::DirectPage { offset, bytes });
    }
    fn push(&mut self, bytes: u8) {
        self.stack(Access::Write, 1 - i32::from(bytes), bytes.into());
        self.environment_writes |= env::S;
        self.barrier = true;
    }
    fn load_a(&mut self, width: Width) {
        self.environment_reads |= env::M;
        self.writes.a = mask(width);
        self.flag_writes = NZ;
    }
    fn alu(&mut self, width: Width, carry: bool, store: bool) {
        self.environment_reads |= env::M;
        self.reads.a = mask(width);
        if store {
            self.writes.a = mask(width);
        }
        self.flag_writes = NZ;
        if carry {
            self.environment_reads |= env::DECIMAL;
            self.flag_reads = C;
            self.flag_writes |= C | V;
        }
    }
    fn call(&mut self, contract: Option<&CallContract>, target: Option<Target>, indirect: bool) {
        self.control = Control::Call { target };
        self.barrier = true;
        self.environment_reads |= env::ALL;
        // Native calls restore D/DBR/modes. I may be changed by imports whose
        // interrupt effects are resolved only by linking.
        self.environment_writes |= env::S | env::PC | env::PBR | env::I;
        if indirect {
            // RTL consumes target bytes; callee return consumes the retained
            // PHK/PER continuation, all relative to pre-RTL S.
            self.stack(Access::Read, 1, 3);
        } else {
            self.push(3);
        }
        if let Some(c) = contract {
            for &(offset, bytes) in &c.arguments {
                self.stack(
                    Access::Read,
                    i32::from(offset) + if indirect { 7 } else { 1 },
                    bytes,
                );
            }
            self.writes = result_registers(c.result);
            self.clobbers = Registers {
                a: !self.writes.a,
                x: !self.writes.x,
                y: !self.writes.y,
            };
        } else {
            // Unannotated test/probe calls cannot establish a narrower proof.
            self.reads = Registers::ALL;
            self.clobbers = Registers::ALL;
            self.flag_reads = NZCV;
        }
        self.flag_clobbers = NZCV;
        // Callees may read through pointers before writing/clobbering memory.
        self.memory(Access::Read, Memory::Unknown);
        self.memory(Access::MayWrite, Memory::Unknown);
        self.dp(
            Access::MayWrite,
            abi::generated::DP_SCRATCH_OFFSET as u16,
            abi::generated::DP_SCRATCH_SIZE as u16,
        );
        self.stack(Access::Read, if indirect { 4 } else { -2 }, 3);
    }
}

impl Instruction {
    pub(super) fn effects(&self, state: Environment) -> InstructionEffects {
        let mut e = InstructionEffects {
            environment_reads: env::E,
            ..Default::default()
        };
        let m = state.m;
        let index = state.index;
        match *self {
            Self::ArgumentPush => return Self::Implied(Implied::Pha).effects(state),
            Self::Implied(op) => match op {
                Implied::Clc | Implied::Sec => e.flag_writes = C,
                Implied::Tsc => {
                    e.environment_reads |= env::S;
                    e.writes.a = 0xffff;
                    e.flag_writes = NZ;
                    e.barrier = true;
                }
                Implied::Tcs => {
                    e.reads.a = 0xffff;
                    e.environment_writes |= env::S;
                    e.barrier = true;
                }
                Implied::Tax | Implied::Tay => {
                    e.environment_reads |= env::X;
                    e.reads.a = mask(index);
                    if op == Implied::Tax {
                        e.writes.x = 0xffff;
                    } else {
                        e.writes.y = 0xffff;
                    }
                    e.flag_writes = NZ;
                }
                Implied::Txa | Implied::Tya => {
                    e.load_a(m);
                    if op == Implied::Txa {
                        e.reads.x = mask(m);
                    } else {
                        e.reads.y = mask(m);
                    }
                }
                Implied::Xba => {
                    e.reads.a = 0xffff;
                    e.writes.a = 0xffff;
                    e.flag_writes = NZ;
                }
                Implied::AslA => {
                    e.environment_reads |= env::M;
                    e.reads.a = mask(m);
                    e.writes.a = mask(m);
                    e.flag_writes = NZ | C;
                }
                Implied::Iny => {
                    e.environment_reads |= env::X;
                    e.reads.y = mask(index);
                    e.writes.y = 0xffff;
                    e.flag_writes = NZ;
                }
                Implied::DecA => {
                    e.environment_reads |= env::M;
                    e.reads.a = mask(m);
                    e.writes.a = mask(m);
                    e.flag_writes = NZ;
                }
                Implied::Dex | Implied::Inx => {
                    e.environment_reads |= env::X;
                    e.reads.x = mask(index);
                    e.writes.x = 0xffff;
                    e.flag_writes = NZ;
                }
                Implied::Phk => {
                    e.environment_reads |= env::PBR;
                    e.push(1);
                }
                Implied::Pha => {
                    e.environment_reads |= env::M;
                    e.reads.a = mask(m);
                    e.push(m.bytes());
                }
                Implied::Rtl => {
                    e.reads = Registers::ALL; // No declared result: conservative probe boundary.
                    e.stack(Access::Read, 1, 3);
                    e.environment_reads |= env::ALL;
                    e.environment_writes |= env::S | env::PC | env::PBR;
                    e.control = Control::Return;
                    e.barrier = true;
                }
                Implied::Nop => {}
            },
            Self::Byte(op, value) => {
                use ByteOp::*;
                let width = if op == LdxDp { index } else { m };
                let bytes = u16::from(width.bytes());
                match op {
                    LdaImm | AdcImm | SbcImm | CmpImm | EorImm | Rep | Sep => {}
                    LdaStack | AdcStack | SbcStack | CmpStack | AndStack | OraStack | EorStack => {
                        e.stack(Access::Read, value.into(), bytes)
                    }
                    StaStack => e.stack(Access::Write, value.into(), bytes),
                    LdaDp | LdxDp | AdcDp | SbcDp | CmpDp | AndDp | OraDp | EorDp => {
                        e.dp(Access::Read, value.into(), bytes)
                    }
                    StaDp => e.dp(Access::Write, value.into(), bytes),
                    AslDp | RolDp | LsrDp | RorDp => {
                        e.dp(Access::Read, value.into(), bytes);
                        e.dp(Access::Write, value.into(), bytes);
                    }
                    LdaIndirect | StaIndirect | LdaIndirectY | StaIndirectY => {
                        e.dp(Access::Read, value.into(), 3);
                        let indexed_y = matches!(op, LdaIndirectY | StaIndirectY);
                        if indexed_y {
                            e.reads.y = 0xffff;
                        }
                        e.memory(
                            if matches!(op, LdaIndirect | LdaIndirectY) {
                                Access::Read
                            } else {
                                Access::MayWrite
                            },
                            Memory::IndirectLong {
                                pointer: value,
                                indexed_y,
                                bytes: width.bytes(),
                            },
                        );
                        e.barrier = true;
                    }
                }
                match op {
                    LdaImm | LdaStack | LdaDp | LdaIndirect | LdaIndirectY => e.load_a(m),
                    StaStack | StaDp | StaIndirect | StaIndirectY => {
                        e.environment_reads |= env::M;
                        e.reads.a = mask(m);
                    }
                    LdxDp => {
                        e.environment_reads |= env::X;
                        e.writes.x = 0xffff;
                        e.flag_writes = NZ;
                    }
                    AdcImm | SbcImm | AdcStack | SbcStack | AdcDp | SbcDp => e.alu(m, true, true),
                    CmpImm | CmpStack | CmpDp => {
                        e.alu(m, false, false);
                        e.flag_writes |= C;
                    }
                    AndDp | AndStack | OraDp | OraStack | EorDp | EorStack | EorImm => {
                        e.alu(m, false, true)
                    }
                    AslDp | RolDp | LsrDp | RorDp => {
                        e.environment_reads |= env::M;
                        e.flag_writes = NZ | C;
                        if matches!(op, RolDp | RorDp) {
                            e.flag_reads = C;
                        }
                    }
                    Rep | Sep => {
                        e.flag_writes = value & NZCV;
                        for (bit, part) in [
                            (0x20, env::M),
                            (0x10, env::X),
                            (8, env::DECIMAL),
                            (4, env::I),
                        ] {
                            if value & bit != 0 {
                                e.environment_writes |= part;
                            }
                        }
                        if op == Sep && value & 0x10 != 0 {
                            e.writes.x = 0xff00;
                            e.writes.y = 0xff00;
                        }
                        e.barrier = true;
                    }
                }
            }
            Self::Word(op, _) => {
                use WordOp::*;
                match op {
                    LdaImm => e.load_a(Width::Word),
                    AdcImm | SbcImm => e.alu(Width::Word, true, true),
                    CmpImm => {
                        e.alu(Width::Word, false, false);
                        e.flag_writes |= C;
                    }
                    AndImm | OraImm | EorImm => e.alu(Width::Word, false, true),
                    LdxImm => {
                        e.environment_reads |= env::X;
                        e.writes.x = 0xffff;
                        e.flag_writes = NZ;
                    }
                    LdyImm => {
                        e.environment_reads |= env::X;
                        e.writes.y = 0xffff;
                        e.flag_writes = NZ;
                    }
                    CpxImm => {
                        e.environment_reads |= env::X;
                        e.reads.x = 0xffff;
                        e.flag_writes = NZ | C;
                    }
                }
            }
            Self::Long(op, address) => {
                e.memory(
                    if op == LongOp::Lda {
                        Access::Read
                    } else {
                        Access::Write
                    },
                    Memory::Long {
                        address,
                        bytes: m.bytes(),
                    },
                );
                e.barrier = true;
                if op == LongOp::Lda {
                    e.load_a(m);
                } else {
                    e.environment_reads |= env::M;
                    e.reads.a = mask(m);
                }
            }
            Self::Reference(op, target, addend, _) => match op {
                ReferenceOp::LdaByte => e.load_a(Width::Byte), // Relocated immediate, not a memory read.
                ReferenceOp::LdaLong | ReferenceOp::StaLong => {
                    e.memory(
                        if op == ReferenceOp::LdaLong {
                            Access::Read
                        } else {
                            Access::Write
                        },
                        Memory::Symbol {
                            target,
                            addend,
                            bytes: m.bytes(),
                        },
                    );
                    e.barrier = true;
                    if op == ReferenceOp::LdaLong {
                        e.load_a(m);
                    } else {
                        e.environment_reads |= env::M;
                        e.reads.a = mask(m);
                    }
                }
                ReferenceOp::Jsl => e.call(None, Some(target), false),
                ReferenceOp::Jml => {
                    e.control = Control::Jump(target);
                    e.environment_writes |= env::PC | env::PBR;
                    e.barrier = true;
                }
            },
            Self::Branch(op, target) => {
                e.flag_reads = match op {
                    Branch::Plus | Branch::Minus => N,
                    Branch::OverflowClear => V,
                    Branch::CarryClear | Branch::CarrySet => C,
                    Branch::NotEqual | Branch::Equal => Z,
                };
                e.control = Control::Branch {
                    predicate: op.opcode(),
                    target,
                };
                e.environment_writes |= env::PC | env::PBR;
            }
            Self::PushReturn(_) => {
                e.environment_reads |= env::PC;
                e.push(2);
            }
            Self::IndirectTransfer(ref contract) => e.call(contract.as_ref(), None, true),
            Self::NativeCall(target, ref contract) => e.call(Some(contract), Some(target), false),
            Self::NativeReturn(result) => {
                e.reads = result_registers(result);
                e.stack(Access::Read, 1, 3);
                e.environment_reads |= env::ALL;
                e.environment_writes |= env::S | env::PC | env::PBR;
                e.control = Control::Return;
                e.barrier = true;
            }
        }
        e
    }
}

#[cfg(test)]
#[path = "effects_tests.rs"]
mod tests;
