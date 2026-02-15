use std::io::Write;
use std::os::windows::process::CommandExt;
use std::process::{Command, Stdio};

/// The type of action available in the actions menu
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActionKind {
    Open,
    OpenInExplorer,
    Properties,
    Delete,
    Rename,
    CopyPath,
}

/// Actions popup menu state
pub struct ActionsMenu {
    pub items: Vec<(&'static str, ActionKind)>,
    pub selected: usize,
}

impl ActionsMenu {
    pub fn new() -> Self {
        Self {
            items: vec![
                ("Open", ActionKind::Open),
                ("Open in Explorer", ActionKind::OpenInExplorer),
                ("Properties", ActionKind::Properties),
                ("Delete", ActionKind::Delete),
                ("Rename", ActionKind::Rename),
                ("Copy Path", ActionKind::CopyPath),
            ],
            selected: 0,
        }
    }

    pub fn move_up(&mut self) {
        if self.selected > 0 {
            self.selected -= 1;
        }
    }

    pub fn move_down(&mut self) {
        if self.selected < self.items.len() - 1 {
            self.selected += 1;
        }
    }

    pub fn selected_action(&self) -> ActionKind {
        self.items[self.selected].1
    }
}

/// Confirmation dialog for dangerous operations
pub struct ConfirmDialog {
    pub message: String,
    pub confirm_selected: bool,
    pub action: ActionKind,
}

impl ConfirmDialog {
    pub fn new(message: String, action: ActionKind) -> Self {
        Self {
            message,
            confirm_selected: false,
            action,
        }
    }
}

/// Rename dialog with text input
pub struct RenameDialog {
    pub original_name: String,
    pub new_name: String,
    pub cursor_pos: usize,
    pub full_path: String,
}

impl RenameDialog {
    pub fn new(name: String, full_path: String) -> Self {
        let cursor_pos = name.len();
        Self {
            original_name: name.clone(),
            new_name: name,
            cursor_pos,
            full_path,
        }
    }
}

/// Date filter mode for search filters
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DateFilterMode {
    None,
    After,
    Before,
    Between,
}

impl DateFilterMode {
    pub fn label(&self) -> &'static str {
        match self {
            DateFilterMode::None => "None",
            DateFilterMode::After => "After",
            DateFilterMode::Before => "Before",
            DateFilterMode::Between => "Between",
        }
    }

    pub fn next(&self) -> Self {
        match self {
            DateFilterMode::None => DateFilterMode::After,
            DateFilterMode::After => DateFilterMode::Before,
            DateFilterMode::Before => DateFilterMode::Between,
            DateFilterMode::Between => DateFilterMode::None,
        }
    }

    pub fn prev(&self) -> Self {
        match self {
            DateFilterMode::None => DateFilterMode::Between,
            DateFilterMode::After => DateFilterMode::None,
            DateFilterMode::Before => DateFilterMode::After,
            DateFilterMode::Between => DateFilterMode::Before,
        }
    }
}

/// Size filter mode for search filters
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SizeFilterMode {
    None,
    GreaterThan,
    LessThan,
    Between,
}

impl SizeFilterMode {
    pub fn label(&self) -> &'static str {
        match self {
            SizeFilterMode::None => "None",
            SizeFilterMode::GreaterThan => ">",
            SizeFilterMode::LessThan => "<",
            SizeFilterMode::Between => "Between",
        }
    }

    pub fn next(&self) -> Self {
        match self {
            SizeFilterMode::None => SizeFilterMode::GreaterThan,
            SizeFilterMode::GreaterThan => SizeFilterMode::LessThan,
            SizeFilterMode::LessThan => SizeFilterMode::Between,
            SizeFilterMode::Between => SizeFilterMode::None,
        }
    }

    pub fn prev(&self) -> Self {
        match self {
            SizeFilterMode::None => SizeFilterMode::Between,
            SizeFilterMode::GreaterThan => SizeFilterMode::None,
            SizeFilterMode::LessThan => SizeFilterMode::GreaterThan,
            SizeFilterMode::Between => SizeFilterMode::LessThan,
        }
    }
}

/// Which field is focused in the search filters dialog
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchFilterField {
    Regex,
    DateMode,
    DateStart,
    DateEnd,
    SizeMode,
    SizeValue,
    SizeEnd,
    Extension,
    Apply,
    Clear,
    Cancel,
}

impl SearchFilterField {
    pub fn next(&self) -> Self {
        match self {
            SearchFilterField::Regex => SearchFilterField::DateMode,
            SearchFilterField::DateMode => SearchFilterField::DateStart,
            SearchFilterField::DateStart => SearchFilterField::DateEnd,
            SearchFilterField::DateEnd => SearchFilterField::SizeMode,
            SearchFilterField::SizeMode => SearchFilterField::SizeValue,
            SearchFilterField::SizeValue => SearchFilterField::SizeEnd,
            SearchFilterField::SizeEnd => SearchFilterField::Extension,
            SearchFilterField::Extension => SearchFilterField::Apply,
            SearchFilterField::Apply => SearchFilterField::Clear,
            SearchFilterField::Clear => SearchFilterField::Cancel,
            SearchFilterField::Cancel => SearchFilterField::Regex,
        }
    }

    pub fn prev(&self) -> Self {
        match self {
            SearchFilterField::Regex => SearchFilterField::Cancel,
            SearchFilterField::DateMode => SearchFilterField::Regex,
            SearchFilterField::DateStart => SearchFilterField::DateMode,
            SearchFilterField::DateEnd => SearchFilterField::DateStart,
            SearchFilterField::SizeMode => SearchFilterField::DateEnd,
            SearchFilterField::SizeValue => SearchFilterField::SizeMode,
            SearchFilterField::SizeEnd => SearchFilterField::SizeValue,
            SearchFilterField::Extension => SearchFilterField::SizeEnd,
            SearchFilterField::Apply => SearchFilterField::Extension,
            SearchFilterField::Clear => SearchFilterField::Apply,
            SearchFilterField::Cancel => SearchFilterField::Clear,
        }
    }

    pub fn is_text_input(&self) -> bool {
        matches!(
            self,
            SearchFilterField::Regex
                | SearchFilterField::DateStart
                | SearchFilterField::DateEnd
                | SearchFilterField::SizeValue
                | SearchFilterField::SizeEnd
                | SearchFilterField::Extension
        )
    }

    pub fn is_mode_selector(&self) -> bool {
        matches!(
            self,
            SearchFilterField::DateMode | SearchFilterField::SizeMode
        )
    }
}

/// Search filters dialog state
pub struct SearchFiltersMenu {
    pub focused_field: SearchFilterField,
    pub regex_pattern: String,
    pub regex_cursor: usize,
    pub date_mode: DateFilterMode,
    pub date_start: String,
    pub date_start_cursor: usize,
    pub date_end: String,
    pub date_end_cursor: usize,
    pub size_mode: SizeFilterMode,
    pub size_value: String,
    pub size_value_cursor: usize,
    pub size_end: String,
    pub size_end_cursor: usize,
    pub extension_filter: String,
    pub extension_cursor: usize,
}

impl SearchFiltersMenu {
    pub fn new() -> Self {
        Self {
            focused_field: SearchFilterField::Regex,
            regex_pattern: String::new(),
            regex_cursor: 0,
            date_mode: DateFilterMode::None,
            date_start: String::new(),
            date_start_cursor: 0,
            date_end: String::new(),
            date_end_cursor: 0,
            size_mode: SizeFilterMode::None,
            size_value: String::new(),
            size_value_cursor: 0,
            size_end: String::new(),
            size_end_cursor: 0,
            extension_filter: String::new(),
            extension_cursor: 0,
        }
    }

    /// Get the current text input and cursor for the focused field
    pub fn current_text_mut(&mut self) -> Option<(&mut String, &mut usize)> {
        match self.focused_field {
            SearchFilterField::Regex => Some((&mut self.regex_pattern, &mut self.regex_cursor)),
            SearchFilterField::DateStart => Some((&mut self.date_start, &mut self.date_start_cursor)),
            SearchFilterField::DateEnd => Some((&mut self.date_end, &mut self.date_end_cursor)),
            SearchFilterField::SizeValue => Some((&mut self.size_value, &mut self.size_value_cursor)),
            SearchFilterField::SizeEnd => Some((&mut self.size_end, &mut self.size_end_cursor)),
            SearchFilterField::Extension => Some((&mut self.extension_filter, &mut self.extension_cursor)),
            _ => None,
        }
    }

    pub fn clear_all(&mut self) {
        self.regex_pattern.clear();
        self.regex_cursor = 0;
        self.date_mode = DateFilterMode::None;
        self.date_start.clear();
        self.date_start_cursor = 0;
        self.date_end.clear();
        self.date_end_cursor = 0;
        self.size_mode = SizeFilterMode::None;
        self.size_value.clear();
        self.size_value_cursor = 0;
        self.size_end.clear();
        self.size_end_cursor = 0;
        self.extension_filter.clear();
        self.extension_cursor = 0;
    }

    pub fn has_any_filter(&self) -> bool {
        !self.regex_pattern.is_empty()
            || self.date_mode != DateFilterMode::None
            || self.size_mode != SizeFilterMode::None
            || !self.extension_filter.is_empty()
    }
}

/// Info dialog for displaying multi-line information
pub struct InfoDialog {
    pub title: String,
    pub lines: Vec<String>,
}

impl InfoDialog {
    pub fn new(title: String, lines: Vec<String>) -> Self {
        Self { title, lines }
    }
}

/// Which menu/dialog is currently active
pub enum ActiveMenu {
    None,
    Actions(ActionsMenu),
    Confirm(ConfirmDialog),
    Rename(RenameDialog),
    SearchFilters(SearchFiltersMenu),
    Info(InfoDialog),
}

/// Copy text to clipboard using clip.exe on Windows
pub fn copy_to_clipboard(text: &str) {
    if let Ok(mut child) = Command::new("clip")
        .stdin(Stdio::piped())
        .spawn()
    {
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(text.as_bytes());
        }
        let _ = child.wait();
    }
}

/// Open a file with its default application
pub fn open_file(path: &str) {
    let _ = Command::new("cmd")
        .args(["/c", "start", "", path])
        .creation_flags(0x08000000) // CREATE_NO_WINDOW
        .spawn();
}

/// Open Windows Explorer with the file selected
pub fn open_in_explorer(path: &str) {
    // Use raw_arg to avoid Rust's automatic argument quoting, which breaks
    // paths containing spaces or special characters like parentheses.
    // explorer.exe expects: /select,"C:\path with spaces\file"
    let _ = Command::new("explorer.exe")
        .raw_arg(format!("/select,\"{}\"", path))
        .creation_flags(0x08000000) // CREATE_NO_WINDOW
        .spawn();
}

/// Show Windows file properties dialog
pub fn show_properties(path: &str) {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    use windows::Win32::UI::Shell::{ShellExecuteExW, SHELLEXECUTEINFOW, SEE_MASK_INVOKEIDLIST};
    use windows::Win32::Foundation::HWND;
    use windows::core::PCWSTR;

    let verb: Vec<u16> = OsStr::new("properties")
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let file: Vec<u16> = OsStr::new(path)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();

    let mut sei = SHELLEXECUTEINFOW {
        cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_INVOKEIDLIST,
        hwnd: HWND::default(),
        lpVerb: PCWSTR(verb.as_ptr()),
        lpFile: PCWSTR(file.as_ptr()),
        lpParameters: PCWSTR::null(),
        lpDirectory: PCWSTR::null(),
        nShow: 5, // SW_SHOW
        ..Default::default()
    };

    unsafe {
        let _ = ShellExecuteExW(&mut sei);
    }
}

/// Parse a size string like "10 MB", "500 KB", "1 GB" into bytes
pub fn parse_size_str(s: &str) -> Option<u64> {
    let s = s.trim().to_uppercase();
    let (num_str, unit) = if s.ends_with("TB") {
        (&s[..s.len() - 2], 1u64 << 40)
    } else if s.ends_with("GB") {
        (&s[..s.len() - 2], 1u64 << 30)
    } else if s.ends_with("MB") {
        (&s[..s.len() - 2], 1u64 << 20)
    } else if s.ends_with("KB") {
        (&s[..s.len() - 2], 1u64 << 10)
    } else if s.ends_with("B") {
        (&s[..s.len() - 1], 1u64)
    } else {
        // Try parsing as pure number (bytes)
        return s.trim().parse::<u64>().ok();
    };

    num_str.trim().parse::<f64>().ok().map(|n| (n * unit as f64) as u64)
}

/// Parse a date string like "2025-01-01" into a Windows FILETIME value
pub fn parse_date_to_filetime(s: &str) -> Option<u64> {
    let parts: Vec<&str> = s.trim().split('-').collect();
    if parts.len() != 3 {
        return None;
    }
    let year: i32 = parts[0].parse().ok()?;
    let month: u32 = parts[1].parse().ok()?;
    let day: u32 = parts[2].parse().ok()?;

    if month < 1 || month > 12 || day < 1 || day > 31 {
        return None;
    }

    // Convert to FILETIME: 100-nanosecond intervals since January 1, 1601
    // Use chrono for reliable conversion
    use chrono::{NaiveDate, NaiveDateTime, NaiveTime};
    let date = NaiveDate::from_ymd_opt(year, month, day)?;
    let datetime = NaiveDateTime::new(date, NaiveTime::from_hms_opt(0, 0, 0)?);

    // Epoch difference: Jan 1, 1601 to Jan 1, 1970 = 11644473600 seconds
    let unix_secs = datetime.and_utc().timestamp();
    let filetime = (unix_secs + 11644473600) as u64 * 10_000_000;
    Some(filetime)
}
