# EmFit 🦀⚡

EmFit is a high-performance file system scanner for Windows NTFS volumes.

## Features

- **Blazing Fast** - Scan entire drives in seconds
- **Accurate Sizes** - Direct MFT parsing for true file sizes
- **Real-time Monitoring** - Track file changes as they happen
- **Instant Search** - Find files across indexed volumes instantly
- **Space Analysis** - WizTree-style directory size breakdown
- **Memory Safe** - Written in Rust with zero-copy parsing

## How It Works

### Phase 1: USN Journal Enumeration

The USN (Update Sequence Number) Journal is NTFS's change journal. We use `FSCTL_ENUM_USN_DATA` to enumerate all files instantly:

```
┌─────────────────────────────────────────────┐
│  FSCTL_ENUM_USN_DATA                        │
│  ↓                                          │
│  USN_RECORD for each file:                  │
│  • File Reference Number (unique ID)        │
│  • Parent Reference Number                  │
│  • File Name                                │
│  • Attributes (directory, hidden, etc.)     │
└─────────────────────────────────────────────┘
```

This gives us the complete file tree structure in seconds, but **not file sizes**.

### Phase 2: MFT Reading (Optional)

For accurate file sizes, we read the Master File Table directly:

```
┌─────────────────────────────────────────────┐
│  Direct MFT Access                          │
│  ↓                                          │
│  MFT Record (1024 bytes each):              │
│  • "FILE" signature verification            │
│  • Fixup array for data integrity           │
│  • $STANDARD_INFORMATION (timestamps)       │
│  • $FILE_NAME (name, parent)                │
│  • $DATA (file size, cluster runs)          │
└─────────────────────────────────────────────┘
```

## Usage

### Scan a Volume

```bash
# Quick scan (USN only)
emfit scan -d C

# Full scan with sizes
emfit scan -d C --mft true

# Exclude hidden/system files
emfit scan -d C --hidden false --system false
```

### Search Files

```bash
# Search for files
emfit search -d C "config"

# Limit results
emfit search -d C "*.log" --max 50
```

### Find Largest Files

```bash
# Largest files
emfit largest -d C --count 20

# Largest directories
emfit largest -d C --dirs --count 20
```

### Analyze Disk Space

```bash
# WizTree-style analysis
emfit tree-size -d C --depth 3
```

### List NTFS Volumes

```bash
emfit volumes
```

### Monitor Changes

```bash
emfit monitor -d C
```

### Count raw USN Enumeration Entries

```bash
emfit.exe usn-count -d C
```

### Export Results

```bash
# Export to JSON
emfit export -d C -o scan.json

# Export to CSV
emfit export -d C -o scan.csv -f csv
```

## Architecture

```
emfit/
├── src/
│   ├── main.rs           # CLI entry point
│   ├── lib.rs            # Library root
│   ├── error.rs          # Error types
│   ├── file_tree.rs      # Tree data structure
│   ├── scanner.rs        # Main scanner logic
│   └── ntfs/
│       ├── mod.rs        # NTFS module root
│       ├── structs.rs    # NTFS data structures
│       ├── winapi.rs     # Windows API bindings
│       ├── mft.rs        # MFT parser
│       └── usn.rs        # USN Journal scanner
└── Cargo.toml
```

## Requirements

- Windows 10/11 (or Server 2016+)
- Administrator privileges (for raw volume access)
- NTFS volumes only

## Building

```bash
# Release CLI build
cargo build --release

# Release GUI build
cargo build --bin emfit-gui --release

# Run tests
cargo test
```

Additional resources:
- [FSCTL Structures](https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-fscc/4dc02779-9d95-43f8-bba4-8d4ce4961458)
- [Microsoft NTFS Technical Reference](https://docs.microsoft.com/en-us/windows-server/storage/file-server/ntfs-overview)
- [The Sleuth Kit](https://github.com/sleuthkit/sleuthkit) - Forensic NTFS parsing