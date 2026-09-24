//! Test-only guard inventory rooted in the typed overflow fixup.
use actionc::compiler::native65816::Compiled;
use actionc::mir65816::emit::Target;

#[derive(Clone, Debug)]
pub struct Guard {
    pub routine: String,
    pub start: u32,
    pub end: u32,
    pub amount: u16,
}
pub fn sites(c: &Compiled) -> Vec<Guard> {
    let mut sites = vec![];
    for m in &c.machine.routines {
        let r = c.image.routines.iter().find(|r| r.id == m.id.0).unwrap();
        let code = &c
            .image
            .segments
            .iter()
            .find(|s| s.address == r.address)
            .unwrap()
            .bytes;
        let before = sites.len();
        for f in &m.code.fixups {
            if f.target != Target::StackOverflow {
                continue;
            }
            assert_eq!((f.addend, f.byte), (0, None));
            let end = f.offset + 3;
            let start = [45, 29, 27]
                .into_iter()
                .filter_map(|size| end.checked_sub(size))
                .find(|&start| code[start..].starts_with(&[0x3b, 0xaa, 0xc5, 0x46]))
                .unwrap();
            let amount = u16::from_le_bytes(code[end - 6..end - 4].try_into().unwrap());
            sites.push(Guard {
                routine: r.name.clone(),
                start: r.address + start as u32,
                end: r.address + end as u32,
                amount,
            });
        }
        assert_eq!(sites.len() - before, 1 + r.calls.len());
        assert_eq!(sites[before].start, r.address);
        assert_eq!(sites[before].amount, r.fixed_frame);
    }
    sites
}
