//! Original MC68000 physical instructions. No assembly-text parsing occurs.
use super::{Mir68kDataId, Mir68kFramePlan};
use crate::nir::RoutineId;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Width {
    Byte,
    Word,
    Long,
}
impl Width {
    pub fn bytes(self) -> u32 {
        match self {
            Self::Byte => 1,
            Self::Word => 2,
            Self::Long => 4,
        }
    }
    pub fn from_bytes(bytes: u32) -> Result<Self, String> {
        match bytes {
            1 => Ok(Self::Byte),
            2 => Ok(Self::Word),
            4 => Ok(Self::Long),
            _ => Err(format!("unsupported scalar width {bytes}")),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MachineBlockId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    Block(MachineBlockId),
    Routine(RoutineId),
    Data(Mir68kDataId),
    Absolute(u32),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Address {
    pub target: Target,
    pub addend: i64,
}
impl Address {
    pub fn new(target: Target) -> Self {
        Self { target, addend: 0 }
    }
    pub fn absolute(address: u32) -> Self {
        Self::new(Target::Absolute(address))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ea {
    D(u8),
    A(u8),
    Indirect(u8),
    Displacement(u8, i16),
    Absolute(Address),
    Immediate(u32),
    ImmediateAddress(Address),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Condition {
    High = 2,
    LowOrSame = 3,
    CarryClear = 4,
    CarrySet = 5,
    NotEqual = 6,
    Equal = 7,
    OverflowClear = 8,
    OverflowSet = 9,
    Plus = 10,
    Minus = 11,
    GreaterOrEqual = 12,
    Less = 13,
    Greater = 14,
    LessOrEqual = 15,
}
impl Condition {
    pub fn inverse_bits(self) -> u16 {
        (self as u16) ^ 1
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Alu {
    Add,
    Sub,
    And,
    Or,
    Xor,
    Compare,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShiftCount {
    Immediate(u8),
    Register(u8),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Instruction {
    /// Original MC68000 MULU.W: low 16-bit inputs, full 32-bit product.
    MultiplyUnsignedWord {
        source: u8,
        destination: u8,
    },
    AddAddress {
        source: Ea,
        destination: u8,
    },
    Alu {
        operation: Alu,
        width: Width,
        source: u8,
        destination: u8,
    },
    CompareImmediate {
        width: Width,
        value: u32,
        destination: u8,
    },
    Negate {
        width: Width,
        register: u8,
    },
    Extend {
        to: Width,
        register: u8,
    },
    SetCondition {
        condition: Condition,
        register: u8,
    },
    LogicalShift {
        width: Width,
        left: bool,
        count: ShiftCount,
        register: u8,
    },
    Move {
        width: Width,
        source: Ea,
        destination: Ea,
    },
    Lea {
        source: Ea,
        destination: u8,
    },
    Jump(Ea),
    Jsr(Ea),
    /// Inverse short Bcc over an absolute-long JMP: always eight bytes.
    Branch {
        condition: Condition,
        target: Address,
    },
    Link {
        register: u8,
        displacement: i16,
    },
    Unlink(u8),
    Rts,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MachineBlock {
    pub id: MachineBlockId,
    pub instructions: Vec<Instruction>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MachineRoutine {
    pub id: RoutineId,
    pub entry: MachineBlockId,
    pub frame: Mir68kFramePlan,
}
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MachineProgram {
    pub blocks: Vec<MachineBlock>,
    pub routines: Vec<MachineRoutine>,
}
