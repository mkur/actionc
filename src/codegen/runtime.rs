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

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum RuntimeHelperSlot {
    Lsh,
    Rsh,
    Mul,
    Div,
    Mod,
    UDiv,
    UMod,
    SArgs,
}

impl RuntimeHelperSlot {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Lsh => "LShift",
            Self::Rsh => "RShift",
            Self::Mul => "MultI",
            Self::Div => "DivI",
            Self::Mod => "RemI",
            Self::UDiv => "DivU16",
            Self::UMod => "RemU16",
            Self::SArgs => "SArgs",
        }
    }

    fn standalone_slot(self) -> Absolute {
        match self {
            Self::Lsh => runtime_helper::LSH_SLOT,
            Self::Rsh => runtime_helper::RSH_SLOT,
            Self::Mul => runtime_helper::MUL_SLOT,
            Self::Div => runtime_helper::DIV_SLOT,
            Self::Mod => runtime_helper::MOD_SLOT,
            Self::UDiv => runtime_helper::DIV_SLOT,
            Self::UMod => runtime_helper::MOD_SLOT,
            Self::SArgs => runtime_helper::SARGS_SLOT,
        }
    }

    pub(crate) fn from_slot_address(address: u16) -> Option<Self> {
        match address {
            address if address == runtime_helper::LSH_SLOT.address() => Some(Self::Lsh),
            address if address == runtime_helper::RSH_SLOT.address() => Some(Self::Rsh),
            address if address == runtime_helper::MUL_SLOT.address() => Some(Self::Mul),
            address if address == runtime_helper::DIV_SLOT.address() => Some(Self::Div),
            address if address == runtime_helper::MOD_SLOT.address() => Some(Self::Mod),
            address if address == runtime_helper::SARGS_SLOT.address() => Some(Self::SArgs),
            _ => None,
        }
    }

    pub(crate) fn is_owned_division(self) -> bool {
        matches!(self, Self::Div | Self::Mod | Self::UDiv | Self::UMod)
    }

    pub(super) fn owned_label(self) -> String {
        format!("ACTION.RUNTIME.ACTIONC::{}", self.name())
    }
}

impl RuntimeHelperTarget {
    pub(super) fn is_default_standalone_slot(&self, helper: RuntimeHelperSlot) -> bool {
        matches!(self, Self::Absolute(address) if *address == helper.standalone_slot())
    }
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
                div: RuntimeHelperTarget::Label(RuntimeHelperSlot::Div.owned_label()),
                rem: RuntimeHelperTarget::Label(RuntimeHelperSlot::Mod.owned_label()),
                sargs: runtime_helper::CARTRIDGE_SARGS.into(),
            },
            RuntimeTarget::StandaloneSlots => Self {
                lsh: runtime_helper::LSH_SLOT.into(),
                rsh: runtime_helper::RSH_SLOT.into(),
                mul: runtime_helper::MUL_SLOT.into(),
                div: RuntimeHelperTarget::Label(RuntimeHelperSlot::Div.owned_label()),
                rem: RuntimeHelperTarget::Label(RuntimeHelperSlot::Mod.owned_label()),
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
            RuntimeHelperSlot::UDiv => RuntimeHelperTarget::Label(RuntimeHelperSlot::UDiv.owned_label()),
            RuntimeHelperSlot::UMod => RuntimeHelperTarget::Label(RuntimeHelperSlot::UMod.owned_label()),
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
            // Cartridge and extracted SYSLIB use different private workspace.
            // Conservatively describe their union while sharing this slot.
            record_zero_page_effect_range(&mut effects, 0xC6, 0xC7);
            effects.record_zero_page_write(ZeroPage::new(0xD3));
        }
        RuntimeHelperSlot::Div | RuntimeHelperSlot::Mod => {
            record_zero_page_effect_range(&mut effects, 0x82, 0x87);
            record_zero_page_effect_range(&mut effects, 0xC2, 0xC3);
        }
        RuntimeHelperSlot::UDiv | RuntimeHelperSlot::UMod => {
            record_zero_page_effect_range(&mut effects, 0x82, 0x87);
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
