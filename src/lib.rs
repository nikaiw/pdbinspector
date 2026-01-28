//! PDB Inspector Library
//!
//! A library for analyzing Microsoft PDB (Program Database) files.
//!
//! # Example
//!
//! ```no_run
//! use pdbinspector::{analyze_pdb_from_bytes, OutputFormat, format_report};
//!
//! let pdb_data = std::fs::read("example.pdb").unwrap();
//! let report = analyze_pdb_from_bytes(&pdb_data, true).unwrap();
//! let output = format_report(&report, OutputFormat::Text, true);
//! println!("{}", output);
//! ```

mod report;

pub use report::{
    analyze_pdb_from_bytes, format_report, CategoryStats, OutputFormat, PdbReport, ReportSummary,
    SymbolDetails, SymbolItem, SymbolStats, TypeDetails, TypeItem, TypeStats,
};

#[cfg(feature = "cli")]
pub use report::run;
