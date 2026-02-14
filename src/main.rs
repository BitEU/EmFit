//! EmFit CLI
//!
//! Command-line interface for the EmFit file scanner.
//! Provides both command-line and interactive search modes.

use clap::{Parser, Subcommand};
use console::{style, Term};
use indicatif::HumanDuration;
use emfit::{
    format_size, FileTree,
    MultiVolumeScanner, ScanConfig, VolumeScanner,
};
use std::io::Write;
use std::time::Instant;

/// EmFit - Ultra-fast NTFS file scanner
///
/// Combines USN Journal enumeration with direct MFT reading
/// for instant, accurate file system scanning.
#[derive(Parser)]
#[command(name = "emfit")]
#[command(author = "EmFit Contributors")]
#[command(version)]
#[command(about = "Ultra-fast NTFS file scanner", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Scan a volume and display statistics
    Scan {
        /// Drive letter to scan (e.g., C)
        #[arg(short, long)]
        drive: char,

        /// Use USN Journal for fast enumeration (default: false, uses MFT for accuracy)
        #[arg(long, default_value = "false")]
        usn: bool,

        /// Use direct MFT reading for accurate sizes
        #[arg(long, default_value = "true")]
        mft: bool,

        /// Include hidden files
        #[arg(long, default_value = "true")]
        hidden: bool,

        /// Include system files
        #[arg(long, default_value = "true")]
        system: bool,

        /// Output format (text, json)
        #[arg(short, long, default_value = "text")]
        output: String,
    },

    /// Search for files matching a pattern
    Search {
        /// Drive letter to search
        #[arg(short, long)]
        drive: char,

        /// Search pattern (use -- before pattern if it starts with -)
        #[arg(allow_hyphen_values = true)]
        pattern: String,

        /// Maximum results
        #[arg(short, long, default_value = "100")]
        max: usize,
    },

    /// Show largest files
    Largest {
        /// Drive letter
        #[arg(short, long)]
        drive: char,

        /// Number of files to show
        #[arg(short, long, default_value = "20")]
        count: usize,

        /// Show directories instead of files
        #[arg(long)]
        dirs: bool,
    },

    /// Analyze disk space usage (WizTree-style)
    TreeSize {
        /// Drive letter
        #[arg(short, long)]
        drive: char,

        /// Path to analyze (default: root)
        #[arg(short, long)]
        path: Option<String>,

        /// Depth to display
        #[arg(long, default_value = "3")]
        depth: usize,
    },

    /// List available NTFS volumes
    Volumes,

    /// Monitor file system changes in real-time
    Monitor {
        /// Drive letter to monitor
        #[arg(short, long)]
        drive: char,
    },

    /// Export scan results
    Export {
        /// Drive letter to scan
        #[arg(short, long)]
        drive: char,

        /// Output file path
        #[arg(short, long)]
        output: String,

        /// Format (json, csv)
        #[arg(short, long, default_value = "json")]
        format: String,
    },

    /// Debug: trace a file's parent chain
    Debug {
        /// Drive letter
        #[arg(short, long)]
        drive: char,

        /// Pattern to search for (use -- before pattern if it starts with -)
        #[arg(allow_hyphen_values = true)]
        pattern: String,
    },

    /// Read a specific MFT record directly
    ReadMft {
        /// Drive letter
        #[arg(short, long)]
        drive: char,

        /// MFT record number to read
        record: u64,
    },

    /// Debug: count raw USN enumeration results
    UsnCount {
        /// Drive letter
        #[arg(short, long)]
        drive: char,
    },
}

fn main() {
    // Initialize logging
    emfit::logging::init();
    emfit::logging::info("MAIN", "EmFit starting up");

    let cli = Cli::parse();

    let result = match cli.command {
        Commands::Scan {
            drive,
            usn,
            mft,
            hidden,
            system,
            output,
        } => cmd_scan(drive, usn, mft, hidden, system, &output),

        Commands::Search { drive, pattern, max } => cmd_search(drive, &pattern, max),

        Commands::Largest { drive, count, dirs } => cmd_largest(drive, count, dirs),

        Commands::TreeSize { drive, path, depth } => cmd_tree_size(drive, path.as_deref(), depth),

        Commands::Volumes => cmd_volumes(),

        Commands::Monitor { drive } => cmd_monitor(drive),

        Commands::Export {
            drive,
            output,
            format,
        } => cmd_export(drive, &output, &format),

        Commands::Debug { drive, pattern } => cmd_debug(drive, &pattern),

        Commands::ReadMft { drive, record } => cmd_read_mft(drive, record),

        Commands::UsnCount { drive } => cmd_usn_count(drive),
    };

    if let Err(e) = result {
        eprintln!("{} {}", style("Error:").red().bold(), e);
        std::process::exit(1);
    }
}

/// Scan command implementation
fn cmd_scan(
    drive: char,
    use_usn: bool,
    use_mft: bool,
    include_hidden: bool,
    include_system: bool,
    output_format: &str,
) -> emfit::Result<()> {
    let term = Term::stdout();
    let start = Instant::now();

    println!(
        "{} Scanning drive {}:",
        style("→").cyan().bold(),
        style(format!("{}:", drive.to_ascii_uppercase())).yellow()
    );

    let config = ScanConfig {
        use_usn,
        use_mft,
        include_hidden,
        include_system,
        calculate_sizes: true,
        show_progress: true,
        batch_size: 1024,
    };

    let mut scanner = VolumeScanner::new(drive).with_config(config);
    let tree = scanner.scan()?;

    let elapsed = start.elapsed();

    if output_format == "json" {
        // JSON output
        println!(
            "{}",
            serde_json::json!({
                "drive": drive.to_string(),
                "files": tree.stats.total_files,
                "directories": tree.stats.total_directories,
                "total_size": tree.stats.total_size,
                "total_size_formatted": format_size(tree.stats.total_size),
                "allocated_size": tree.stats.total_allocated,
                "orphaned": tree.stats.orphaned_files,
                "elapsed_seconds": elapsed.as_secs_f64(),
            })
        );
    } else {
        // Text output
        println!();
        println!(
            "{} Scan complete in {}",
            style("✓").green().bold(),
            style(HumanDuration(elapsed)).cyan()
        );
        println!();
        println!("  {} {}", style("Files:").bold(), tree.stats.total_files);
        println!(
            "  {} {}",
            style("Directories:").bold(),
            tree.stats.total_directories
        );
        println!(
            "  {} {}",
            style("Total Size:").bold(),
            style(format_size(tree.stats.total_size)).yellow()
        );
        println!(
            "  {} {}",
            style("Allocated:").bold(),
            format_size(tree.stats.total_allocated)
        );
        if tree.stats.orphaned_files > 0 {
            println!(
                "  {} {}",
                style("Orphaned:").bold(),
                style(tree.stats.orphaned_files).red()
            );
        }
        println!();
        println!(
            "  {} {:.0} files/sec",
            style("Speed:").bold(),
            tree.stats.total_files as f64 / elapsed.as_secs_f64()
        );
    }

    Ok(())
}

/// Search command implementation
fn cmd_search(drive: char, pattern: &str, max_results: usize) -> emfit::Result<()> {
    println!(
        "{} Searching for '{}' on {}:",
        style("→").cyan().bold(),
        style(pattern).yellow(),
        drive.to_ascii_uppercase()
    );

    let start = Instant::now();

    // Use MFT for complete file enumeration including hard links
    let config = ScanConfig {
        use_usn: false,
        use_mft: true,
        calculate_sizes: false, // Skip size calculation for search speed
        show_progress: true,
        ..Default::default()
    };

    let mut scanner = VolumeScanner::new(drive).with_config(config);
    let tree = scanner.scan()?;

    let results = tree.search(pattern, max_results);

    println!();
    println!(
        "Found {} results in {:.2}s:",
        style(results.len()).green(),
        start.elapsed().as_secs_f64()
    );
    println!();

    for (i, result) in results.iter().enumerate() {
        let icon = if result.is_directory { "📁" } else { "📄" };
        println!(
            "  {} {} {}",
            style(format!("{:3}.", i + 1)).dim(),
            icon,
            style(&result.path).cyan()
        );
        if result.file_size > 0 {
            println!(
                "      {} {}",
                style("Size:").dim(),
                format_size(result.file_size)
            );
        }
    }

    Ok(())
}

/// Largest files/directories command
fn cmd_largest(drive: char, count: usize, show_dirs: bool) -> emfit::Result<()> {
    let item_type = if show_dirs { "directories" } else { "files" };
    println!(
        "{} Finding {} largest {} on {}:",
        style("→").cyan().bold(),
        count,
        item_type,
        drive.to_ascii_uppercase()
    );

    let config = ScanConfig {
        calculate_sizes: true,
        show_progress: true,
        ..Default::default()
    };

    let mut scanner = VolumeScanner::new(drive).with_config(config);
    let tree = scanner.scan()?;

    let results = if show_dirs {
        tree.largest_directories(count)
    } else {
        tree.largest_files(count)
    };

    println!();
    println!("Largest {}:", item_type);
    println!();

    for (i, result) in results.iter().enumerate() {
        let size_str = format_size(result.file_size);
        let icon = if result.is_directory { "📁" } else { "📄" };

        println!(
            "  {} {} {} {}",
            style(format!("{:3}.", i + 1)).dim(),
            style(format!("{:>12}", size_str)).yellow(),
            icon,
            style(&result.path).cyan()
        );
    }

    Ok(())
}

/// Tree size analysis command
fn cmd_tree_size(drive: char, path: Option<&str>, depth: usize) -> emfit::Result<()> {
    println!(
        "{} Analyzing disk space on {}:",
        style("→").cyan().bold(),
        drive.to_ascii_uppercase()
    );

    let config = ScanConfig {
        calculate_sizes: true,
        show_progress: true,
        ..Default::default()
    };

    let mut scanner = VolumeScanner::new(drive).with_config(config);
    let tree = scanner.scan()?;

    // Find starting node
    let start_node = if let Some(p) = path {
        // Would need path-to-record-number lookup
        tree.root()
    } else {
        tree.root()
    };

    println!();
    println!("Directory Size Analysis:");
    println!();

    if let Some(root) = start_node {
        print_tree_node(&tree, &root, 0, depth);
    }

    Ok(())
}

fn print_tree_node(tree: &FileTree, node: &emfit::TreeNode, indent: usize, max_depth: usize) {
    if indent > max_depth {
        return;
    }

    let indent_str = "  ".repeat(indent);
    let size_str = format_size(node.total_size);

    if node.is_directory {
        println!(
            "{}📁 {} {}",
            indent_str,
            style(format!("{:>12}", size_str)).yellow(),
            style(&node.name).cyan()
        );

        // Get children sorted by size
        let mut children = tree.get_children(&node.key());
        children.sort_by(|a, b| b.total_size.cmp(&a.total_size));

        // Show top children
        for child in children.iter().take(10) {
            if child.is_directory {
                print_tree_node(tree, child, indent + 1, max_depth);
            }
        }
    }
}

/// List volumes command
fn cmd_volumes() -> emfit::Result<()> {
    println!("{} Detecting NTFS volumes...", style("→").cyan().bold());
    println!();

    let volumes = MultiVolumeScanner::detect_ntfs_volumes();

    if volumes.is_empty() {
        println!("  No NTFS volumes found.");
    } else {
        println!("Available NTFS volumes:");
        println!();
        for letter in volumes {
            print!("  {} {}:", style("•").green(), letter);

            // Try to get volume info
            if let Ok(handle) = emfit::ntfs::open_volume(letter) {
                if let Ok(data) = emfit::ntfs::winapi::get_ntfs_volume_data(&handle) {
                    let total = data.total_clusters * data.bytes_per_cluster as u64;
                    let free = data.free_clusters * data.bytes_per_cluster as u64;
                    println!(
                        " {} total, {} free",
                        style(format_size(total)).yellow(),
                        style(format_size(free)).green()
                    );
                } else {
                    println!();
                }
            } else {
                println!(" (access denied)");
            }
        }
    }

    Ok(())
}

/// Monitor command
fn cmd_monitor(drive: char) -> emfit::Result<()> {
    use emfit::ChangeMonitor;

    println!(
        "{} Monitoring file system changes on {}:",
        style("→").cyan().bold(),
        drive.to_ascii_uppercase()
    );
    println!("Press Ctrl+C to stop.");
    println!();

    let mut monitor = ChangeMonitor::new(drive)?;

    // Would need a dummy tree for this - simplified implementation
    println!(
        "  {} Monitor initialized",
        style("✓").green()
    );
    println!("  Watching for changes...");

    // In a real implementation, we'd poll the monitor in a loop
    // For now, just demonstrate it compiles
    println!();
    println!("  (Real-time monitoring would run here)");

    Ok(())
}

/// Export command
fn cmd_export(drive: char, output: &str, format: &str) -> emfit::Result<()> {
    println!(
        "{} Exporting scan results to {}",
        style("→").cyan().bold(),
        style(output).yellow()
    );

    let config = ScanConfig {
        calculate_sizes: true,
        show_progress: true,
        ..Default::default()
    };

    let mut scanner = VolumeScanner::new(drive).with_config(config);
    let tree = scanner.scan()?;

    let mut file = std::fs::File::create(output)?;

    match format {
        "csv" => {
            writeln!(file, "Path,Name,Size,Allocated,IsDirectory,Modified")?;
            for entry in tree.iter() {
                let node = entry.value();
                writeln!(
                    file,
                    "\"{}\",\"{}\",{},{},{},{}",
                    tree.build_path(node.record_number),
                    node.name,
                    node.file_size,
                    node.allocated_size,
                    node.is_directory,
                    node.modification_time
                )?;
            }
        }
        _ => {
            // JSON format
            writeln!(file, "{{")?;
            writeln!(file, "  \"drive\": \"{}\",", drive)?;
            writeln!(file, "  \"stats\": {{")?;
            writeln!(file, "    \"files\": {},", tree.stats.total_files)?;
            writeln!(file, "    \"directories\": {},", tree.stats.total_directories)?;
            writeln!(file, "    \"total_size\": {}", tree.stats.total_size)?;
            writeln!(file, "  }},")?;
            writeln!(file, "  \"files\": [")?;

            let mut first = true;
            for entry in tree.iter() {
                let node = entry.value();
                if !first {
                    writeln!(file, ",")?;
                }
                first = false;
                write!(
                    file,
                    "    {{\"path\": \"{}\", \"size\": {}, \"is_dir\": {}}}",
                    tree.build_path(node.record_number).replace('\\', "\\\\"),
                    node.file_size,
                    node.is_directory
                )?;
            }

            writeln!(file)?;
            writeln!(file, "  ]")?;
            writeln!(file, "}}")?;
        }
    }

    println!(
        "{} Exported {} entries to {}",
        style("✓").green().bold(),
        tree.len(),
        output
    );

    Ok(())
}

/// Debug command - trace parent chain for a file
fn cmd_debug(drive: char, pattern: &str) -> emfit::Result<()> {
    println!(
        "{} Debug: tracing parent chain for '{}' on {}:",
        style("→").cyan().bold(),
        style(pattern).yellow(),
        drive.to_ascii_uppercase()
    );

    let config = ScanConfig {
        use_usn: false,
        use_mft: true,
        calculate_sizes: false,
        show_progress: true,
        ..Default::default()
    };

    let mut scanner = VolumeScanner::new(drive).with_config(config);
    let tree = scanner.scan()?;

    // Find matching files
    let results = tree.search(pattern, 10);
    
    if results.is_empty() {
        println!("No matches found for '{}'", pattern);
        return Ok(());
    }

    println!("\nFound {} matches:\n", results.len());

    for result in results {
        println!("=== {} ===", result.name);
        println!("  Record Number: {}", result.record_number);
        println!("  Displayed Path: {}", result.path);

        // Try build_path_debug to see on-demand resolution
        println!("\n  Attempting on-demand path resolution:");
        let resolved_path = tree.build_path_debug(result.record_number);
        println!("  Resolved Path: {}", resolved_path);

        // Trace the parent chain manually
        println!("\n  Parent Chain:");
        let mut current = result.record_number;
        let mut depth = 0;
        
        while depth < 20 {  // Prevent infinite loops
            if let Some(node) = tree.get(current) {
                println!("    [{}] Record {} -> '{}' (parent: {})", 
                    depth, 
                    node.record_number, 
                    node.name,
                    node.parent_record_number
                );
                
                if node.parent_record_number == 5 {
                    println!("    [{}] Record 5 -> ROOT", depth + 1);
                    break;
                }
                
                if node.parent_record_number == 0 || node.parent_record_number == node.record_number {
                    println!("    [STOP] Invalid parent reference");
                    break;
                }
                
                // Check if parent exists
                if tree.get(node.parent_record_number).is_none() {
                    println!("    [MISSING] Parent record {} NOT FOUND in tree!", node.parent_record_number);
                    
                    // Let's see if it's a masking issue - check if full reference exists
                    println!("    Checking nearby records...");
                    for offset in 0..5u64 {
                        let test_rec = node.parent_record_number + offset;
                        if let Some(found) = tree.get(test_rec) {
                            println!("      Found at {}: '{}'", test_rec, found.name);
                        }
                    }
                    break;
                }
                
                current = node.parent_record_number;
            } else {
                println!("    [MISSING] Record {} NOT FOUND in tree!", current);
                break;
            }
            depth += 1;
        }
        println!();
    }

    // Also check specific known records
    println!("\n=== Known Records Check ===");
    for (name, expected_rec) in [("root", 5u64), ("$Recycle.Bin approx", 36u64)] {
        if let Some(node) = tree.get(expected_rec) {
            println!("  Record {}: '{}' (is_dir: {}, parent: {})", 
                expected_rec, node.name, node.is_directory, node.parent_record_number);
        } else {
            println!("  Record {}: NOT FOUND (expected: {})", expected_rec, name);
        }
    }

    // Check for the specific missing record if pattern contains a number
    if let Ok(record_num) = pattern.parse::<u64>() {
        println!("\n=== Direct Record Lookup: {} ===", record_num);
        if let Some(node) = tree.get(record_num) {
            println!("  Found: '{}' (is_dir: {}, parent: {})", 
                node.name, node.is_directory, node.parent_record_number);
        } else {
            println!("  Record {} NOT FOUND in tree", record_num);
        }
    }

    // Special check for record 253045 (the missing SID folder)
    println!("\n=== Check for Record 253045 (suspected missing SID folder) ===");
    if let Some(node) = tree.get(253045) {
        println!("  Found: '{}' (is_dir: {}, parent: {})",
            node.name, node.is_directory, node.parent_record_number);
    } else {
        println!("  Record 253045 NOT FOUND in tree");
        // Check children of $Recycle.Bin (record 36)
        println!("\n  Children of $Recycle.Bin (record 36):");
        if let Some(recycle_bin) = tree.get(36) {
            let children = tree.get_children(&recycle_bin.key());
             if children.is_empty() {
                 println!("    No children found!");
             } else {
                 for child in children.iter().take(20) {
                     println!("    - Record {}: '{}' (parent: {})",
                         child.record_number, child.name, child.parent_record_number);
                 }
             }
         }
     }

    // Test OpenFileById for matching files
    println!("\n=== OpenFileById Test ===");
    let results = tree.search(pattern, 10);
    for result in &results {
        if let Some(node) = tree.get(result.record_number) {
            println!("  Testing record {} (FRN 0x{:016X}):",
                node.record_number, node.file_reference_number);
            println!("    MFT data: size={}, mod_time={}", node.file_size, node.modification_time);

            // Try to get metadata via OpenFileById
            match tree.refresh_single_metadata(&node.key()) {
                Some((size, mod_time)) => {
                    println!("    OpenFileById: size={}, mod_time={}", size, mod_time);
                }
                None => {
                    println!("    OpenFileById: FAILED");
                    // Try with full FRN using correct volume handle
                    use emfit::ntfs::{open_volume_for_file_id, get_file_metadata_by_id};
                    if let Ok(vol_handle) = open_volume_for_file_id(drive) {
                        match get_file_metadata_by_id(&vol_handle, node.file_reference_number) {
                            Ok(meta) => println!("    Direct FRN call: size={}", meta.file_size),
                            Err(e) => println!("    Direct FRN call error: {}", e),
                        }
                    }
                }
            }
        }
    }

    Ok(())
}

/// Read a specific MFT record directly
fn cmd_read_mft(drive: char, record_num: u64) -> emfit::Result<()> {
    use emfit::ntfs::{open_volume, MftParser};
    use emfit::ntfs::winapi::get_ntfs_volume_data;

    println!(
        "{} Reading MFT record {} from {}:",
        style("→").cyan().bold(),
        style(record_num).yellow(),
        drive.to_ascii_uppercase()
    );

    let handle = open_volume(drive)?;
    let volume_data = get_ntfs_volume_data(&handle)?;
    
    println!("  Volume data:");
    println!("    Bytes per sector: {}", volume_data.bytes_per_sector);
    println!("    Bytes per cluster: {}", volume_data.bytes_per_cluster);
    println!("    Bytes per MFT record: {}", volume_data.bytes_per_file_record_segment);
    println!("    Total MFT records (est): {}", volume_data.estimated_mft_records());

    let mut parser = MftParser::new(handle, volume_data.clone())?;
    parser.load_mft_extents(drive)?;

    let extents = parser.extents();
    println!("\n  MFT Extents: {} (fragmented: {})", extents.len(), extents.len() > 1);
    if !extents.is_empty() {
        let record_size = volume_data.bytes_per_file_record_segment as u64;
        let cluster_size = volume_data.bytes_per_cluster as u64;
        let records_per_cluster = cluster_size / record_size;
        
        println!("  Records per cluster: {}", records_per_cluster);
        println!("  Extent details:");
        let mut total_records = 0u64;
        for (i, ext) in extents.iter().enumerate() {
            let records_in_extent = ext.cluster_count * records_per_cluster;
            let start_record = total_records;
            let end_record = start_record + records_in_extent - 1;
            println!("    [{}] VCN: {}, LCN: {}, Clusters: {} -> Records {}-{}", 
                i, ext.vcn, ext.lcn, ext.cluster_count, start_record, end_record);
            total_records += records_in_extent;
        }
        println!("  Total records covered by extents: {}", total_records);
        
        // Check if record_num falls within extents
        let target_vcn = record_num * record_size / cluster_size;
        println!("\n  Target record {} is at VCN {}", record_num, target_vcn);
        
        let mut found = false;
        for ext in extents.iter() {
            if target_vcn >= ext.vcn && target_vcn < ext.vcn + ext.cluster_count {
                println!("  -> Falls within extent VCN {}-{}", ext.vcn, ext.vcn + ext.cluster_count - 1);
                found = true;
                break;
            }
        }
        if !found {
            println!("  -> WARNING: Target VCN {} is NOT within any extent!", target_vcn);
        }
    }
    
    println!("\n  Reading record {}...", record_num);
    
    match parser.read_record(record_num) {
        Ok(mut data) => {
            println!("  Raw data read successfully ({} bytes)", data.len());
            
            // Check the signature
            let sig = &data[0..4];
            println!("  Signature: {:02X} {:02X} {:02X} {:02X} (expect 46 49 4C 45 = 'FILE')", 
                sig[0], sig[1], sig[2], sig[3]);
            
            // Parse the record
            match parser.parse_record(record_num, &mut data) {
                Ok(entry) => {
                    println!("\n  Parsed MFT Record:");
                    println!("    Name: '{}'", entry.name);
                    println!("    Record Number: {}", entry.record_number);
                    println!("    Parent Record: {}", entry.parent_record_number);
                    println!("    Is Directory: {}", entry.is_directory);
                    println!("    Is Valid (in use): {}", entry.is_valid);
                    println!("    Is Complete: {}", entry.is_complete);
                    println!("    File Size: {}", entry.file_size);
                    println!("    Attributes: 0x{:08X}", entry.attributes);
                    println!("    Hard Link Count: {}", entry.hard_link_count);
                    
                    if !entry.is_valid {
                        println!("\n  WARNING: Record is marked as NOT IN USE!");
                    }
                }
                Err(e) => {
                    println!("  Failed to parse record: {}", e);
                }
            }
        }
        Err(e) => {
            println!("  Failed to read record: {}", e);
        }
    }

    Ok(())
}

/// Debug: count raw USN enumeration results
fn cmd_usn_count(drive: char) -> emfit::Result<()> {
    use emfit::ntfs::{open_volume, UsnScanner};

    println!(
        "{} Counting raw USN enumeration for {}:",
        style("→").cyan().bold(),
        drive.to_ascii_uppercase()
    );

    let handle = open_volume(drive)?;
    let mut scanner = UsnScanner::new(handle);
    scanner.initialize()?;

    let mut file_count = 0u64;
    let mut dir_count = 0u64;
    let mut max_frn = 0u64;

    println!("  Enumerating...");

    scanner.enumerate_all(|entry| {
        if entry.is_directory {
            dir_count += 1;
        } else {
            file_count += 1;
        }
        if entry.record_number > max_frn {
            max_frn = entry.record_number;
        }
    })?;

    println!("\n  Results:");
    println!("    Files: {}", file_count);
    println!("    Directories: {}", dir_count);
    println!("    Total: {}", file_count + dir_count);
    println!("    Max FRN seen: {}", max_frn);

    Ok(())
}
