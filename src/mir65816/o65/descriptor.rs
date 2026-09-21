use super::profile::*;

pub(crate) struct Writer(pub Vec<u8>);
impl Writer {
    pub fn byte(&mut self, v: u8) {
        self.0.push(v);
    }
    pub fn word(&mut self, v: u16) {
        self.0.extend(v.to_le_bytes());
    }
    pub fn long(&mut self, v: u32) {
        self.0.extend(v.to_le_bytes());
    }
    pub fn count(&mut self, n: usize) -> Result<(), String> {
        if n > MAX_ITEMS {
            return Err("descriptor count limit".into());
        }
        self.long(n as u32);
        Ok(())
    }
    pub fn string(&mut self, s: &str) -> Result<(), String> {
        if s.len() > MAX_STRING {
            return Err("descriptor string limit".into());
        }
        self.long(s.len() as u32);
        self.0.extend(s.as_bytes());
        Ok(())
    }
}
fn contract(w: &mut Writer, c: &Contract) -> Result<(), String> {
    c.verify()?;
    w.string(&c.abi)?;
    w.long(c.signature);
    w.count(c.arguments.len())?;
    for a in &c.arguments {
        w.long(a.offset);
        w.long(a.size);
        w.long(a.alignment);
    }
    w.byte(c.result);
    w.long(c.incoming);
    w.word(c.stack_peak);
    w.byte(c.irq_effect);
    w.byte(c.kind);
    w.byte(c.domains);
    Ok(())
}
pub(crate) fn encode(p: &Profile) -> Result<Vec<u8>, String> {
    let mut w = Writer(b"A8O1".to_vec());
    w.long(0);
    w.word(1);
    w.word(0);
    w.byte(3);
    w.byte(0);
    w.word(p.nmi_extra_stack);
    w.long(p.entry);
    w.count(p.routines.len())?;
    for r in &p.routines {
        w.long(r.id);
        w.string(&r.name)?;
        w.long(r.offset);
        w.long(r.size);
        contract(&mut w, &r.contract)?;
        w.word(r.frame);
        w.word(r.spill);
        w.long(r.local_peak);
    }
    w.count(p.objects.len())?;
    for o in &p.objects {
        w.byte(o.kind);
        w.long(o.id);
        w.string(&o.name)?;
        w.byte(o.location.section.map_or(0, |s| s as u8));
        w.long(o.location.offset);
        w.long(o.size);
        w.long(o.alignment);
        w.byte(u8::from(o.mutable));
        w.byte(u8::from(o.alias));
    }
    w.count(p.imports.len())?;
    for i in &p.imports {
        w.string(&i.name)?;
        contract(&mut w, &i.contract)?;
    }
    w.count(p.relocations.len())?;
    for r in &p.relocations {
        w.byte(r.section as u8);
        w.long(r.offset);
        w.byte(r.encoding as u8);
        match r.target {
            Reference::Section(s) => {
                w.byte(s as u8);
                w.long(0);
            }
            Reference::Import(i) => {
                w.byte(0);
                w.long(i);
            }
        }
        w.long(r.value);
        w.byte(u8::from(r.zero_extend));
    }
    if w.0.len() >= LIMIT as usize {
        return Err("descriptor size limit".into());
    }
    let size = w.0.len() as u32;
    w.0[4..8].copy_from_slice(&size.to_le_bytes());
    Ok(w.0)
}
