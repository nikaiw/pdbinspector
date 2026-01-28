# pdbinspector

A command-line tool to inspect and analyze Microsoft PDB (Program Database) files.

**[Try it online](https://nikaiw.github.io/pdbinspector/)** (WebAssembly)

## Features

- Extract type information (structs, classes, enums, unions)
- List symbols (functions, global data, public symbols, exports)
- Demangle MSVC C++ symbol names
- Decode MSVC string literals (`??_C@...` -> `L"string"`)
- Classify public symbols as functions or data
- Output in text, JSON, or grep-friendly format

## Installation

### From Releases

Download pre-built binaries from the [Releases](https://github.com/nikaiw/pdbinspector/releases) page:

- **Windows**: `pdbinspector-windows-x64.zip`
- **macOS (Intel)**: `pdbinspector-macos-x64.tar.gz`
- **macOS (Apple Silicon)**: `pdbinspector-macos-arm64.tar.gz`
- **Linux**: `pdbinspector-linux-x64.tar.gz`

### From Source

```bash
cargo build --release
```

## Usage

### Generate a report for a PDB file

```bash
# Text output
pdbinspector report myfile.pdb

# JSON output
pdbinspector report myfile.pdb --format json

# With detailed symbol lists
pdbinspector report myfile.pdb --detailed

# Show mangled names (hidden by default)
pdbinspector report myfile.pdb --detailed --mangled
pdbinspector report myfile.pdb -d -m  # short form

# Save to file
pdbinspector report myfile.pdb --output report.json --format json
```

## Grep format

Tab-separated output for easy filtering with grep/awk/cut:

```
CATEGORY	NAME	MANGLED_NAME	OFFSET	SEGMENT
```

```bash
# Grep-friendly output
pdbinspector report file.pdb --detailed --format grep

# Filter by category
pdbinspector report file.pdb --detailed --format grep | grep "^public_function"

# Search by name
pdbinspector report file.pdb --detailed --format grep | grep -i create

# Extract names only
pdbinspector report file.pdb --detailed --format grep | awk -F'\t' '{print $2}'

# Count by category
pdbinspector report file.pdb --detailed --format grep | cut -f1 | sort | uniq -c
```

## JSON filtering with jq

```bash
# List public functions
pdbinspector report file.pdb --detailed --format json | jq '.symbol_details.public_functions'

# Search by name
pdbinspector report file.pdb --detailed --format json | jq '[.symbol_details.public_functions[] | select(.name | contains("Create"))]'

# Get symbol counts
pdbinspector report file.pdb --format json | jq '.symbols'
```

## Acknowledgments

- [pdb](https://github.com/getsentry/pdb) - Rust crate for parsing PDB files
- Test fixtures from the [getsentry/pdb](https://github.com/getsentry/pdb) repository

## License

MIT
