use std::collections::HashSet;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process;

use atrcopy_rs::atascii_to_ascii;

fn main() {
    let mut args = env::args().skip(1);
    let Some(path) = args.next() else {
        print_help();
        process::exit(2);
    };

    let command = args.next();
    let options: Vec<String> = args.collect();
    let data = match fs::read(&path) {
        Ok(data) => data,
        Err(err) => {
            eprintln!("failed to read {path}: {err}");
            process::exit(1);
        }
    };

    let image = match AtrImage::parse(data) {
        Ok(image) => image,
        Err(err) => {
            eprintln!("{path}: {err}");
            process::exit(1);
        }
    };

    match command.as_deref() {
        None | Some("list") | Some("ls") => print_directory(&path, &image),
        Some("extract") | Some("x") => {
            let mut all = false;
            let mut out_dir = PathBuf::from(".");
            let mut text_mode = TextMode::Auto;
            let mut wanted = Vec::new();
            let mut iter = options.iter();
            while let Some(option) = iter.next() {
                match option.as_str() {
                    "--all" => all = true,
                    "-o" | "--out-dir" => {
                        let Some(dir) = iter.next() else {
                            eprintln!("{path}: expected directory after {option}");
                            process::exit(2);
                        };
                        out_dir = PathBuf::from(dir);
                    }
                    "--raw-only" => text_mode = TextMode::Never,
                    "--text" => {
                        let Some(mode) = iter.next() else {
                            eprintln!("{path}: expected auto, always, or never after --text");
                            process::exit(2);
                        };
                        text_mode = match TextMode::parse(mode) {
                            Some(mode) => mode,
                            None => {
                                eprintln!("{path}: invalid --text mode `{mode}`");
                                process::exit(2);
                            }
                        };
                    }
                    option if option.starts_with("--text=") => {
                        let mode = option.trim_start_matches("--text=");
                        text_mode = match TextMode::parse(mode) {
                            Some(mode) => mode,
                            None => {
                                eprintln!("{path}: invalid --text mode `{mode}`");
                                process::exit(2);
                            }
                        };
                    }
                    _ => wanted.push(option.clone()),
                }
            }
            if !all && wanted.is_empty() {
                eprintln!("{path}: extract needs --all or at least one Atari filename");
                process::exit(2);
            }
            if let Err(err) = extract_files(&image, all, &wanted, &out_dir, text_mode) {
                eprintln!("{path}: {err}");
                process::exit(1);
            }
        }
        Some("add") | Some("put-copy") => {
            let mut output = None;
            let mut specs = Vec::new();
            let mut iter = options.iter();
            while let Some(option) = iter.next() {
                match option.as_str() {
                    "-o" | "--output" => {
                        let Some(path) = iter.next() else {
                            eprintln!("{path}: expected output ATR after {option}");
                            process::exit(2);
                        };
                        output = Some(PathBuf::from(path));
                    }
                    option if option.starts_with("--output=") => {
                        output = Some(PathBuf::from(option.trim_start_matches("--output=")));
                    }
                    _ => specs.push(option.clone()),
                }
            }
            let Some(output) = output else {
                eprintln!("{path}: add needs -o <output.atr>");
                process::exit(2);
            };
            if output == PathBuf::from(&path) {
                eprintln!("{path}: add output must be a different ATR path");
                process::exit(2);
            }
            if specs.is_empty() {
                eprintln!("{path}: add needs at least one host file");
                process::exit(2);
            }
            let additions = match parse_add_specs(&specs) {
                Ok(additions) => additions,
                Err(err) => {
                    eprintln!("{path}: {err}");
                    process::exit(2);
                }
            };
            match image.add_files(&additions) {
                Ok(output_image) => {
                    if let Err(err) = fs::write(&output, output_image.bytes) {
                        eprintln!("failed to write {}: {err}", output.display());
                        process::exit(1);
                    }
                    println!("wrote {}", output.display());
                }
                Err(err) => {
                    eprintln!("{path}: {err}");
                    process::exit(1);
                }
            }
        }
        Some("-h") | Some("--help") | Some("help") => print_help(),
        Some(command) => {
            eprintln!("unknown command: {command}");
            print_help();
            process::exit(2);
        }
    }
}

fn print_help() {
    eprintln!("usage:");
    eprintln!("  atrcopy-rs <disk.atr> [list]");
    eprintln!("  atrcopy-rs <disk.atr> extract --all [-o <dir>] [--text=auto|always|never]");
    eprintln!(
        "  atrcopy-rs <disk.atr> extract <NAME.EXT>... [-o <dir>] [--text=auto|always|never]"
    );
    eprintln!("  atrcopy-rs <disk.atr> extract ... --raw-only");
    eprintln!("  atrcopy-rs <disk.atr> add -o <out.atr> <host-file>[=<ATARI.EXT>]...");
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TextMode {
    Auto,
    Always,
    Never,
}

impl TextMode {
    fn parse(value: &str) -> Option<Self> {
        match value {
            "auto" => Some(Self::Auto),
            "always" => Some(Self::Always),
            "never" => Some(Self::Never),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
struct AtrImage {
    bytes: Vec<u8>,
    sector_size: usize,
    sectors: usize,
}

impl AtrImage {
    fn parse(bytes: Vec<u8>) -> Result<Self, String> {
        if bytes.len() < 16 {
            return Err("file is too small to be an ATR image".to_string());
        }
        if bytes[0] != 0x96 || bytes[1] != 0x02 {
            return Err("missing ATR magic $0296".to_string());
        }
        let sector_size = u16::from_le_bytes([bytes[4], bytes[5]]) as usize;
        if !matches!(sector_size, 128 | 256) {
            return Err(format!("unsupported sector size {sector_size}"));
        }
        let payload = bytes.len() - 16;
        let sectors = if sector_size == 128 {
            payload / 128
        } else if payload <= 384 {
            payload / 128
        } else {
            3 + ((payload - 384) / sector_size)
        };
        Ok(Self {
            bytes,
            sector_size,
            sectors,
        })
    }

    fn sector(&self, sector: u16) -> Option<&[u8]> {
        let (start, size) = self.sector_range(sector)?;
        self.bytes.get(start..start + size)
    }

    fn sector_mut(&mut self, sector: u16) -> Option<&mut [u8]> {
        let (start, size) = self.sector_range(sector)?;
        self.bytes.get_mut(start..start + size)
    }

    fn sector_range(&self, sector: u16) -> Option<(usize, usize)> {
        if sector == 0 || usize::from(sector) > self.sectors {
            return None;
        }
        let index = usize::from(sector) - 1;
        let start = if self.sector_size == 128 || index < 3 {
            16 + index * 128
        } else {
            16 + 384 + (index - 3) * self.sector_size
        };
        let size = if self.sector_size == 128 || index < 3 {
            128
        } else {
            self.sector_size
        };
        Some((start, size))
    }

    fn directory_tree(&self) -> Result<Vec<TreeEntry>, String> {
        let mut entries = Vec::new();
        let mut visited = HashSet::new();
        self.collect_directory_tree("", 361, 8, &mut visited, &mut entries)?;
        Ok(entries)
    }

    fn collect_directory_tree(
        &self,
        prefix: &str,
        start_sector: u16,
        sector_count: u16,
        visited: &mut HashSet<u16>,
        output: &mut Vec<TreeEntry>,
    ) -> Result<(), String> {
        if !visited.insert(start_sector) {
            return Ok(());
        }

        for entry in self.directory_from_range(start_sector, sector_count)? {
            let path = if prefix.is_empty() {
                entry.name.clone()
            } else {
                format!("{prefix}/{}", entry.name)
            };
            output.push(TreeEntry {
                path: path.clone(),
                entry: entry.clone(),
            });
            if entry.is_directory() && !entry.is_deleted() {
                self.collect_directory_tree(
                    &path,
                    entry.start_sector,
                    entry.sector_count,
                    visited,
                    output,
                )?;
            }
        }

        Ok(())
    }

    fn directory_from_range(
        &self,
        start_sector: u16,
        sector_count: u16,
    ) -> Result<Vec<DirEntry>, String> {
        let mut entries = Vec::new();
        for sector in start_sector..start_sector.saturating_add(sector_count) {
            let Some(data) = self.sector(sector) else {
                continue;
            };
            for entry in data.chunks_exact(16) {
                if let Some(dir_entry) = DirEntry::parse(entry) {
                    entries.push(dir_entry);
                }
            }
        }
        Ok(entries)
    }

    fn read_file(&self, entry: &DirEntry) -> Result<Vec<u8>, String> {
        let mut sector = entry.start_sector;
        let mut remaining = entry.sector_count;
        let mut output = Vec::new();

        while sector != 0 && remaining > 0 {
            let data = self
                .sector(sector)
                .ok_or_else(|| format!("file {} references missing sector {sector}", entry.name))?;
            if data.len() < 128 {
                return Err(format!("sector {sector} is too small"));
            }

            let tail = SectorTail::from_sector(data)?;
            let used = usize::from(tail.used);
            output.extend_from_slice(&data[..used]);
            sector = tail.next_sector;
            remaining -= 1;
        }

        Ok(output)
    }

    fn add_files(mut self, additions: &[AddSpec]) -> Result<Self, String> {
        let mut targets = HashSet::new();
        for addition in additions {
            let target = addition.target_name()?;
            if !targets.insert(target.clone()) {
                return Err(format!("duplicate target filename `{target}`"));
            }
        }

        let mut used = self.used_sector_map(additions)?;
        for addition in additions {
            let data = fs::read(&addition.host_path)
                .map_err(|err| format!("failed to read {}: {err}", addition.host_path.display()))?;
            let target = addition.target_name()?;
            let (directory_slot, flags) = self.root_entry_slot(&target)?;
            let sectors = self.allocate_file_sectors(data.len(), &mut used)?;
            self.write_file_chain(&sectors, &data, directory_slot)?;
            let encoded_name = encode_dos_filename(&target)?;
            self.write_root_dir_slot(
                directory_slot,
                &encoded_name,
                sectors.len() as u16,
                sectors[0],
                flags,
            )?;
            println!(
                "added {} as {} ({} bytes, {} sectors)",
                addition.host_path.display(),
                target,
                data.len(),
                sectors.len()
            );
        }
        self.write_vtoc_used_sector_map(&used)?;
        Ok(self)
    }

    fn used_sector_map(&self, additions: &[AddSpec]) -> Result<Vec<bool>, String> {
        let mut used = self.vtoc_used_sector_map()?;

        let replacement_names: HashSet<String> = additions
            .iter()
            .map(|addition| addition.target_name())
            .collect::<Result<HashSet<_>, _>>()?;

        let entries = self.directory_tree()?;
        for entry in &entries {
            if entry.entry.is_deleted() {
                continue;
            }
            let normalized = normalize_filename(&entry.path);
            if !normalized.contains('/') && replacement_names.contains(&normalized) {
                if entry.entry.is_directory() {
                    return Err(format!(
                        "cannot replace directory {} with a file",
                        entry.entry.name
                    ));
                }
                self.mark_file_chain_free(&entry.entry, &mut used)?;
            }
        }
        for entry in &entries {
            if entry.entry.is_deleted() {
                continue;
            }
            let normalized = normalize_filename(&entry.path);
            if !normalized.contains('/') && replacement_names.contains(&normalized) {
                continue;
            }
            self.mark_entry_sectors_used(&entry.entry, &mut used)?;
        }

        self.reserve_filesystem_sectors(&mut used)?;
        Ok(used)
    }

    fn mark_file_chain_free(&self, entry: &DirEntry, used: &mut [bool]) -> Result<(), String> {
        for sector in self.file_chain_sectors(entry)? {
            used[sector] = false;
        }
        Ok(())
    }

    fn mark_entry_sectors_used(&self, entry: &DirEntry, used: &mut [bool]) -> Result<(), String> {
        if entry.is_directory() {
            let start = usize::from(entry.start_sector);
            let end = start
                .checked_add(usize::from(entry.sector_count))
                .ok_or_else(|| format!("directory {} sector range overflows", entry.name))?;
            if end > used.len() {
                return Err(format!(
                    "directory {} references sectors outside the disk",
                    entry.name
                ));
            }
            used[start..end].fill(true);
        } else {
            for sector in self.file_chain_sectors(entry)? {
                used[sector] = true;
            }
        }
        Ok(())
    }

    fn file_chain_sectors(&self, entry: &DirEntry) -> Result<Vec<usize>, String> {
        let mut sector = entry.start_sector;
        let mut remaining = entry.sector_count;
        let mut visited = HashSet::new();
        let mut sectors = Vec::with_capacity(usize::from(remaining));
        while remaining > 0 {
            if sector == 0 {
                return Err(format!(
                    "file {} chain ends before its {} sectors were read",
                    entry.name, entry.sector_count
                ));
            }
            if !visited.insert(sector) {
                return Err(format!(
                    "file {} contains a cycle at sector {sector}",
                    entry.name
                ));
            }
            let index = usize::from(sector);
            if index > self.sectors {
                return Err(format!(
                    "file {} references missing sector {sector}",
                    entry.name
                ));
            }
            sectors.push(index);
            let data = self
                .sector(sector)
                .ok_or_else(|| format!("file {} references missing sector {sector}", entry.name))?;
            let tail = SectorTail::from_sector(data)?;
            sector = tail.next_sector;
            remaining -= 1;
        }
        Ok(sectors)
    }

    fn vtoc_used_sector_map(&self) -> Result<Vec<bool>, String> {
        let version = self.vtoc_version()?;
        let mut used = vec![true; self.sectors + 1];
        for (sector, is_used) in used.iter_mut().enumerate().skip(1) {
            let (vtoc_sector, byte_index, mask) = self.vtoc_bitmap_location(sector, version)?;
            let vtoc = self
                .sector(vtoc_sector)
                .ok_or_else(|| format!("missing VTOC sector {vtoc_sector}"))?;
            *is_used = vtoc[byte_index] & mask == 0;
        }
        self.reserve_filesystem_sectors(&mut used)?;
        Ok(used)
    }

    fn write_vtoc_used_sector_map(&mut self, used: &[bool]) -> Result<(), String> {
        if used.len() <= self.sectors {
            return Err("VTOC allocation map is smaller than the disk".to_string());
        }

        let version = self.vtoc_version()?;
        for (sector, &is_used) in used.iter().enumerate().take(self.sectors + 1).skip(1) {
            let (vtoc_sector, byte_index, mask) = self.vtoc_bitmap_location(sector, version)?;
            let vtoc = self
                .sector_mut(vtoc_sector)
                .ok_or_else(|| format!("missing VTOC sector {vtoc_sector}"))?;
            if is_used {
                vtoc[byte_index] &= !mask;
            } else {
                vtoc[byte_index] |= mask;
            }
        }

        let free_count = used[1..=self.sectors]
            .iter()
            .filter(|&&is_used| !is_used)
            .count();
        let free_count = u16::try_from(free_count)
            .map_err(|_| format!("free sector count {free_count} exceeds the VTOC field"))?;
        let vtoc = self
            .sector_mut(360)
            .ok_or_else(|| "missing VTOC sector 360".to_string())?;
        vtoc[3..5].copy_from_slice(&free_count.to_le_bytes());
        Ok(())
    }

    fn vtoc_version(&self) -> Result<u8, String> {
        let version = self
            .sector(360)
            .and_then(|vtoc| vtoc.first())
            .copied()
            .ok_or_else(|| "missing VTOC sector 360".to_string())?;
        if matches!(version, 0x02 | 0x03) {
            Ok(version)
        } else {
            Err(format!("unsupported VTOC format ${version:02X}"))
        }
    }

    fn vtoc_bitmap_location(&self, sector: usize, version: u8) -> Result<(u16, usize, u8), String> {
        let first_bitmap_bits = (self.sector_size - 10) * 8;
        let (vtoc_sector, byte_index) = if sector < first_bitmap_bits {
            (360, 10 + sector / 8)
        } else {
            // MYDOS continues its bitmap at byte 0 of sectors 359, 358, and so on.
            if version != 0x03 {
                return Err(format!(
                    "VTOC format ${version:02X} does not describe sector {sector}"
                ));
            }
            let extension_bit = sector - first_bitmap_bits;
            let bits_per_sector = self.sector_size * 8;
            let extension_index = extension_bit / bits_per_sector;
            let extension_index = u16::try_from(extension_index)
                .map_err(|_| format!("sector {sector} needs too many VTOC extensions"))?;
            let vtoc_sector = 359u16.checked_sub(extension_index).ok_or_else(|| {
                format!("sector {sector} cannot be represented by this MYDOS VTOC")
            })?;
            (vtoc_sector, (extension_bit % bits_per_sector) / 8)
        };
        let mask = 1 << (7 - sector % 8);
        Ok((vtoc_sector, byte_index, mask))
    }

    fn reserve_filesystem_sectors(&self, used: &mut [bool]) -> Result<(), String> {
        if self.sectors >= 1 {
            used[1..=3.min(self.sectors)].fill(true);
        }
        if self.sectors >= 360 {
            used[360..=368.min(self.sectors)].fill(true);
        }

        let version = self.vtoc_version()?;
        if self.sectors > 0 {
            let (lowest_vtoc_sector, _, _) = self.vtoc_bitmap_location(self.sectors, version)?;
            for sector in lowest_vtoc_sector..360 {
                let sector = usize::from(sector);
                if sector < used.len() {
                    used[sector] = true;
                }
            }
        }
        Ok(())
    }

    fn allocate_file_sectors(&self, len: usize, used: &mut [bool]) -> Result<Vec<u16>, String> {
        let capacity = self.data_sector_capacity();
        let needed = len.max(1).div_ceil(capacity);
        let mut sectors = Vec::with_capacity(needed);
        for (sector, is_used) in used
            .iter_mut()
            .enumerate()
            .take(self.sectors.min(0x03ff) + 1)
            .skip(4)
        {
            if !*is_used {
                *is_used = true;
                sectors.push(sector as u16);
                if sectors.len() == needed {
                    return Ok(sectors);
                }
            }
        }
        Err(format!(
            "not enough free sectors: need {needed}, found {}",
            sectors.len()
        ))
    }

    fn data_sector_capacity(&self) -> usize {
        if self.sector_size == 128 {
            125
        } else {
            253
        }
    }

    fn write_file_chain(
        &mut self,
        sectors: &[u16],
        data: &[u8],
        directory_slot: usize,
    ) -> Result<(), String> {
        let capacity = self.data_sector_capacity();
        for (index, &sector) in sectors.iter().enumerate() {
            let start = index * capacity;
            let end = (start + capacity).min(data.len());
            let chunk = &data[start..end];
            let next = sectors.get(index + 1).copied().unwrap_or(0);
            let sector_data = self
                .sector_mut(sector)
                .ok_or_else(|| format!("allocated missing sector {sector}"))?;
            sector_data.fill(0);
            sector_data[..chunk.len()].copy_from_slice(chunk);
            SectorTail {
                next_sector: next,
                used: chunk.len() as u8,
            }
            .write_to_sector(sector_data, directory_slot)?;
        }
        Ok(())
    }

    fn root_entry_slot(&self, target: &str) -> Result<(usize, u8), String> {
        let mut empty_slot = None;
        let mut deleted_slot = None;
        let mut default_flags = None;

        for slot in 0..64 {
            let sector = 361 + (slot / 8) as u16;
            let offset = (slot % 8) * 16;
            let entry = &self
                .sector(sector)
                .ok_or_else(|| format!("missing root directory sector {sector}"))?
                [offset..offset + 16];
            let flags = entry[0];
            if flags == 0x00 {
                empty_slot.get_or_insert(slot);
            } else if flags & 0x80 != 0 {
                deleted_slot.get_or_insert(slot);
            } else if dos_filename(&entry[5..16]).eq_ignore_ascii_case(target) {
                return Ok((slot, flags));
            } else if flags & 0x10 == 0 {
                default_flags.get_or_insert(flags);
            }
        }

        empty_slot
            .or(deleted_slot)
            .map(|slot| (slot, default_flags.unwrap_or(0x42)))
            .ok_or_else(|| "root directory has no free entries".to_string())
    }

    fn write_root_dir_slot(
        &mut self,
        slot: usize,
        encoded_name: &[u8; 11],
        sector_count: u16,
        start_sector: u16,
        flags: u8,
    ) -> Result<(), String> {
        let sector = 361 + (slot / 8) as u16;
        let offset = (slot % 8) * 16;
        let entry = &mut self
            .sector_mut(sector)
            .ok_or_else(|| format!("missing root directory sector {sector}"))?[offset..offset + 16];
        entry.fill(0);
        entry[0] = flags;
        entry[1..3].copy_from_slice(&sector_count.to_le_bytes());
        entry[3..5].copy_from_slice(&start_sector.to_le_bytes());
        entry[5..16].copy_from_slice(encoded_name);
        Ok(())
    }
}

#[derive(Debug, Clone)]
struct AddSpec {
    host_path: PathBuf,
    atari_name: Option<String>,
}

impl AddSpec {
    fn target_name(&self) -> Result<String, String> {
        let name = if let Some(name) = &self.atari_name {
            name.clone()
        } else {
            self.host_path
                .file_name()
                .and_then(|name| name.to_str())
                .ok_or_else(|| format!("{} has no file name", self.host_path.display()))?
                .to_string()
        };
        let normalized = normalize_filename(&name);
        if normalized.contains('/') {
            return Err(format!(
                "writing into subdirectories is not supported yet: {name}"
            ));
        }
        encode_dos_filename(&normalized)?;
        Ok(normalized)
    }
}

#[derive(Debug, Clone)]
struct TreeEntry {
    path: String,
    entry: DirEntry,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SectorTail {
    next_sector: u16,
    used: u8,
}

impl SectorTail {
    fn from_sector(data: &[u8]) -> Result<Self, String> {
        match data.len() {
            128 => Ok(Self {
                next_sector: (u16::from(data[125] & 0x03) << 8) | u16::from(data[126]),
                used: data[127].min(125),
            }),
            256 => Ok(Self {
                next_sector: (u16::from(data[253] & 0x03) << 8) | u16::from(data[254]),
                used: data[255].min(253),
            }),
            len => Err(format!("unsupported sector payload size {len}")),
        }
    }

    fn write_to_sector(self, data: &mut [u8], directory_slot: usize) -> Result<(), String> {
        match data.len() {
            128 => {
                if self.next_sector > 0x03ff {
                    return Err(format!(
                        "128-byte sector chain target {} is too large",
                        self.next_sector
                    ));
                }
                if directory_slot > 0x3f {
                    return Err(format!(
                        "directory slot {directory_slot} is too large for Atari DOS sector chains"
                    ));
                }
                data[125] = ((directory_slot as u8) << 2) | ((self.next_sector >> 8) as u8 & 0x03);
                data[126] = self.next_sector as u8;
                data[127] = self.used.min(125);
                Ok(())
            }
            256 => {
                if self.next_sector > 0x03ff {
                    return Err(format!(
                        "256-byte sector chain target {} is too large",
                        self.next_sector
                    ));
                }
                if directory_slot > 0x3f {
                    return Err(format!(
                        "directory slot {directory_slot} is too large for Atari DOS sector chains"
                    ));
                }
                data[253] = ((directory_slot as u8) << 2) | ((self.next_sector >> 8) as u8 & 0x03);
                data[254] = self.next_sector as u8;
                data[255] = self.used.min(253);
                Ok(())
            }
            len => Err(format!("unsupported sector payload size {len}")),
        }
    }
}

#[derive(Debug, Clone)]
struct DirEntry {
    flags: u8,
    sector_count: u16,
    start_sector: u16,
    name: String,
}

impl DirEntry {
    fn parse(bytes: &[u8]) -> Option<Self> {
        let flags = bytes[0];
        if flags == 0x00 || flags == 0x80 {
            return None;
        }
        let sector_count = u16::from_le_bytes([bytes[1], bytes[2]]);
        let start_sector = u16::from_le_bytes([bytes[3], bytes[4]]);
        let name = dos_filename(&bytes[5..16]);
        if name.is_empty() {
            return None;
        }
        Some(Self {
            flags,
            sector_count,
            start_sector,
            name,
        })
    }

    fn is_deleted(&self) -> bool {
        self.flags & 0x80 != 0
    }

    fn is_locked(&self) -> bool {
        self.flags & 0x20 != 0
    }

    fn is_directory(&self) -> bool {
        self.flags & 0x10 != 0
    }
}

fn dos_filename(bytes: &[u8]) -> String {
    let base = atascii_filename_part(&bytes[..8]);
    let ext = atascii_filename_part(&bytes[8..11]);
    if ext.is_empty() {
        base
    } else {
        format!("{base}.{ext}")
    }
}

fn atascii_filename_part(bytes: &[u8]) -> String {
    bytes
        .iter()
        .copied()
        .take_while(|byte| *byte != b' ')
        .map(|byte| {
            let byte = byte & 0x7f;
            if byte.is_ascii_graphic() {
                byte as char
            } else {
                '_'
            }
        })
        .collect()
}

fn print_directory(path: &str, image: &AtrImage) {
    let entries = match image.directory_tree() {
        Ok(entries) => entries,
        Err(err) => {
            eprintln!("{path}: {err}");
            process::exit(1);
        }
    };
    println!(
        "{path}: ATR sector_size={} sectors={} files={}",
        image.sector_size,
        image.sectors,
        entries.len()
    );
    for entry in entries {
        let marker = if entry.entry.is_deleted() {
            "D"
        } else if entry.entry.is_directory() {
            "/"
        } else if entry.entry.is_locked() {
            "L"
        } else {
            " "
        };
        println!(
            "{marker} {:>4} {:>4} {}",
            entry.entry.start_sector, entry.entry.sector_count, entry.path
        );
    }
}

fn extract_files(
    image: &AtrImage,
    all: bool,
    wanted: &[String],
    out_dir: &Path,
    text_mode: TextMode,
) -> Result<(), String> {
    fs::create_dir_all(out_dir)
        .map_err(|err| format!("failed to create {}: {err}", out_dir.display()))?;
    let entries = image.directory_tree()?;
    let wanted: Vec<String> = wanted.iter().map(|name| normalize_filename(name)).collect();
    let mut extracted = 0usize;

    for entry in entries
        .iter()
        .filter(|entry| !entry.entry.is_deleted() && !entry.entry.is_directory())
    {
        if !all && !wanted.iter().any(|name| wanted_matches_entry(name, entry)) {
            continue;
        }
        let bytes = image.read_file(&entry.entry)?;
        let out_path = out_dir.join(host_path(&entry.path));
        if let Some(parent) = out_path.parent() {
            fs::create_dir_all(parent)
                .map_err(|err| format!("failed to create {}: {err}", parent.display()))?;
        }
        if should_decode_text(&entry.path, text_mode) {
            let raw_path = raw_atascii_path(&out_path);
            fs::write(&raw_path, &bytes)
                .map_err(|err| format!("failed to write {}: {err}", raw_path.display()))?;
            fs::write(&out_path, atascii_to_ascii(&bytes))
                .map_err(|err| format!("failed to write {}: {err}", out_path.display()))?;
            println!(
                "extracted {} -> {} (+ raw {})",
                entry.path,
                out_path.display(),
                raw_path.display()
            );
        } else {
            fs::write(&out_path, bytes)
                .map_err(|err| format!("failed to write {}: {err}", out_path.display()))?;
            println!("extracted {} -> {}", entry.path, out_path.display());
        }
        extracted += 1;
    }

    if extracted == 0 {
        return Err("no matching files found".to_string());
    }
    Ok(())
}

fn parse_add_specs(specs: &[String]) -> Result<Vec<AddSpec>, String> {
    let mut additions = Vec::new();
    for spec in specs {
        let (host, atari_name) = match spec.split_once('=') {
            Some((host, atari)) if !host.is_empty() && !atari.is_empty() => {
                (host, Some(atari.to_string()))
            }
            Some(_) => {
                return Err(format!(
                    "invalid add spec `{spec}`; expected host[=ATARI.EXT]"
                ))
            }
            None => (spec.as_str(), None),
        };
        additions.push(AddSpec {
            host_path: PathBuf::from(host),
            atari_name,
        });
    }
    Ok(additions)
}

fn encode_dos_filename(name: &str) -> Result<[u8; 11], String> {
    let name = normalize_filename(name);
    if name.is_empty() {
        return Err("Atari filename cannot be empty".to_string());
    }
    let mut parts = name.split('.');
    let base = parts.next().unwrap_or_default();
    let ext = parts.next();
    if parts.next().is_some() {
        return Err(format!(
            "Atari filename `{name}` has more than one extension"
        ));
    }
    if base.is_empty() || base.len() > 8 {
        return Err(format!(
            "Atari filename `{name}` must have a 1..8 character base name"
        ));
    }
    if ext.is_some_and(|ext| ext.len() > 3) {
        return Err(format!(
            "Atari filename `{name}` must have an extension up to 3 characters"
        ));
    }
    let mut encoded = [b' '; 11];
    encode_filename_part(base, &mut encoded[..8], &name)?;
    if let Some(ext) = ext {
        encode_filename_part(ext, &mut encoded[8..11], &name)?;
    }
    Ok(encoded)
}

fn encode_filename_part(part: &str, output: &mut [u8], full_name: &str) -> Result<(), String> {
    for (index, byte) in part.bytes().enumerate() {
        if !byte.is_ascii_alphanumeric() && !matches!(byte, b'_' | b'-') {
            return Err(format!(
                "Atari filename `{full_name}` contains unsupported character `{}`",
                byte as char
            ));
        }
        output[index] = byte.to_ascii_uppercase();
    }
    Ok(())
}

fn should_decode_text(path: &str, mode: TextMode) -> bool {
    match mode {
        TextMode::Always => true,
        TextMode::Never => false,
        TextMode::Auto => text_like_extension(path),
    }
}

fn text_like_extension(path: &str) -> bool {
    let Some(ext) = path.rsplit('.').next() else {
        return false;
    };
    matches!(
        ext.to_ascii_uppercase().as_str(),
        "ACT" | "ASM" | "DOC" | "TXT" | "EXC" | "HLP" | "LST" | "BAS" | "DEM" | "DM1" | "DM2"
    )
}

fn raw_atascii_path(path: &Path) -> PathBuf {
    let mut name = path
        .file_name()
        .map(|name| name.to_os_string())
        .unwrap_or_default();
    name.push(".atascii");
    path.with_file_name(name)
}

fn normalize_filename(name: &str) -> String {
    name.trim()
        .replace('\\', "/")
        .trim_start_matches("./")
        .to_ascii_uppercase()
}

fn host_filename(name: &str) -> String {
    name.chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-') {
                ch
            } else {
                '_'
            }
        })
        .collect()
}

fn host_path(path: &str) -> PathBuf {
    path.split('/').map(host_filename).collect()
}

fn wanted_matches_entry(wanted: &str, entry: &TreeEntry) -> bool {
    let path = normalize_filename(&entry.path);
    let plain_name = normalize_filename(&entry.entry.name);
    let wanted = wanted.trim_end_matches('/');
    wanted == path || wanted == plain_name || path.starts_with(&format!("{wanted}/"))
}

#[cfg(test)]
mod tests {
    use super::{
        encode_dos_filename, host_path, parse_add_specs, raw_atascii_path, should_decode_text,
        wanted_matches_entry, AddSpec, AtrImage, DirEntry, SectorTail, TextMode, TreeEntry,
    };
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn image_with_vtoc(sector_size: usize, sectors: usize, version: u8) -> AtrImage {
        let payload_size = if sector_size == 128 {
            sectors * 128
        } else {
            3 * 128 + sectors.saturating_sub(3) * sector_size
        };
        let mut bytes = vec![0; 16 + payload_size];
        bytes[0..2].copy_from_slice(&0x0296u16.to_le_bytes());
        bytes[4..6].copy_from_slice(&(sector_size as u16).to_le_bytes());
        let mut image = AtrImage::parse(bytes).unwrap();
        image.sector_mut(360).unwrap()[0] = version;
        image
    }

    fn blank_dos_image(sector_size: usize, sectors: usize, version: u8) -> AtrImage {
        let mut image = image_with_vtoc(sector_size, sectors, version);
        let mut used = vec![false; sectors + 1];
        image.reserve_filesystem_sectors(&mut used).unwrap();
        image.write_vtoc_used_sector_map(&used).unwrap();
        let free_count = used[1..].iter().filter(|&&is_used| !is_used).count() as u16;
        image.sector_mut(360).unwrap()[1..3].copy_from_slice(&free_count.to_le_bytes());
        image
    }

    fn temp_host_file(name: &str, data: &[u8]) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path =
            std::env::temp_dir().join(format!("atrcopy-rs-{}-{unique}-{name}", std::process::id()));
        fs::write(&path, data).unwrap();
        path
    }

    fn add_spec(path: PathBuf, atari_name: &str) -> AddSpec {
        AddSpec {
            host_path: path,
            atari_name: Some(atari_name.to_string()),
        }
    }

    #[test]
    fn parses_128_byte_atari_dos_sector_tail() {
        let mut sector = [0u8; 128];
        sector[125] = 0x02;
        sector[126] = 0x34;
        sector[127] = 125;

        assert_eq!(
            SectorTail::from_sector(&sector).unwrap(),
            SectorTail {
                next_sector: 0x0234,
                used: 125,
            }
        );
    }

    #[test]
    fn parses_256_byte_mydos_sector_tail() {
        let mut sector = [0u8; 256];
        sector[253] = 0x1e;
        sector[254] = 0x34;
        sector[255] = 253;

        assert_eq!(
            SectorTail::from_sector(&sector).unwrap(),
            SectorTail {
                next_sector: 0x0234,
                used: 253,
            }
        );
    }

    #[test]
    fn parses_256_byte_mydos_sector_tail_with_directory_slot_bits() {
        let mut sector = [0u8; 256];
        sector[253] = 0x04;
        sector[254] = 0x17;
        sector[255] = 253;

        assert_eq!(
            SectorTail::from_sector(&sector).unwrap(),
            SectorTail {
                next_sector: 0x0017,
                used: 253,
            }
        );
    }

    #[test]
    fn writes_128_byte_atari_dos_sector_tail() {
        let mut sector = [0u8; 128];
        SectorTail {
            next_sector: 0x0234,
            used: 125,
        }
        .write_to_sector(&mut sector, 7)
        .unwrap();

        assert_eq!(sector[125], 0x1e);
        assert_eq!(sector[126], 0x34);
        assert_eq!(sector[127], 125);
    }

    #[test]
    fn writes_256_byte_mydos_sector_tail() {
        let mut sector = [0u8; 256];
        SectorTail {
            next_sector: 0x0234,
            used: 253,
        }
        .write_to_sector(&mut sector, 7)
        .unwrap();

        assert_eq!(sector[253], 0x1e);
        assert_eq!(sector[254], 0x34);
        assert_eq!(sector[255], 253);
    }

    #[test]
    fn add_uses_and_updates_dos_2_vtoc_bitmap() {
        let mut image = blank_dos_image(256, 720, 0x02);
        let mut used = image.vtoc_used_sector_map().unwrap();
        used[4] = true;
        image.write_vtoc_used_sector_map(&used).unwrap();
        let free_before =
            u16::from_le_bytes([image.sector(360).unwrap()[3], image.sector(360).unwrap()[4]]);

        let host_path = temp_host_file("new.bin", b"new file");
        let output = image
            .add_files(&[add_spec(host_path.clone(), "NEW.BIN")])
            .unwrap();
        fs::remove_file(host_path).unwrap();

        let entries = output.directory_from_range(361, 8).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].start_sector, 5);
        assert_eq!(output.read_file(&entries[0]).unwrap(), b"new file");

        let used = output.vtoc_used_sector_map().unwrap();
        assert!(used[4], "an allocated but unreferenced sector was reused");
        assert!(used[5], "the new file sector was left free in the VTOC");
        assert!(!used[6], "the next DOS allocation should start at sector 6");
        let free_after = u16::from_le_bytes([
            output.sector(360).unwrap()[3],
            output.sector(360).unwrap()[4],
        ]);
        assert_eq!(free_after, free_before - 1);
    }

    #[test]
    fn replacing_file_releases_unused_tail_sectors_in_vtoc() {
        let mut image = blank_dos_image(256, 720, 0x02);
        image.write_file_chain(&[4, 5], &[0x55; 300], 0).unwrap();
        image
            .write_root_dir_slot(0, b"REPLACE BIN", 2, 4, 0x42)
            .unwrap();
        let mut used = image.vtoc_used_sector_map().unwrap();
        used[4] = true;
        used[5] = true;
        image.write_vtoc_used_sector_map(&used).unwrap();
        let free_before =
            u16::from_le_bytes([image.sector(360).unwrap()[3], image.sector(360).unwrap()[4]]);

        let host_path = temp_host_file("replacement.bin", b"short");
        let output = image
            .add_files(&[add_spec(host_path.clone(), "REPLACE.BIN")])
            .unwrap();
        fs::remove_file(host_path).unwrap();

        let entry = &output.directory_from_range(361, 8).unwrap()[0];
        assert_eq!((entry.start_sector, entry.sector_count), (4, 1));
        assert_eq!(output.read_file(entry).unwrap(), b"short");
        let used = output.vtoc_used_sector_map().unwrap();
        assert!(used[4]);
        assert!(!used[5], "the unused part of the old chain was leaked");
        let free_after = u16::from_le_bytes([
            output.sector(360).unwrap()[3],
            output.sector(360).unwrap()[4],
        ]);
        assert_eq!(free_after, free_before + 1);
    }

    #[test]
    fn add_repairs_vtoc_bits_for_existing_directory_chains() {
        let mut image = blank_dos_image(256, 720, 0x02);
        image.write_file_chain(&[4], b"existing", 0).unwrap();
        image
            .write_root_dir_slot(0, b"EXIST   BIN", 1, 4, 0x42)
            .unwrap();
        assert!(
            !image.vtoc_used_sector_map().unwrap()[4],
            "the fixture must reproduce the old atr-copy corruption"
        );

        let host_path = temp_host_file("new.bin", b"new");
        let output = image
            .add_files(&[add_spec(host_path.clone(), "NEW.BIN")])
            .unwrap();
        fs::remove_file(host_path).unwrap();

        let entries = output.directory_from_range(361, 8).unwrap();
        assert_eq!(entries[0].start_sector, 4);
        assert_eq!(entries[1].start_sector, 5);
        assert_eq!(output.read_file(&entries[0]).unwrap(), b"existing");
        let used = output.vtoc_used_sector_map().unwrap();
        assert!(
            used[4],
            "the pre-existing file was not repaired in the VTOC"
        );
        assert!(used[5], "the added file was not recorded in the VTOC");
    }

    #[test]
    fn updates_extended_128_byte_mydos_vtoc_and_free_count() {
        let mut image = blank_dos_image(128, 1040, 0x03);
        let total_before = image.sector(360).unwrap()[1..3].to_vec();
        let free_before =
            u16::from_le_bytes([image.sector(360).unwrap()[3], image.sector(360).unwrap()[4]]);
        let mut used = image.vtoc_used_sector_map().unwrap();
        assert!(!used[1000]);
        used[1000] = true;
        image.write_vtoc_used_sector_map(&used).unwrap();

        let updated = image.vtoc_used_sector_map().unwrap();
        assert!(updated[1000]);
        assert_eq!(image.sector(360).unwrap()[1..3], total_before);
        let free_after =
            u16::from_le_bytes([image.sector(360).unwrap()[3], image.sector(360).unwrap()[4]]);
        assert_eq!(free_after, free_before - 1);
        assert_eq!(image.sector(359).unwrap()[7] & 0x80, 0);
    }

    #[test]
    fn rejects_ambiguous_extended_dos_2_vtoc() {
        let image = image_with_vtoc(128, 1040, 0x02);
        let error = image.vtoc_used_sector_map().unwrap_err();
        assert!(error.contains("does not describe sector 944"), "{error}");
    }

    #[test]
    fn encodes_dos_filename_as_padded_83_name() {
        assert_eq!(encode_dos_filename("tn.com").unwrap(), *b"TN      COM");
        assert!(encode_dos_filename("TOO-LONGG.COM").is_err());
        assert!(encode_dos_filename("BAD.NAME.X").is_err());
    }

    #[test]
    fn parses_add_specs_with_optional_atari_name() {
        let specs =
            parse_add_specs(&["build/TN-C.COM=TN.COM".to_string(), "README".to_string()]).unwrap();

        assert_eq!(
            specs[0].host_path,
            std::path::PathBuf::from("build/TN-C.COM")
        );
        assert_eq!(specs[0].atari_name.as_deref(), Some("TN.COM"));
        assert_eq!(specs[1].host_path, std::path::PathBuf::from("README"));
        assert_eq!(specs[1].atari_name, None);
    }

    #[test]
    fn matches_subdirectory_paths_and_prefixes() {
        let entry = TreeEntry {
            path: "SRC/LIB.ACT".to_string(),
            entry: DirEntry {
                flags: 0x46,
                sector_count: 1,
                start_sector: 10,
                name: "LIB.ACT".to_string(),
            },
        };

        assert!(wanted_matches_entry("SRC/LIB.ACT", &entry));
        assert!(wanted_matches_entry("LIB.ACT", &entry));
        assert!(wanted_matches_entry("SRC", &entry));
        assert!(wanted_matches_entry("SRC/", &entry));
        assert!(!wanted_matches_entry("DOCS", &entry));
    }

    #[test]
    fn converts_atari_path_to_host_path() {
        assert_eq!(
            host_path("SRC/LIB.ACT"),
            std::path::PathBuf::from("SRC/LIB.ACT")
        );
    }

    #[test]
    fn detects_text_files_in_auto_mode() {
        assert!(should_decode_text("SRC/LIB.ACT", TextMode::Auto));
        assert!(should_decode_text("README.DOC", TextMode::Auto));
        assert!(!should_decode_text("GAME.COM", TextMode::Auto));
        assert!(should_decode_text("GAME.COM", TextMode::Always));
        assert!(!should_decode_text("SRC/LIB.ACT", TextMode::Never));
    }

    #[test]
    fn raw_atascii_sidecar_appends_suffix_after_filename() {
        assert_eq!(
            raw_atascii_path(std::path::Path::new("SRC/LIB.ACT")),
            std::path::PathBuf::from("SRC/LIB.ACT.atascii")
        );
    }
}
