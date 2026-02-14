use crate::tui::app::App;
use crate::tui::colors;
use crate::tui::table::SortColumn;
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Cell, Paragraph, Row, Table};

pub fn draw(frame: &mut Frame, app: &mut App) {
    let area = frame.area();

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // Search bar
            Constraint::Min(5),   // Table
            Constraint::Length(1), // Status bar
        ])
        .split(area);

    draw_search_bar(frame, app, chunks[0]);
    draw_table(frame, app, chunks[1]);
    draw_status_bar(frame, app, chunks[2]);

    // Show cursor in search bar when focused
    if app.search.focused {
        // Account for border (1) + space (1) + search icon " \u{1F50D} " (approx 4 display cols)
        let cursor_x = chunks[0].x + 1 + 4 + app.search.cursor_pos as u16;
        let cursor_y = chunks[0].y + 1;
        frame.set_cursor_position(Position::new(cursor_x, cursor_y));
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
    let header_columns: [(& str, SortColumn); 6] = [
        ("Name", SortColumn::Name),
        ("Path", SortColumn::Path),
        ("Size", SortColumn::Size),
        ("Ext", SortColumn::Extension),
        ("Date Modified", SortColumn::DateModified),
        ("Type", SortColumn::Type),
    ];

    let header = Row::new(header_columns.iter().map(|(name, col)| {
        let text = if app.table.sort_column == *col {
            format!("{}{}", name, app.table.sort_order.indicator())
        } else {
            name.to_string()
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

            let is_selected = app.table.selected == Some(logical_idx);

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

            let name_cell = Cell::from(format!("{} {}", icon, name))
                .style(Style::default().fg(name_color).bg(bg).add_modifier(fg_modifier));
            let path_cell = Cell::from(path).style(Style::default().fg(Color::Gray).bg(bg));
            let size_cell = Cell::from(size_str)
                .style(Style::default().fg(Color::Green).bg(bg));
            let ext_cell = Cell::from(ext).style(Style::default().fg(Color::Blue).bg(bg));
            let date_cell = Cell::from(date_str).style(Style::default().fg(Color::White).bg(bg));
            let type_cell = Cell::from(type_str)
                .style(Style::default().fg(Color::DarkGray).bg(bg).add_modifier(Modifier::ITALIC));

            Row::new(vec![
                name_cell, path_cell, size_cell, ext_cell, date_cell, type_cell,
            ])
        })
        .collect();

    let widths = [
        Constraint::Length(25),     // Name
        Constraint::Fill(1),        // Path (takes remaining space)
        Constraint::Length(12),     // Size
        Constraint::Length(8),      // Ext
        Constraint::Length(20),     // Date Modified
        Constraint::Length(18),     // Type
    ];

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
        let selected_count = if app.table.selected.is_some() {
            1
        } else {
            0
        };
        let total_size: u64 = app.trees.iter().map(|t| t.stats.total_size).sum();
        let total_size_str = crate::format_size(total_size);
        format!(
            " {} objects | {} selected | {} total",
            obj_count, selected_count, total_size_str
        )
    };

    let right_text = " Tab:Search  F1-F6:Sort  Ctrl+Q:Quit ";

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
