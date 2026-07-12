use super::*;

pub mod runtime_helper {
    use super::Absolute;

    pub const LSH_SLOT: Absolute = Absolute::new(0x04E4);
    pub const RSH_SLOT: Absolute = Absolute::new(0x04E6);
    pub const MUL_SLOT: Absolute = Absolute::new(0x04E8);
    pub const DIV_SLOT: Absolute = Absolute::new(0x04EA);
    pub const MOD_SLOT: Absolute = Absolute::new(0x04EC);
    pub const SARGS_SLOT: Absolute = Absolute::new(0x04EE);

    pub const CARTRIDGE_LSH: Absolute = Absolute::new(0xB5C0);
    pub const CARTRIDGE_RSH: Absolute = Absolute::new(0xA0E6);
    pub const CARTRIDGE_MUL: Absolute = Absolute::new(0xA000);
    pub const CARTRIDGE_DIV: Absolute = Absolute::new(0xA090);
    pub const CARTRIDGE_MOD: Absolute = Absolute::new(0xA0DE);
    pub const CARTRIDGE_SARGS: Absolute = Absolute::new(0xA0F5);
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum RuntimeHelperTarget {
    Absolute(Absolute),
    Label(String),
}

impl From<Absolute> for RuntimeHelperTarget {
    fn from(value: Absolute) -> Self {
        Self::Absolute(value)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum RuntimeTarget {
    Cartridge,
    StandaloneSlots,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum RuntimeHelperSlot {
    Lsh,
    Rsh,
    Mul,
    Div,
    Mod,
    SArgs,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct RuntimeHelperTargets {
    lsh: RuntimeHelperTarget,
    rsh: RuntimeHelperTarget,
    mul: RuntimeHelperTarget,
    div: RuntimeHelperTarget,
    rem: RuntimeHelperTarget,
    sargs: RuntimeHelperTarget,
}

impl RuntimeHelperTargets {
    pub(super) fn default_for_target(target: RuntimeTarget) -> Self {
        match target {
            RuntimeTarget::Cartridge => Self {
                lsh: runtime_helper::CARTRIDGE_LSH.into(),
                rsh: runtime_helper::CARTRIDGE_RSH.into(),
                mul: runtime_helper::CARTRIDGE_MUL.into(),
                div: runtime_helper::CARTRIDGE_DIV.into(),
                rem: runtime_helper::CARTRIDGE_MOD.into(),
                sargs: runtime_helper::CARTRIDGE_SARGS.into(),
            },
            RuntimeTarget::StandaloneSlots => Self {
                lsh: runtime_helper::LSH_SLOT.into(),
                rsh: runtime_helper::RSH_SLOT.into(),
                mul: runtime_helper::MUL_SLOT.into(),
                div: runtime_helper::DIV_SLOT.into(),
                rem: runtime_helper::MOD_SLOT.into(),
                sargs: runtime_helper::SARGS_SLOT.into(),
            },
        }
    }

    pub(super) fn apply_set(&mut self, address: u16, value: RuntimeHelperTarget) {
        match address {
            address if address == runtime_helper::LSH_SLOT.address() => self.lsh = value,
            address if address == runtime_helper::RSH_SLOT.address() => self.rsh = value,
            address if address == runtime_helper::MUL_SLOT.address() => self.mul = value,
            address if address == runtime_helper::DIV_SLOT.address() => self.div = value,
            address if address == runtime_helper::MOD_SLOT.address() => self.rem = value,
            address if address == runtime_helper::SARGS_SLOT.address() => self.sargs = value,
            _ => {}
        }
    }

    pub(super) fn target(&self, slot: RuntimeHelperSlot) -> RuntimeHelperTarget {
        match slot {
            RuntimeHelperSlot::Lsh => self.lsh.clone(),
            RuntimeHelperSlot::Rsh => self.rsh.clone(),
            RuntimeHelperSlot::Mul => self.mul.clone(),
            RuntimeHelperSlot::Div => self.div.clone(),
            RuntimeHelperSlot::Mod => self.rem.clone(),
            RuntimeHelperSlot::SArgs => self.sargs.clone(),
        }
    }
}

pub(super) fn runtime_helper_effects(slot: RuntimeHelperSlot) -> RoutineEffects {
    let mut effects = RoutineEffects::known_empty();
    match slot {
        RuntimeHelperSlot::Lsh | RuntimeHelperSlot::Rsh => {
            effects.record_zero_page_write(ZeroPage::new(0x85));
        }
        RuntimeHelperSlot::Mul => {
            record_zero_page_effect_range(&mut effects, 0x82, 0x87);
            record_zero_page_effect_range(&mut effects, 0xC0, 0xC2);
        }
        RuntimeHelperSlot::Div | RuntimeHelperSlot::Mod => {
            record_zero_page_effect_range(&mut effects, 0x82, 0x87);
            effects.record_zero_page_write(ZeroPage::new(0xC2));
        }
        RuntimeHelperSlot::SArgs => {
            record_zero_page_effect_range(&mut effects, 0x82, 0x85);
            record_zero_page_effect_range(&mut effects, 0xA0, 0xA2);
            effects.record_unknown_absolute_write();
        }
    }
    effects
}

fn record_zero_page_effect_range(effects: &mut RoutineEffects, start: u8, end: u8) {
    for address in start..=end {
        effects.record_zero_page_write(ZeroPage::new(address));
    }
}
