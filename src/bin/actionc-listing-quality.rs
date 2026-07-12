use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::fs;
use std::io::{self, Read};

fn main() {
    let args = env::args().skip(1).collect::<Vec<_>>();
    if args.len() > 1 {
        eprintln!("usage: actionc-listing-quality [listing.lst]");
        std::process::exit(2);
    }

    let input = if let Some(path) = args.first() {
        fs::read_to_string(path).unwrap_or_else(|err| {
            eprintln!("actionc-listing-quality: failed to read {path}: {err}");
            std::process::exit(1);
        })
    } else {
        let mut input = String::new();
        io::stdin()
            .read_to_string(&mut input)
            .unwrap_or_else(|err| {
                eprintln!("actionc-listing-quality: failed to read stdin: {err}");
                std::process::exit(1);
            });
        input
    };

    let listing = Listing::parse(&input);
    let metrics = ListingMetrics::from_listing(&listing);
    print!("{}", metrics.report());
}

#[derive(Debug, Default)]
struct Listing {
    instructions: Vec<ListingInstruction>,
    data_rows: Vec<ListingDataRow>,
    data_labels: Vec<ListingDataLabel>,
    procs: Vec<ListingProc>,
    spill_labels: BTreeSet<String>,
}

#[derive(Debug, Clone)]
struct ListingInstruction {
    address: u16,
    bytes: Vec<u8>,
    mnemonic: String,
    operand: String,
    proc_index: Option<usize>,
}

#[derive(Debug, Clone)]
struct ListingDataRow {
    #[allow(dead_code)]
    address: u16,
    bytes: Vec<u8>,
}

#[derive(Debug, Clone)]
struct ListingDataLabel {
    name: String,
    address: u16,
}

#[derive(Debug, Clone)]
struct ListingProc {
    name: String,
    start: u16,
    end: u16,
    instruction_indices: Vec<usize>,
}

#[derive(Debug, Clone)]
struct ListingMetrics {
    instruction_count: usize,
    code_bytes: usize,
    data_bytes: usize,
    mnemonic_counts: BTreeMap<String, usize>,
    lda_sta_instruction_percent: f64,
    lda_sta_byte_percent: f64,
    spill_data_label_count: usize,
    adjacent_sta_lda_pairs: usize,
    zero_page_absolute_accesses: usize,
    branch_over_jmp_patterns: usize,
    branch_over_jmp_branchable: usize,
    lda_cmp_zero_branch_patterns: usize,
    jsr_rts_patterns: usize,
    load_add_one_store_patterns: usize,
    load_sub_one_store_patterns: usize,
    data_like_procs: Vec<DataLikeProc>,
    routine_metrics: Vec<RoutineMetrics>,
    spill_label_metrics: Vec<SpillLabelMetrics>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DataLikeProc {
    name: String,
    start: u16,
    end: u16,
    instruction_count: usize,
    reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RoutineMetrics {
    name: String,
    start: u16,
    end: u16,
    instruction_count: usize,
    code_bytes: usize,
    lda_count: usize,
    sta_count: usize,
    lda_sta_count: usize,
    adjacent_sta_lda_pairs: usize,
    spill_accesses: usize,
    unique_spills_accessed: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SpillLabelMetrics {
    name: String,
    address: u16,
    reads: usize,
    writes: usize,
    first_access_proc: String,
    first_access_address: Option<u16>,
}

impl Listing {
    fn parse(input: &str) -> Self {
        let mut listing = Listing::default();
        let mut current_proc: Option<usize> = None;

        for line in input.lines() {
            if let Some(proc) = parse_proc_start(line) {
                listing.procs.push(proc);
                current_proc = Some(listing.procs.len() - 1);
                continue;
            }
            if line.starts_with("; ===== END PROC ") {
                current_proc = None;
                continue;
            }
            if let Some(label) = parse_data_label(line) {
                if is_spill_label(&label.name) {
                    listing.spill_labels.insert(label.name.clone());
                }
                listing.data_labels.push(label);
                continue;
            }
            if let Some(parsed) = parse_listing_row(line) {
                match parsed {
                    ParsedRow::Instruction(mut instruction) => {
                        instruction.proc_index = current_proc;
                        let index = listing.instructions.len();
                        if let Some(proc_index) = current_proc {
                            listing.procs[proc_index].instruction_indices.push(index);
                        }
                        listing.instructions.push(instruction);
                    }
                    ParsedRow::Data(row) => listing.data_rows.push(row),
                }
            }
        }

        listing
    }
}

impl ListingMetrics {
    fn from_listing(listing: &Listing) -> Self {
        let instruction_count = listing.instructions.len();
        let code_bytes = listing
            .instructions
            .iter()
            .map(|instruction| instruction.bytes.len())
            .sum::<usize>();
        let data_bytes = listing
            .data_rows
            .iter()
            .map(|row| row.bytes.len())
            .sum::<usize>();
        let mut mnemonic_counts = BTreeMap::new();
        let mut lda_sta_bytes = 0usize;
        for instruction in &listing.instructions {
            *mnemonic_counts
                .entry(instruction.mnemonic.clone())
                .or_insert(0) += 1;
            if instruction.mnemonic == "LDA" || instruction.mnemonic == "STA" {
                lda_sta_bytes += instruction.bytes.len();
            }
        }
        let lda_sta_count = mnemonic_counts.get("LDA").copied().unwrap_or(0)
            + mnemonic_counts.get("STA").copied().unwrap_or(0);

        Self {
            instruction_count,
            code_bytes,
            data_bytes,
            mnemonic_counts,
            lda_sta_instruction_percent: percent(lda_sta_count, instruction_count),
            lda_sta_byte_percent: percent(lda_sta_bytes, code_bytes),
            spill_data_label_count: listing.spill_labels.len(),
            adjacent_sta_lda_pairs: count_adjacent_sta_lda_pairs(&listing.instructions),
            zero_page_absolute_accesses: count_zero_page_absolute_accesses(&listing.instructions),
            branch_over_jmp_patterns: count_branch_over_jmp(&listing.instructions).0,
            branch_over_jmp_branchable: count_branch_over_jmp(&listing.instructions).1,
            lda_cmp_zero_branch_patterns: count_lda_cmp_zero_branch(&listing.instructions),
            jsr_rts_patterns: count_adjacent_mnemonics(&listing.instructions, "JSR", "RTS"),
            load_add_one_store_patterns: count_load_addsub_one_store(&listing.instructions, true),
            load_sub_one_store_patterns: count_load_addsub_one_store(&listing.instructions, false),
            data_like_procs: data_like_procs(listing),
            routine_metrics: routine_metrics(listing),
            spill_label_metrics: spill_label_metrics(listing),
        }
    }

    fn report(&self) -> String {
        let mut lines = Vec::new();
        lines.push(format!("instruction count: {}", self.instruction_count));
        lines.push(format!("code bytes: {}", self.code_bytes));
        lines.push(format!("data bytes: {}", self.data_bytes));
        lines.push("mnemonic counts:".to_string());
        for (mnemonic, count) in &self.mnemonic_counts {
            lines.push(format!("  {mnemonic}: {count}"));
        }
        lines.push(format!(
            "LDA+STA instruction percentage: {:.1}%",
            self.lda_sta_instruction_percent
        ));
        lines.push(format!(
            "LDA+STA byte percentage: {:.1}%",
            self.lda_sta_byte_percent
        ));
        lines.push(format!(
            "spill data label count: {}",
            self.spill_data_label_count
        ));
        lines.push(format!(
            "adjacent STA m; LDA m pairs: {}",
            self.adjacent_sta_lda_pairs
        ));
        lines.push(format!(
            "absolute $00xx direct accesses: {}",
            self.zero_page_absolute_accesses
        ));
        lines.push(format!(
            "branch-over-JMP patterns: {}",
            self.branch_over_jmp_patterns
        ));
        lines.push(format!(
            "branch-over-JMP branchable targets: {}",
            self.branch_over_jmp_branchable
        ));
        lines.push(format!(
            "LDA x; CMP #$00; BEQ/BNE patterns: {}",
            self.lda_cmp_zero_branch_patterns
        ));
        lines.push(format!("JSR f; RTS patterns: {}", self.jsr_rts_patterns));
        lines.push(format!(
            "load/add-one/store patterns: {}",
            self.load_add_one_store_patterns
        ));
        lines.push(format!(
            "load/sub-one/store patterns: {}",
            self.load_sub_one_store_patterns
        ));
        lines.push(format!(
            "PROC blocks that look like data tables: {}",
            self.data_like_procs.len()
        ));
        for proc in &self.data_like_procs {
            lines.push(format!(
                "  {} ${:04X}..${:04X}: {} instructions ({})",
                proc.name, proc.start, proc.end, proc.instruction_count, proc.reason
            ));
        }
        let ranked = spill_pressure_ranking(&self.routine_metrics);
        lines.push(format!(
            "per-routine spill pressure ranking: {} routines",
            self.routine_metrics.len()
        ));
        for routine in ranked.into_iter().take(10) {
            lines.push(format!(
                "  {} ${:04X}..${:04X}: score={} instr={} bytes={} lda+sta={} spill_accesses={} unique_spills={} sta/lda_pairs={}",
                routine.name,
                routine.start,
                routine.end,
                routine.spill_pressure_score(),
                routine.instruction_count,
                routine.code_bytes,
                routine.lda_sta_count,
                routine.spill_accesses,
                routine.unique_spills_accessed,
                routine.adjacent_sta_lda_pairs
            ));
        }
        let ranked_spills = spill_label_ranking(&self.spill_label_metrics);
        lines.push(format!(
            "per-spill label use ranking: {} labels",
            self.spill_label_metrics.len()
        ));
        for spill in ranked_spills.into_iter().take(10) {
            let first = spill
                .first_access_address
                .map(|address| format!("{} ${address:04X}", spill.first_access_proc))
                .unwrap_or_else(|| "unused".to_string());
            lines.push(format!(
                "  {} ${:04X}: accesses={} reads={} writes={} first={}",
                spill.name,
                spill.address,
                spill.reads + spill.writes,
                spill.reads,
                spill.writes,
                first
            ));
        }
        lines.push(String::new());
        lines.join("\n")
    }
}

impl RoutineMetrics {
    fn spill_pressure_score(&self) -> usize {
        self.spill_accesses
            .saturating_mul(4)
            .saturating_add(self.unique_spills_accessed.saturating_mul(8))
            .saturating_add(self.adjacent_sta_lda_pairs.saturating_mul(2))
            .saturating_add(self.lda_sta_count)
    }
}

#[derive(Debug)]
enum ParsedRow {
    Instruction(ListingInstruction),
    Data(ListingDataRow),
}

fn parse_listing_row(line: &str) -> Option<ParsedRow> {
    if line.len() < 17 {
        return None;
    }
    let address = u16::from_str_radix(line.get(0..4)?, 16).ok()?;
    if line.get(4..6)? != "  " {
        return None;
    }
    let raw = line.get(6..14)?.trim();
    let text = line.get(16..)?.trim();
    let bytes = parse_raw_bytes(raw)?;
    if text.starts_with(".BYTE") {
        return Some(ParsedRow::Data(ListingDataRow { address, bytes }));
    }
    let mut parts = text.splitn(2, char::is_whitespace);
    let mnemonic = parts.next()?.trim().to_ascii_uppercase();
    let operand = parts.next().unwrap_or("").trim().to_string();
    Some(ParsedRow::Instruction(ListingInstruction {
        address,
        bytes,
        mnemonic,
        operand,
        proc_index: None,
    }))
}

fn parse_raw_bytes(raw: &str) -> Option<Vec<u8>> {
    if raw.is_empty() {
        return None;
    }
    raw.split_whitespace()
        .map(|byte| u8::from_str_radix(byte, 16).ok())
        .collect()
}

fn parse_proc_start(line: &str) -> Option<ListingProc> {
    let rest = line.strip_prefix("; ===== PROC ")?;
    let rest = rest.strip_suffix(" =====")?;
    let (name, range) = rest.rsplit_once(" $")?;
    let (start, end) = range.split_once("..$")?;
    Some(ListingProc {
        name: name.to_string(),
        start: u16::from_str_radix(start, 16).ok()?,
        end: u16::from_str_radix(end, 16).ok()?,
        instruction_indices: Vec::new(),
    })
}

fn parse_data_label(line: &str) -> Option<ListingDataLabel> {
    let rest = line.strip_prefix("; ===== DATA ")?;
    let rest = rest.strip_suffix(" =====")?;
    let (name, address) = rest.rsplit_once(" $")?;
    Some(ListingDataLabel {
        name: name.to_string(),
        address: u16::from_str_radix(address, 16).ok()?,
    })
}

fn is_spill_label(name: &str) -> bool {
    name.starts_with("spill")
        || name.contains(":spill")
        || name.starts_with("__mir6502_spill")
        || name.contains("_spill_")
}

fn count_adjacent_sta_lda_pairs(instructions: &[ListingInstruction]) -> usize {
    instructions
        .windows(2)
        .filter(|pair| {
            pair[0].mnemonic == "STA"
                && pair[1].mnemonic == "LDA"
                && direct_memory_operand(&pair[0].operand).is_some()
                && direct_memory_operand(&pair[0].operand)
                    == direct_memory_operand(&pair[1].operand)
        })
        .count()
}

fn count_zero_page_absolute_accesses(instructions: &[ListingInstruction]) -> usize {
    instructions
        .iter()
        .filter(|instruction| {
            matches!(
                instruction.mnemonic.as_str(),
                "LDA"
                    | "LDX"
                    | "LDY"
                    | "STA"
                    | "STX"
                    | "STY"
                    | "ADC"
                    | "SBC"
                    | "AND"
                    | "ORA"
                    | "EOR"
                    | "CMP"
                    | "INC"
                    | "DEC"
            ) && direct_memory_operand(&instruction.operand)
                .is_some_and(|address| address <= 0x00FF)
                && instruction.operand.trim().starts_with("$00")
        })
        .count()
}

fn count_branch_over_jmp(instructions: &[ListingInstruction]) -> (usize, usize) {
    let mut total = 0;
    let mut branchable = 0;
    for pair in instructions.windows(2) {
        if !is_branch(&pair[0].mnemonic) || pair[1].mnemonic != "JMP" {
            continue;
        }
        let Some(branch_target) = direct_memory_operand(&pair[0].operand) else {
            continue;
        };
        let after_jmp = pair[1].address.saturating_add(pair[1].bytes.len() as u16);
        if branch_target != after_jmp {
            continue;
        }
        total += 1;
        if direct_memory_operand(&pair[1].operand)
            .is_some_and(|target| branch_offset_fits(pair[0].address, target))
        {
            branchable += 1;
        }
    }
    (total, branchable)
}

fn count_lda_cmp_zero_branch(instructions: &[ListingInstruction]) -> usize {
    instructions
        .windows(3)
        .filter(|triple| {
            triple[0].mnemonic == "LDA"
                && triple[1].mnemonic == "CMP"
                && is_zero_immediate(&triple[1].operand)
                && matches!(triple[2].mnemonic.as_str(), "BEQ" | "BNE")
        })
        .count()
}

fn count_adjacent_mnemonics(
    instructions: &[ListingInstruction],
    first: &str,
    second: &str,
) -> usize {
    instructions
        .windows(2)
        .filter(|pair| pair[0].mnemonic == first && pair[1].mnemonic == second)
        .count()
}

fn count_load_addsub_one_store(instructions: &[ListingInstruction], add: bool) -> usize {
    instructions
        .windows(4)
        .filter(|window| {
            let op = if add { "ADC" } else { "SBC" };
            window[0].mnemonic == "LDA"
                && window[1].mnemonic == if add { "CLC" } else { "SEC" }
                && window[2].mnemonic == op
                && window[3].mnemonic == "STA"
                && is_one_immediate(&window[2].operand)
                && direct_memory_operand(&window[0].operand)
                    == direct_memory_operand(&window[3].operand)
                && direct_memory_operand(&window[0].operand).is_some()
        })
        .count()
}

fn data_like_procs(listing: &Listing) -> Vec<DataLikeProc> {
    listing
        .procs
        .iter()
        .filter_map(|proc| {
            let instructions = proc
                .instruction_indices
                .iter()
                .map(|index| &listing.instructions[*index])
                .collect::<Vec<_>>();
            if instructions.is_empty() {
                return None;
            }
            let has_control_transfer = instructions.iter().any(|instruction| {
                is_branch(&instruction.mnemonic)
                    || matches!(instruction.mnemonic.as_str(), "JMP" | "JSR" | "RTS" | "RTI")
            });
            let brk_count = instructions
                .iter()
                .filter(|instruction| instruction.mnemonic == "BRK")
                .count();
            let unusual_count = instructions
                .iter()
                .filter(|instruction| !is_standard_6502_mnemonic(&instruction.mnemonic))
                .count();
            let trailing_without_return = instructions.last().is_some_and(|instruction| {
                !matches!(instruction.mnemonic.as_str(), "JMP" | "RTS" | "RTI")
            });

            let reason = if brk_count * 2 >= instructions.len() {
                Some(format!("{brk_count} BRK-like bytes"))
            } else if unusual_count >= 2 {
                Some(format!("{unusual_count} non-6502 mnemonics"))
            } else if !has_control_transfer && trailing_without_return && instructions.len() >= 4 {
                Some("no control transfer and no terminating return/jump".to_string())
            } else {
                None
            }?;

            Some(DataLikeProc {
                name: proc.name.clone(),
                start: proc.start,
                end: proc.end,
                instruction_count: instructions.len(),
                reason,
            })
        })
        .collect()
}

fn routine_metrics(listing: &Listing) -> Vec<RoutineMetrics> {
    let spill_addresses = listing
        .data_labels
        .iter()
        .filter(|label| is_spill_label(&label.name))
        .map(|label| (label.address, label.name.clone()))
        .collect::<BTreeMap<_, _>>();

    listing
        .procs
        .iter()
        .map(|proc| {
            let instructions = proc
                .instruction_indices
                .iter()
                .map(|index| &listing.instructions[*index])
                .collect::<Vec<_>>();
            let instruction_count = instructions.len();
            let code_bytes = instructions
                .iter()
                .map(|instruction| instruction.bytes.len())
                .sum::<usize>();
            let lda_count = instructions
                .iter()
                .filter(|instruction| instruction.mnemonic == "LDA")
                .count();
            let sta_count = instructions
                .iter()
                .filter(|instruction| instruction.mnemonic == "STA")
                .count();
            let mut accessed_spills = BTreeSet::new();
            let mut spill_accesses = 0usize;
            for instruction in &instructions {
                if let Some(address) = direct_memory_operand(&instruction.operand)
                    && let Some(name) = spill_addresses.get(&address)
                {
                    spill_accesses += 1;
                    accessed_spills.insert(name.clone());
                }
            }
            RoutineMetrics {
                name: proc.name.clone(),
                start: proc.start,
                end: proc.end,
                instruction_count,
                code_bytes,
                lda_count,
                sta_count,
                lda_sta_count: lda_count + sta_count,
                adjacent_sta_lda_pairs: count_adjacent_sta_lda_pairs_for_indices(
                    &listing.instructions,
                    &proc.instruction_indices,
                ),
                spill_accesses,
                unique_spills_accessed: accessed_spills.len(),
            }
        })
        .collect()
}

fn spill_label_metrics(listing: &Listing) -> Vec<SpillLabelMetrics> {
    listing
        .data_labels
        .iter()
        .filter(|label| is_spill_label(&label.name))
        .map(|label| {
            let mut reads = 0usize;
            let mut writes = 0usize;
            let mut first_access: Option<&ListingInstruction> = None;
            for instruction in &listing.instructions {
                if direct_memory_operand(&instruction.operand) != Some(label.address) {
                    continue;
                }
                first_access.get_or_insert(instruction);
                match spill_access_kind(&instruction.mnemonic) {
                    SpillAccessKind::Read => reads += 1,
                    SpillAccessKind::Write => writes += 1,
                    SpillAccessKind::ReadWrite => {
                        reads += 1;
                        writes += 1;
                    }
                }
            }
            let first_access_proc = first_access
                .and_then(|instruction| instruction.proc_index)
                .and_then(|index| listing.procs.get(index))
                .map(|proc| proc.name.clone())
                .unwrap_or_else(|| "<program>".to_string());
            SpillLabelMetrics {
                name: label.name.clone(),
                address: label.address,
                reads,
                writes,
                first_access_proc,
                first_access_address: first_access.map(|instruction| instruction.address),
            }
        })
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SpillAccessKind {
    Read,
    Write,
    ReadWrite,
}

fn spill_access_kind(mnemonic: &str) -> SpillAccessKind {
    match mnemonic {
        "STA" | "STX" | "STY" => SpillAccessKind::Write,
        "INC" | "DEC" | "ASL" | "LSR" | "ROL" | "ROR" => SpillAccessKind::ReadWrite,
        _ => SpillAccessKind::Read,
    }
}

fn spill_pressure_ranking(metrics: &[RoutineMetrics]) -> Vec<&RoutineMetrics> {
    let mut ranked = metrics.iter().collect::<Vec<_>>();
    ranked.sort_by(|left, right| {
        right
            .spill_pressure_score()
            .cmp(&left.spill_pressure_score())
            .then_with(|| right.code_bytes.cmp(&left.code_bytes))
            .then_with(|| left.name.cmp(&right.name))
    });
    ranked
}

fn spill_label_ranking(metrics: &[SpillLabelMetrics]) -> Vec<&SpillLabelMetrics> {
    let mut ranked = metrics.iter().collect::<Vec<_>>();
    ranked.sort_by(|left, right| {
        (right.reads + right.writes)
            .cmp(&(left.reads + left.writes))
            .then_with(|| right.writes.cmp(&left.writes))
            .then_with(|| left.name.cmp(&right.name))
    });
    ranked
}

fn count_adjacent_sta_lda_pairs_for_indices(
    instructions: &[ListingInstruction],
    indices: &[usize],
) -> usize {
    indices
        .windows(2)
        .filter(|pair| {
            let first = &instructions[pair[0]];
            let second = &instructions[pair[1]];
            first.mnemonic == "STA"
                && second.mnemonic == "LDA"
                && direct_memory_operand(&first.operand).is_some()
                && direct_memory_operand(&first.operand) == direct_memory_operand(&second.operand)
        })
        .count()
}

fn direct_memory_operand(operand: &str) -> Option<u16> {
    let operand = operand.split(';').next().unwrap_or("").trim();
    if operand.contains(',')
        || operand.contains('(')
        || operand.contains(')')
        || operand.starts_with('#')
    {
        return None;
    }
    let hex = operand.strip_prefix('$')?;
    u16::from_str_radix(hex, 16).ok()
}

fn is_zero_immediate(operand: &str) -> bool {
    matches!(operand.trim(), "#$00" | "#0" | "#$0")
}

fn is_one_immediate(operand: &str) -> bool {
    matches!(operand.trim(), "#$01" | "#1" | "#$1")
}

fn is_branch(mnemonic: &str) -> bool {
    matches!(
        mnemonic,
        "BCC" | "BCS" | "BEQ" | "BMI" | "BNE" | "BPL" | "BVC" | "BVS"
    )
}

fn is_standard_6502_mnemonic(mnemonic: &str) -> bool {
    matches!(
        mnemonic,
        "ADC"
            | "AND"
            | "ASL"
            | "BCC"
            | "BCS"
            | "BEQ"
            | "BIT"
            | "BMI"
            | "BNE"
            | "BPL"
            | "BRK"
            | "BVC"
            | "BVS"
            | "CLC"
            | "CLD"
            | "CLI"
            | "CLV"
            | "CMP"
            | "CPX"
            | "CPY"
            | "DEC"
            | "DEX"
            | "DEY"
            | "EOR"
            | "INC"
            | "INX"
            | "INY"
            | "JMP"
            | "JSR"
            | "LDA"
            | "LDX"
            | "LDY"
            | "LSR"
            | "NOP"
            | "ORA"
            | "PHA"
            | "PHP"
            | "PLA"
            | "PLP"
            | "ROL"
            | "ROR"
            | "RTI"
            | "RTS"
            | "SBC"
            | "SEC"
            | "SED"
            | "SEI"
            | "STA"
            | "STX"
            | "STY"
            | "TAX"
            | "TAY"
            | "TSX"
            | "TXA"
            | "TXS"
            | "TYA"
    )
}

fn branch_offset_fits(branch_address: u16, target: u16) -> bool {
    let base = i32::from(branch_address) + 2;
    let offset = i32::from(target) - base;
    (-128..=127).contains(&offset)
}

fn percent(part: usize, whole: usize) -> f64 {
    if whole == 0 {
        0.0
    } else {
        (part as f64) * 100.0 / (whole as f64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_instruction_data_and_proc_boundaries() {
        let listing = Listing::parse(
            "
; ===== DATA __mir6502_spill_0 $3000 =====
3000  00        .BYTE $00
; ===== PROC Main $3001..$3004 =====
3001  A9 01     LDA #$01
3003  60        RTS
; ===== END PROC Main =====
",
        );

        assert_eq!(listing.data_rows.len(), 1);
        assert_eq!(listing.data_labels.len(), 1);
        assert_eq!(listing.instructions.len(), 2);
        assert_eq!(listing.procs.len(), 1);
        assert_eq!(listing.spill_labels.len(), 1);
    }

    #[test]
    fn counts_core_instruction_shape_metrics() {
        let listing = Listing::parse(
            "
; ===== PROC Main $3000..$3025 =====
3000  AD E0 00  LDA $00E0
3003  8D E1 00  STA $00E1
3006  AD E1 00  LDA $00E1
3009  C9 00     CMP #$00
300B  F0 03     BEQ $3010
300D  4C 70 30  JMP $3070
3010  20 00 40  JSR $4000
3013  60        RTS
3014  AD 20 30  LDA $3020
3017  18        CLC
3018  69 01     ADC #$01
301A  8D 20 30  STA $3020
301D  AD 21 30  LDA $3021
3020  38        SEC
3021  E9 01     SBC #$01
3023  8D 21 30  STA $3021
; ===== END PROC Main =====
",
        );
        let metrics = ListingMetrics::from_listing(&listing);

        assert_eq!(metrics.zero_page_absolute_accesses, 3);
        assert_eq!(metrics.adjacent_sta_lda_pairs, 1);
        assert_eq!(metrics.lda_cmp_zero_branch_patterns, 1);
        assert_eq!(metrics.branch_over_jmp_patterns, 1);
        assert_eq!(metrics.branch_over_jmp_branchable, 1);
        assert_eq!(metrics.jsr_rts_patterns, 1);
        assert_eq!(metrics.load_add_one_store_patterns, 1);
        assert_eq!(metrics.load_sub_one_store_patterns, 1);
    }

    #[test]
    fn identifies_data_like_proc_blocks() {
        let listing = Listing::parse(
            "
; ===== PROC MaybeTable $3400..$3406 =====
3400  00        BRK
3401  00        BRK
3402  03        SLO ($12,X)
3404  04        NOP $10
; ===== END PROC MaybeTable =====
",
        );
        let metrics = ListingMetrics::from_listing(&listing);

        assert_eq!(metrics.data_like_procs.len(), 1);
        assert_eq!(metrics.data_like_procs[0].name, "MaybeTable");
    }

    #[test]
    fn report_is_deterministic() {
        let listing = Listing::parse(
            "
4000  A9 00     LDA #$00
4002  8D 00 20  STA $2000
",
        );
        let report = ListingMetrics::from_listing(&listing).report();

        assert!(report.contains("instruction count: 2"));
        assert!(report.contains("  LDA: 1\n"));
        assert!(report.contains("  STA: 1\n"));
    }

    #[test]
    fn ranks_routines_by_spill_pressure() {
        let listing = Listing::parse(
            "
; ===== DATA __mir6502_spill_0 $3000 =====
3000  00        .BYTE $00
; ===== DATA __mir6502_spill_1 $3001 =====
3001  00        .BYTE $00
; ===== PROC Small $3010..$3015 =====
3010  AD 00 30  LDA $3000
3013  60        RTS
; ===== END PROC Small =====
; ===== PROC Hot $3020..$302B =====
3020  AD 00 30  LDA $3000
3023  8D 01 30  STA $3001
3026  AD 01 30  LDA $3001
3029  60        RTS
; ===== END PROC Hot =====
",
        );
        let metrics = ListingMetrics::from_listing(&listing);
        let ranked = spill_pressure_ranking(&metrics.routine_metrics);

        assert_eq!(metrics.routine_metrics.len(), 2);
        assert_eq!(ranked[0].name, "Hot");
        assert_eq!(ranked[0].spill_accesses, 3);
        assert_eq!(ranked[0].unique_spills_accessed, 2);
        assert_eq!(metrics.spill_label_metrics.len(), 2);
        assert_eq!(metrics.spill_label_metrics[0].name, "__mir6502_spill_0");
        assert_eq!(metrics.spill_label_metrics[0].reads, 2);
        assert_eq!(metrics.spill_label_metrics[0].writes, 0);
        assert_eq!(metrics.spill_label_metrics[1].name, "__mir6502_spill_1");
        assert_eq!(metrics.spill_label_metrics[1].reads, 1);
        assert_eq!(metrics.spill_label_metrics[1].writes, 1);

        let report = metrics.report();
        assert!(report.contains("per-routine spill pressure ranking: 2 routines"));
        assert!(report.contains("Hot $3020..$302B"));
        assert!(report.contains("spill_accesses=3"));
        assert!(report.contains("per-spill label use ranking: 2 labels"));
        assert!(
            report.contains("__mir6502_spill_1 $3001: accesses=2 reads=1 writes=1 first=Hot $3023")
        );
    }
}
