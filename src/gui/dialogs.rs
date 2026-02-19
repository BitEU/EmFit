use eframe::egui;

// ============================================================================
// Date / Size filter types (mirrored from TUI menu)
// ============================================================================

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
    pub fn all() -> &'static [DateFilterMode] {
        &[
            DateFilterMode::None,
            DateFilterMode::After,
            DateFilterMode::Before,
            DateFilterMode::Between,
        ]
    }
}

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
    pub fn all() -> &'static [SizeFilterMode] {
        &[
            SizeFilterMode::None,
            SizeFilterMode::GreaterThan,
            SizeFilterMode::LessThan,
            SizeFilterMode::Between,
        ]
    }
}

// ============================================================================
// Search filters state
// ============================================================================

#[derive(Clone)]
pub struct SearchFilters {
    pub regex_pattern: String,
    pub date_mode: DateFilterMode,
    pub date_start: String,
    pub date_end: String,
    pub size_mode: SizeFilterMode,
    pub size_value: String,
    pub size_end: String,
    pub extension_filter: String,
}

impl SearchFilters {
    pub fn new() -> Self {
        Self {
            regex_pattern: String::new(),
            date_mode: DateFilterMode::None,
            date_start: String::new(),
            date_end: String::new(),
            size_mode: SizeFilterMode::None,
            size_value: String::new(),
            size_end: String::new(),
            extension_filter: String::new(),
        }
    }

    pub fn clear_all(&mut self) {
        *self = Self::new();
    }

    pub fn has_any_filter(&self) -> bool {
        !self.regex_pattern.is_empty()
            || self.date_mode != DateFilterMode::None
            || self.size_mode != SizeFilterMode::None
            || !self.extension_filter.is_empty()
    }
}

// ============================================================================
// Search filters dialog
// ============================================================================

/// Returns true when the dialog should remain open.
pub fn show_search_filters_dialog(
    ctx: &egui::Context,
    filters: &mut SearchFilters,
    applied: &mut bool,
) -> bool {
    let mut open = true;

    egui::Window::new("Search Filters")
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .default_width(420.0)
        .show(ctx, |ui| {
            ui.spacing_mut().item_spacing.y = 6.0;

            // --- Pattern ---
            ui.heading("Pattern");
            ui.horizontal(|ui| {
                ui.label("Regex:");
                ui.text_edit_singleline(&mut filters.regex_pattern)
                    .on_hover_text("e.g. .*\\.log$");
            });
            ui.separator();

            // --- Date ---
            ui.heading("Date Modified");
            ui.horizontal(|ui| {
                ui.label("Mode:");
                egui::ComboBox::from_id_salt("date_mode")
                    .selected_text(filters.date_mode.label())
                    .show_ui(ui, |ui| {
                        for mode in DateFilterMode::all() {
                            ui.selectable_value(&mut filters.date_mode, *mode, mode.label());
                        }
                    });
            });
            if filters.date_mode != DateFilterMode::None {
                ui.horizontal(|ui| {
                    ui.label("Start:");
                    ui.text_edit_singleline(&mut filters.date_start)
                        .on_hover_text("YYYY-MM-DD");
                });
                if filters.date_mode == DateFilterMode::Between {
                    ui.horizontal(|ui| {
                        ui.label("End:");
                        ui.text_edit_singleline(&mut filters.date_end)
                            .on_hover_text("YYYY-MM-DD");
                    });
                }
            }
            ui.separator();

            // --- Size ---
            ui.heading("File Size");
            ui.horizontal(|ui| {
                ui.label("Mode:");
                egui::ComboBox::from_id_salt("size_mode")
                    .selected_text(filters.size_mode.label())
                    .show_ui(ui, |ui| {
                        for mode in SizeFilterMode::all() {
                            ui.selectable_value(&mut filters.size_mode, *mode, mode.label());
                        }
                    });
            });
            if filters.size_mode != SizeFilterMode::None {
                ui.horizontal(|ui| {
                    ui.label("Value:");
                    ui.text_edit_singleline(&mut filters.size_value)
                        .on_hover_text("e.g. 10MB, 1GB, 500KB");
                });
                if filters.size_mode == SizeFilterMode::Between {
                    ui.horizontal(|ui| {
                        ui.label("End:");
                        ui.text_edit_singleline(&mut filters.size_end)
                            .on_hover_text("e.g. 100MB");
                    });
                }
            }
            ui.separator();

            // --- Extension ---
            ui.heading("Extension");
            ui.horizontal(|ui| {
                ui.label("Extensions:");
                ui.text_edit_singleline(&mut filters.extension_filter)
                    .on_hover_text("e.g. pdf;docx;txt");
            });
            ui.separator();

            // --- Buttons ---
            ui.horizontal(|ui| {
                if ui.button("Apply").clicked() {
                    *applied = true;
                    open = false;
                }
                if ui.button("Clear").clicked() {
                    filters.clear_all();
                }
                if ui.button("Cancel").clicked() {
                    open = false;
                }
            });
        });

    open
}

// ============================================================================
// Confirm dialog
// ============================================================================

/// Returns `Some(true)` for confirmed, `Some(false)` for cancelled, `None` while open.
pub fn show_confirm_dialog(ctx: &egui::Context, message: &str) -> Option<bool> {
    let mut result: Option<bool> = None;
    let mut open = true;

    egui::Window::new("Confirm")
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .open(&mut open)
        .show(ctx, |ui| {
            ui.label(message);
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button("Yes").clicked() {
                    result = Some(true);
                }
                if ui.button("No").clicked() {
                    result = Some(false);
                }
            });
        });

    if !open {
        return Some(false);
    }
    result
}

// ============================================================================
// Rename dialog
// ============================================================================

/// Returns `Some(new_name)` when confirmed, `None` while still editing.
pub fn show_rename_dialog(
    ctx: &egui::Context,
    new_name: &mut String,
    original_name: &str,
) -> Option<Option<String>> {
    let mut result: Option<Option<String>> = None;
    let mut open = true;

    egui::Window::new("Rename")
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .open(&mut open)
        .show(ctx, |ui| {
            ui.label(format!("Renaming: {}", original_name));
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.label("New name:");
                let response = ui.text_edit_singleline(new_name);
                if response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    result = Some(Some(new_name.clone()));
                }
            });
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                if ui.button("Rename").clicked() {
                    result = Some(Some(new_name.clone()));
                }
                if ui.button("Cancel").clicked() {
                    result = Some(None);
                }
            });
        });

    if !open {
        return Some(None);
    }
    result
}

// ============================================================================
// Info / About dialog
// ============================================================================

pub fn show_info_dialog(ctx: &egui::Context, title: &str, lines: &[String]) -> bool {
    let mut open = true;

    egui::Window::new(title)
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .show(ctx, |ui| {
            for line in lines {
                if line.is_empty() {
                    ui.add_space(4.0);
                } else {
                    ui.label(line);
                }
            }
            ui.add_space(8.0);
            if ui.button("OK").clicked() {
                open = false;
            }
        });

    open
}

// ============================================================================
// Size / date parsing helpers (same as TUI menu)
// ============================================================================

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
        return s.trim().parse::<u64>().ok();
    };
    num_str
        .trim()
        .parse::<f64>()
        .ok()
        .map(|n| (n * unit as f64) as u64)
}

pub fn parse_date_to_filetime(s: &str) -> Option<u64> {
    let parts: Vec<&str> = s.trim().split('-').collect();
    if parts.len() != 3 {
        return None;
    }
    let year: i32 = parts[0].parse().ok()?;
    let month: u32 = parts[1].parse().ok()?;
    let day: u32 = parts[2].parse().ok()?;

    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }

    use chrono::{NaiveDate, NaiveDateTime, NaiveTime};
    let date = NaiveDate::from_ymd_opt(year, month, day)?;
    let datetime = NaiveDateTime::new(date, NaiveTime::from_hms_opt(0, 0, 0)?);
    let unix_secs = datetime.and_utc().timestamp();
    let filetime = (unix_secs + 11644473600) as u64 * 10_000_000;
    Some(filetime)
}

// ============================================================================
// OS helpers (clipboard, open, properties)
// ============================================================================

pub fn copy_to_clipboard(text: &str) {
    if let Ok(mut clipboard) = arboard::Clipboard::new() {
        let _ = clipboard.set_text(text.to_owned());
    }
}

pub fn open_file(path: &str) {
    let _ = open::that(path);
}

pub fn open_in_explorer(path: &str) {
    use std::os::windows::process::CommandExt;
    let _ = std::process::Command::new("explorer.exe")
        .raw_arg(format!("/select,\"{}\"", path))
        .creation_flags(0x08000000)
        .spawn();
}

pub fn show_properties(path: &str) {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::Shell::{ShellExecuteExW, SHELLEXECUTEINFOW, SEE_MASK_INVOKEIDLIST};
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
        nShow: 5,
        ..Default::default()
    };

    unsafe {
        let _ = ShellExecuteExW(&mut sei);
    }
}
