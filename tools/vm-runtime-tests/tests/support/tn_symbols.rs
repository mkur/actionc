// CompiledProgram exposes a listing, but no structured 6502 symbol map yet.
// Use .lines() throughout so LF and CRLF listings take the same parsing path.
pub fn routine_address(listing: &str, name: &str) -> u16 {
    let header = listing
        .lines()
        .filter_map(|line| line.strip_prefix("; ===== PROC "))
        .find(|line| line.split_whitespace().next() == Some(name))
        .unwrap_or_else(|| panic!("missing routine {name}"));
    // Classic ranges can begin with local data, before the executable entry.
    let address = header
        .split_once(" entry ")
        .map(|(_, entry)| entry.split_whitespace().next().unwrap())
        .unwrap_or_else(|| header.split_whitespace().nth(1).unwrap())
        .split("..")
        .next()
        .unwrap();
    u16::from_str_radix(address.trim_start_matches('$'), 16).unwrap()
}

pub fn global_address(listing: &str, name: &str) -> u16 {
    let label = format!("global_{name}:");
    // Listings preserve declaration spelling; Action! identifiers ignore case.
    let mut lines = listing
        .lines()
        .skip_while(|line| !line.eq_ignore_ascii_case(&label));
    assert!(lines.next().is_some(), "missing global {name}");
    lines
        .find_map(|line| {
            let (_, comment) = line.split_once(';')?;
            let address = comment.trim_start().strip_prefix('$')?.split_once(':')?.0;
            u16::from_str_radix(address, 16).ok()
        })
        .unwrap_or_else(|| panic!("missing address for {name}"))
}

#[test]
fn global_lookup_preserves_identifier_boundaries_with_lf_and_crlf() {
    let listing = "global_currentDirectory:\n    .BYTE $00 ; $3000: 00\n\
                   global_currentDir:\n    .BYTE $00,$00 ; $3001: 00 00\n\
                   global_currentDir_2:\n    .BYTE $00 ; $3003: 00\n";
    for listing in [listing.to_owned(), listing.replace('\n', "\r\n")] {
        for name in ["currentdir", "currentDir", "CURRENTDIR"] {
            assert_eq!(global_address(&listing, name), 0x3001);
        }
        assert_eq!(global_address(&listing, "currentdirectory"), 0x3000);
        assert_eq!(global_address(&listing, "currentdir_2"), 0x3003);
    }
}
