#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum NativeRegister {
    A,
    X,
    Y,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(super) enum NativeValue {
    #[default]
    Unknown,
    Immediate(u8),
    Register(NativeRegister),
    MemoryByte(u16),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct NativeFlags {
    pub(super) zero: NativeValue,
    pub(super) negative: NativeValue,
    pub(super) carry_known: Option<bool>,
}

impl Default for NativeFlags {
    fn default() -> Self {
        Self {
            zero: NativeValue::Unknown,
            negative: NativeValue::Unknown,
            carry_known: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct NativeMemoryFact {
    pub(super) address: u16,
    pub(super) value: NativeValue,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(super) struct NativeMemoryState {
    facts: Vec<NativeMemoryFact>,
}

impl NativeMemoryState {
    pub(super) fn value(&self, address: u16) -> Option<NativeValue> {
        self.facts
            .iter()
            .find(|fact| fact.address == address)
            .map(|fact| fact.value)
    }

    fn set(&mut self, address: u16, value: NativeValue) {
        self.invalidate(address);
        self.facts
            .retain(|fact| !native_value_references_memory(fact.value, address));
        self.facts.push(NativeMemoryFact { address, value });
    }

    fn invalidate(&mut self, address: u16) {
        self.facts.retain(|fact| {
            fact.address != address && !native_value_references_memory(fact.value, address)
        });
    }

    fn invalidate_values_referencing_register(&mut self, register: NativeRegister) {
        self.facts
            .retain(|fact| !native_value_references_register(fact.value, register));
    }

    fn clear(&mut self) {
        self.facts.clear();
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(super) struct NativeProcessorState {
    a: NativeValue,
    x: NativeValue,
    y: NativeValue,
    memory: NativeMemoryState,
    flags: NativeFlags,
}

#[cfg(test)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct NativeProcessorSnapshot {
    pub(super) a: NativeValue,
    pub(super) x: NativeValue,
    pub(super) y: NativeValue,
    pub(super) memory: Vec<NativeMemoryFact>,
    pub(super) flags: NativeFlags,
}

impl NativeProcessorState {
    #[cfg(test)]
    pub(super) fn a(&self) -> NativeValue {
        self.a
    }

    #[cfg(test)]
    pub(super) fn x(&self) -> NativeValue {
        self.x
    }

    #[cfg(test)]
    pub(super) fn memory_value(&self, address: u16) -> Option<NativeValue> {
        self.memory.value(address)
    }

    #[cfg(test)]
    pub(super) fn flags(&self) -> NativeFlags {
        self.flags
    }

    #[cfg(test)]
    pub(super) fn snapshot(&self) -> NativeProcessorSnapshot {
        NativeProcessorSnapshot {
            a: self.a,
            x: self.x,
            y: self.y,
            memory: self.memory.facts.clone(),
            flags: self.flags,
        }
    }

    pub(super) fn load_a_immediate(&mut self, value: u8) {
        self.set_a(NativeValue::Immediate(value));
    }

    pub(super) fn can_skip_load_a_memory(&self, address: u16) -> bool {
        self.memory
            .value(address)
            .is_some_and(|value| self.a == value)
            || self.a == NativeValue::MemoryByte(address)
    }

    pub(super) fn flags_match_a_value(&self) -> bool {
        self.a != NativeValue::Unknown && self.flags.zero == self.a && self.flags.negative == self.a
    }

    pub(super) fn load_x_immediate(&mut self, value: u8) {
        self.set_x(NativeValue::Immediate(value));
    }

    pub(super) fn load_y_immediate(&mut self, value: u8) {
        self.set_y(NativeValue::Immediate(value));
    }

    pub(super) fn y_immediate(&self) -> Option<u8> {
        match self.y {
            NativeValue::Immediate(value) => Some(value),
            _ => None,
        }
    }

    pub(super) fn can_skip_load_y_immediate(&self, value: u8) -> bool {
        let value = NativeValue::Immediate(value);
        self.y == value && self.flags.zero == value && self.flags.negative == value
    }

    pub(super) fn increment_y(&mut self) {
        if let Some(value) = self.y_immediate() {
            self.set_y(NativeValue::Immediate(value.wrapping_add(1)));
        } else {
            self.invalidate_y();
        }
    }

    pub(super) fn decrement_y(&mut self) {
        if let Some(value) = self.y_immediate() {
            self.set_y(NativeValue::Immediate(value.wrapping_sub(1)));
        } else {
            self.invalidate_y();
        }
    }

    pub(super) fn load_a_memory(&mut self, address: u16) {
        self.set_a(
            self.memory
                .value(address)
                .unwrap_or(NativeValue::MemoryByte(address)),
        );
    }

    pub(super) fn load_x_memory(&mut self, address: u16) {
        self.set_x(
            self.memory
                .value(address)
                .unwrap_or(NativeValue::MemoryByte(address)),
        );
    }

    pub(super) fn load_y_memory(&mut self, address: u16) {
        self.set_y(
            self.memory
                .value(address)
                .unwrap_or(NativeValue::MemoryByte(address)),
        );
    }

    pub(super) fn store_a_memory(&mut self, address: u16) {
        self.memory
            .set(address, self.register_value(NativeRegister::A));
    }

    pub(super) fn store_x_memory(&mut self, address: u16) {
        self.memory
            .set(address, self.register_value(NativeRegister::X));
    }

    pub(super) fn store_y_memory(&mut self, address: u16) {
        self.memory
            .set(address, self.register_value(NativeRegister::Y));
    }

    pub(super) fn clc(&mut self) {
        self.flags.carry_known = Some(false);
    }

    pub(super) fn sec(&mut self) {
        self.flags.carry_known = Some(true);
    }

    pub(super) fn arithmetic_a(&mut self) {
        self.invalidate_a();
        self.flags.carry_known = None;
    }

    pub(super) fn load_a_unknown(&mut self) {
        self.invalidate_a();
    }

    pub(super) fn transfer_a_to_x(&mut self) {
        self.set_x(self.register_value(NativeRegister::A));
    }

    pub(super) fn transfer_a_to_y(&mut self) {
        self.set_y(self.register_value(NativeRegister::A));
    }

    pub(super) fn invalidate_flags(&mut self) {
        self.flags = NativeFlags::default();
    }

    pub(super) fn mutate_memory(&mut self, address: u16) {
        self.memory.invalidate(address);
        self.flags = NativeFlags::default();
    }

    pub(super) fn mutate_unknown_memory(&mut self) {
        self.memory.clear();
        self.flags = NativeFlags::default();
    }

    pub(super) fn call_unknown(&mut self) {
        *self = Self::default();
    }

    pub(super) fn bind_label(&mut self) {
        *self = Self::default();
    }

    fn set_a(&mut self, value: NativeValue) {
        self.invalidate_values_referencing_register(NativeRegister::A);
        self.a = value;
        self.set_value_flags(value);
    }

    fn set_x(&mut self, value: NativeValue) {
        self.invalidate_values_referencing_register(NativeRegister::X);
        self.x = value;
        self.set_value_flags(value);
    }

    fn set_y(&mut self, value: NativeValue) {
        self.invalidate_values_referencing_register(NativeRegister::Y);
        self.y = value;
        self.set_value_flags(value);
    }

    fn invalidate_a(&mut self) {
        self.invalidate_values_referencing_register(NativeRegister::A);
        self.a = NativeValue::Register(NativeRegister::A);
        self.set_value_flags(NativeValue::Register(NativeRegister::A));
    }

    fn invalidate_y(&mut self) {
        self.invalidate_values_referencing_register(NativeRegister::Y);
        self.y = NativeValue::Unknown;
        self.set_value_flags(NativeValue::Unknown);
    }

    fn set_value_flags(&mut self, value: NativeValue) {
        self.flags.zero = value;
        self.flags.negative = value;
    }

    fn invalidate_values_referencing_register(&mut self, register: NativeRegister) {
        self.memory.invalidate_values_referencing_register(register);
        if register != NativeRegister::A && native_value_references_register(self.a, register) {
            self.a = NativeValue::Unknown;
        }
        if register != NativeRegister::X && native_value_references_register(self.x, register) {
            self.x = NativeValue::Unknown;
        }
        if register != NativeRegister::Y && native_value_references_register(self.y, register) {
            self.y = NativeValue::Unknown;
        }
        if native_value_references_register(self.flags.zero, register) {
            self.flags.zero = NativeValue::Unknown;
        }
        if native_value_references_register(self.flags.negative, register) {
            self.flags.negative = NativeValue::Unknown;
        }
    }

    fn register_value(&self, register: NativeRegister) -> NativeValue {
        match register {
            NativeRegister::A => known_or_register(self.a, register),
            NativeRegister::X => known_or_register(self.x, register),
            NativeRegister::Y => known_or_register(self.y, register),
        }
    }
}

fn known_or_register(value: NativeValue, register: NativeRegister) -> NativeValue {
    match value {
        NativeValue::Unknown => NativeValue::Register(register),
        value => value,
    }
}

fn native_value_references_register(value: NativeValue, register: NativeRegister) -> bool {
    matches!(value, NativeValue::Register(value_register) if value_register == register)
}

fn native_value_references_memory(value: NativeValue, address: u16) -> bool {
    matches!(value, NativeValue::MemoryByte(value_address) if value_address == address)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_state_unknown(state: &NativeProcessorState) {
        assert_eq!(
            state.snapshot(),
            NativeProcessorSnapshot {
                a: NativeValue::Unknown,
                x: NativeValue::Unknown,
                y: NativeValue::Unknown,
                memory: Vec::new(),
                flags: NativeFlags::default(),
            }
        );
    }

    #[test]
    fn unknown_accumulator_and_flags_do_not_form_a_known_match() {
        let mut state = NativeProcessorState::default();

        assert!(!state.flags_match_a_value());
        state.load_a_unknown();
        assert!(state.flags_match_a_value());
        state.load_a_immediate(0x42);
        assert!(state.flags_match_a_value());
        state.call_unknown();
        assert!(!state.flags_match_a_value());
    }

    fn state_with_live_facts() -> NativeProcessorState {
        let mut state = NativeProcessorState::default();

        state.load_a_immediate(0x44);
        state.store_a_memory(0x3000);
        state.load_x_memory(0x3000);
        state.load_y_immediate(0x80);
        state.sec();

        state
    }

    #[test]
    fn native_state_tracks_basic_register_and_memory_values() {
        let mut state = NativeProcessorState::default();

        state.load_a_immediate(0x44);
        state.store_a_memory(0x3000);
        state.load_x_memory(0x3000);

        assert_eq!(state.a(), NativeValue::Immediate(0x44));
        assert_eq!(state.x(), NativeValue::Immediate(0x44));
        assert_eq!(
            state.memory_value(0x3000),
            Some(NativeValue::Immediate(0x44))
        );
    }

    #[test]
    fn native_state_invalidates_memory_aliases_on_source_register_change() {
        let mut state = NativeProcessorState::default();

        state.store_a_memory(0x3000);
        assert_eq!(
            state.memory_value(0x3000),
            Some(NativeValue::Register(NativeRegister::A))
        );

        state.load_a_immediate(0x12);
        assert_eq!(state.memory_value(0x3000), None);
        assert_eq!(state.a(), NativeValue::Immediate(0x12));
    }

    #[test]
    fn native_state_invalidates_memory_alias_chains() {
        let mut state = NativeProcessorState::default();

        state.load_a_memory(0x3000);
        state.store_a_memory(0x3010);
        assert_eq!(
            state.memory_value(0x3010),
            Some(NativeValue::MemoryByte(0x3000))
        );

        state.mutate_memory(0x3000);
        assert_eq!(state.memory_value(0x3010), None);
    }

    #[test]
    fn native_state_clears_all_facts_at_label_joins() {
        let mut state = state_with_live_facts();

        state.bind_label();

        assert_state_unknown(&state);
    }

    #[test]
    fn native_state_clears_all_facts_after_unknown_calls() {
        let mut state = state_with_live_facts();

        state.call_unknown();

        assert_state_unknown(&state);
    }

    #[test]
    fn native_state_arithmetic_tracks_unknown_a_flags_and_invalidates_carry() {
        let mut state = NativeProcessorState::default();

        state.load_a_immediate(1);
        state.clc();
        state.arithmetic_a();

        assert_eq!(state.a(), NativeValue::Register(NativeRegister::A));
        assert_eq!(state.flags().carry_known, None);
        assert_eq!(state.flags().zero, NativeValue::Register(NativeRegister::A));
    }

    #[test]
    fn native_state_invalidates_y_dependent_facts_when_y_changes() {
        let mut state = NativeProcessorState::default();

        state.store_y_memory(0x0080);
        assert_eq!(
            state.memory_value(0x0080),
            Some(NativeValue::Register(NativeRegister::Y))
        );
        state.load_a_memory(0x0080);
        assert_eq!(state.a(), NativeValue::Register(NativeRegister::Y));
        assert_eq!(state.flags().zero, NativeValue::Register(NativeRegister::Y));

        state.load_y_immediate(0);

        assert_eq!(state.a(), NativeValue::Unknown);
        assert_eq!(state.memory_value(0x0080), None);
        assert_eq!(state.snapshot().y, NativeValue::Immediate(0));
        assert_eq!(state.flags().zero, NativeValue::Immediate(0));
        assert_eq!(state.flags().negative, NativeValue::Immediate(0));
    }

    #[test]
    fn native_state_invalidates_register_aliases_when_source_register_changes() {
        let mut state = NativeProcessorState::default();

        state.transfer_a_to_x();
        state.transfer_a_to_y();
        assert_eq!(state.x(), NativeValue::Register(NativeRegister::A));
        assert_eq!(state.snapshot().y, NativeValue::Register(NativeRegister::A));

        state.load_a_immediate(0x7F);

        assert_eq!(state.a(), NativeValue::Immediate(0x7F));
        assert_eq!(state.x(), NativeValue::Unknown);
        assert_eq!(state.snapshot().y, NativeValue::Unknown);
    }

    #[test]
    fn native_state_memory_mutation_invalidates_direct_and_dependent_facts() {
        let mut state = NativeProcessorState::default();

        state.load_a_memory(0x3000);
        state.store_a_memory(0x3010);
        state.load_a_immediate(0x55);
        state.store_a_memory(0x3020);
        state.sec();

        state.mutate_memory(0x3000);

        assert_eq!(state.memory_value(0x3010), None);
        assert_eq!(
            state.memory_value(0x3020),
            Some(NativeValue::Immediate(0x55))
        );
        assert_eq!(state.flags(), NativeFlags::default());
    }

    #[test]
    fn native_state_unknown_memory_mutation_preserves_registers_but_drops_memory_and_flags() {
        let mut state = NativeProcessorState::default();

        state.load_a_immediate(0x11);
        state.load_x_immediate(0x22);
        state.load_y_immediate(0x33);
        state.store_a_memory(0x3000);
        state.sec();

        state.mutate_unknown_memory();

        assert_eq!(state.a(), NativeValue::Immediate(0x11));
        assert_eq!(state.x(), NativeValue::Immediate(0x22));
        assert_eq!(state.snapshot().y, NativeValue::Immediate(0x33));
        assert!(state.snapshot().memory.is_empty());
        assert_eq!(state.flags(), NativeFlags::default());
    }
}
