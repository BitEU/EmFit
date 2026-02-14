use ratatui::style::Color;

pub fn color_for_extension(ext: &str) -> Color {
    match ext {
        "java" | "jar" | "class" => Color::Red,
        "py" | "pyc" | "pyw" | "pyx" => Color::Yellow,
        "exe" | "dll" | "bat" | "cmd" | "msi" | "com" | "scr" | "sys" | "drv" => Color::Cyan,
        "xml" | "html" | "htm" | "json" | "yaml" | "yml" | "toml" | "css" => Color::Green,
        "svg" | "png" | "jpg" | "jpeg" | "gif" | "bmp" | "ico" | "webp" | "tiff" => {
            Color::Magenta
        }
        "rs" | "go" | "c" | "cpp" | "h" | "hpp" | "cs" | "js" | "ts" => Color::LightBlue,
        "zip" | "tar" | "gz" | "bz2" | "7z" | "rar" | "xz" => Color::LightRed,
        "pdf" | "doc" | "docx" | "xls" | "xlsx" | "ppt" | "pptx" => Color::LightYellow,
        "md" | "txt" | "log" | "ini" | "cfg" | "conf" => Color::Gray,
        _ => Color::White,
    }
}

pub fn icon_for_entry(is_directory: bool, ext: &str) -> &'static str {
    if is_directory {
        return "\u{1F4C1}"; // folder
    }
    match ext {
        "exe" | "msi" | "com" => "\u{2699}\u{FE0F}",    // gear
        "dll" | "sys" | "drv" => "\u{1F527}",            // wrench
        "bat" | "cmd" => "\u{2699}\u{FE0F}",             // gear
        "txt" | "log" => "\u{1F4DD}",                    // memo
        "md" => "\u{1F4C3}",                             // page with curl
        "pdf" => "\u{1F4D5}",                            // book
        "zip" | "tar" | "gz" | "7z" | "rar" | "xz" => "\u{1F4E6}", // package
        "png" | "jpg" | "jpeg" | "gif" | "bmp" | "webp" | "tiff" => "\u{1F5BC}\u{FE0F}", // picture
        "svg" => "\u{1F309}",                            // bridge at night (for vector)
        "mp3" | "wav" | "flac" | "ogg" => "\u{1F3B5}",  // music
        "mp4" | "avi" | "mkv" | "mov" => "\u{1F3AC}",   // movie
        "py" => "\u{1F40D}",                             // snake
        "rs" => "\u{1F980}",                             // crab
        "java" | "jar" => "\u{2615}",                    // coffee
        "js" | "ts" | "html" | "xml" | "json" => "\u{1F310}", // globe
        "c" | "cpp" | "h" | "hpp" => "\u{1F4C4}",       // page
        "ini" | "cfg" | "conf" | "toml" | "yaml" | "yml" => "\u{2699}\u{FE0F}", // gear
        _ => "\u{1F4C4}",                               // page
    }
}

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
