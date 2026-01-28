use anyhow::Result;
use clap::{Parser, Subcommand, ValueEnum};
use std::path::PathBuf;

mod report;

#[derive(Parser)]
#[command(name = "pdbinspector")]
#[command(about = "Inspect and analyze Microsoft PDB (Program Database) files")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Generate symbol report for a single PDB file
    Report {
        /// Path to the PDB file to analyze
        pdb_file: PathBuf,
        /// Output file path (prints to stdout if not specified)
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Output format
        #[arg(short, long, value_enum, default_value = "text")]
        format: OutputFormatArg,
        /// Include detailed lists of types and symbols
        #[arg(short, long)]
        detailed: bool,
        /// Show mangled names in output (hidden by default)
        #[arg(short = 'm', long)]
        mangled: bool,
    },
}

#[derive(Copy, Clone, PartialEq, Eq, ValueEnum)]
enum OutputFormatArg {
    Text,
    Json,
    Grep,
}

impl From<OutputFormatArg> for report::OutputFormat {
    fn from(arg: OutputFormatArg) -> Self {
        match arg {
            OutputFormatArg::Text => report::OutputFormat::Text,
            OutputFormatArg::Json => report::OutputFormat::Json,
            OutputFormatArg::Grep => report::OutputFormat::Grep,
        }
    }
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Report {
            pdb_file,
            output,
            format,
            detailed,
            mangled,
        } => {
            report::run(pdb_file, output, format.into(), detailed, !mangled)?;
        }
    }

    Ok(())
}
