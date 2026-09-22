//! Admitted native instruction forms; operand encoding size is not CPU width.
use super::super::abi::ResultLocation;
use super::effects::CallContract;
use super::{Label, Target};

macro_rules! instruction_set {
    ($name:ident { $($variant:ident = $byte:literal),* $(,)? }) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        #[cfg_attr(not(any(test, feature = "native65816-state-proof")), allow(dead_code))]
        pub(super) enum $name { $($variant),* }
        impl $name { pub(super) fn opcode(self) -> u8 { match self { $(Self::$variant => $byte),* } } }
    };
}
instruction_set!(Implied { Clc=0x18, Sec=0x38, Tcs=0x1b, Tsc=0x3b, Tax=0xaa,
    Tay=0xa8, Tya=0x98, Txa=0x8a, Xba=0xeb, Phk=0x4b, Pha=0x48,
    DecA=0x3a, Rtl=0x6b, Dex=0xca, Inx=0xe8, Nop=0xea });
instruction_set!(ByteOp { LdaImm=0xa9, AdcImm=0x69, SbcImm=0xe9, CmpImm=0xc9,
    EorImm=0x49, LdaStack=0xa3, StaStack=0x83, AdcStack=0x63, SbcStack=0xe3,
    CmpStack=0xc3, LdaDp=0xa5, StaDp=0x85, LdxDp=0xa6, AdcDp=0x65,
    SbcDp=0xe5, CmpDp=0xc5, AndDp=0x25, OraDp=0x05, EorDp=0x45,
    AslDp=0x06, RolDp=0x26, LsrDp=0x46, RorDp=0x66,
    LdaIndirect=0xa7, StaIndirect=0x87, LdaIndirectY=0xb7, StaIndirectY=0x97,
    Rep=0xc2, Sep=0xe2 });
instruction_set!(WordOp { LdaImm=0xa9, AdcImm=0x69, SbcImm=0xe9, CmpImm=0xc9,
    AndImm=0x29, LdyImm=0xa0, CpxImm=0xe0 });
instruction_set!(LongOp { Lda=0xaf, Sta=0x8f });
instruction_set!(ReferenceOp { LdaLong=0xaf, StaLong=0x8f, LdaByte=0xa9, Jsl=0x22, Jml=0x5c });
instruction_set!(Branch { Plus=0x10, CarryClear=0x90, CarrySet=0xb0, NotEqual=0xd0, Equal=0xf0 });

/// Compound transfers retain their instruction-level phases and ABI summary.
#[derive(Clone, Debug)]
pub(super) enum Instruction {
    Implied(Implied),
    Byte(ByteOp, u8),
    Word(WordOp, u16),
    Long(LongOp, u32),
    Reference(ReferenceOp, Target, u32, Option<u8>),
    Branch(Branch, Label),
    PushReturn(Label),
    IndirectTransfer(Option<CallContract>),
    NativeCall(Target, CallContract),
    NativeReturn(Option<ResultLocation>),
}
