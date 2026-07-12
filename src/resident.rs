#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResidentVariableKind {
    Byte,
    ByteArray { len: u16 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResidentVariableStorage {
    Absolute,
    ZeroPage,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResidentVariable {
    pub name: &'static str,
    pub address: u16,
    pub kind: ResidentVariableKind,
    pub storage: ResidentVariableStorage,
}

pub const RESIDENT_VARIABLES: &[ResidentVariable] = &[
    ResidentVariable {
        name: "COLOR",
        address: 0x02FD,
        kind: ResidentVariableKind::Byte,
        storage: ResidentVariableStorage::Absolute,
    },
    ResidentVariable {
        name: "DEVICE",
        address: 0x00B7,
        kind: ResidentVariableKind::Byte,
        storage: ResidentVariableStorage::ZeroPage,
    },
    ResidentVariable {
        name: "EOF",
        address: 0x05C0,
        kind: ResidentVariableKind::ByteArray { len: 8 },
        storage: ResidentVariableStorage::Absolute,
    },
    ResidentVariable {
        name: "LIST",
        address: 0x049A,
        kind: ResidentVariableKind::Byte,
        storage: ResidentVariableStorage::Absolute,
    },
    ResidentVariable {
        name: "TRACE",
        address: 0x04C3,
        kind: ResidentVariableKind::Byte,
        storage: ResidentVariableStorage::Absolute,
    },
];

pub fn resident_variable(name: &str) -> Option<&'static ResidentVariable> {
    RESIDENT_VARIABLES
        .iter()
        .find(|variable| variable.name.eq_ignore_ascii_case(name))
}
