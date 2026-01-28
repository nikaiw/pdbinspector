use anyhow::Result;
use clap::{Parser, Subcommand, ValueEnum};
use pdbinspector::OutputFormat;
use std::path::PathBuf;

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

impl From<OutputFormatArg> for OutputFormat {
    fn from(arg: OutputFormatArg) -> Self {
        match arg {
            OutputFormatArg::Text => OutputFormat::Text,
            OutputFormatArg::Json => OutputFormat::Json,
            OutputFormatArg::Grep => OutputFormat::Grep,
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
            pdbinspector::run(pdb_file, output, format.into(), detailed, !mangled)?;
        }
    }

    Ok(())
}
