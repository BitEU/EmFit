use eframe::egui::Color32;

/// Map a file extension to an egui colour (matches the TUI palette).
pub fn color_for_extension(ext: &str) -> Color32 {
    match ext {
        "java" | "jar" | "class" => Color32::from_rgb(255, 80, 80),
        "py" | "pyc" | "pyw" | "pyx" => Color32::from_rgb(255, 220, 80),
        "exe" | "dll" | "bat" | "cmd" | "msi" | "com" | "scr" | "sys" | "drv" => {
            Color32::from_rgb(80, 220, 255)
        }
        "xml" | "html" | "htm" | "json" | "yaml" | "yml" | "toml" | "css" => {
            Color32::from_rgb(80, 200, 80)
        }
        "svg" | "png" | "jpg" | "jpeg" | "gif" | "bmp" | "ico" | "webp" | "tiff" => {
            Color32::from_rgb(220, 80, 220)
        }
        "rs" | "go" | "c" | "cpp" | "h" | "hpp" | "cs" | "js" | "ts" => {
            Color32::from_rgb(100, 180, 255)
        }
        "zip" | "tar" | "gz" | "bz2" | "7z" | "rar" | "xz" => Color32::from_rgb(255, 120, 120),
        "pdf" | "doc" | "docx" | "xls" | "xlsx" | "ppt" | "pptx" => {
            Color32::from_rgb(255, 240, 120)
        }
        "md" | "txt" | "log" | "ini" | "cfg" | "conf" => Color32::from_rgb(180, 180, 180),
        _ => Color32::from_rgb(220, 220, 220),
    }
}

/// Get an icon string for a file entry.
pub fn icon_for_entry(is_directory: bool, ext: &str) -> &'static str {
    if is_directory {
        return "\u{1F4C1}"; // folder
    }
    match ext {
        "exe" | "msi" | "com" => "\u{2699}\u{FE0F}",
        "dll" | "sys" | "drv" => "\u{1F527}",
        "bat" | "cmd" => "\u{2699}\u{FE0F}",
        "txt" | "log" => "\u{1F4DD}",
        "md" => "\u{1F4C3}",
        "pdf" => "\u{1F4D5}",
        "zip" | "tar" | "gz" | "7z" | "rar" | "xz" => "\u{1F4E6}",
        "png" | "jpg" | "jpeg" | "gif" | "bmp" | "webp" | "tiff" => "\u{1F5BC}\u{FE0F}",
        "svg" => "\u{1F309}",
        "mp3" | "wav" | "flac" | "ogg" => "\u{1F3B5}",
        "mp4" | "avi" | "mkv" | "mov" => "\u{1F3AC}",
        "py" => "\u{1F40D}",
        "rs" => "\u{1F980}",
        "java" | "jar" => "\u{2615}",
        "js" | "ts" | "html" | "xml" | "json" => "\u{1F310}",
        "c" | "cpp" | "h" | "hpp" => "\u{1F4C4}",
        "ini" | "cfg" | "conf" | "toml" | "yaml" | "yml" => "\u{2699}\u{FE0F}",
        _ => "\u{1F4C4}",
    }
}

/// Human-readable type label.
pub fn type_label(is_directory: bool, ext: &str) -> &'static str {
    if is_directory {
        return "Folder";
    }
    match ext {
        "exe" => "Application",
        "dll" => "DLL Library",
        "sys" | "drv" => "System File",
        "bat" | "cmd" => "Batch File",
        "msi" => "Installer",
        "com" | "scr" => "Executable",
        "txt" => "Text File",
        "log" => "Log File",
        "md" => "Markdown",
        "pdf" => "PDF Document",
        "doc" | "docx" => "Word Document",
        "xls" | "xlsx" => "Spreadsheet",
        "ppt" | "pptx" => "Presentation",
        "png" => "PNG Image",
        "jpg" | "jpeg" => "JPEG Image",
        "gif" => "GIF Image",
        "bmp" => "Bitmap Image",
        "svg" => "SVG Image",
        "webp" => "WebP Image",
        "ico" => "Icon",
        "mp3" => "MP3 Audio",
        "wav" => "WAV Audio",
        "flac" => "FLAC Audio",
        "ogg" => "OGG Audio",
        "mp4" => "MP4 Video",
        "avi" => "AVI Video",
        "mkv" => "MKV Video",
        "mov" => "MOV Video",
        "zip" => "ZIP Archive",
        "7z" => "7-Zip Archive",
        "rar" => "RAR Archive",
        "tar" => "TAR Archive",
        "gz" => "GZip Archive",
        "xz" => "XZ Archive",
        "rs" => "Rust Source",
        "py" => "Python Script",
        "java" => "Java Source",
        "jar" => "Java Archive",
        "class" => "Java Class",
        "c" => "C Source",
        "cpp" => "C++ Source",
        "h" | "hpp" => "Header File",
        "cs" => "C# Source",
        "go" => "Go Source",
        "js" => "JavaScript",
        "ts" => "TypeScript",
        "json" => "JSON",
        "xml" => "XML Document",
        "html" | "htm" => "HTML",
        "css" => "Stylesheet",
        "yaml" | "yml" => "YAML",
        "toml" => "TOML",
        "ini" | "cfg" | "conf" => "Configuration",
        "" => "File",
        _ => "File",
    }
}

/// Treemap leaf colour by extension (richer palette).
pub fn leaf_color(name: &str, is_directory: bool, index: usize) -> Color32 {
    if is_directory {
        let palette = [
            Color32::from_rgb(40, 105, 135),
            Color32::from_rgb(50, 115, 120),
            Color32::from_rgb(60, 95, 145),
            Color32::from_rgb(45, 125, 110),
            Color32::from_rgb(55, 108, 128),
        ];
        return palette[index % palette.len()];
    }

    let ext = name.rsplit('.').next().unwrap_or("").to_lowercase();
    match ext.as_str() {
        "exe" | "com" | "scr" => Color32::from_rgb(200, 55, 55),
        "dll" | "sys" | "drv" | "ocx" => Color32::from_rgb(175, 65, 65),
        "msi" | "bat" | "cmd" | "ps1" => Color32::from_rgb(185, 80, 50),
        "zip" | "rar" | "7z" | "gz" | "tar" | "xz" | "bz2" | "cab" | "iso" => {
            Color32::from_rgb(200, 175, 35)
        }
        "mp4" | "mkv" | "avi" | "mov" | "wmv" | "flv" | "webm" | "m4v" | "ts" => {
            Color32::from_rgb(160, 45, 195)
        }
        "mp3" | "wav" | "flac" | "ogg" | "aac" | "wma" | "m4a" | "opus" => {
            Color32::from_rgb(35, 175, 135)
        }
        "jpg" | "jpeg" | "png" | "gif" | "bmp" | "tiff" | "webp" | "ico" | "svg" | "psd"
        | "raw" | "cr2" | "nef" | "dng" => Color32::from_rgb(195, 125, 35),
        "pdf" => Color32::from_rgb(200, 50, 50),
        "doc" | "docx" | "odt" | "rtf" => Color32::from_rgb(55, 130, 200),
        "xls" | "xlsx" | "ods" | "csv" => Color32::from_rgb(45, 165, 65),
        "ppt" | "pptx" | "odp" => Color32::from_rgb(200, 105, 35),
        "txt" | "log" | "md" | "cfg" | "ini" | "conf" | "yml" | "yaml" | "toml" => {
            Color32::from_rgb(120, 120, 120)
        }
        "rs" | "go" | "c" | "cpp" | "h" | "hpp" | "cs" => Color32::from_rgb(75, 150, 220),
        "py" | "pyw" => Color32::from_rgb(55, 140, 185),
        "js" | "jsx" | "tsx" => Color32::from_rgb(215, 195, 45),
        "java" | "kt" | "scala" => Color32::from_rgb(170, 100, 55),
        "html" | "htm" | "css" | "scss" => Color32::from_rgb(215, 75, 45),
        "json" | "xml" | "sql" => Color32::from_rgb(140, 160, 55),
        "pak" | "rpf" | "bdt" | "pack" | "assets" | "resource" | "forge" | "wad" => {
            Color32::from_rgb(185, 75, 165)
        }
        "vdi" | "vmdk" | "vhd" | "vhdx" | "qcow2" | "img" | "bin" | "001" => {
            Color32::from_rgb(100, 65, 165)
        }
        "db" | "sqlite" | "mdf" | "ldf" | "bak" => Color32::from_rgb(135, 115, 45),
        "ttf" | "otf" | "woff" | "woff2" => Color32::from_rgb(160, 140, 100),
        _ => {
            let h = ext
                .bytes()
                .fold(0u32, |a, b| a.wrapping_mul(31).wrapping_add(b as u32));
            hsl_to_rgb((h % 360) as f64, 0.45, 0.38)
        }
    }
}

/// Depth-cycling border colour for directory containers.
pub fn depth_border_color(depth: usize) -> Color32 {
    match depth % 7 {
        0 => Color32::from_rgb(0, 190, 230),
        1 => Color32::from_rgb(90, 200, 70),
        2 => Color32::from_rgb(230, 190, 40),
        3 => Color32::from_rgb(210, 90, 200),
        4 => Color32::from_rgb(70, 140, 240),
        5 => Color32::from_rgb(230, 130, 50),
        _ => Color32::from_rgb(130, 210, 180),
    }
}

/// Dark tinted background inside directory containers.
pub fn depth_bg_color(depth: usize) -> Color32 {
    match depth % 7 {
        0 => Color32::from_rgb(8, 22, 28),
        1 => Color32::from_rgb(12, 24, 10),
        2 => Color32::from_rgb(26, 22, 8),
        3 => Color32::from_rgb(24, 12, 24),
        4 => Color32::from_rgb(10, 16, 30),
        5 => Color32::from_rgb(26, 16, 8),
        _ => Color32::from_rgb(12, 24, 20),
    }
}

fn hsl_to_rgb(h: f64, s: f64, l: f64) -> Color32 {
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let x = c * (1.0 - ((h / 60.0) % 2.0 - 1.0).abs());
    let m = l - c / 2.0;
    let (r, g, b) = match h as u32 {
        0..=59 => (c, x, 0.0),
        60..=119 => (x, c, 0.0),
        120..=179 => (0.0, c, x),
        180..=239 => (0.0, x, c),
        240..=299 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    Color32::from_rgb(
        ((r + m) * 255.0) as u8,
        ((g + m) * 255.0) as u8,
        ((b + m) * 255.0) as u8,
    )
}
