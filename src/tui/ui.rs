use crate::file_tree::NodeKey;
use crate::tui::app::{App, MenuBarState};
use crate::tui::colors;
use crate::tui::menu::{ActiveMenu, SearchFilterField};
use crate::tui::table::SortColumn;
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Cell, Clear, Paragraph, Row, Table};

pub fn draw(frame: &mut Frame, app: &mut App) {
    let area = frame.area();

    // If treemap is active, draw treemap view instead
    if app.treemap.is_some() {
        // Rebuild layout when terminal size changes so border padding stays correct
        let needs_rebuild = {
            let tm = app.treemap.as_ref().unwrap();
            let tw = area.width.max(10) as f64;
            let th = area.height.saturating_sub(2).max(4) as f64;
            (tw - tm.screen_w).abs() > 0.5 || (th - tm.screen_h).abs() > 0.5
        };
        if needs_rebuild {
            let current_key = app.treemap.as_ref().unwrap().current_key;
            let breadcrumb = app.treemap.as_ref().unwrap().breadcrumb.clone();
            let mut tm = app.treemap.take().unwrap();
            tm.set_screen_size(area.width, area.height);
            if current_key == NodeKey::root() {
                tm.build_from_trees(&app.trees);
            } else if let Some(tree) = app.trees.iter().find(|t| t.get_by_key(&current_key).is_some()) {
                tm.build_from_node(tree, &current_key);
                tm.breadcrumb = breadcrumb;
            }
            app.treemap = Some(tm);
        }
        let tm = app.treemap.as_ref().unwrap();
        crate::tui::treemap::draw_treemap(frame, tm, area);
        return;
    }

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // Menu bar
            Constraint::Length(3), // Search bar
            Constraint::Min(5),   // Table
            Constraint::Length(1), // Status bar
        ])
        .split(area);

    draw_menu_bar_strip(frame, app, chunks[0]);
    draw_search_bar(frame, app, chunks[1]);
    draw_table(frame, app, chunks[2]);
    draw_status_bar(frame, app, chunks[3]);

    // Draw menu overlays
    match &app.active_menu {
        ActiveMenu::None => {}
        ActiveMenu::Actions(actions) => {
            draw_actions_menu(frame, actions, area);
        }
        ActiveMenu::Confirm(confirm) => {
            draw_confirm_dialog(frame, confirm, area);
        }
        ActiveMenu::Rename(rename) => {
            draw_rename_dialog(frame, rename, area);
        }
        ActiveMenu::SearchFilters(filters) => {
            draw_search_filters(frame, filters, area);
        }
        ActiveMenu::Info(info) => {
            draw_info_dialog(frame, info, area);
        }
    }

    // Draw menu bar dropdown if open
    if let Some(ref menu_bar) = app.menu_bar {
        draw_menu_bar_dropdown(frame, menu_bar, area);
    }

    // Show cursor in search bar when focused (and no menu is active)
    if matches!(app.active_menu, ActiveMenu::None) && app.menu_bar.is_none() && app.search.focused {
        // Account for border (1) + space (1) + search icon " \u{1F50D} " (approx 4 display cols)
        let cursor_x = chunks[1].x + 1 + 4 + app.search.cursor_pos as u16;
        let cursor_y = chunks[1].y + 1;
        frame.set_cursor_position(Position::new(cursor_x, cursor_y));
    }
}

fn draw_menu_bar_strip(frame: &mut Frame, app: &App, area: Rect) {
    let menu_labels = [" File ", " Edit ", " View ", " Tools ", " Help "];

    let active_idx = app.menu_bar.as_ref().map(|mb| mb.active_menu_index);

    let mut spans = Vec::new();
    for (i, label) in menu_labels.iter().enumerate() {
        let style = if Some(i) == active_idx {
            Style::default()
                .fg(Color::Black)
                .bg(Color::White)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default()
                .fg(Color::White)
                .bg(Color::Rgb(40, 40, 50))
        };
        spans.push(Span::styled(*label, style));
    }

    // Fill rest with background
    let labels_width: usize = menu_labels.iter().map(|l| l.len()).sum();
    let remaining = (area.width as usize).saturating_sub(labels_width);
    if remaining > 0 {
        spans.push(Span::styled(
            " ".repeat(remaining),
            Style::default().bg(Color::Rgb(40, 40, 50)),
        ));
    }

    let line = Line::from(spans);
    frame.render_widget(Paragraph::new(line), area);
}

fn draw_menu_bar_dropdown(frame: &mut Frame, menu_bar: &MenuBarState, area: Rect) {
    let menu_labels = [" File ", " Edit ", " View ", " Tools ", " Help "];
    let idx = menu_bar.active_menu_index;

    if idx >= menu_bar.menus.len() {
        return;
    }

    let menu = &menu_bar.menus[idx];

    // Calculate position below the menu label
    let mut x_offset: u16 = 0;
    for i in 0..idx {
        x_offset += menu_labels.get(i).map(|l| l.len() as u16).unwrap_or(6);
    }

    let max_label_len = menu.items.iter().map(|item| {
        let total = item.label.len() + if item.shortcut.is_empty() { 0 } else { item.shortcut.len() + 4 };
        total
    }).max().unwrap_or(10);

    let width = (max_label_len as u16 + 4).max(20).min(area.width.saturating_sub(x_offset));
    let height = (menu.items.len() as u16 + 2).min(area.height.saturating_sub(2));

    let popup_area = Rect::new(
        x_offset.min(area.width.saturating_sub(width)),
        1,
        width,
        height,
    );

    frame.render_widget(Clear, popup_area);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));

    let inner = block.inner(popup_area);
    frame.render_widget(block, popup_area);

    for (i, item) in menu.items.iter().enumerate() {
        if i as u16 >= inner.height {
            break;
        }

        let is_selected = i == menu_bar.active_item_index;
        let style = if is_selected {
            Style::default()
                .fg(Color::Black)
                .bg(Color::Cyan)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::White)
        };

        let item_area = Rect::new(inner.x, inner.y + i as u16, inner.width, 1);

        let text = if item.shortcut.is_empty() {
            format!(" {} ", item.label)
        } else {
            let padding = (inner.width as usize).saturating_sub(item.label.len() + item.shortcut.len() + 4);
            format!(" {} {:>pad$}{} ", item.label, "", item.shortcut, pad = padding)
        };

        frame.render_widget(Paragraph::new(text).style(style), item_area);
    }
}

fn draw_search_bar(frame: &mut Frame, app: &App, area: Rect) {
    let border_style = if app.search.focused {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(border_style)
        .title(" Search ");

    let search_text = format!(" \u{1F50D} {}", app.search.query);
    let paragraph = Paragraph::new(search_text)
        .block(block)
        .style(Style::default().fg(Color::White));

    frame.render_widget(paragraph, area);
}

fn draw_table(frame: &mut Frame, app: &mut App, area: Rect) {
    // Calculate visible rows (area height minus borders minus header)
    let table_inner_height = area.height.saturating_sub(3) as usize;
    app.table.visible_rows = table_inner_height;

    // Build header
    let header_columns: [(&str, SortColumn); 6] = [
        ("Name", SortColumn::Name),
        ("Path", SortColumn::Path),
        ("Size", SortColumn::Size),
        ("Ext", SortColumn::Extension),
        ("Date Modified", SortColumn::DateModified),
        ("Type", SortColumn::Type),
    ];

    let h_off_header = app.table.horizontal_offset as usize;
    let header = Row::new(header_columns.iter().map(|(name, col)| {
        let text = if app.table.sort_column == *col {
            format!("{}{}", name, app.table.sort_order.indicator())
        } else {
            name.to_string()
        };
        let text = if h_off_header > 0 && text.len() > h_off_header {
            text[h_off_header..].to_string()
        } else if h_off_header > 0 {
            String::new()
        } else {
            text
        };
        Cell::from(text).style(
            Style::default()
                .fg(Color::White)
                .bg(Color::Rgb(0, 95, 135))
                .add_modifier(Modifier::BOLD),
        )
    }))
    .height(1);

    // Build visible rows only
    let start = app.table.scroll_offset;
    let end = (start + table_inner_height).min(app.filtered_indices.len());

    let rows: Vec<Row> = (start..end)
        .enumerate()
        .map(|(visual_idx, logical_idx)| {
            let entry_idx = app.filtered_indices[logical_idx];
            let entry = &app.all_entries[entry_idx];

            let is_selected = app.table.selections.contains(&logical_idx)
                || app.table.selected == Some(logical_idx);

            // Build row data lazily (path resolution only for visible rows)
            let (name, path, size_str, ext, date_str, type_str, is_dir) =
                if let Some(row_data) = app.get_row_data(entry_idx) {
                    let size = if row_data.is_directory {
                        String::new()
                    } else {
                        crate::format_size(row_data.file_size)
                    };
                    let date = if row_data.modification_time > 0 {
                        crate::format_filetime(row_data.modification_time)
                    } else {
                        String::new()
                    };
                    let type_label =
                        colors::type_label(row_data.is_directory, &row_data.extension).to_string();
                    (
                        row_data.name,
                        row_data.path,
                        size,
                        row_data.extension,
                        date,
                        type_label,
                        row_data.is_directory,
                    )
                } else {
                    // Fallback: use cached data without path
                    let size = if entry.is_directory {
                        String::new()
                    } else {
                        crate::format_size(entry.file_size)
                    };
                    let date = if entry.modification_time > 0 {
                        crate::format_filetime(entry.modification_time)
                    } else {
                        String::new()
                    };
                    let type_label =
                        colors::type_label(entry.is_directory, &entry.extension).to_string();
                    (
                        entry.name.clone(),
                        String::new(),
                        size,
                        entry.extension.clone(),
                        date,
                        type_label,
                        entry.is_directory,
                    )
                };

            let icon = colors::icon_for_entry(is_dir, &ext);
            let name_color = if is_dir {
                Color::LightBlue
            } else {
                colors::color_for_extension(&ext)
            };

            // Alternating row background
            let bg = if is_selected {
                Color::Rgb(60, 60, 80)
            } else if visual_idx % 2 == 1 {
                Color::Rgb(25, 25, 35)
            } else {
                Color::Reset
            };

            let fg_modifier = if is_selected {
                Modifier::BOLD
            } else {
                Modifier::empty()
            };

            // Apply horizontal offset to all text content
            let h_off = app.table.horizontal_offset as usize;
            let apply_offset = |s: String| -> String {
                let char_count = s.chars().count();
                if h_off == 0 || char_count <= h_off {
                    if h_off > 0 && char_count <= h_off {
                        String::new()
                    } else {
                        s
                    }
                } else {
                    s.chars().skip(h_off).collect()
                }
            };

            let name_text = apply_offset(format!("{} {}", icon, name));
            let path_text = apply_offset(path);
            let size_text = apply_offset(size_str);
            let ext_text = apply_offset(ext);
            let date_text = apply_offset(date_str);
            let type_text = apply_offset(type_str);

            let name_cell = Cell::from(name_text)
                .style(Style::default().fg(name_color).bg(bg).add_modifier(fg_modifier));
            let path_cell = Cell::from(path_text).style(Style::default().fg(Color::Gray).bg(bg));
            let size_cell = Cell::from(size_text)
                .style(Style::default().fg(Color::Green).bg(bg));
            let ext_cell = Cell::from(ext_text).style(Style::default().fg(Color::Blue).bg(bg));
            let date_cell = Cell::from(date_text).style(Style::default().fg(Color::White).bg(bg));
            let type_cell = Cell::from(type_text)
                .style(Style::default().fg(Color::DarkGray).bg(bg).add_modifier(Modifier::ITALIC));

            Row::new(vec![
                name_cell, path_cell, size_cell, ext_cell, date_cell, type_cell,
            ])
        })
        .collect();

    let widths: Vec<Constraint> = app.table.column_widths.iter().map(|&w| {
        if w == 0 {
            Constraint::Fill(1)
        } else {
            Constraint::Length(w)
        }
    }).collect();

    let table = Table::new(rows, widths)
        .header(header)
        .block(
            Block::default()
                .borders(Borders::NONE),
        );

    frame.render_widget(table, area);
}

fn draw_status_bar(frame: &mut Frame, app: &App, area: Rect) {
    let left_text = if app.is_scanning {
        format!(" \u{23F3} Scanning... {}", app.scan_progress)
    } else if app.is_sorting {
        " Sorting...".to_string()
    } else {
        let obj_count = app.filtered_indices.len();
        let selected_count = app.table.selections.len().max(
            if app.table.selected.is_some() { 1 } else { 0 }
        );
        let total_size: u64 = app.trees.iter().map(|t| t.stats.total_size).sum();
        let total_size_str = crate::format_size(total_size);
        format!(
            " {} objects | {} selected | {} total",
            obj_count, selected_count, total_size_str
        )
    };

    let right_text = " Tab:Search  F1-F6:Sort  \u{2190}\u{2192}:Scroll  M:Menu  Ctrl+F:Filters  T:Treemap  F10:MenuBar  Ctrl+Q:Quit ";

    // Build the status line: left-aligned text + padding + right-aligned text
    let available_width = area.width as usize;
    let left_len = left_text.len();
    let right_len = right_text.len();

    let status_str = if left_len + right_len < available_width {
        let padding = available_width - left_len - right_len;
        format!("{}{:padding$}{}", left_text, "", right_text, padding = padding)
    } else {
        // Not enough space, just show left text
        format!("{:width$}", left_text, width = available_width)
    };

    let status = Paragraph::new(status_str)
        .style(Style::default().fg(Color::White).bg(Color::Rgb(0, 95, 135)));

    frame.render_widget(status, area);
}

/// Helper to create a centered popup area
fn centered_rect(width: u16, height: u16, area: Rect) -> Rect {
    let x = area.x + area.width.saturating_sub(width) / 2;
    let y = area.y + area.height.saturating_sub(height) / 2;
    Rect::new(x, y, width.min(area.width), height.min(area.height))
}

fn draw_actions_menu(
    frame: &mut Frame,
    actions: &crate::tui::menu::ActionsMenu,
    area: Rect,
) {
    let width = 28;
    let height = (actions.items.len() as u16) + 2; // +2 for borders
    let popup_area = centered_rect(width, height, area);

    frame.render_widget(Clear, popup_area);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan))
        .title(" Actions ")
        .title_style(Style::default().fg(Color::White).add_modifier(Modifier::BOLD));

    let inner = block.inner(popup_area);
    frame.render_widget(block, popup_area);

    for (i, (label, _)) in actions.items.iter().enumerate() {
        let is_selected = i == actions.selected;
        let style = if is_selected {
            Style::default()
                .fg(Color::Black)
                .bg(Color::Cyan)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::White)
        };

        let item_area = Rect::new(inner.x, inner.y + i as u16, inner.width, 1);
        let text = format!(" {} ", label);
        frame.render_widget(Paragraph::new(text).style(style), item_area);
    }
}

fn draw_confirm_dialog(
    frame: &mut Frame,
    confirm: &crate::tui::menu::ConfirmDialog,
    area: Rect,
) {
    let width = (confirm.message.len() as u16 + 4).max(30).min(area.width - 4);
    let height = 5;
    let popup_area = centered_rect(width, height, area);

    frame.render_widget(Clear, popup_area);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Red))
        .title(" Confirm ")
        .title_style(Style::default().fg(Color::Red).add_modifier(Modifier::BOLD));

    let inner = block.inner(popup_area);
    frame.render_widget(block, popup_area);

    // Message
    let msg_area = Rect::new(inner.x + 1, inner.y, inner.width.saturating_sub(2), 1);
    frame.render_widget(
        Paragraph::new(confirm.message.as_str()).style(Style::default().fg(Color::White)),
        msg_area,
    );

    // Buttons
    let btn_y = inner.y + 2;
    let yes_style = if confirm.confirm_selected {
        Style::default().fg(Color::Black).bg(Color::Red).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::White)
    };
    let no_style = if !confirm.confirm_selected {
        Style::default().fg(Color::Black).bg(Color::Green).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::White)
    };

    let btn_x = inner.x + inner.width / 2 - 8;
    frame.render_widget(
        Paragraph::new(" [Yes] ").style(yes_style),
        Rect::new(btn_x, btn_y, 7, 1),
    );
    frame.render_widget(
        Paragraph::new(" [No] ").style(no_style),
        Rect::new(btn_x + 9, btn_y, 6, 1),
    );
}

fn draw_rename_dialog(
    frame: &mut Frame,
    rename: &crate::tui::menu::RenameDialog,
    area: Rect,
) {
    let width = 50.min(area.width - 4);
    let height = 5;
    let popup_area = centered_rect(width, height, area);

    frame.render_widget(Clear, popup_area);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Yellow))
        .title(" Rename ")
        .title_style(Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD));

    let inner = block.inner(popup_area);
    frame.render_widget(block, popup_area);

    // Label
    let label_area = Rect::new(inner.x + 1, inner.y, inner.width.saturating_sub(2), 1);
    frame.render_widget(
        Paragraph::new("New name:").style(Style::default().fg(Color::Gray)),
        label_area,
    );

    // Text input
    let input_area = Rect::new(inner.x + 1, inner.y + 1, inner.width.saturating_sub(2), 1);
    frame.render_widget(
        Paragraph::new(rename.new_name.as_str())
            .style(Style::default().fg(Color::White).bg(Color::Rgb(40, 40, 50))),
        input_area,
    );

    // Cursor
    let cursor_x = input_area.x + rename.cursor_pos as u16;
    frame.set_cursor_position(Position::new(cursor_x, input_area.y));
}

fn draw_search_filters(
    frame: &mut Frame,
    filters: &crate::tui::menu::SearchFiltersMenu,
    area: Rect,
) {
    let width = 60u16.min(area.width.saturating_sub(4));
    let height = 16u16;
    let popup_area = centered_rect(width, height, area);

    frame.render_widget(Clear, popup_area);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan))
        .title(" Search Filters (Tab to navigate, Enter to apply) ")
        .title_style(Style::default().fg(Color::White).add_modifier(Modifier::BOLD));

    let inner = block.inner(popup_area);
    frame.render_widget(block, popup_area);

    let label_w = 13u16;
    let field_w = inner.width.saturating_sub(label_w + 1);

    // Helper to draw a text field with placeholder
    let draw_field =
        |frame: &mut Frame, y: u16, label: &str, value: &str, placeholder: &str, focused: bool| {
            let label_area = Rect::new(inner.x + 1, y, label_w, 1);
            let value_area =
                Rect::new(inner.x + label_w + 1, y, field_w.saturating_sub(1), 1);

            let label_style = if focused {
                Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::Gray)
            };
            frame.render_widget(Paragraph::new(label).style(label_style), label_area);

            let (display, style) = if value.is_empty() {
                let bg = if focused {
                    Color::Rgb(0, 50, 70)
                } else {
                    Color::Rgb(30, 30, 40)
                };
                (
                    format!("{:w$}", placeholder, w = field_w.saturating_sub(1) as usize),
                    Style::default().fg(Color::DarkGray).bg(bg).add_modifier(Modifier::ITALIC),
                )
            } else {
                let bg = if focused {
                    Color::Rgb(0, 50, 70)
                } else {
                    Color::Rgb(30, 30, 40)
                };
                (
                    format!("{:w$}", value, w = field_w.saturating_sub(1) as usize),
                    Style::default().fg(Color::White).bg(bg),
                )
            };
            frame.render_widget(Paragraph::new(display).style(style), value_area);
        };

    // Helper to draw a mode selector with arrows
    let draw_mode_field =
        |frame: &mut Frame, y: u16, label: &str, mode_label: &str, focused: bool| {
            let label_area = Rect::new(inner.x + 1, y, label_w, 1);
            let value_area =
                Rect::new(inner.x + label_w + 1, y, field_w.saturating_sub(1), 1);

            let label_style = if focused {
                Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::Gray)
            };
            frame.render_widget(Paragraph::new(label).style(label_style), label_area);

            let bg = if focused {
                Color::Rgb(0, 50, 70)
            } else {
                Color::Rgb(30, 30, 40)
            };
            let style = Style::default().fg(Color::White).bg(bg);
            let display = format!(" < {:^10} > ", mode_label);
            frame.render_widget(Paragraph::new(display).style(style), value_area);
        };

    let mut y = inner.y;

    // Section header: Pattern
    frame.render_widget(
        Paragraph::new("-- Pattern ----------------------------------------")
            .style(Style::default().fg(Color::DarkGray)),
        Rect::new(inner.x + 1, y, inner.width.saturating_sub(2), 1),
    );
    y += 1;

    draw_field(frame, y, " Regex:", &filters.regex_pattern, "e.g. .*\\.log$", filters.focused_field == SearchFilterField::Regex);
    y += 1;

    // Section header: Date
    frame.render_widget(
        Paragraph::new("-- Date ------------------------------------------")
            .style(Style::default().fg(Color::DarkGray)),
        Rect::new(inner.x + 1, y, inner.width.saturating_sub(2), 1),
    );
    y += 1;

    draw_mode_field(frame, y, " Mode:", filters.date_mode.label(), filters.focused_field == SearchFilterField::DateMode);
    y += 1;
    draw_field(frame, y, " Start:", &filters.date_start, "YYYY-MM-DD", filters.focused_field == SearchFilterField::DateStart);
    y += 1;
    draw_field(frame, y, " End:", &filters.date_end, "YYYY-MM-DD", filters.focused_field == SearchFilterField::DateEnd);
    y += 1;

    // Section header: Size
    frame.render_widget(
        Paragraph::new("-- Size ------------------------------------------")
            .style(Style::default().fg(Color::DarkGray)),
        Rect::new(inner.x + 1, y, inner.width.saturating_sub(2), 1),
    );
    y += 1;

    draw_mode_field(frame, y, " Mode:", filters.size_mode.label(), filters.focused_field == SearchFilterField::SizeMode);
    y += 1;
    draw_field(frame, y, " Value:", &filters.size_value, "e.g. 10MB, 1GB, 500KB", filters.focused_field == SearchFilterField::SizeValue);
    y += 1;
    draw_field(frame, y, " End:", &filters.size_end, "e.g. 100MB (for Between)", filters.focused_field == SearchFilterField::SizeEnd);
    y += 1;

    // Section header: Extension
    frame.render_widget(
        Paragraph::new("-- Extension -------------------------------------")
            .style(Style::default().fg(Color::DarkGray)),
        Rect::new(inner.x + 1, y, inner.width.saturating_sub(2), 1),
    );
    y += 1;

    draw_field(frame, y, " Extension:", &filters.extension_filter, "e.g. pdf;docx;txt", filters.focused_field == SearchFilterField::Extension);
    y += 1;

    // Buttons row with some spacing
    let btn_y = (y + 1).min(inner.y + inner.height - 1);
    let btn_area = Rect::new(inner.x + 1, btn_y, inner.width.saturating_sub(2), 1);

    let apply_style = if filters.focused_field == SearchFilterField::Apply {
        Style::default().fg(Color::Black).bg(Color::Green).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::Green)
    };
    let clear_style = if filters.focused_field == SearchFilterField::Clear {
        Style::default().fg(Color::Black).bg(Color::Yellow).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::Yellow)
    };
    let cancel_style = if filters.focused_field == SearchFilterField::Cancel {
        Style::default().fg(Color::Black).bg(Color::Red).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::Red)
    };

    let buttons = Line::from(vec![
        Span::raw("    "),
        Span::styled(" [Apply] ", apply_style),
        Span::raw("    "),
        Span::styled(" [Clear] ", clear_style),
        Span::raw("    "),
        Span::styled(" [Cancel] ", cancel_style),
    ]);

    frame.render_widget(Paragraph::new(buttons), btn_area);

    // Show cursor on focused text input
    if filters.focused_field.is_text_input() {
        let cursor_y = match filters.focused_field {
            SearchFilterField::Regex => inner.y + 1,
            SearchFilterField::DateStart => inner.y + 4,
            SearchFilterField::DateEnd => inner.y + 5,
            SearchFilterField::SizeValue => inner.y + 8,
            SearchFilterField::SizeEnd => inner.y + 9,
            SearchFilterField::Extension => inner.y + 11,
            _ => inner.y,
        };
        let cursor_offset = match filters.focused_field {
            SearchFilterField::Regex => filters.regex_cursor,
            SearchFilterField::DateStart => filters.date_start_cursor,
            SearchFilterField::DateEnd => filters.date_end_cursor,
            SearchFilterField::SizeValue => filters.size_value_cursor,
            SearchFilterField::SizeEnd => filters.size_end_cursor,
            SearchFilterField::Extension => filters.extension_cursor,
            _ => 0,
        };
        let cursor_x = inner.x + label_w + 1 + cursor_offset as u16;
        frame.set_cursor_position(Position::new(cursor_x, cursor_y));
    }
}

fn draw_info_dialog(
    frame: &mut Frame,
    info: &crate::tui::menu::InfoDialog,
    area: Rect,
) {
    let max_line_len = info.lines.iter().map(|l| l.len()).max().unwrap_or(20);
    let width = ((max_line_len + 4) as u16).max(30).min(area.width.saturating_sub(4));
    let height = ((info.lines.len() + 3) as u16).min(area.height.saturating_sub(4));
    let popup_area = centered_rect(width, height, area);

    frame.render_widget(Clear, popup_area);

    let title = format!(" {} ", info.title);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan))
        .title(title)
        .title_style(Style::default().fg(Color::White).add_modifier(Modifier::BOLD));

    let inner = block.inner(popup_area);
    frame.render_widget(block, popup_area);

    for (i, line) in info.lines.iter().enumerate() {
        if i as u16 >= inner.height {
            break;
        }
        let line_area = Rect::new(inner.x + 1, inner.y + i as u16, inner.width.saturating_sub(2), 1);
        frame.render_widget(
            Paragraph::new(line.as_str()).style(Style::default().fg(Color::White)),
            line_area,
        );
    }
}
