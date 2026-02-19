# EmFit

High-performance NTFS file scanner for Windows. Combines the speed of Everything with WizTree's disk analysis.

## Features

- **Sub-second scanning** - Direct MFT/USN access indexes entire drives instantly
- **Instant search** - Wildcard patterns, path scoping, regex, size/date filters
- **Disk analysis** - Interactive treemap visualization of directory sizes
- **Multi-drive support** - Scan and search across multiple volumes simultaneously
- **Real-time monitoring** - USN journal tracking for live file system changes
- **Batch operations** - Multi-select files for open, delete, rename, copy paths

## Installation

```powershell
cargo build --release
```

Binary: `target/release/emfit.exe`

## Usage

### Interactive TUI (default)

```powershell
emfit
```

**Keyboard shortcuts:**
- `/` or `Tab` - Focus search bar
- `F1-F6` - Sort by column
- `↑/↓`, `Pg Up/Pg Dwown`, or `j/k` - Navigate
- `Space` - Multi-select
- `Ctrl+A` - Select all
- `Shift+↑/↓` - Range select
- `Ctrl+↑/↓` - Move without selecting
- `Enter` - Open file
- `m` - Actions menu (open, delete, rename, etc.)
- `t` - Toggle treemap view
- `Ctrl+F` - Advanced filters (regex, size, date, extension)
- `Ctrl+C/Q` - Quit

### Search Syntax

**Basic patterns:**
```
config              # Contains "config"
*.log               # Extension match
temp*               # Starts with "temp"
*cache*             # Contains "cache"
```

**Path scoping:**
```
`C:\Users\Steven\Documents` hello.cpp    # Search specific folder
`C:\Projects` *.rs                       # All Rust files in Projects
```

**Advanced filters** (`Ctrl+F`):
- **Regex:** `^test.*\.txt$`
- **Size:** `> 100MB`, `< 1GB`, `between 50KB and 500KB`
- **Date:** After, Before, or Between specific dates
- **Extension:** Comma-separated list

**Multiple patterns** (semicolon-separated):
```
*.cpp; *.h; Makefile
```

### CLI Mode

**Scan volume:**
```powershell
emfit cli scan -d C --mft true
```

**Search files:**
```powershell
emfit cli search -d C "*.dll" --max 100
```

**Largest files:**
```powershell
emfit cli largest -d C --count 50
```

**Largest directories:**
```powershell
emfit cli largest -d C --dirs --count 20
```

**Disk space analysis:**
```powershell
emfit cli tree-size -d C --depth 3
```

**List NTFS volumes:**
```powershell
emfit cli volumes
```

**Monitor changes:**
```powershell
emfit cli monitor -d C
```

**Export results:**
```powershell
emfit cli export -d C -o output.json -f json
emfit cli export -d C -o output.csv -f csv
```

## How It Works

EmFit uses two NTFS features for maximum performance:

**1. USN Journal Enumeration** (`FSCTL_ENUM_USN_DATA`)
- Scans entire file system in seconds
- Provides file names, parent references, and attributes
- Doesn't include file sizes

**2. Direct MFT Reading**
- Reads Master File Table for accurate metadata
- Parses `$STANDARD_INFORMATION`, `$FILE_NAME`, and `$DATA` attributes
- Gets true file sizes and timestamps

## Preset Filters

Create `Filters.csv` in the executable directory:

```csv
Name,Extensions,Description
Videos,mp4;mkv;avi;mov;wmv,Video files
Images,jpg;png;gif;bmp;webp,Image files
Documents,pdf;doc;docx;txt;md,Documents
Archives,zip;rar;7z;tar;gz,Compressed files
Code,rs;cpp;c;h;py;js,Source code
```

Access via menu bar in TUI.

## Performance

- **Typical scan time:** 500GB drive with 1M files in ~2 seconds
- **Memory usage:** ~200-300MB for 1M files
- **Search latency:** <1ms for indexed results

## Requirements

- Windows 10/11
- Administrator privileges (for direct MFT/USN access)
- NTFS volumes only

## Technical Details

- **Language:** Rust
- **TUI Framework:** Ratatui
- **Threading:** Rayon + Crossbeam for parallel processing
- **Parsing:** Zero-copy MFT record parsing
- **Optimization:** LTO + single codegen unit for release builds

## License

See repository for license information.
