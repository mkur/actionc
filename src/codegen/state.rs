use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(super) enum RegisterValue {
    #[default]
    Unknown,
    Immediate(u8),
    Fact(ValueFact),
}

impl RegisterValue {
    pub(super) fn immediate(self) -> Option<u8> {
        match self {
            Self::Unknown => None,
            Self::Immediate(value) => Some(value),
            Self::Fact(ValueFact::Immediate(value)) => Some(value),
            Self::Fact(_) => None,
        }
    }

    pub(super) fn value_fact(self, register: RegisterName) -> ValueFact {
        match self {
            Self::Unknown => ValueFact::Register(register),
            Self::Immediate(value) => ValueFact::Immediate(value),
            Self::Fact(fact) => fact,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(super) enum FlagValue {
    #[default]
    Unknown,
    Known(bool),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum RegisterName {
    A,
    X,
    Y,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum LogicFactOp {
    And,
    Or,
    Xor,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ValueAtomFact {
    Unknown,
    Immediate(u8),
    Register(RegisterName),
    SlotByte { slot: StorageSlot, byte_index: u16 },
    AddressByte { address: u16, byte_index: u16 },
}

impl From<ValueFact> for ValueAtomFact {
    fn from(value: ValueFact) -> Self {
        match value {
            ValueFact::Unknown => Self::Unknown,
            ValueFact::Immediate(value) => Self::Immediate(value),
            ValueFact::Register(register) => Self::Register(register),
            ValueFact::SlotByte { slot, byte_index } => Self::SlotByte { slot, byte_index },
            ValueFact::AddressByte {
                address,
                byte_index,
            } => Self::AddressByte {
                address,
                byte_index,
            },
            ValueFact::Logic { .. } | ValueFact::Subtract { .. } => Self::Register(RegisterName::A),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct ByteCompareFact {
    pub(super) left: ValueAtomFact,
    pub(super) right: ValueAtomFact,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct MemoryByte {
    pub(super) slot: StorageSlot,
    pub(super) byte_index: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct MemoryContentFact {
    pub(super) byte: MemoryByte,
    pub(super) value: ValueFact,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(super) struct TrackedMemory {
    pub(super) values: Vec<MemoryContentFact>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(super) enum ValueFact {
    #[default]
    Unknown,
    Immediate(u8),
    Register(RegisterName),
    SlotByte {
        slot: StorageSlot,
        byte_index: u16,
    },
    AddressByte {
        address: u16,
        byte_index: u16,
    },
    Logic {
        op: LogicFactOp,
        left: ValueAtomFact,
        right: ValueAtomFact,
    },
    Subtract {
        left: ValueAtomFact,
        right: ValueAtomFact,
        borrow: Option<ByteCompareFact>,
    },
}

pub(super) fn value_fact_immediate(value: ValueFact) -> Option<u8> {
    match value {
        ValueFact::Immediate(value) => Some(value),
        ValueFact::AddressByte {
            address,
            byte_index,
        } => Some(Immediate::new(address).byte(byte_index)),
        _ => None,
    }
}

pub(super) fn apply_logic_fact_op(op: LogicFactOp, left: u8, right: u8) -> u8 {
    match op {
        LogicFactOp::And => left & right,
        LogicFactOp::Or => left | right,
        LogicFactOp::Xor => left ^ right,
    }
}

pub(super) fn value_atom_references_memory_byte(value: ValueAtomFact, byte: MemoryByte) -> bool {
    matches!(value, ValueAtomFact::SlotByte { slot, byte_index } if byte == MemoryByte { slot, byte_index })
}

pub(super) fn value_atom_references_register(value: ValueAtomFact, register: RegisterName) -> bool {
    matches!(value, ValueAtomFact::Register(value_register) if value_register == register)
}

pub(super) fn byte_compare_references_memory_byte(
    compare: ByteCompareFact,
    byte: MemoryByte,
) -> bool {
    value_atom_references_memory_byte(compare.left, byte)
        || value_atom_references_memory_byte(compare.right, byte)
}

pub(super) fn byte_compare_references_register(
    compare: ByteCompareFact,
    register: RegisterName,
) -> bool {
    value_atom_references_register(compare.left, register)
        || value_atom_references_register(compare.right, register)
}

pub(super) fn value_fact_references_memory_byte(value: ValueFact, byte: MemoryByte) -> bool {
    match value {
        ValueFact::SlotByte { slot, byte_index } => byte == MemoryByte { slot, byte_index },
        ValueFact::Logic { left, right, .. } => {
            value_atom_references_memory_byte(left, byte)
                || value_atom_references_memory_byte(right, byte)
        }
        ValueFact::Subtract {
            left,
            right,
            borrow,
        } => {
            value_atom_references_memory_byte(left, byte)
                || value_atom_references_memory_byte(right, byte)
                || borrow.is_some_and(|borrow| byte_compare_references_memory_byte(borrow, byte))
        }
        ValueFact::Unknown
        | ValueFact::Immediate(_)
        | ValueFact::Register(_)
        | ValueFact::AddressByte { .. } => false,
    }
}

pub(super) fn value_fact_references_register(value: ValueFact, register: RegisterName) -> bool {
    match value {
        ValueFact::Register(value_register) => value_register == register,
        ValueFact::Logic { left, right, .. } => {
            value_atom_references_register(left, register)
                || value_atom_references_register(right, register)
        }
        ValueFact::Subtract {
            left,
            right,
            borrow,
        } => {
            value_atom_references_register(left, register)
                || value_atom_references_register(right, register)
                || borrow.is_some_and(|borrow| {
                    value_atom_references_register(borrow.left, register)
                        || value_atom_references_register(borrow.right, register)
                })
        }
        ValueFact::Unknown
        | ValueFact::Immediate(_)
        | ValueFact::SlotByte { .. }
        | ValueFact::AddressByte { .. } => false,
    }
}

pub(super) fn compare_fact_references_memory_byte(compare: CompareFact, byte: MemoryByte) -> bool {
    match compare {
        CompareFact::Byte { left, right } => {
            value_fact_references_memory_byte(left, byte)
                || value_fact_references_memory_byte(right, byte)
        }
        CompareFact::WordSubtract { low, high } => {
            byte_compare_references_memory_byte(low, byte)
                || byte_compare_references_memory_byte(high, byte)
        }
    }
}

pub(super) fn compare_fact_references_register(
    compare: CompareFact,
    register: RegisterName,
) -> bool {
    match compare {
        CompareFact::Byte { left, right } => {
            value_fact_references_register(left, register)
                || value_fact_references_register(right, register)
        }
        CompareFact::WordSubtract { low, high } => {
            byte_compare_references_register(low, register)
                || byte_compare_references_register(high, register)
        }
    }
}

pub(super) fn semantic_flag_references_memory_byte(
    flag: SemanticFlagFact,
    byte: MemoryByte,
) -> bool {
    match flag {
        SemanticFlagFact::FromValue(value) => value_fact_references_memory_byte(value, byte),
        SemanticFlagFact::FromCompare(compare) => {
            compare_fact_references_memory_byte(compare, byte)
        }
        SemanticFlagFact::Unknown | SemanticFlagFact::Known(_) => false,
    }
}

pub(super) fn semantic_flag_references_register(
    flag: SemanticFlagFact,
    register: RegisterName,
) -> bool {
    match flag {
        SemanticFlagFact::FromValue(value) => value_fact_references_register(value, register),
        SemanticFlagFact::FromCompare(compare) => {
            compare_fact_references_register(compare, register)
        }
        SemanticFlagFact::Unknown | SemanticFlagFact::Known(_) => false,
    }
}

pub(super) fn register_value_is_call_stable(value: RegisterValue) -> bool {
    matches!(
        value,
        RegisterValue::Immediate(_)
            | RegisterValue::Fact(ValueFact::Immediate(_) | ValueFact::AddressByte { .. })
    )
}

pub(super) fn register_value_survives_known_call(
    value: RegisterValue,
    effects: RoutineEffects,
) -> bool {
    match value {
        RegisterValue::Unknown => false,
        RegisterValue::Immediate(_) => true,
        RegisterValue::Fact(fact) => value_fact_survives_known_call(fact, effects),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum CompareFact {
    Byte {
        left: ValueFact,
        right: ValueFact,
    },
    WordSubtract {
        low: ByteCompareFact,
        high: ByteCompareFact,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(super) enum SemanticFlagFact {
    #[default]
    Unknown,
    Known(bool),
    FromValue(ValueFact),
    FromCompare(CompareFact),
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(super) struct ProcessorState {
    pub(super) a: RegisterValue,
    pub(super) x: RegisterValue,
    pub(super) y: RegisterValue,
    pub(super) carry: FlagValue,
    pub(super) zero: SemanticFlagFact,
    pub(super) negative: SemanticFlagFact,
    pub(super) compare: Option<CompareFact>,
    pub(super) pending_word_compare_low: Option<ByteCompareFact>,
    pub(super) zp: TrackedZeroPage,
    pub(super) memory: TrackedMemory,
    pub(super) pointers: TrackedPointers,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct TrackedZeroPage {
    pub(super) values: [RegisterValue; TRACKED_ZERO_PAGE.len()],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct PreparedPointer {
    pub(super) key: String,
    pub(super) deps: Vec<PreparedDependency>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct PreparedDependency {
    pub(super) address: u16,
    pub(super) size: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct PreparedPointerFact {
    pub(super) key: String,
    pub(super) deps: Vec<PreparedDependency>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct TrackedPointers {
    pub(super) values: [Option<PreparedPointer>; TRACKED_POINTERS.len()],
}

impl Default for TrackedZeroPage {
    fn default() -> Self {
        Self {
            values: [RegisterValue::Unknown; TRACKED_ZERO_PAGE.len()],
        }
    }
}

impl TrackedZeroPage {
    pub(super) fn invalidate_values_referencing_register(&mut self, register: RegisterName) {
        for value in &mut self.values {
            if matches!(*value, RegisterValue::Fact(fact) if value_fact_references_register(fact, register))
            {
                *value = RegisterValue::Unknown;
            }
        }
    }
}

impl Default for TrackedPointers {
    fn default() -> Self {
        Self {
            values: std::array::from_fn(|_| None),
        }
    }
}

pub(super) const TRACKED_ZERO_PAGE: [u8; 18] = [
    0x82, 0x83, 0x84, 0x85, 0x86, 0x87, 0xA0, 0xA1, 0xA2, 0xAC, 0xAD, 0xAE, 0xAF, 0xB7, 0xC0, 0xC1,
    0xC2, 0xC3,
];

pub(super) const TRACKED_POINTERS: [u8; 3] = [0xAC, 0xAE, 0xC0];

impl TrackedMemory {
    pub(super) fn value(&self, byte: MemoryByte) -> Option<ValueFact> {
        self.values
            .iter()
            .find(|fact| fact.byte == byte)
            .map(|fact| fact.value)
    }

    pub(super) fn set(&mut self, byte: MemoryByte, value: ValueFact) {
        self.invalidate(byte);
        self.values.retain(|fact| {
            !value_fact_references_memory_byte(fact.value, byte) || fact.byte == byte
        });
        self.values.push(MemoryContentFact { byte, value });
    }

    pub(super) fn invalidate(&mut self, byte: MemoryByte) {
        self.values.retain(|fact| {
            fact.byte != byte && !value_fact_references_memory_byte(fact.value, byte)
        });
    }

    pub(super) fn clear(&mut self) {
        self.values.clear();
    }

    pub(super) fn invalidate_values_referencing_register(&mut self, register: RegisterName) {
        self.values
            .retain(|fact| !value_fact_references_register(fact.value, register));
    }

    pub(super) fn preserved_after_known_call(&self, effects: RoutineEffects) -> Self {
        Self {
            values: self
                .values
                .iter()
                .copied()
                .filter(|fact| {
                    memory_byte_survives_known_call(fact.byte, effects)
                        && value_fact_survives_known_call(fact.value, effects)
                })
                .collect(),
        }
    }

    #[cfg(test)]
    pub(super) fn address_word(&self, slot: StorageSlot) -> Option<u16> {
        let low = self.value(MemoryByte {
            slot,
            byte_index: 0,
        })?;
        let high = self.value(MemoryByte {
            slot,
            byte_index: 1,
        })?;
        match (low, high) {
            (
                ValueFact::AddressByte {
                    address,
                    byte_index: 0,
                },
                ValueFact::AddressByte {
                    address: high_address,
                    byte_index: 1,
                },
            ) if address == high_address => Some(address),
            _ => None,
        }
    }
}

fn register_value_proves_equal(
    current: RegisterValue,
    register: RegisterName,
    expected: ValueFact,
) -> bool {
    match (current, expected) {
        (_, ValueFact::Unknown) => false,
        (RegisterValue::Unknown, ValueFact::Register(expected)) if expected == register => true,
        (RegisterValue::Unknown, _) => false,
        (RegisterValue::Immediate(actual), ValueFact::Immediate(expected)) => actual == expected,
        (RegisterValue::Fact(ValueFact::Unknown), _) => false,
        (RegisterValue::Fact(ValueFact::Register(actual)), ValueFact::Register(expected))
            if actual == register && expected == register =>
        {
            false
        }
        (RegisterValue::Fact(actual), expected) => actual == expected,
        (RegisterValue::Immediate(_), _) => false,
    }
}

impl ProcessorState {
    pub(super) fn reset(&mut self) {
        *self = Self::default();
    }

    pub(super) fn y_immediate(&self) -> Option<u8> {
        self.y.immediate()
    }

    pub(super) fn a_immediate(&self) -> Option<u8> {
        self.a.immediate()
    }

    pub(super) fn x_immediate(&self) -> Option<u8> {
        self.x.immediate()
    }

    pub(super) fn a_value_fact(&self) -> ValueFact {
        self.a.value_fact(RegisterName::A)
    }

    #[cfg(test)]
    pub(super) fn x_value_fact(&self) -> ValueFact {
        self.x.value_fact(RegisterName::X)
    }

    #[cfg(test)]
    pub(super) fn y_value_fact(&self) -> ValueFact {
        self.y.value_fact(RegisterName::Y)
    }

    #[cfg(test)]
    pub(super) fn carry(&self) -> FlagValue {
        self.carry
    }

    #[cfg(test)]
    pub(super) fn zero(&self) -> SemanticFlagFact {
        self.zero
    }

    #[cfg(test)]
    pub(super) fn negative(&self) -> SemanticFlagFact {
        self.negative
    }

    #[cfg(test)]
    pub(super) fn compare(&self) -> Option<CompareFact> {
        self.compare
    }

    #[cfg(test)]
    pub(super) fn zp_immediate(&self, zero_page: ZeroPage) -> Option<u8> {
        self.zp_value(zero_page).immediate()
    }

    pub(super) fn zp_value(&self, zero_page: ZeroPage) -> RegisterValue {
        tracked_zero_page_index(zero_page)
            .map(|index| self.zp.values[index])
            .unwrap_or(RegisterValue::Unknown)
    }

    pub(super) fn set_a_immediate(&mut self, value: u8) {
        self.invalidate_facts_referencing_register(RegisterName::A);
        self.a = RegisterValue::Immediate(value);
        self.set_value_flags(ValueFact::Immediate(value));
    }

    pub(super) fn set_a_fact(&mut self, fact: ValueFact) {
        self.invalidate_facts_referencing_register(RegisterName::A);
        self.a = RegisterValue::Fact(fact);
        self.set_value_flags(fact);
    }

    pub(super) fn set_x_immediate(&mut self, value: u8) {
        self.invalidate_facts_referencing_register(RegisterName::X);
        self.x = RegisterValue::Immediate(value);
        self.set_value_flags(ValueFact::Immediate(value));
    }

    pub(super) fn set_x_fact(&mut self, fact: ValueFact) {
        self.invalidate_facts_referencing_register(RegisterName::X);
        self.x = RegisterValue::Fact(fact);
        self.set_value_flags(fact);
    }

    pub(super) fn set_x_value_fact(&mut self, fact: ValueFact) {
        if let Some(value) = value_fact_immediate(fact) {
            self.set_x_immediate(value);
        } else {
            self.set_x_fact(fact);
        }
    }

    pub(super) fn set_y_immediate(&mut self, value: u8) {
        self.invalidate_facts_referencing_register(RegisterName::Y);
        self.y = RegisterValue::Immediate(value);
        self.set_value_flags(ValueFact::Immediate(value));
    }

    pub(super) fn set_y_fact(&mut self, fact: ValueFact) {
        self.invalidate_facts_referencing_register(RegisterName::Y);
        self.y = RegisterValue::Fact(fact);
        self.set_value_flags(fact);
    }

    pub(super) fn set_y_value_fact(&mut self, fact: ValueFact) {
        if let Some(value) = value_fact_immediate(fact) {
            self.set_y_immediate(value);
        } else {
            self.set_y_fact(fact);
        }
    }

    pub(super) fn set_a_logic_result(&mut self, op: LogicFactOp, right: ValueFact) {
        let left = self.a_value_fact();
        if let (Some(left), Some(right)) = (value_fact_immediate(left), value_fact_immediate(right))
        {
            self.set_a_immediate(apply_logic_fact_op(op, left, right));
            return;
        }
        if value_fact_references_register(left, RegisterName::A)
            || value_fact_references_register(right, RegisterName::A)
        {
            self.set_a_fact(ValueFact::Unknown);
            return;
        }
        self.set_a_fact(ValueFact::Logic {
            op,
            left: ValueAtomFact::from(left),
            right: ValueAtomFact::from(right),
        });
    }

    pub(super) fn set_a_subtract_result(&mut self, right: ValueFact) {
        let left = self.a_value_fact();
        let borrow = self.pending_word_compare_low.take();
        if value_fact_references_register(left, RegisterName::A)
            || value_fact_references_register(right, RegisterName::A)
            || borrow
                .is_some_and(|borrow| byte_compare_references_register(borrow, RegisterName::A))
        {
            self.set_a_fact(ValueFact::Unknown);
            self.carry = FlagValue::Unknown;
            self.pending_word_compare_low = None;
            return;
        }
        let high = ByteCompareFact {
            left: ValueAtomFact::from(left),
            right: ValueAtomFact::from(right),
        };
        self.invalidate_facts_referencing_register(RegisterName::A);
        self.a = RegisterValue::Fact(ValueFact::Subtract {
            left: high.left,
            right: high.right,
            borrow,
        });
        if let Some(low) = borrow {
            self.set_compare_flags_from_word_subtract(low, high);
        } else {
            self.set_value_flags(ValueFact::Subtract {
                left: high.left,
                right: high.right,
                borrow,
            });
        }
        self.carry = FlagValue::Unknown;
    }

    pub(super) fn set_y_hint(&mut self, value: Option<u8>) {
        self.invalidate_facts_referencing_register(RegisterName::Y);
        self.y = match value {
            Some(value) => RegisterValue::Immediate(value),
            None => RegisterValue::Unknown,
        };
    }

    pub(super) fn set_carry(&mut self, value: bool) {
        self.carry = FlagValue::Known(value);
        self.compare = None;
        self.pending_word_compare_low = None;
    }

    pub(super) fn set_value_flags(&mut self, value: ValueFact) {
        match value {
            ValueFact::Immediate(value) => {
                self.zero = SemanticFlagFact::Known(value == 0);
                self.negative = SemanticFlagFact::Known(value & 0x80 != 0);
            }
            ValueFact::AddressByte {
                address,
                byte_index,
            } => {
                let value = Immediate::new(address).byte(byte_index);
                self.zero = SemanticFlagFact::Known(value == 0);
                self.negative = SemanticFlagFact::Known(value & 0x80 != 0);
            }
            ValueFact::Unknown => {
                self.zero = SemanticFlagFact::Unknown;
                self.negative = SemanticFlagFact::Unknown;
            }
            value => {
                self.zero = SemanticFlagFact::FromValue(value);
                self.negative = SemanticFlagFact::FromValue(value);
            }
        }
        self.compare = None;
    }

    pub(super) fn invalidate_value_flags(&mut self) {
        self.zero = SemanticFlagFact::Unknown;
        self.negative = SemanticFlagFact::Unknown;
        self.compare = None;
    }

    pub(super) fn value_flags_match(&self, value: ValueFact) -> bool {
        match value {
            ValueFact::Immediate(value) => {
                self.zero == SemanticFlagFact::Known(value == 0)
                    && self.negative == SemanticFlagFact::Known(value & 0x80 != 0)
            }
            ValueFact::AddressByte {
                address,
                byte_index,
            } => {
                let value = Immediate::new(address).byte(byte_index);
                self.zero == SemanticFlagFact::Known(value == 0)
                    && self.negative == SemanticFlagFact::Known(value & 0x80 != 0)
            }
            ValueFact::Unknown => false,
            value => {
                self.zero == SemanticFlagFact::FromValue(value)
                    && self.negative == SemanticFlagFact::FromValue(value)
            }
        }
    }

    pub(super) fn accumulator_matches_load_result(&self, value: ValueFact) -> bool {
        self.accumulator_value_matches(value) && self.value_flags_match(value)
    }

    pub(super) fn x_matches_load_result(&self, value: ValueFact) -> bool {
        self.x_value_matches(value) && self.value_flags_match(value)
    }

    pub(super) fn y_matches_load_result(&self, value: ValueFact) -> bool {
        self.y_value_matches(value) && self.value_flags_match(value)
    }

    pub(super) fn accumulator_value_matches(&self, value: ValueFact) -> bool {
        register_value_proves_equal(self.a, RegisterName::A, value)
    }

    pub(super) fn x_value_matches(&self, value: ValueFact) -> bool {
        register_value_proves_equal(self.x, RegisterName::X, value)
    }

    pub(super) fn y_value_matches(&self, value: ValueFact) -> bool {
        register_value_proves_equal(self.y, RegisterName::Y, value)
    }

    pub(super) fn set_compare_flags(&mut self, compare: CompareFact) {
        self.compare = Some(compare);
        self.zero = SemanticFlagFact::FromCompare(compare);
        self.negative = SemanticFlagFact::FromCompare(compare);
        match compare {
            CompareFact::Byte {
                left: ValueFact::Immediate(left),
                right: ValueFact::Immediate(right),
            } => {
                self.carry = FlagValue::Known(left >= right);
                self.zero = SemanticFlagFact::Known(left == right);
                self.negative = SemanticFlagFact::Known(left.wrapping_sub(right) & 0x80 != 0);
            }
            _ => self.carry = FlagValue::Unknown,
        }
        self.pending_word_compare_low = match compare {
            CompareFact::Byte { left, right } => Some(ByteCompareFact {
                left: ValueAtomFact::from(left),
                right: ValueAtomFact::from(right),
            }),
            CompareFact::WordSubtract { .. } => None,
        };
    }

    pub(super) fn set_compare_flags_from_word_subtract(
        &mut self,
        low: ByteCompareFact,
        high: ByteCompareFact,
    ) {
        let compare = CompareFact::WordSubtract { low, high };
        self.compare = Some(compare);
        self.zero = SemanticFlagFact::FromCompare(compare);
        self.negative = SemanticFlagFact::FromCompare(compare);
        self.pending_word_compare_low = None;
    }

    pub(super) fn set_zp_value(&mut self, zero_page: ZeroPage, value: RegisterValue) {
        if let Some(index) = tracked_zero_page_index(zero_page) {
            self.zp.values[index] = value;
        }
        self.invalidate_prepared_pointer_for_zero_page(zero_page);
    }

    pub(super) fn set_zp_from_a(&mut self, zero_page: ZeroPage) {
        self.set_zp_value(zero_page, self.a);
    }

    pub(super) fn set_zp_from_x(&mut self, zero_page: ZeroPage) {
        self.set_zp_value(zero_page, self.x);
    }

    pub(super) fn set_zp_from_y(&mut self, zero_page: ZeroPage) {
        self.set_zp_value(zero_page, self.y);
    }

    pub(super) fn memory_value(&self, slot: StorageSlot, byte_index: u16) -> Option<ValueFact> {
        self.memory.value(MemoryByte { slot, byte_index })
    }

    #[cfg(test)]
    pub(super) fn memory_address_word(&self, slot: StorageSlot) -> Option<u16> {
        self.memory.address_word(slot)
    }

    pub(super) fn set_memory_byte_from_a(&mut self, slot: StorageSlot, byte_index: u16) {
        self.set_memory_byte(slot, byte_index, self.a.value_fact(RegisterName::A));
    }

    pub(super) fn set_memory_byte_from_x(&mut self, slot: StorageSlot, byte_index: u16) {
        self.set_memory_byte(slot, byte_index, self.x.value_fact(RegisterName::X));
    }

    pub(super) fn set_memory_byte_from_y(&mut self, slot: StorageSlot, byte_index: u16) {
        self.set_memory_byte(slot, byte_index, self.y.value_fact(RegisterName::Y));
    }

    pub(super) fn set_memory_byte(&mut self, slot: StorageSlot, byte_index: u16, value: ValueFact) {
        let byte = MemoryByte { slot, byte_index };
        self.invalidate_register_memory_aliases(byte);
        self.memory.set(byte, value);
    }

    pub(super) fn invalidate_register_memory_aliases(&mut self, byte: MemoryByte) {
        if value_fact_references_memory_byte(self.a.value_fact(RegisterName::A), byte) {
            self.clear_a();
        }
        if value_fact_references_memory_byte(self.x.value_fact(RegisterName::X), byte) {
            self.clear_x();
        }
        if value_fact_references_memory_byte(self.y.value_fact(RegisterName::Y), byte) {
            self.clear_y();
        }
        if semantic_flag_references_memory_byte(self.zero, byte) {
            self.zero = SemanticFlagFact::Unknown;
        }
        if semantic_flag_references_memory_byte(self.negative, byte) {
            self.negative = SemanticFlagFact::Unknown;
        }
        if self
            .compare
            .is_some_and(|compare| compare_fact_references_memory_byte(compare, byte))
        {
            self.compare = None;
            self.carry = FlagValue::Unknown;
        }
        if self
            .pending_word_compare_low
            .is_some_and(|compare| byte_compare_references_memory_byte(compare, byte))
        {
            self.pending_word_compare_low = None;
        }
    }

    pub(super) fn set_memory_address_word(&mut self, slot: StorageSlot, address: u16) {
        if slot.size < 2 {
            return;
        }
        self.set_memory_byte(
            slot,
            0,
            ValueFact::AddressByte {
                address,
                byte_index: 0,
            },
        );
        self.set_memory_byte(
            slot,
            1,
            ValueFact::AddressByte {
                address,
                byte_index: 1,
            },
        );
    }

    pub(super) fn invalidate_memory_byte(&mut self, slot: StorageSlot, byte_index: u16) {
        let byte = MemoryByte { slot, byte_index };
        self.invalidate_register_memory_aliases(byte);
        self.memory.invalidate(byte);
    }

    pub(super) fn invalidate_facts_referencing_register(&mut self, register: RegisterName) {
        self.zp.invalidate_values_referencing_register(register);
        self.memory.invalidate_values_referencing_register(register);
        if register != RegisterName::A
            && value_fact_references_register(self.a.value_fact(RegisterName::A), register)
        {
            self.a = RegisterValue::Unknown;
        }
        if register != RegisterName::X
            && value_fact_references_register(self.x.value_fact(RegisterName::X), register)
        {
            self.x = RegisterValue::Unknown;
        }
        if register != RegisterName::Y
            && value_fact_references_register(self.y.value_fact(RegisterName::Y), register)
        {
            self.y = RegisterValue::Unknown;
        }
        if semantic_flag_references_register(self.zero, register) {
            self.zero = SemanticFlagFact::Unknown;
        }
        if semantic_flag_references_register(self.negative, register) {
            self.negative = SemanticFlagFact::Unknown;
        }
        if self
            .compare
            .is_some_and(|compare| compare_fact_references_register(compare, register))
        {
            self.compare = None;
            self.carry = FlagValue::Unknown;
        }
        if self
            .pending_word_compare_low
            .is_some_and(|compare| byte_compare_references_register(compare, register))
        {
            self.pending_word_compare_low = None;
        }
    }

    pub(super) fn clear_y(&mut self) {
        self.invalidate_facts_referencing_register(RegisterName::Y);
        self.y = RegisterValue::Unknown;
        self.set_value_flags(ValueFact::Register(RegisterName::Y));
    }

    pub(super) fn clear_a(&mut self) {
        self.invalidate_facts_referencing_register(RegisterName::A);
        self.a = RegisterValue::Unknown;
        self.set_value_flags(ValueFact::Register(RegisterName::A));
    }

    pub(super) fn clear_x(&mut self) {
        self.invalidate_facts_referencing_register(RegisterName::X);
        self.x = RegisterValue::Unknown;
        self.set_value_flags(ValueFact::Register(RegisterName::X));
    }

    pub(super) fn invalidate_after_call(&mut self) {
        self.reset();
    }

    pub(super) fn invalidate_after_known_call(&mut self, effects: RoutineEffects) {
        debug_assert!(effects.known);
        let a = self.a;
        let x = self.x;
        let y = self.y;
        let zero_page = self.zp;
        let memory = self.memory.clone();
        let pointers = self.pointers.clone();
        self.reset();
        if effects.preserves_a && register_value_survives_known_call(a, effects) {
            self.a = a;
        }
        if effects.preserves_x && register_value_survives_known_call(x, effects) {
            self.x = x;
        }
        if effects.preserves_y && register_value_survives_known_call(y, effects) {
            self.y = y;
        }
        for (index, address) in TRACKED_ZERO_PAGE.iter().copied().enumerate() {
            let value = zero_page.values[index];
            if !effects.writes_zero_page(ZeroPage::new(address))
                && register_value_is_call_stable(value)
            {
                self.zp.values[index] = value;
            }
        }
        self.memory = memory.preserved_after_known_call(effects);
        for (index, pointer) in TRACKED_POINTERS.iter().copied().enumerate() {
            if let Some(prepared) = pointers.values[index].clone()
                && prepared_pointer_survives_known_call(ZeroPage::new(pointer), &prepared, effects)
            {
                self.pointers.values[index] = Some(prepared);
            }
        }
    }

    pub(super) fn stable_zero_page_facts_preserved_by_known_call(
        &self,
        effects: RoutineEffects,
    ) -> usize {
        debug_assert!(effects.known);
        TRACKED_ZERO_PAGE
            .iter()
            .copied()
            .enumerate()
            .filter(|(index, address)| {
                !effects.writes_zero_page(ZeroPage::new(*address))
                    && register_value_is_call_stable(self.zp.values[*index])
            })
            .count()
    }

    pub(super) fn register_facts_preserved_by_known_call(&self, effects: RoutineEffects) -> usize {
        debug_assert!(effects.known);
        [
            (effects.preserves_a, self.a),
            (effects.preserves_x, self.x),
            (effects.preserves_y, self.y),
        ]
        .into_iter()
        .filter(|(preserves, value)| {
            *preserves
                && !matches!(value, RegisterValue::Unknown)
                && register_value_survives_known_call(*value, effects)
        })
        .count()
    }

    pub(super) fn stable_memory_facts_preserved_by_known_call(
        &self,
        effects: RoutineEffects,
    ) -> usize {
        debug_assert!(effects.known);
        self.memory.preserved_after_known_call(effects).values.len()
    }

    pub(super) fn invalidate_after_jump(&mut self) {
        self.reset();
    }

    pub(super) fn invalidate_accumulator(&mut self) {
        self.clear_a();
        self.set_value_flags(ValueFact::Unknown);
    }

    pub(super) fn invalidate_index_y(&mut self) {
        self.clear_y();
        self.set_value_flags(ValueFact::Unknown);
    }

    pub(super) fn invalidate_index_x(&mut self) {
        self.clear_x();
        self.set_value_flags(ValueFact::Unknown);
    }

    pub(super) fn invalidate_carry(&mut self) {
        self.carry = FlagValue::Unknown;
        self.compare = None;
        self.pending_word_compare_low = None;
    }

    pub(super) fn invalidate_zp(&mut self, zero_page: ZeroPage) {
        self.set_zp_value(zero_page, RegisterValue::Unknown);
        self.memory.invalidate(MemoryByte {
            slot: StorageSlot::zero_page(zero_page.address(), 1),
            byte_index: 0,
        });
    }

    pub(super) fn invalidate_all_zp(&mut self) {
        self.zp = TrackedZeroPage::default();
        self.invalidate_prepared_pointers();
    }

    pub(super) fn invalidate_memory(&mut self) {
        self.memory.clear();
    }

    pub(super) fn mark_prepared_pointer(&mut self, pointer: ZeroPage, fact: PreparedPointerFact) {
        if let Some(index) = tracked_pointer_index(pointer) {
            self.pointers.values[index] = Some(PreparedPointer {
                key: fact.key,
                deps: fact.deps,
            });
        }
    }

    pub(super) fn prepared_pointer_matches(&self, pointer: ZeroPage, key: &str) -> bool {
        tracked_pointer_index(pointer)
            .and_then(|index| self.pointers.values[index].as_ref())
            .is_some_and(|prepared| prepared.key == key)
    }

    pub(super) fn zero_page_matches_known_byte(
        &self,
        zero_page: ZeroPage,
        value: ValueFact,
    ) -> bool {
        let Some(expected) = value_fact_immediate(value) else {
            return false;
        };
        match self.zp_value(zero_page) {
            RegisterValue::Immediate(actual) => actual == expected,
            RegisterValue::Fact(fact) => value_fact_immediate(fact) == Some(expected),
            RegisterValue::Unknown => false,
        }
    }

    pub(super) fn invalidate_prepared_pointers(&mut self) {
        self.pointers = TrackedPointers::default();
    }

    pub(super) fn invalidate_prepared_pointer_for_zero_page(&mut self, zero_page: ZeroPage) {
        let address = zero_page.address();
        for (index, pointer) in TRACKED_POINTERS.iter().enumerate() {
            if address == *pointer || address == pointer.wrapping_add(1) {
                self.pointers.values[index] = None;
            }
        }
    }

    pub(super) fn invalidate_prepared_pointers_touching_range(&mut self, address: u16, size: u16) {
        for prepared in &mut self.pointers.values {
            if prepared.as_ref().is_some_and(|prepared| {
                prepared
                    .deps
                    .iter()
                    .any(|dep| ranges_overlap(dep.address, dep.size, address, size))
            }) {
                *prepared = None;
            }
        }
    }
}

pub(super) fn tracked_zero_page_index(zero_page: ZeroPage) -> Option<usize> {
    TRACKED_ZERO_PAGE
        .iter()
        .position(|address| *address == zero_page.address())
}

pub(super) fn tracked_pointer_index(pointer: ZeroPage) -> Option<usize> {
    TRACKED_POINTERS
        .iter()
        .position(|address| *address == pointer.address())
}

pub(super) fn prepared_pointer_survives_known_call(
    pointer: ZeroPage,
    prepared: &PreparedPointer,
    effects: RoutineEffects,
) -> bool {
    !effects.writes_pointer_pair(pointer)
        && prepared
            .deps
            .iter()
            .all(|dep| zero_page_dependency_survives_known_call(*dep, effects))
}

pub(super) fn zero_page_dependency_survives_known_call(
    dependency: PreparedDependency,
    effects: RoutineEffects,
) -> bool {
    let size = dependency.size.max(1);
    let Some(end) = dependency.address.checked_add(size) else {
        return false;
    };
    if end <= 0x100 {
        return (dependency.address..end)
            .all(|address| !effects.writes_zero_page(ZeroPage::new(address as u8)));
    }
    !effects.writes_absolute_range(dependency.address, size)
}

pub(super) fn memory_byte_survives_known_call(byte: MemoryByte, effects: RoutineEffects) -> bool {
    match byte.slot.space {
        AddressSpace::ZeroPage => {
            !effects.writes_zero_page(byte.slot.zero_page_byte(byte.byte_index))
        }
        AddressSpace::Absolute => {
            !effects.writes_absolute_range(byte.slot.byte_address(byte.byte_index), 1)
        }
        AddressSpace::AbsoluteX | AddressSpace::IndirectIndexedY => false,
    }
}

pub(super) fn value_atom_survives_known_call(
    value: ValueAtomFact,
    effects: RoutineEffects,
) -> bool {
    match value {
        ValueAtomFact::Immediate(_) | ValueAtomFact::AddressByte { .. } => true,
        ValueAtomFact::SlotByte { slot, byte_index } => {
            memory_byte_survives_known_call(MemoryByte { slot, byte_index }, effects)
        }
        ValueAtomFact::Unknown | ValueAtomFact::Register(_) => false,
    }
}

pub(super) fn value_fact_survives_known_call(value: ValueFact, effects: RoutineEffects) -> bool {
    match value {
        ValueFact::Immediate(_) | ValueFact::AddressByte { .. } => true,
        ValueFact::SlotByte { slot, byte_index } => {
            memory_byte_survives_known_call(MemoryByte { slot, byte_index }, effects)
        }
        ValueFact::Logic { left, right, .. } => {
            value_atom_survives_known_call(left, effects)
                && value_atom_survives_known_call(right, effects)
        }
        ValueFact::Subtract {
            left,
            right,
            borrow,
        } => {
            value_atom_survives_known_call(left, effects)
                && value_atom_survives_known_call(right, effects)
                && borrow.is_none_or(|borrow| {
                    value_atom_survives_known_call(borrow.left, effects)
                        && value_atom_survives_known_call(borrow.right, effects)
                })
        }
        ValueFact::Unknown | ValueFact::Register(_) => false,
    }
}
