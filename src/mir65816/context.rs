//! Bank-zero domain initialization and synthetic first-task images. These are
//! loader/assembly interface operations, not a scheduler or memory allocator.
use super::abi::generated as g;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DomainKind {
    Task,
    Irq,
    Bootstrap,
}
#[derive(Clone, Copy, Debug)]
pub struct Domain {
    pub direct_page: u16,
    pub owner: u32,
    pub kind: DomainKind,
    pub stack_low: u16,
    pub stack_high: u16,
    pub body_s: u16,
    pub nmi_extra: u16,
}
impl Domain {
    pub fn floor(&self) -> Result<u16, String> {
        let headroom = match self.kind {
            DomainKind::Irq => g::INTERRUPT_IRQ_HEADROOM_BASE_BYTES,
            _ => g::INTERRUPT_TASK_OR_BOOTSTRAP_HEADROOM_BASE_BYTES,
        } + u32::from(self.nmi_extra);
        let floor = u32::from(self.stack_low)
            .checked_add(headroom)
            .and_then(|v| v.checked_sub(1))
            .filter(|&v| v <= u32::from(self.stack_high))
            .ok_or("stack lacks interrupt headroom")?;
        if self.direct_page & 255 != 0
            || self.owner >= 1 << 24
            || self.body_s & 1 != 0
            || u32::from(self.body_s) < floor
            || self.body_s > self.stack_high
        {
            return Err("invalid native domain layout".into());
        }
        let dp = u32::from(self.direct_page);
        if dp <= u32::from(self.stack_high) && dp + 256 > u32::from(self.stack_low) {
            return Err("domain direct page overlaps its stack".into());
        }
        if self.kind == DomainKind::Irq && u32::from(self.body_s) < floor + 6 {
            return Err("IRQ bridge needs six bytes below body S".into());
        }
        Ok(floor as u16)
    }
    pub fn bytes(&self) -> Result<[u8; 256], String> {
        let floor = self.floor()?;
        let mut bytes = [0; 256];
        bytes[g::DP_OWNER_POINTER_OFFSET as usize..g::DP_OWNER_POINTER_OFFSET as usize + 3]
            .copy_from_slice(&self.owner.to_le_bytes()[..3]);
        bytes[g::DP_DOMAIN_KIND_OFFSET as usize] = match self.kind {
            DomainKind::Task => g::DOMAIN_TASK,
            DomainKind::Irq => g::DOMAIN_IRQ,
            DomainKind::Bootstrap => g::DOMAIN_BOOTSTRAP,
        } as u8;
        bytes[g::DP_STACK_FLOOR_OFFSET as usize..g::DP_STACK_FLOOR_OFFSET as usize + 2]
            .copy_from_slice(&floor.to_le_bytes());
        bytes[g::DP_STACK_CEILING_OFFSET as usize..g::DP_STACK_CEILING_OFFSET as usize + 2]
            .copy_from_slice(&self.stack_high.to_le_bytes());
        Ok(bytes)
    }
}
#[derive(Clone, Debug)]
pub struct FirstTask {
    pub saved_s: u16,
    /// Bytes begin at saved_s+1. A context record contains saved_s, little endian.
    pub bytes: [u8; 19],
}
impl FirstTask {
    pub fn new(
        domain: &Domain,
        entry: u32,
        argument: u32,
        return_stub: u32,
        entry_local_peak: u16,
        masked: bool,
    ) -> Result<Self, String> {
        let floor = domain.floor()?;
        if domain.kind != DomainKind::Task
            || [entry, argument, return_stub].iter().any(|&v| v >= 1 << 24)
        {
            return Err("invalid first-task addresses/domain".into());
        }
        let entry_s = domain
            .body_s
            .checked_sub(6)
            .ok_or("task entry stack underflow")?;
        if entry_s
            .checked_sub(entry_local_peak)
            .is_none_or(|s| s < floor)
        {
            return Err("first-task entry exceeds stack floor".into());
        }
        let saved_s = domain
            .body_s
            .checked_sub(g::FIRST_TASK_SAVED_S_BELOW_EMPTY_BODY_S as u16)
            .ok_or("task image stack underflow")?;
        if saved_s < domain.stack_low {
            return Err("first-task image exceeds stack allocation".into());
        }
        let mut bytes = [0; 19];
        let mut put = |offset: u32, data: &[u8]| {
            let at = offset as usize - 1;
            bytes[at..at + data.len()].copy_from_slice(data);
        };
        put(g::SAVED_FRAME_D_OFFSET, &domain.direct_page.to_le_bytes());
        put(g::SAVED_FRAME_P_OFFSET, &[if masked { 4 } else { 0 }]);
        put(g::SAVED_FRAME_PC_OFFSET, &(entry as u16).to_le_bytes());
        put(g::SAVED_FRAME_PBR_OFFSET, &[(entry >> 16) as u8]);
        put(
            g::FIRST_TASK_RETURN_PC_OFFSET_FROM_SAVED_S,
            &(return_stub as u16).wrapping_sub(1).to_le_bytes(),
        );
        put(
            g::FIRST_TASK_RETURN_PBR_OFFSET_FROM_SAVED_S,
            &[(return_stub >> 16) as u8],
        );
        put(
            g::FIRST_TASK_ARGUMENT_OFFSET_FROM_SAVED_S,
            &argument.to_le_bytes()[..3],
        );
        Ok(Self { saved_s, bytes })
    }
}
