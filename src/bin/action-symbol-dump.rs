use std::env;
use std::fs;
use std::process;

const DEFAULT_GLOBAL_TABLE_PTR: u16 = 0x00B1;
const MEMORY_SIZE: usize = 0x10000;

#[derive(Debug)]
struct Options {
    memory_path: String,
    table_ptr: u16,
    format: Format,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Format {
    Markdown,
    Json,
}

#[derive(Debug)]
struct SymbolEntry {
    slot: u8,
    name_addr: u16,
    name: String,
    vtype: u8,
    address: Option<u16>,
    class: String,
    args: Vec<String>,
}

fn main() {
    let options = parse_args().unwrap_or_else(|err| {
        eprintln!("{err}");
        eprintln!("usage: action-symbol-dump [--table-ptr <addr>] [--json] <memory.bin>");
        process::exit(2);
    });

    let memory = fs::read(&options.memory_path).unwrap_or_else(|err| {
        eprintln!("read {}: {err}", options.memory_path);
        process::exit(1);
    });
    if memory.len() != MEMORY_SIZE {
        eprintln!(
            "{} must be a raw 64K memory image, got {} bytes",
            options.memory_path,
            memory.len()
        );
        process::exit(1);
    }

    let entries = decode_table(&memory, options.table_ptr).unwrap_or_else(|err| {
        eprintln!("{err}");
        process::exit(1);
    });

    match options.format {
        Format::Markdown => print_markdown(&entries),
        Format::Json => print_json(&entries),
    }
}

fn parse_args() -> Result<Options, String> {
    let mut table_ptr = DEFAULT_GLOBAL_TABLE_PTR;
    let mut format = Format::Markdown;
    let mut memory_path = None;
    let mut args = env::args().skip(1);

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--table-ptr" => {
                let value = args
                    .next()
                    .ok_or_else(|| "--table-ptr requires an address".to_string())?;
                table_ptr = parse_address(&value)?;
            }
            "--json" => format = Format::Json,
            "--markdown" => format = Format::Markdown,
            "-h" | "--help" => {
                return Err(
                    "usage: action-symbol-dump [--table-ptr <addr>] [--json] <memory.bin>"
                        .to_string(),
                );
            }
            _ if arg.starts_with('-') => return Err(format!("unknown option `{arg}`")),
            _ => {
                if memory_path.replace(arg).is_some() {
                    return Err("only one memory image path is supported".to_string());
                }
            }
        }
    }

    let memory_path = memory_path.ok_or_else(|| "missing memory image path".to_string())?;
    Ok(Options {
        memory_path,
        table_ptr,
        format,
    })
}

fn parse_address(value: &str) -> Result<u16, String> {
    let trimmed = value.trim();
    let parsed = if let Some(hex) = trimmed.strip_prefix('$') {
        u16::from_str_radix(hex, 16)
    } else if let Some(hex) = trimmed
        .strip_prefix("0x")
        .or_else(|| trimmed.strip_prefix("0X"))
    {
        u16::from_str_radix(hex, 16)
    } else {
        trimmed.parse::<u16>()
    };
    parsed.map_err(|_| format!("invalid address `{value}`"))
}

fn decode_table(memory: &[u8], table_ptr: u16) -> Result<Vec<SymbolEntry>, String> {
    let st_high = read_word(memory, table_ptr) as usize;
    if st_high == 0 || st_high + 0x1FF >= memory.len() {
        return Err(format!(
            "table pointer ${table_ptr:04X} contains invalid index address ${st_high:04X}"
        ));
    }

    let st_low = st_high + 256;
    let mut entries = Vec::new();
    for slot in 0..=255u16 {
        let high = memory[st_high + slot as usize];
        if high == 0 {
            continue;
        }
        let low = memory[st_low + slot as usize];
        let name_addr = u16::from(low) | (u16::from(high) << 8);
        if let Some(entry) = decode_entry(memory, slot as u8, name_addr) {
            entries.push(entry);
        }
    }
    entries.sort_by(|left, right| {
        left.name
            .to_ascii_uppercase()
            .cmp(&right.name.to_ascii_uppercase())
            .then(left.name_addr.cmp(&right.name_addr))
    });
    Ok(entries)
}

fn decode_entry(memory: &[u8], slot: u8, name_addr: u16) -> Option<SymbolEntry> {
    let name_index = name_addr as usize;
    if name_index >= memory.len() {
        return None;
    }
    let name_len = memory[name_index] as usize;
    let name_start = name_index + 1;
    let name_end = name_start.checked_add(name_len)?;
    let entry = name_end;
    if name_len == 0 || entry + 3 >= memory.len() {
        return None;
    }

    let vtype = memory[entry];
    if vtype == 0x88 {
        return None;
    }

    let name = decode_atascii_name(&memory[name_start..name_end]);
    let address = if vtype == 27 {
        None
    } else {
        Some(read_word(memory, entry as u16 + 1))
    };
    let class = describe_type(memory, entry, vtype);
    let args = if is_routine(vtype) {
        let numargs = memory[entry + 3] as usize;
        (0..numargs)
            .filter_map(|index| memory.get(entry + 4 + index))
            .map(|arg| describe_type(memory, entry, arg | 0x80))
            .collect()
    } else {
        Vec::new()
    };

    Some(SymbolEntry {
        slot,
        name_addr,
        name,
        vtype,
        address,
        class,
        args,
    })
}

fn describe_type(memory: &[u8], entry: usize, vtype: u8) -> String {
    if vtype == 27 {
        return format!("DEFINE `{}`", decode_define(memory, entry + 3));
    }
    if vtype == 39 {
        return "TYPE".to_string();
    }

    let mut parts = Vec::new();
    if is_routine(vtype) {
        if (vtype & 0xF7) == 0xC0 {
            parts.push("PROC".to_string());
        } else {
            parts.push(format!("{} FUNC", base_type(vtype)));
        }
    } else if vtype < 128 {
        if (vtype & 7) == 0 {
            if (vtype & 8) == 8 {
                parts.push("RECORD POINTER".to_string());
            } else {
                parts.push("RECORD".to_string());
            }
        } else {
            parts.push(format!("{} record field", base_type(vtype)));
        }
    } else {
        parts.push(base_type(vtype).to_string());
        if (vtype & 0x10) != 0 {
            parts.push("ARRAY".to_string());
        }
    }
    parts.join(" ")
}

fn is_routine(vtype: u8) -> bool {
    (vtype & 0x40) != 0 && (vtype & 0x10) == 0
}

fn base_type(vtype: u8) -> &'static str {
    match vtype & 7 {
        1 => "CHAR",
        2 => "BYTE",
        3 => "INT",
        4 => "CARD",
        _ => "",
    }
}

fn decode_define(memory: &[u8], address: usize) -> String {
    if address >= memory.len() {
        return String::new();
    }
    let len = memory[address] as usize;
    let start = address + 1;
    let end = start.saturating_add(len).min(memory.len());
    decode_atascii_name(&memory[start..end])
}

fn decode_atascii_name(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|byte| match byte {
            0x20..=0x7E => *byte as char,
            _ => '.',
        })
        .collect()
}

fn read_word(memory: &[u8], address: u16) -> u16 {
    let index = address as usize;
    u16::from(memory[index]) | (u16::from(memory[index + 1]) << 8)
}

fn print_markdown(entries: &[SymbolEntry]) {
    println!("| Slot | Name Addr | Name | Address | Type | Args |");
    println!("| --- | --- | --- | --- | --- | --- |");
    for entry in entries {
        let address = entry
            .address
            .map(|address| format!("${address:04X}"))
            .unwrap_or_else(|| "-".to_string());
        let args = if entry.args.is_empty() {
            String::new()
        } else {
            entry.args.join(", ")
        };
        println!(
            "| ${:02X} | ${:04X} | `{}` | {} | `{}` | `{}` |",
            entry.slot,
            entry.name_addr,
            escape_markdown(&entry.name),
            address,
            escape_markdown(&entry.class),
            escape_markdown(&args)
        );
    }
}

fn print_json(entries: &[SymbolEntry]) {
    println!("[");
    for (index, entry) in entries.iter().enumerate() {
        let comma = if index + 1 == entries.len() { "" } else { "," };
        let address = entry
            .address
            .map(|address| format!("\"${address:04X}\""))
            .unwrap_or_else(|| "null".to_string());
        let args = entry
            .args
            .iter()
            .map(|arg| format!("\"{}\"", escape_json(arg)))
            .collect::<Vec<_>>()
            .join(", ");
        println!(
            "  {{\"slot\":\"${:02X}\",\"name_addr\":\"${:04X}\",\"name\":\"{}\",\"vtype\":\"${:02X}\",\"address\":{},\"class\":\"{}\",\"args\":[{}]}}{}",
            entry.slot,
            entry.name_addr,
            escape_json(&entry.name),
            entry.vtype,
            address,
            escape_json(&entry.class),
            args,
            comma
        );
    }
    println!("]");
}

fn escape_markdown(value: &str) -> String {
    value.replace('|', "\\|").replace('`', "\\`")
}

fn escape_json(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_routine_entry_from_official_table_shape() {
        let mut memory = vec![0; MEMORY_SIZE];
        memory[0x00B1] = 0x00;
        memory[0x00B2] = 0x20;
        memory[0x2001] = 0x30;
        memory[0x2101] = 0x00;

        let name = b"Plot";
        memory[0x3000] = name.len() as u8;
        memory[0x3001..0x3005].copy_from_slice(name);
        memory[0x3005] = 0xC0;
        memory[0x3006] = 0xC3;
        memory[0x3007] = 0xA6;
        memory[0x3008] = 2;
        memory[0x3009] = 4;
        memory[0x300A] = 2;

        let entries = decode_table(&memory, DEFAULT_GLOBAL_TABLE_PTR).unwrap();

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "Plot");
        assert_eq!(entries[0].address, Some(0xA6C3));
        assert_eq!(entries[0].class, "PROC");
        assert_eq!(entries[0].args, vec!["CARD", "BYTE"]);
    }
}
