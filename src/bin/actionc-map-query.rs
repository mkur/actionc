use std::env;
use std::fs;
use std::path::PathBuf;
use std::process;

use actionc::codegen::{
    CODE_ORIGIN, CodegenProfile, CodegenSourceRange, CodegenStorageSymbol, RoutineRange,
    SkippedRange, generate_profile_with_origin,
};
use actionc::diagnostic::Diagnostic;
use actionc::includes::load_program_with_includes;
use actionc::map_query::{
    AddressOwnership, MapQuery, RangeItem, RangeOverlap, SourceMatch, StorageOwner, SymbolMatch,
};
use actionc::semantic::analyze;
use actionc::source::decode_source;

#[derive(Debug)]
struct Options {
    source: PathBuf,
    source_text: String,
    queries: PathBuf,
    profile: CodegenProfile,
    origin: u16,
    output: OutputFormat,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OutputFormat {
    Text,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Query {
    Owner(u16),
    Source(u16),
    Symbol(String),
    Routine(String),
    Range(u16, u16),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct QueryLine {
    line: usize,
    text: String,
    query: Query,
}

#[derive(Debug)]
enum QueryResult<'a> {
    Owner(AddressOwnership<'a>),
    Source {
        address: u16,
        source: Option<SourceMatch<'a>>,
    },
    Symbol {
        name: String,
        matches: Vec<SymbolMatch<'a>>,
    },
    Routine {
        name: String,
        routine: Option<&'a RoutineRange>,
    },
    Range {
        start: u16,
        end: u16,
        overlaps: Vec<RangeOverlap<'a>>,
    },
}

fn main() {
    let options = parse_options();
    let program = load_program_with_includes(&options.source)
        .and_then(|program| analyze(&program).map(|_| program))
        .unwrap_or_else(|diagnostics| {
            print_diagnostics(diagnostics);
            process::exit(1);
        });
    let output = generate_profile_with_origin(&program, options.origin, options.profile)
        .unwrap_or_else(|diagnostics| {
            print_diagnostics(diagnostics);
            process::exit(1);
        });
    let queries = read_queries(&options.queries);
    let query = MapQuery::with_source(&output.map, &options.source_text);

    println!("source: {}", options.source.display());
    println!(
        "profile: {}  origin {}",
        profile_label(options.profile),
        format_addr(output.origin)
    );
    println!("queries: {}", options.queries.display());

    for query_line in queries {
        let result = run_query(&query, &query_line.query);
        render_result(options.output, &query_line, &result);
    }
}

fn parse_options() -> Options {
    let mut args = env::args().skip(1);
    let mut queries = None;
    let mut profile = CodegenProfile::default();
    let mut origin = CODE_ORIGIN;
    let mut source = None;
    let output = OutputFormat::Text;

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" => {
                print_help();
                process::exit(0);
            }
            "--queries" => {
                let Some(path) = args.next() else {
                    eprintln!("--queries requires a query file path");
                    print_help();
                    process::exit(2);
                };
                queries = Some(PathBuf::from(path));
            }
            _ if arg.starts_with("--queries=") => {
                queries = Some(PathBuf::from(&arg["--queries=".len()..]));
            }
            "--profile" => {
                let Some(value) = args.next() else {
                    eprintln!("--profile requires legacy or modern");
                    print_help();
                    process::exit(2);
                };
                profile = parse_profile(&value);
            }
            _ if arg.starts_with("--profile=") => {
                profile = parse_profile(&arg["--profile=".len()..]);
            }
            "--origin" => {
                let Some(value) = args.next() else {
                    eprintln!("--origin requires an address");
                    print_help();
                    process::exit(2);
                };
                origin = parse_address(&value).unwrap_or_else(|err| {
                    eprintln!("{err}");
                    process::exit(2);
                });
            }
            _ if arg.starts_with("--origin=") => {
                origin = parse_address(&arg["--origin=".len()..]).unwrap_or_else(|err| {
                    eprintln!("{err}");
                    process::exit(2);
                });
            }
            _ if arg.starts_with('-') => {
                eprintln!("unknown option: {arg}");
                print_help();
                process::exit(2);
            }
            _ if source.is_none() => source = Some(PathBuf::from(arg)),
            _ => {
                eprintln!("unexpected argument: {arg}");
                print_help();
                process::exit(2);
            }
        }
    }

    let Some(source) = source else {
        print_help();
        process::exit(2);
    };
    let Some(queries) = queries else {
        eprintln!("missing --queries <file>");
        print_help();
        process::exit(2);
    };
    let source_bytes = fs::read(&source).unwrap_or_else(|err| {
        eprintln!("read {}: {err}", source.display());
        process::exit(1);
    });
    let source_text = decode_source(&source_bytes);

    Options {
        source,
        source_text,
        queries,
        profile,
        origin,
        output,
    }
}

fn read_queries(path: &PathBuf) -> Vec<QueryLine> {
    let text = fs::read_to_string(path).unwrap_or_else(|err| {
        eprintln!("read {}: {err}", path.display());
        process::exit(1);
    });
    let mut queries = Vec::new();
    for (index, line) in text.lines().enumerate() {
        let line_number = index + 1;
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let query = parse_query(trimmed).unwrap_or_else(|err| {
            eprintln!("{}:{line_number}: {err}", path.display());
            process::exit(2);
        });
        queries.push(QueryLine {
            line: line_number,
            text: trimmed.to_string(),
            query,
        });
    }
    queries
}

fn parse_query(line: &str) -> Result<Query, String> {
    let parts = line.split_whitespace().collect::<Vec<_>>();
    let Some(command) = parts.first() else {
        return Err("empty query".to_string());
    };
    match command.to_ascii_lowercase().as_str() {
        "owner" => {
            require_arity(&parts, 2)?;
            Ok(Query::Owner(parse_address(parts[1])?))
        }
        "source" => {
            require_arity(&parts, 2)?;
            Ok(Query::Source(parse_address(parts[1])?))
        }
        "symbol" => {
            require_arity(&parts, 2)?;
            Ok(Query::Symbol(parts[1].to_string()))
        }
        "routine" => {
            require_arity(&parts, 2)?;
            Ok(Query::Routine(parts[1].to_string()))
        }
        "range" => {
            require_arity(&parts, 3)?;
            Ok(Query::Range(
                parse_address(parts[1])?,
                parse_address(parts[2])?,
            ))
        }
        _ => Err(format!("unknown query command: {command}")),
    }
}

fn require_arity(parts: &[&str], expected: usize) -> Result<(), String> {
    if parts.len() == expected {
        return Ok(());
    }
    Err(format!(
        "{} expects {} argument(s), got {}",
        parts[0],
        expected.saturating_sub(1),
        parts.len().saturating_sub(1)
    ))
}

fn run_query<'a>(query: &MapQuery<'a>, request: &Query) -> QueryResult<'a> {
    match request {
        Query::Owner(address) => QueryResult::Owner(query.owner(*address)),
        Query::Source(address) => QueryResult::Source {
            address: *address,
            source: query.source_at(*address),
        },
        Query::Symbol(name) => QueryResult::Symbol {
            name: name.clone(),
            matches: query.symbol(name),
        },
        Query::Routine(name) => QueryResult::Routine {
            name: name.clone(),
            routine: query.routine(name),
        },
        Query::Range(start, end) => QueryResult::Range {
            start: *start,
            end: *end,
            overlaps: query.range(*start, *end),
        },
    }
}

fn render_result(format: OutputFormat, query_line: &QueryLine, result: &QueryResult<'_>) {
    match format {
        OutputFormat::Text => render_text_result(query_line, result),
    }
}

fn render_text_result(query_line: &QueryLine, result: &QueryResult<'_>) {
    println!();
    println!("== {}:{} ==", query_line.line, query_line.text);
    match result {
        QueryResult::Owner(owner) => {
            println!("address {}", format_addr(owner.address));
            print_optional("storage", owner.storage.as_ref(), format_storage_owner);
            print_optional("routine", owner.routine, format_routine);
            print_optional("skipped", owner.skipped, format_skipped_range);
            print_optional("source", owner.source.as_ref(), format_source_match);
        }
        QueryResult::Source { address, source } => {
            println!("address {}", format_addr(*address));
            print_optional("source", source.as_ref(), format_source_match);
        }
        QueryResult::Symbol { name, matches } => {
            println!("symbol {name}");
            if matches.is_empty() {
                println!("  <no matches>");
            }
            for item in matches {
                match item {
                    SymbolMatch::Routine(routine) => println!("  {}", format_routine(routine)),
                    SymbolMatch::Storage(symbol) => println!("  {}", format_storage_symbol(symbol)),
                }
            }
        }
        QueryResult::Routine { name, routine } => {
            println!("routine {name}");
            print_optional("match", *routine, format_routine);
        }
        QueryResult::Range {
            start,
            end,
            overlaps,
        } => {
            println!("range {}..{}", format_addr(*start), format_addr(*end));
            if overlaps.is_empty() {
                println!("  <no overlaps>");
            }
            for overlap in overlaps {
                println!("  {}", format_range_overlap(overlap));
            }
        }
    }
}

fn print_optional<T>(label: &str, value: Option<T>, format: impl FnOnce(T) -> String) {
    match value {
        Some(value) => println!("  {label}: {}", format(value)),
        None => println!("  {label}: <none>"),
    }
}

fn format_range_overlap(overlap: &RangeOverlap<'_>) -> String {
    let range = format!(
        "{}..{}",
        format_addr(overlap.start),
        format_addr(overlap.end)
    );
    match &overlap.item {
        RangeItem::Routine(routine) => format!("{range} {}", format_routine(routine)),
        RangeItem::Storage(symbol) => format!("{range} {}", format_storage_symbol(symbol)),
        RangeItem::Skipped(skipped) => format!("{range} {}", format_skipped_range(skipped)),
        RangeItem::Source(source) => format!("{range} {}", format_source_range(source)),
    }
}

fn format_storage_owner(owner: &StorageOwner<'_>) -> String {
    format!(
        "{} offset +{}",
        format_storage_symbol(owner.symbol),
        owner.offset
    )
}

fn format_storage_symbol(symbol: &CodegenStorageSymbol) -> String {
    let scope = match &symbol.scope {
        actionc::codegen::CodegenSymbolScope::Global => "global".to_string(),
        actionc::codegen::CodegenSymbolScope::Routine(name) => format!("routine {name}"),
    };
    let end = symbol.address.wrapping_add(symbol.size);
    format!(
        "{} {:?} {} {}..{}",
        scope,
        symbol.kind,
        symbol.name,
        format_addr(symbol.address),
        format_addr(end)
    )
}

fn format_routine(routine: &RoutineRange) -> String {
    format!(
        "routine {} {}..{} len {}",
        routine.name,
        format_addr(routine.start),
        format_addr(routine.end),
        routine.end.saturating_sub(routine.start)
    )
}

fn format_skipped_range(range: &SkippedRange) -> String {
    let end = range.start.wrapping_add(range.len);
    format!(
        "skipped {}..{} len {}",
        format_addr(range.start),
        format_addr(end),
        range.len
    )
}

fn format_source_match(source: &SourceMatch<'_>) -> String {
    let mut formatted = format_source_range(source.range);
    if let Some(location) = &source.location {
        formatted.push_str(&format!(
            " at {}:{} | {}",
            location.line, location.column, location.excerpt
        ));
    }
    formatted
}

fn format_source_range(range: &CodegenSourceRange) -> String {
    let name = range
        .name
        .as_ref()
        .map(|name| format!(" {name}"))
        .unwrap_or_default();
    format!(
        "{:?}{name} source {}..{} code {}..{}",
        range.kind,
        range.source_span.start,
        range.source_span.end,
        format_addr(range.start),
        format_addr(range.end)
    )
}

fn print_diagnostics(diagnostics: Vec<Diagnostic>) {
    for diagnostic in diagnostics {
        eprintln!(
            "{}..{}: {}",
            diagnostic.span.start, diagnostic.span.end, diagnostic.message
        );
    }
}

fn parse_profile(value: &str) -> CodegenProfile {
    match value {
        "legacy" | "compat" => CodegenProfile::Compat,
        "modern" => CodegenProfile::Modern,
        _ => {
            eprintln!("invalid codegen profile: {value}");
            process::exit(2);
        }
    }
}

fn profile_label(profile: CodegenProfile) -> &'static str {
    match profile {
        CodegenProfile::Compat => "legacy",
        CodegenProfile::Modern => "modern",
    }
}

fn parse_address(value: &str) -> Result<u16, String> {
    let parsed = if let Some(hex) = value.strip_prefix('$') {
        u16::from_str_radix(hex, 16)
    } else if let Some(hex) = value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"))
    {
        u16::from_str_radix(hex, 16)
    } else {
        value.parse::<u16>()
    };

    parsed.map_err(|_| format!("invalid address: {value}"))
}

fn format_addr(address: u16) -> String {
    format!("${address:04X}")
}

fn print_help() {
    eprintln!(
        "usage: actionc-map-query [--profile legacy|modern] [--origin <addr>] --queries <queries.txt> <source.act>"
    );
    eprintln!();
    eprintln!("query file commands:");
    eprintln!("  owner <addr>");
    eprintln!("  source <addr>");
    eprintln!("  symbol <name>");
    eprintln!("  routine <name>");
    eprintln!("  range <start> <end>   # end is exclusive");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_owner_query() {
        assert_eq!(parse_query("owner $3026"), Ok(Query::Owner(0x3026)));
    }

    #[test]
    fn parses_range_query_with_mixed_address_formats() {
        assert_eq!(
            parse_query("range $3000 0x3030"),
            Ok(Query::Range(0x3000, 0x3030))
        );
    }

    #[test]
    fn rejects_wrong_arity() {
        assert!(parse_query("source").unwrap_err().contains("expects"));
    }
}
