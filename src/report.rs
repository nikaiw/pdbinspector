use anyhow::{Context, Result};
use msvc_demangler::{demangle, DemangleFlags};
use pdb::{ClassKind, FallibleIterator, SymbolData, TypeData, PDB};
use serde::Serialize;
use std::{collections::HashSet, io::Cursor, path::Path};

#[cfg(feature = "cli")]
use memmap2::MmapOptions;
#[cfg(feature = "cli")]
use std::{fs::File, path::PathBuf};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum OutputFormat {
    #[default]
    Text,
    Json,
    Grep,
}

impl std::str::FromStr for OutputFormat {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "text" => Ok(OutputFormat::Text),
            "json" => Ok(OutputFormat::Json),
            "grep" => Ok(OutputFormat::Grep),
            _ => Err(format!("unknown format: {}", s)),
        }
    }
}

#[derive(Serialize, Default)]
pub struct PdbReport {
    pub file_path: String,
    pub status: String,
    pub error: Option<String>,
    pub summary: ReportSummary,
    pub types: TypeStats,
    pub symbols: SymbolStats,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub type_details: Option<TypeDetails>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub symbol_details: Option<SymbolDetails>,
}

#[derive(Serialize, Default)]
pub struct ReportSummary {
    pub total_types: usize,
    pub total_symbols: usize,
    pub has_functions: bool,
    pub has_structs: bool,
    pub has_classes: bool,
    pub has_enums: bool,
    pub has_unions: bool,
    pub has_global_data: bool,
    pub has_public_functions: bool,
    pub has_public_data: bool,
}

#[derive(Serialize, Default)]
pub struct TypeStats {
    pub classes: CategoryStats,
    pub structs: CategoryStats,
    pub interfaces: CategoryStats,
    pub unions: CategoryStats,
    pub enumerations: CategoryStats,
    pub procedures: CategoryStats,
    pub arrays: CategoryStats,
    pub pointers: CategoryStats,
}

#[derive(Serialize, Default)]
pub struct SymbolStats {
    pub functions: CategoryStats,
    pub global_data: CategoryStats,
    pub public_functions: CategoryStats,
    pub public_data: CategoryStats,
    pub constants: CategoryStats,
    pub exports: CategoryStats,
    pub labels: CategoryStats,
}

#[derive(Serialize, Default, Clone)]
pub struct CategoryStats {
    pub count: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_size: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avg_size: Option<f64>,
}

impl CategoryStats {
    fn increment(&mut self, size: Option<u64>) {
        self.count += 1;
        if let Some(s) = size {
            let total = self.total_size.get_or_insert(0);
            *total += s;
        }
    }

    fn finalize(&mut self) {
        if let Some(total) = self.total_size {
            if self.count > 0 {
                self.avg_size = Some(total as f64 / self.count as f64);
            }
        }
    }
}

#[derive(Serialize, Default)]
pub struct TypeDetails {
    pub classes: Vec<TypeItem>,
    pub structs: Vec<TypeItem>,
    pub interfaces: Vec<TypeItem>,
    pub unions: Vec<TypeItem>,
    pub enumerations: Vec<TypeItem>,
}

#[derive(Serialize, Clone)]
pub struct TypeItem {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
}

#[derive(Serialize, Default)]
pub struct SymbolDetails {
    pub functions: Vec<SymbolItem>,
    pub global_data: Vec<SymbolItem>,
    pub public_functions: Vec<SymbolItem>,
    pub public_data: Vec<SymbolItem>,
    pub constants: Vec<SymbolItem>,
    pub exports: Vec<SymbolItem>,
}

#[derive(Serialize, Clone)]
pub struct SymbolItem {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mangled_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub offset: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub segment: Option<u16>,
}

/// Decode special character from MSVC string literal encoding
fn decode_special_char(c1: char, c2: char) -> Option<char> {
    match (c1, c2) {
        ('A', 'A') => None, // Handled separately as literal char prefix
        ('C', 'A') => Some('\n'),
        ('C', 'B') => Some('+'),
        ('C', 'C') => Some('<'),
        ('C', 'D') => Some('='),
        ('C', 'E') => Some('>'),
        ('C', 'F') => Some('%'),
        ('C', 'G') => Some('&'),
        ('C', 'I') => Some('['),
        ('C', 'J') => Some(']'),
        ('C', 'K') => Some('{'),
        ('C', 'L') => Some('|'),
        ('C', 'M') => Some('}'),
        ('C', 'N') => Some('~'),
        ('C', 'O') => Some('_'),
        ('C', 'P') => Some('`'),
        ('D', 'A') => Some('\r'),
        ('D', 'N') => Some('@'),
        ('D', 'O') => Some('#'),
        ('D', 'P') => Some('$'),
        ('D', 'Q') => Some('^'),
        ('E', 'A') => Some('!'),
        ('E', 'B') => Some('"'),
        ('F', 'L') => Some('*'),
        _ => None,
    }
}

/// Decode digit escape from MSVC string literal encoding
fn decode_digit_escape(d: char) -> Option<char> {
    match d {
        '0' => Some(','),
        '1' => Some('/'),
        '2' => Some('\\'),
        '3' => Some(':'),
        '4' => Some('.'),
        '5' => Some(' '),
        '6' => Some('\n'),
        '7' => Some('\t'),
        '8' => Some('\''),
        '9' => Some('-'),
        _ => None,
    }
}

/// Decode MSVC string literal (??_C@...) to readable string
fn decode_string_literal(encoded: &str) -> Option<String> {
    if !encoded.starts_with("??_C@") {
        return None;
    }

    let rest = &encoded[5..];
    let is_wide = rest.starts_with("_1");

    // Format: _[01]<len>@<hash>@<encoded>@ or _[01]<len><hash>@<encoded>@
    let parts: Vec<&str> = rest.splitn(3, '@').collect();
    if parts.len() < 2 {
        return None;
    }

    // The encoded string is in the last non-empty part before final @
    // Empty parts mean empty string literal
    let encoded_str = if parts.len() >= 3 && !parts[2].is_empty() {
        parts[2].trim_end_matches('@')
    } else if parts.len() >= 2 {
        parts[1].trim_end_matches('@')
    } else {
        ""
    };
    let mut result = String::new();
    let mut chars = encoded_str.chars().peekable();

    while let Some(c) = chars.next() {
        if c == '?' {
            match chars.peek() {
                Some('$') => {
                    chars.next();
                    let c1 = chars.next().unwrap_or(' ');
                    let c2 = chars.next().unwrap_or(' ');

                    if c1 == 'A' && c2 == 'A' {
                        // ?$AA = next char or escape sequence
                        match chars.peek() {
                            Some('?') => {
                                chars.next();
                                match chars.peek() {
                                    Some('$') => {
                                        chars.next();
                                        let e1 = chars.next().unwrap_or(' ');
                                        let e2 = chars.next().unwrap_or(' ');
                                        if let Some(ch) = decode_special_char(e1, e2) {
                                            result.push(ch);
                                        }
                                    }
                                    Some(&d) if d.is_ascii_digit() => {
                                        chars.next();
                                        if let Some(ch) = decode_digit_escape(d) {
                                            result.push(ch);
                                        }
                                    }
                                    _ => {}
                                }
                            }
                            Some(&ch) => {
                                chars.next();
                                result.push(ch);
                            }
                            None => {}
                        }
                    } else if let Some(ch) = decode_special_char(c1, c2) {
                        result.push(ch);
                    }
                }
                Some(&d) if d.is_ascii_digit() => {
                    chars.next();
                    if let Some(ch) = decode_digit_escape(d) {
                        result.push(ch);
                    }
                }
                _ => {}
            }
        } else if c.is_ascii_alphanumeric() || c == '_' {
            result.push(c);
        }
    }

    // Empty result is valid for empty string literals
    let prefix = if is_wide { "L" } else { "" };
    Some(format!("{}\"{}\"", prefix, result))
}

/// Remove non-printable characters from symbol names
fn sanitize_name(name: &str) -> String {
    name.chars().filter(|c| *c >= ' ' && *c != '\x7F').collect()
}

/// Demangle MSVC C++ mangled names
fn demangle_name(name: &str) -> String {
    // Strip leading non-printable chars (linker artifacts like \x7F)
    let name = name.trim_start_matches(|c: char| c < ' ' || c == '\x7F');

    // Try string literal decoding first
    if name.starts_with("??_C@") {
        if let Some(decoded) = decode_string_literal(name) {
            return decoded;
        }
    }

    // MSVC mangled names start with ? or @
    if name.starts_with('?') || name.starts_with('@') {
        let result = demangle(name, DemangleFlags::COMPLETE).unwrap_or_else(|_| name.to_string());
        sanitize_name(&result)
    } else {
        sanitize_name(name)
    }
}

/// Analyze a PDB file from raw bytes.
///
/// This is the main entry point for WASM and other non-CLI use cases.
///
/// # Arguments
/// * `data` - The raw bytes of the PDB file
/// * `detailed` - If true, include detailed lists of types and symbols
///
/// # Returns
/// A `PdbReport` containing the analysis results
pub fn analyze_pdb_from_bytes(data: &[u8], detailed: bool) -> Result<PdbReport> {
    let cursor = Cursor::new(data);
    let mut pdb = PDB::open(cursor).context("invalid PDB")?;
    analyze_pdb_inner(&mut pdb, "<bytes>", detailed)
}

#[cfg(feature = "cli")]
fn analyze_pdb(path: &Path, detailed: bool) -> Result<PdbReport> {
    let file = File::open(path).with_context(|| format!("cannot open {:?}", path))?;

    // SAFETY: File is opened read-only and kept open for lifetime of mmap.
    // The file is not modified during the mmap's lifetime.
    #[allow(unsafe_code)]
    let mmap = unsafe { MmapOptions::new().map(&file)? };
    let cursor = Cursor::new(&mmap[..]);

    let mut pdb = PDB::open(cursor).context("invalid PDB")?;
    let mut report = analyze_pdb_inner(&mut pdb, &path.display().to_string(), detailed)?;
    report.file_path = path.display().to_string();
    Ok(report)
}

fn analyze_pdb_inner<'s, S: pdb::Source<'s> + 's>(
    pdb: &mut PDB<'s, S>,
    file_path: &str,
    detailed: bool,
) -> Result<PdbReport> {
    let mut report = PdbReport {
        file_path: file_path.to_string(),
        status: "ok".into(),
        ..Default::default()
    };

    let mut type_details = if detailed {
        Some(TypeDetails::default())
    } else {
        None
    };

    let mut symbol_details = if detailed {
        Some(SymbolDetails::default())
    } else {
        None
    };

    // Analyze types (TPI stream)
    if let Ok(tpi) = pdb.type_information() {
        let mut iter = tpi.iter();
        while let Some(typ) = iter.next().unwrap_or(None) {
            report.summary.total_types += 1;

            if let Ok(parsed) = typ.parse() {
                match parsed {
                    TypeData::Class(class) => {
                        let name = class.name.to_string().to_string();
                        let size = class.size;

                        match class.kind {
                            ClassKind::Class => {
                                report.types.classes.increment(Some(size));
                                report.summary.has_classes = true;
                                if let Some(ref mut details) = type_details {
                                    details.classes.push(TypeItem {
                                        name,
                                        size: Some(size),
                                    });
                                }
                            }
                            ClassKind::Struct => {
                                report.types.structs.increment(Some(size));
                                report.summary.has_structs = true;
                                if let Some(ref mut details) = type_details {
                                    details.structs.push(TypeItem {
                                        name,
                                        size: Some(size),
                                    });
                                }
                            }
                            ClassKind::Interface => {
                                report.types.interfaces.increment(Some(size));
                                if let Some(ref mut details) = type_details {
                                    details.interfaces.push(TypeItem {
                                        name,
                                        size: Some(size),
                                    });
                                }
                            }
                        }
                    }
                    TypeData::Union(union) => {
                        report.types.unions.increment(Some(union.size));
                        report.summary.has_unions = true;
                        if let Some(ref mut details) = type_details {
                            details.unions.push(TypeItem {
                                name: union.name.to_string().to_string(),
                                size: Some(union.size),
                            });
                        }
                    }
                    TypeData::Enumeration(enumeration) => {
                        report.types.enumerations.increment(None);
                        report.summary.has_enums = true;
                        if let Some(ref mut details) = type_details {
                            details.enumerations.push(TypeItem {
                                name: enumeration.name.to_string().to_string(),
                                size: None,
                            });
                        }
                    }
                    TypeData::Procedure(_) => {
                        report.types.procedures.increment(None);
                    }
                    TypeData::Array(arr) => {
                        report
                            .types
                            .arrays
                            .increment(Some(arr.dimensions[0] as u64));
                    }
                    TypeData::Pointer(_) => {
                        report.types.pointers.increment(None);
                    }
                    _ => {}
                }
            }
        }
    }

    // Track seen function names to avoid duplicates across streams
    let mut seen_functions: HashSet<String> = HashSet::new();

    // Analyze global symbols stream
    if let Ok(global_symbols) = pdb.global_symbols() {
        let mut iter = global_symbols.iter();
        while let Some(sym) = iter.next().unwrap_or(None) {
            report.summary.total_symbols += 1;

            if let Ok(parsed) = sym.parse() {
                match parsed {
                    SymbolData::Procedure(proc) => {
                        let name = proc.name.to_string().to_string();
                        if seen_functions.insert(name.clone()) {
                            report.symbols.functions.increment(Some(proc.len.into()));
                            report.summary.has_functions = true;
                            if let Some(ref mut details) = symbol_details {
                                details.functions.push(SymbolItem {
                                    name,
                                    mangled_name: None,
                                    offset: Some(proc.offset.offset.into()),
                                    segment: Some(proc.offset.section),
                                });
                            }
                        }
                    }
                    SymbolData::Data(data) => {
                        report.symbols.global_data.increment(None);
                        report.summary.has_global_data = true;
                        if let Some(ref mut details) = symbol_details {
                            details.global_data.push(SymbolItem {
                                name: data.name.to_string().to_string(),
                                mangled_name: None,
                                offset: Some(data.offset.offset.into()),
                                segment: Some(data.offset.section),
                            });
                        }
                    }
                    SymbolData::Constant(constant) => {
                        report.symbols.constants.increment(None);
                        if let Some(ref mut details) = symbol_details {
                            details.constants.push(SymbolItem {
                                name: constant.name.to_string().to_string(),
                                mangled_name: None,
                                offset: None,
                                segment: None,
                            });
                        }
                    }
                    SymbolData::Export(export) => {
                        report.symbols.exports.increment(None);
                        if let Some(ref mut details) = symbol_details {
                            details.exports.push(SymbolItem {
                                name: export.name.to_string().to_string(),
                                mangled_name: None,
                                offset: None,
                                segment: None,
                            });
                        }
                    }
                    SymbolData::Label(label) => {
                        report.symbols.labels.increment(None);
                        let _ = label; // suppress unused warning
                    }
                    _ => {}
                }
            }
        }
    }

    // Analyze module streams for procedures (functions) and handle public symbols
    if let Ok(dbi) = pdb.debug_information() {
        if let Ok(mut modules) = dbi.modules() {
            while let Some(module) = modules.next().unwrap_or(None) {
                if let Ok(Some(module_info)) = pdb.module_info(&module) {
                    if let Ok(mut symbols) = module_info.symbols() {
                        while let Some(sym) = symbols.next().unwrap_or(None) {
                            if let Ok(parsed) = sym.parse() {
                                match parsed {
                                    SymbolData::Procedure(proc) => {
                                        let name = proc.name.to_string().to_string();
                                        if seen_functions.insert(name.clone()) {
                                            report
                                                .symbols
                                                .functions
                                                .increment(Some(proc.len.into()));
                                            report.summary.has_functions = true;
                                            if let Some(ref mut details) = symbol_details {
                                                details.functions.push(SymbolItem {
                                                    name,
                                                    mangled_name: None,
                                                    offset: Some(proc.offset.offset.into()),
                                                    segment: Some(proc.offset.section),
                                                });
                                            }
                                        }
                                    }
                                    SymbolData::Data(data) => {
                                        report.symbols.global_data.increment(None);
                                        report.summary.has_global_data = true;
                                        if let Some(ref mut details) = symbol_details {
                                            details.global_data.push(SymbolItem {
                                                name: data.name.to_string().to_string(),
                                                mangled_name: None,
                                                offset: Some(data.offset.offset.into()),
                                                segment: Some(data.offset.section),
                                            });
                                        }
                                    }
                                    SymbolData::Public(public) => {
                                        let raw_name = public.name.to_string().to_string();
                                        let mangled = sanitize_name(&raw_name);
                                        let demangled = demangle_name(&raw_name);

                                        if public.function {
                                            report.symbols.public_functions.increment(None);
                                            report.summary.has_public_functions = true;
                                        } else {
                                            report.symbols.public_data.increment(None);
                                            report.summary.has_public_data = true;
                                        }

                                        if let Some(ref mut details) = symbol_details {
                                            let item = SymbolItem {
                                                name: demangled,
                                                mangled_name: Some(mangled),
                                                offset: Some(public.offset.offset.into()),
                                                segment: Some(public.offset.section),
                                            };
                                            if public.function {
                                                details.public_functions.push(item);
                                            } else {
                                                details.public_data.push(item);
                                            }
                                        }
                                    }
                                    _ => {}
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    // Also check global symbols for public symbols if module info didn't have them
    if report.symbols.public_functions.count == 0 && report.symbols.public_data.count == 0 {
        if let Ok(symbol_table) = pdb.global_symbols() {
            let mut iter = symbol_table.iter();
            while let Some(sym) = iter.next().unwrap_or(None) {
                if let Ok(SymbolData::Public(public)) = sym.parse() {
                    let raw_name = public.name.to_string().to_string();
                    let mangled = sanitize_name(&raw_name);
                    let demangled = demangle_name(&raw_name);

                    if public.function {
                        report.symbols.public_functions.increment(None);
                        report.summary.has_public_functions = true;
                    } else {
                        report.symbols.public_data.increment(None);
                        report.summary.has_public_data = true;
                    }
                    report.summary.total_symbols += 1;

                    if let Some(ref mut details) = symbol_details {
                        let item = SymbolItem {
                            name: demangled,
                            mangled_name: Some(mangled),
                            offset: Some(public.offset.offset.into()),
                            segment: Some(public.offset.section),
                        };
                        if public.function {
                            details.public_functions.push(item);
                        } else {
                            details.public_data.push(item);
                        }
                    }
                }
            }
        }
    }

    // Finalize averages
    report.types.classes.finalize();
    report.types.structs.finalize();
    report.types.interfaces.finalize();
    report.types.unions.finalize();
    report.types.arrays.finalize();

    report.symbols.functions.finalize();

    report.type_details = type_details;
    report.symbol_details = symbol_details;

    Ok(report)
}

fn format_check(has: bool, name: &str, count: usize) -> String {
    if has {
        format!("[X] {} ({})", name, count)
    } else {
        format!("[ ] {} (0)", name)
    }
}

fn format_text_report(report: &PdbReport, no_mangled: bool) -> String {
    let mut output = String::new();

    output.push_str(&format!(
        "=== PDB Report: {} ===\n\n",
        Path::new(&report.file_path)
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| report.file_path.clone())
    ));

    if report.status != "ok" {
        output.push_str(&format!("Status: {}\n", report.status));
        if let Some(ref err) = report.error {
            output.push_str(&format!("Error: {}\n", err));
        }
        return output;
    }

    // Summary
    output.push_str("Summary:\n");
    output.push_str(&format!(
        "  {:30} {:30}\n",
        format_check(
            report.summary.has_public_functions,
            "Public Functions",
            report.symbols.public_functions.count
        ),
        format_check(
            report.summary.has_public_data,
            "Public Data",
            report.symbols.public_data.count
        )
    ));
    output.push_str(&format!(
        "  {:30} {:30}\n",
        format_check(
            report.summary.has_functions,
            "Functions",
            report.symbols.functions.count
        ),
        format_check(
            report.summary.has_structs,
            "Structs",
            report.types.structs.count
        )
    ));
    output.push_str(&format!(
        "  {:30} {:30}\n",
        format_check(
            report.summary.has_classes,
            "Classes",
            report.types.classes.count
        ),
        format_check(
            report.summary.has_global_data,
            "Global Data",
            report.symbols.global_data.count
        )
    ));
    output.push_str(&format!(
        "  {:30} {:30}\n",
        format_check(
            report.summary.has_enums,
            "Enumerations",
            report.types.enumerations.count
        ),
        format_check(
            report.summary.has_unions,
            "Unions",
            report.types.unions.count
        )
    ));
    output.push('\n');

    // Type Statistics
    output.push_str("Type Statistics:\n");
    if report.types.classes.count > 0 {
        let avg = report
            .types
            .classes
            .avg_size
            .map(|a| format!(" (avg: {:.0} B)", a))
            .unwrap_or_default();
        output.push_str(&format!(
            "  Classes:       {:>6}{}\n",
            report.types.classes.count, avg
        ));
    }
    if report.types.structs.count > 0 {
        let avg = report
            .types
            .structs
            .avg_size
            .map(|a| format!(" (avg: {:.0} B)", a))
            .unwrap_or_default();
        output.push_str(&format!(
            "  Structs:       {:>6}{}\n",
            report.types.structs.count, avg
        ));
    }
    if report.types.interfaces.count > 0 {
        output.push_str(&format!(
            "  Interfaces:    {:>6}\n",
            report.types.interfaces.count
        ));
    }
    if report.types.unions.count > 0 {
        let avg = report
            .types
            .unions
            .avg_size
            .map(|a| format!(" (avg: {:.0} B)", a))
            .unwrap_or_default();
        output.push_str(&format!(
            "  Unions:        {:>6}{}\n",
            report.types.unions.count, avg
        ));
    }
    if report.types.enumerations.count > 0 {
        output.push_str(&format!(
            "  Enumerations:  {:>6}\n",
            report.types.enumerations.count
        ));
    }
    if report.types.procedures.count > 0 {
        output.push_str(&format!(
            "  Procedures:    {:>6}\n",
            report.types.procedures.count
        ));
    }
    if report.types.arrays.count > 0 {
        output.push_str(&format!(
            "  Arrays:        {:>6}\n",
            report.types.arrays.count
        ));
    }
    if report.types.pointers.count > 0 {
        output.push_str(&format!(
            "  Pointers:      {:>6}\n",
            report.types.pointers.count
        ));
    }
    output.push('\n');

    // Symbol Statistics
    output.push_str("Symbol Statistics:\n");
    if report.symbols.functions.count > 0 {
        output.push_str(&format!(
            "  Functions:        {:>6}\n",
            report.symbols.functions.count
        ));
    }
    if report.symbols.global_data.count > 0 {
        output.push_str(&format!(
            "  Global Data:      {:>6}\n",
            report.symbols.global_data.count
        ));
    }
    if report.symbols.public_functions.count > 0 {
        output.push_str(&format!(
            "  Public Functions: {:>6}\n",
            report.symbols.public_functions.count
        ));
    }
    if report.symbols.public_data.count > 0 {
        output.push_str(&format!(
            "  Public Data:      {:>6}\n",
            report.symbols.public_data.count
        ));
    }
    if report.symbols.constants.count > 0 {
        output.push_str(&format!(
            "  Constants:        {:>6}\n",
            report.symbols.constants.count
        ));
    }
    if report.symbols.exports.count > 0 {
        output.push_str(&format!(
            "  Exports:          {:>6}\n",
            report.symbols.exports.count
        ));
    }
    if report.symbols.labels.count > 0 {
        output.push_str(&format!(
            "  Labels:           {:>6}\n",
            report.symbols.labels.count
        ));
    }

    // Detailed lists if requested
    if let Some(ref type_details) = report.type_details {
        output.push_str("\n--- Type Details ---\n");

        if !type_details.classes.is_empty() {
            output.push_str(&format!("\nClasses ({}):\n", type_details.classes.len()));
            for item in &type_details.classes {
                let size_str = item.size.map(|s| format!(" ({} B)", s)).unwrap_or_default();
                output.push_str(&format!("  {}{}\n", item.name, size_str));
            }
        }

        if !type_details.structs.is_empty() {
            output.push_str(&format!("\nStructs ({}):\n", type_details.structs.len()));
            for item in &type_details.structs {
                let size_str = item.size.map(|s| format!(" ({} B)", s)).unwrap_or_default();
                output.push_str(&format!("  {}{}\n", item.name, size_str));
            }
        }

        if !type_details.enumerations.is_empty() {
            output.push_str(&format!(
                "\nEnumerations ({}):\n",
                type_details.enumerations.len()
            ));
            for item in &type_details.enumerations {
                output.push_str(&format!("  {}\n", item.name));
            }
        }

        if !type_details.unions.is_empty() {
            output.push_str(&format!("\nUnions ({}):\n", type_details.unions.len()));
            for item in &type_details.unions {
                let size_str = item.size.map(|s| format!(" ({} B)", s)).unwrap_or_default();
                output.push_str(&format!("  {}{}\n", item.name, size_str));
            }
        }
    }

    if let Some(ref symbol_details) = report.symbol_details {
        output.push_str("\n--- Symbol Details ---\n");

        if !symbol_details.functions.is_empty() {
            output.push_str(&format!(
                "\nFunctions ({}):\n",
                symbol_details.functions.len()
            ));
            for item in &symbol_details.functions {
                output.push_str(&format!("  {}\n", item.name));
            }
        }

        if !symbol_details.global_data.is_empty() {
            output.push_str(&format!(
                "\nGlobal Data ({}):\n",
                symbol_details.global_data.len()
            ));
            for item in &symbol_details.global_data {
                output.push_str(&format!("  {}\n", item.name));
            }
        }

        if !symbol_details.exports.is_empty() {
            output.push_str(&format!("\nExports ({}):\n", symbol_details.exports.len()));
            for item in &symbol_details.exports {
                output.push_str(&format!("  {}\n", item.name));
            }
        }

        if !symbol_details.public_functions.is_empty() {
            output.push_str(&format!(
                "\nPublic Functions ({}):\n",
                symbol_details.public_functions.len()
            ));
            for item in &symbol_details.public_functions {
                let mangled_suffix = if no_mangled {
                    String::new()
                } else {
                    item.mangled_name
                        .as_ref()
                        .filter(|m| *m != &item.name)
                        .map(|m| format!("  [{}]", m))
                        .unwrap_or_default()
                };
                output.push_str(&format!("  {}{}\n", item.name, mangled_suffix));
            }
        }

        if !symbol_details.public_data.is_empty() {
            output.push_str(&format!(
                "\nPublic Data ({}):\n",
                symbol_details.public_data.len()
            ));
            for item in &symbol_details.public_data {
                let mangled_suffix = if no_mangled {
                    String::new()
                } else {
                    item.mangled_name
                        .as_ref()
                        .filter(|m| *m != &item.name)
                        .map(|m| format!("  [{}]", m))
                        .unwrap_or_default()
                };
                output.push_str(&format!("  {}{}\n", item.name, mangled_suffix));
            }
        }
    }

    output
}

/// Get mangled name (always returns it, empty string if none)
fn get_mangled_name(item: &SymbolItem) -> &str {
    item.mangled_name.as_deref().unwrap_or("")
}

fn format_grep_report(report: &PdbReport, no_mangled: bool) -> String {
    let mut output = String::new();

    // Format: CATEGORY<TAB>NAME<TAB>MANGLED<TAB>OFFSET<TAB>SEGMENT
    // or:     CATEGORY<TAB>NAME<TAB>OFFSET<TAB>SEGMENT (if no_mangled)
    // Note: MANGLED is empty if same as NAME
    // Types
    if let Some(ref type_details) = report.type_details {
        for item in &type_details.classes {
            output.push_str(&format!(
                "class\t{}\t{}\n",
                item.name,
                item.size.map(|s| s.to_string()).unwrap_or_default()
            ));
        }
        for item in &type_details.structs {
            output.push_str(&format!(
                "struct\t{}\t{}\n",
                item.name,
                item.size.map(|s| s.to_string()).unwrap_or_default()
            ));
        }
        for item in &type_details.enumerations {
            output.push_str(&format!("enum\t{}\n", item.name));
        }
        for item in &type_details.unions {
            output.push_str(&format!(
                "union\t{}\t{}\n",
                item.name,
                item.size.map(|s| s.to_string()).unwrap_or_default()
            ));
        }
    }

    // Symbols
    if let Some(ref symbol_details) = report.symbol_details {
        for item in &symbol_details.functions {
            if no_mangled {
                output.push_str(&format!(
                    "function\t{}\t{}\t{}\n",
                    item.name,
                    item.offset.map(|o| o.to_string()).unwrap_or_default(),
                    item.segment.map(|s| s.to_string()).unwrap_or_default()
                ));
            } else {
                output.push_str(&format!(
                    "function\t{}\t{}\t{}\t{}\n",
                    item.name,
                    get_mangled_name(item),
                    item.offset.map(|o| o.to_string()).unwrap_or_default(),
                    item.segment.map(|s| s.to_string()).unwrap_or_default()
                ));
            }
        }
        for item in &symbol_details.global_data {
            if no_mangled {
                output.push_str(&format!(
                    "global_data\t{}\t{}\t{}\n",
                    item.name,
                    item.offset.map(|o| o.to_string()).unwrap_or_default(),
                    item.segment.map(|s| s.to_string()).unwrap_or_default()
                ));
            } else {
                output.push_str(&format!(
                    "global_data\t{}\t{}\t{}\t{}\n",
                    item.name,
                    get_mangled_name(item),
                    item.offset.map(|o| o.to_string()).unwrap_or_default(),
                    item.segment.map(|s| s.to_string()).unwrap_or_default()
                ));
            }
        }
        for item in &symbol_details.public_functions {
            if no_mangled {
                output.push_str(&format!(
                    "public_function\t{}\t{}\t{}\n",
                    item.name,
                    item.offset.map(|o| o.to_string()).unwrap_or_default(),
                    item.segment.map(|s| s.to_string()).unwrap_or_default()
                ));
            } else {
                output.push_str(&format!(
                    "public_function\t{}\t{}\t{}\t{}\n",
                    item.name,
                    get_mangled_name(item),
                    item.offset.map(|o| o.to_string()).unwrap_or_default(),
                    item.segment.map(|s| s.to_string()).unwrap_or_default()
                ));
            }
        }
        for item in &symbol_details.public_data {
            if no_mangled {
                output.push_str(&format!(
                    "public_data\t{}\t{}\t{}\n",
                    item.name,
                    item.offset.map(|o| o.to_string()).unwrap_or_default(),
                    item.segment.map(|s| s.to_string()).unwrap_or_default()
                ));
            } else {
                output.push_str(&format!(
                    "public_data\t{}\t{}\t{}\t{}\n",
                    item.name,
                    get_mangled_name(item),
                    item.offset.map(|o| o.to_string()).unwrap_or_default(),
                    item.segment.map(|s| s.to_string()).unwrap_or_default()
                ));
            }
        }
        for item in &symbol_details.constants {
            if no_mangled {
                output.push_str(&format!("constant\t{}\n", item.name));
            } else {
                output.push_str(&format!(
                    "constant\t{}\t{}\n",
                    item.name,
                    get_mangled_name(item)
                ));
            }
        }
        for item in &symbol_details.exports {
            if no_mangled {
                output.push_str(&format!("export\t{}\n", item.name));
            } else {
                output.push_str(&format!(
                    "export\t{}\t{}\n",
                    item.name,
                    get_mangled_name(item)
                ));
            }
        }
    }

    output
}

/// Format a PdbReport into a string based on the output format.
///
/// # Arguments
/// * `report` - The PDB report to format
/// * `format` - The output format (Text, Json, or Grep)
/// * `no_mangled` - If true, hide mangled names in output
///
/// # Returns
/// A formatted string representation of the report
pub fn format_report(report: &PdbReport, format: OutputFormat, no_mangled: bool) -> String {
    match format {
        OutputFormat::Text => format_text_report(report, no_mangled),
        OutputFormat::Json => serde_json::to_string_pretty(report)
            .unwrap_or_else(|e| format!("{{\"error\": \"JSON serialization failed: {}\"}}", e)),
        OutputFormat::Grep => format_grep_report(report, no_mangled),
    }
}

/// Generate a report for a single PDB file
#[cfg(feature = "cli")]
pub fn run(
    pdb_file: PathBuf,
    output: Option<PathBuf>,
    format: OutputFormat,
    detailed: bool,
    no_mangled: bool,
) -> Result<()> {
    let report = match analyze_pdb(&pdb_file, detailed) {
        Ok(r) => r,
        Err(e) => PdbReport {
            file_path: pdb_file.display().to_string(),
            status: "parse_error".into(),
            error: Some(e.to_string()),
            ..Default::default()
        },
    };

    let output_str = format_report(&report, format, no_mangled);

    if let Some(out_path) = output {
        std::fs::write(&out_path, &output_str)?;
        println!("Report written to: {}", out_path.display());
    } else {
        println!("{}", output_str);
    }

    Ok(())
}
