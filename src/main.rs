//! RustyScan CLI
//!
//! Command-line interface for the RustyScan file scanner.
//! Provides both command-line and interactive search modes.

use clap::{Parser, Subcommand};
use console::{style, Term};
use indicatif::HumanDuration;
use rustyscan::{
    format_size, FileTree,
    MultiVolumeScanner, ScanConfig, VolumeScanner,
};
use std::io::Write;
use std::time::Instant;

/// RustyScan - Ultra-fast NTFS file scanner
///
/// Combines USN Journal enumeration with direct MFT reading
/// for instant, accurate file system scanning.
#[derive(Parser)]
#[command(name = "rustyscan")]
#[command(author = "RustyScan Contributors")]
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

        /// Use USN Journal for fast enumeration
        #[arg(long, default_value = "true")]
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

        /// Search pattern
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
}

fn main() {
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
) -> rustyscan::Result<()> {
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
fn cmd_search(drive: char, pattern: &str, max_results: usize) -> rustyscan::Result<()> {
    println!(
        "{} Searching for '{}' on {}:",
        style("→").cyan().bold(),
        style(pattern).yellow(),
        drive.to_ascii_uppercase()
    );

    let start = Instant::now();

    // Use all available data sources for complete results
    let config = ScanConfig {
        use_usn: true,
        use_mft: true, // Always use MFT for complete file enumeration
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
fn cmd_largest(drive: char, count: usize, show_dirs: bool) -> rustyscan::Result<()> {
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
fn cmd_tree_size(drive: char, path: Option<&str>, depth: usize) -> rustyscan::Result<()> {
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

fn print_tree_node(tree: &FileTree, node: &rustyscan::TreeNode, indent: usize, max_depth: usize) {
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
        let mut children = tree.get_children(node.record_number);
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
fn cmd_volumes() -> rustyscan::Result<()> {
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
            if let Ok(handle) = rustyscan::ntfs::open_volume(letter) {
                if let Ok(data) = rustyscan::ntfs::winapi::get_ntfs_volume_data(&handle) {
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
fn cmd_monitor(drive: char) -> rustyscan::Result<()> {
    use rustyscan::ChangeMonitor;

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
fn cmd_export(drive: char, output: &str, format: &str) -> rustyscan::Result<()> {
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
