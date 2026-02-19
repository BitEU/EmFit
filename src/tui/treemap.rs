use crate::file_tree::{FileTree, NodeKey};
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Paragraph};
use std::sync::Arc;

// ============================================================================
// Data Structures
// ============================================================================

/// A single rectangle in the treemap.
///
/// Directories with `children_rendered == true` are *container* rects – they
/// render as bordered boxes whose interior is overwritten by child rects.
/// Everything else (files, tiny directories) is a *leaf* rect – a solid
/// coloured block.
#[derive(Debug, Clone)]
pub struct TreemapRect {
    pub name: String,
    pub size: u64,
    pub is_directory: bool,
    pub depth: usize,
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    pub key: NodeKey,
    /// When true the rect is a directory container whose children have been
    /// laid out inside it.  The renderer draws a border+title and the children
    /// paint on top of the interior background.
    pub children_rendered: bool,
}

/// Persistent state for the treemap view.
pub struct TreemapState {
    pub rects: Vec<TreemapRect>,
    pub selected: usize,
    pub breadcrumb: Vec<(NodeKey, String)>,
    pub current_key: NodeKey,
    /// Terminal width in cells – set before building layout.
    pub screen_w: f64,
    /// Terminal height of the treemap canvas (after subtracting chrome).
    pub screen_h: f64,
}

impl TreemapState {
    pub fn new() -> Self {
        Self {
            rects: Vec::new(),
            selected: 0,
            breadcrumb: Vec::new(),
            current_key: NodeKey::root(),
            screen_w: 160.0,
            screen_h: 45.0,
        }
    }

    /// Call before every `build_*` to tell the layout how many cells are
    /// available so it can compute pixel-perfect padding for borders.
    pub fn set_screen_size(&mut self, w: u16, h: u16) {
        self.screen_w = w.max(10) as f64;
        // Reserve 2 rows for breadcrumb + info bar
        self.screen_h = h.saturating_sub(2).max(4) as f64;
    }

    // ====================================================================
    // Build entry points
    // ====================================================================

    /// Build treemap from the root of all scanned drive trees.
    pub fn build_from_trees(&mut self, trees: &[Arc<FileTree>]) {
        self.rects.clear();
        self.selected = 0;
        self.breadcrumb.clear();
        self.current_key = NodeKey::root();

        for tree in trees {
            if let Some(root) = tree.root() {
                let drive = format!("{}:", tree.drive_letter);
                self.breadcrumb.push((root.key(), drive));
                self.layout_children(tree, &root.key(), 0.0, 0.0, 1.0, 1.0, 0);
            }
        }
        self.snap_selection();
    }

    /// Build treemap for a specific directory.
    pub fn build_from_node(&mut self, tree: &FileTree, key: &NodeKey) {
        self.rects.clear();
        self.selected = 0;
        self.current_key = *key;
        self.layout_children(tree, key, 0.0, 0.0, 1.0, 1.0, 0);
        self.snap_selection();
    }

    // ====================================================================
    // Hierarchical squarified layout
    // ====================================================================

    /// Lay out the children of `parent_key` inside the normalised rectangle
    /// (x, y, w, h).  Directories big enough to show nested content become
    /// container rects whose children are recursively laid out inside the
    /// inner area (border cells subtracted).
    fn layout_children(
        &mut self,
        tree: &FileTree,
        parent_key: &NodeKey,
        x: f64,
        y: f64,
        w: f64,
        h: f64,
        depth: usize,
    ) {
        const MAX_DEPTH: usize = 8;

        let w_cells = w * self.screen_w;
        let h_cells = h * self.screen_h;

        if w_cells < 2.0 || h_cells < 1.0 || depth > MAX_DEPTH {
            return;
        }

        let children = tree.get_children(parent_key);
        let mut items: Vec<(String, u64, bool, NodeKey)> = Vec::new();

        for child in &children {
            if child.name == "." || child.name == ".." {
                continue;
            }
            let size = if child.is_directory {
                if child.total_size > 0 { child.total_size } else { child.file_size }
            } else {
                child.file_size
            };
            if size > 0 {
                items.push((child.name.clone(), size, child.is_directory, child.key()));
            }
        }
        if items.is_empty() {
            return;
        }

        // Squarify needs items sorted largest-first
        items.sort_by(|a, b| b.1.cmp(&a.1));

        let limit = match depth {
            0 => 2000,
            1 => 1000,
            2 => 500,
            3 => 250,
            4 => 120,
            5 => 60,
            _ => 30,
        };
        items.truncate(limit);

        let total: f64 = items.iter().map(|i| i.1 as f64).sum();
        if total <= 0.0 {
            return;
        }
        self.squarify_strip(tree, &items, x, y, w, h, total, depth);
    }

    /// Recursively split items into strips with the best aspect ratios,
    /// placing each item via `place_item`.
    fn squarify_strip(
        &mut self,
        tree: &FileTree,
        items: &[(String, u64, bool, NodeKey)],
        x: f64,
        y: f64,
        w: f64,
        h: f64,
        total: f64,
        depth: usize,
    ) {
        if items.is_empty() || w < 0.0005 || h < 0.0005 {
            return;
        }

        if items.len() == 1 {
            self.place_item(tree, &items[0], x, y, w, h, depth);
            return;
        }

        let vertical = w >= h;
        let short = if vertical { h } else { w };
        let long = if vertical { w } else { h };

        // Find optimal strip size (minimise worst aspect ratio)
        let mut best_split = 1;
        let mut best_aspect = f64::MAX;
        let mut running = 0.0;

        for i in 0..items.len() {
            running += items[i].1 as f64;
            let frac = running / total;
            let strip_len = frac * long;

            let strip_sum: f64 = items[..=i].iter().map(|it| it.1 as f64).sum();
            let mut worst: f64 = 0.0;
            for j in 0..=i {
                let item_short = (items[j].1 as f64 / strip_sum) * short;
                let a = if item_short > 0.0 && strip_len > 0.0 {
                    (strip_len / item_short).max(item_short / strip_len)
                } else {
                    f64::MAX
                };
                worst = worst.max(a);
            }
            if worst <= best_aspect {
                best_aspect = worst;
                best_split = i + 1;
            } else {
                break;
            }
        }

        let strip = &items[..best_split];
        let strip_total: f64 = strip.iter().map(|i| i.1 as f64).sum();
        let strip_frac = strip_total / total;

        let (sx, sy, sw, sh) = if vertical {
            (x, y, strip_frac * w, h)
        } else {
            (x, y, w, strip_frac * h)
        };

        // Place items within the strip
        let mut pos = 0.0;
        for item in strip {
            let ifrac = item.1 as f64 / strip_total;
            let (ix, iy, iw, ih) = if vertical {
                (sx, sy + pos * sh, sw, ifrac * sh)
            } else {
                (sx + pos * sw, sy, ifrac * sw, sh)
            };
            pos += ifrac;
            self.place_item(tree, item, ix, iy, iw, ih, depth);
        }

        // Recurse into remaining items
        if best_split < items.len() {
            let rest = &items[best_split..];
            let rest_total = total - strip_total;
            let (rx, ry, rw, rh) = if vertical {
                (x + strip_frac * w, y, w * (1.0 - strip_frac), h)
            } else {
                (x, y + strip_frac * h, w, h * (1.0 - strip_frac))
            };
            self.squarify_strip(tree, rest, rx, ry, rw, rh, rest_total, depth);
        }
    }

    /// Place a single item.  If it is a directory large enough to display
    /// nested children, it becomes a container rect and we recurse into its
    /// children, laying them out inside the border.
    fn place_item(
        &mut self,
        tree: &FileTree,
        item: &(String, u64, bool, NodeKey),
        x: f64,
        y: f64,
        w: f64,
        h: f64,
        depth: usize,
    ) {
        let w_cells = w * self.screen_w;
        let h_cells = h * self.screen_h;

        // A directory can nest children if it has enough room for a border
        // (1 col each side, 1 row top header, 1 row bottom) plus at least
        // a 4×2 inner area.
        let can_nest = item.2 && h_cells >= 4.0 && w_cells >= 6.0 && depth < 8;

        self.rects.push(TreemapRect {
            name: item.0.clone(),
            size: item.1,
            is_directory: item.2,
            depth,
            x,
            y,
            w,
            h,
            key: item.3,
            children_rendered: can_nest,
        });

        if can_nest {
            // Subtract border cells (1 column each side, 1 row top, 1 row bottom)
            let bx = 1.0 / self.screen_w;
            let by = 1.0 / self.screen_h;
            let inner_x = x + bx;
            let inner_y = y + by;
            let inner_w = w - 2.0 * bx;
            let inner_h = h - 2.0 * by;

            if inner_w > 0.0 && inner_h > 0.0 {
                self.layout_children(tree, &item.3, inner_x, inner_y, inner_w, inner_h, depth + 1);
            }
        }
    }

    // ====================================================================
    // Navigation – skip container rects (they are just frames)
    // ====================================================================

    pub fn move_next(&mut self) {
        if self.rects.is_empty() {
            return;
        }
        let start = self.selected;
        loop {
            self.selected = (self.selected + 1) % self.rects.len();
            if !self.rects[self.selected].children_rendered || self.selected == start {
                break;
            }
        }
    }

    pub fn move_prev(&mut self) {
        if self.rects.is_empty() {
            return;
        }
        let start = self.selected;
        loop {
            self.selected = if self.selected == 0 {
                self.rects.len() - 1
            } else {
                self.selected - 1
            };
            if !self.rects[self.selected].children_rendered || self.selected == start {
                break;
            }
        }
    }

    pub fn selected_rect(&self) -> Option<&TreemapRect> {
        self.rects.get(self.selected)
    }

    /// Make sure `selected` doesn't point at a container rect.
    fn snap_selection(&mut self) {
        if self.rects.is_empty() {
            return;
        }
        if !self.rects[self.selected].children_rendered {
            return;
        }
        for (i, r) in self.rects.iter().enumerate() {
            if !r.children_rendered {
                self.selected = i;
                return;
            }
        }
    }
}

// ============================================================================
// Colour palette
// ============================================================================

/// Border colour for directory containers, cycling through depth.
fn depth_border_color(depth: usize) -> Color {
    match depth % 7 {
        0 => Color::Rgb(0, 190, 230),   // cyan
        1 => Color::Rgb(90, 200, 70),   // green
        2 => Color::Rgb(230, 190, 40),  // gold
        3 => Color::Rgb(210, 90, 200),  // magenta
        4 => Color::Rgb(70, 140, 240),  // blue
        5 => Color::Rgb(230, 130, 50),  // orange
        _ => Color::Rgb(130, 210, 180), // teal
    }
}

/// Dark tinted background inside directory containers.
fn depth_bg_color(depth: usize) -> Color {
    match depth % 7 {
        0 => Color::Rgb(8, 22, 28),
        1 => Color::Rgb(12, 24, 10),
        2 => Color::Rgb(26, 22, 8),
        3 => Color::Rgb(24, 12, 24),
        4 => Color::Rgb(10, 16, 30),
        5 => Color::Rgb(26, 16, 8),
        _ => Color::Rgb(12, 24, 20),
    }
}

/// Colour for leaf rectangles (files or tiny directories).
fn leaf_color(name: &str, is_directory: bool, index: usize) -> Color {
    if is_directory {
        let palette = [
            Color::Rgb(40, 105, 135),
            Color::Rgb(50, 115, 120),
            Color::Rgb(60, 95, 145),
            Color::Rgb(45, 125, 110),
            Color::Rgb(55, 108, 128),
        ];
        return palette[index % palette.len()];
    }

    let ext = name.rsplit('.').next().unwrap_or("").to_lowercase();
    match ext.as_str() {
        // ── executables / system ────────────────────────────────────────
        "exe" | "com" | "scr"                        => Color::Rgb(200, 55, 55),
        "dll" | "sys" | "drv" | "ocx"                => Color::Rgb(175, 65, 65),
        "msi" | "bat" | "cmd" | "ps1"                => Color::Rgb(185, 80, 50),

        // ── archives ────────────────────────────────────────────────────
        "zip" | "rar" | "7z" | "gz" | "tar"
        | "xz" | "bz2" | "cab" | "iso"              => Color::Rgb(200, 175, 35),

        // ── video ───────────────────────────────────────────────────────
        "mp4" | "mkv" | "avi" | "mov" | "wmv"
        | "flv" | "webm" | "m4v" | "ts"             => Color::Rgb(160, 45, 195),

        // ── audio ───────────────────────────────────────────────────────
        "mp3" | "wav" | "flac" | "ogg"
        | "aac" | "wma" | "m4a" | "opus"            => Color::Rgb(35, 175, 135),

        // ── images ──────────────────────────────────────────────────────
        "jpg" | "jpeg" | "png" | "gif" | "bmp"
        | "tiff" | "webp" | "ico" | "svg" | "psd"
        | "raw" | "cr2" | "nef" | "dng"             => Color::Rgb(195, 125, 35),

        // ── documents ───────────────────────────────────────────────────
        "pdf"                                        => Color::Rgb(200, 50, 50),
        "doc" | "docx" | "odt" | "rtf"              => Color::Rgb(55, 130, 200),
        "xls" | "xlsx" | "ods" | "csv"              => Color::Rgb(45, 165, 65),
        "ppt" | "pptx" | "odp"                      => Color::Rgb(200, 105, 35),

        // ── text / config ───────────────────────────────────────────────
        "txt" | "log" | "md" | "cfg" | "ini"
        | "conf" | "yml" | "yaml" | "toml"          => Color::Rgb(120, 120, 120),

        // ── code ────────────────────────────────────────────────────────
        "rs" | "go" | "c" | "cpp" | "h" | "hpp"
        | "cs"                                       => Color::Rgb(75, 150, 220),
        "py" | "pyw"                                 => Color::Rgb(55, 140, 185),
        "js" | "ts" | "jsx" | "tsx"                  => Color::Rgb(215, 195, 45),
        "java" | "kt" | "scala"                     => Color::Rgb(170, 100, 55),
        "html" | "htm" | "css" | "scss"              => Color::Rgb(215, 75, 45),
        "json" | "xml" | "sql"                       => Color::Rgb(140, 160, 55),

        // ── game data ───────────────────────────────────────────────────
        "pak" | "rpf" | "bdt" | "pack" | "assets"
        | "resource" | "forge" | "wad"               => Color::Rgb(185, 75, 165),

        // ── virtual disks / dumps ───────────────────────────────────────
        "vdi" | "vmdk" | "vhd" | "vhdx" | "qcow2"
        | "img" | "bin" | "001"                      => Color::Rgb(100, 65, 165),

        // ── databases ───────────────────────────────────────────────────
        "db" | "sqlite" | "mdf" | "ldf" | "bak"     => Color::Rgb(135, 115, 45),

        // ── fonts ───────────────────────────────────────────────────────
        "ttf" | "otf" | "woff" | "woff2"            => Color::Rgb(160, 140, 100),

        // ── streaming / media DB ────────────────────────────────────────
        "stream" | "streamdb"                        => Color::Rgb(155, 55, 125),

        // ── MFT dumps ───────────────────────────────────────────────────
        "mft"                                        => Color::Rgb(200, 145, 35),

        // ── fallback: hash extension to a hue ───────────────────────────
        _ => {
            let h = ext.bytes().fold(0u32, |a, b| a.wrapping_mul(31).wrapping_add(b as u32));
            hsl_to_rgb((h % 360) as f64, 0.45, 0.38)
        }
    }
}

fn hsl_to_rgb(h: f64, s: f64, l: f64) -> Color {
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let x = c * (1.0 - ((h / 60.0) % 2.0 - 1.0).abs());
    let m = l - c / 2.0;
    let (r, g, b) = match h as u32 {
        0..=59   => (c, x, 0.0),
        60..=119 => (x, c, 0.0),
        120..=179=> (0.0, c, x),
        180..=239=> (0.0, x, c),
        240..=299=> (x, 0.0, c),
        _        => (c, 0.0, x),
    };
    Color::Rgb(
        ((r + m) * 255.0) as u8,
        ((g + m) * 255.0) as u8,
        ((b + m) * 255.0) as u8,
    )
}

// ============================================================================
// Rendering
// ============================================================================

/// Draw the complete treemap view: breadcrumb bar, treemap canvas, info bar.
pub fn draw_treemap(frame: &mut Frame, state: &TreemapState, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // breadcrumb
            Constraint::Min(5),   // treemap canvas
            Constraint::Length(1), // info bar
        ])
        .split(area);

    // ── Breadcrumb ──────────────────────────────────────────────────────
    let crumb: String = state
        .breadcrumb
        .iter()
        .map(|(_, n)| n.as_str())
        .collect::<Vec<_>>()
        .join(" \u{25B8} ");
    frame.render_widget(
        Paragraph::new(format!(" \u{1F4C1} {} ", crumb))
            .style(Style::default().fg(Color::White).bg(Color::Rgb(30, 30, 50))),
        chunks[0],
    );

    // ── Treemap canvas ──────────────────────────────────────────────────
    let map = chunks[1];
    let mw = map.width as f64;
    let mh = map.height as f64;

    // Rects are ordered parent-before-children, so container backgrounds
    // are painted first and children overwrite the interior.
    for (i, rect) in state.rects.iter().enumerate() {
        let rx = map.x + (rect.x * mw) as u16;
        let ry = map.y + (rect.y * mh) as u16;
        let rw = (((rect.x + rect.w) * mw) as u16).saturating_sub((rect.x * mw) as u16);
        let rh = (((rect.y + rect.h) * mh) as u16).saturating_sub((rect.y * mh) as u16);

        if rw == 0 || rh == 0 {
            continue;
        }
        let cw = rw.min(map.right().saturating_sub(rx));
        let ch = rh.min(map.bottom().saturating_sub(ry));
        if cw == 0 || ch == 0 {
            continue;
        }
        let cell = Rect::new(rx, ry, cw, ch);
        let is_sel = i == state.selected;

        if rect.children_rendered {
            draw_container(frame, rect, cell, is_sel);
        } else {
            draw_leaf(frame, rect, cell, is_sel, i);
        }
    }

    // ── Info bar ────────────────────────────────────────────────────────
    let info = if let Some(r) = state.selected_rect() {
        let icon = if r.is_directory { "\u{1F4C1}" } else { "\u{1F4C4}" };
        format!(
            " {} {} \u{2500} {} | \u{2190}\u{2191}\u{2193}\u{2192}:Nav  Enter:Drill  Bksp:Up  Esc/T:Close",
            icon,
            r.name,
            crate::format_size(r.size),
        )
    } else {
        " Treemap | Arrows:Nav  Enter:Drill  Backspace:Up  Esc/T:Close".into()
    };
    frame.render_widget(
        Paragraph::new(info)
            .style(Style::default().fg(Color::White).bg(Color::Rgb(0, 80, 120))),
        chunks[2],
    );
}

// ────────────────────────────────────────────────────────────────────────────
// Directory container: bordered box with a title line.  The dark interior
// will be overwritten by children that paint after this rect.
// ────────────────────────────────────────────────────────────────────────────
fn draw_container(frame: &mut Frame, rect: &TreemapRect, area: Rect, selected: bool) {
    let bg = depth_bg_color(rect.depth);
    let border_fg = depth_border_color(rect.depth);

    // Fill background
    frame.render_widget(
        Paragraph::new("").style(Style::default().bg(bg)),
        area,
    );

    // Title
    let title = fit_title(&rect.name, rect.size, area.width);
    let has_title = !title.is_empty();

    // Selection: paint the top title row in CGA blue so the user can see
    // which container is highlighted without adding any extra borders.
    let title_bg = if selected { CGA_BLUE } else { bg };

    let mut block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_fg));

    if has_title {
        block = block
            .title(title)
            .title_style(
                Style::default()
                    .fg(Color::White)
                    .bg(title_bg)
                    .add_modifier(Modifier::BOLD),
            );
    }

    frame.render_widget(block, area);

    // If selected and the title was empty (box too small), paint the whole
    // interior CGA blue so selection is still visible.
    if selected && !has_title {
        frame.render_widget(
            Paragraph::new("").style(Style::default().bg(CGA_BLUE)),
            area,
        );
    }
}

/// Fit a "name (size)" title into the available width.
fn fit_title(name: &str, size: u64, width: u16) -> String {
    if width < 6 {
        return String::new();
    }
    let avail = (width as usize).saturating_sub(4);
    let size_str = crate::format_size(size);

    // Try: " name (size) "
    let full = format!(" {} ({}) ", name, size_str);
    if full.len() <= avail + 2 {
        return full;
    }
    // Try: " name "
    let short = format!(" {} ", name);
    if short.len() <= avail + 2 {
        return short;
    }
    // Truncate
    let max = avail.saturating_sub(2);
    if max == 0 {
        return String::new();
    }
    let trunc: String = name.chars().take(max).collect();
    format!(" {}\u{2026} ", trunc)
}

// ────────────────────────────────────────────────────────────────────────────
// Leaf rectangle: solid coloured block for a file or tiny directory.
// Selection = MS-DOS CGA colour 1 (eye-searing blue) background.
// Each row is rendered as a single padded span for clean full-width fill.
// ────────────────────────────────────────────────────────────────────────────

/// Classic CGA colour 1 – the eye-searing DOS blue.
const CGA_BLUE: Color = Color::Rgb(0, 0, 170);

fn draw_leaf(frame: &mut Frame, rect: &TreemapRect, area: Rect, selected: bool, idx: usize) {
    let bg = if selected {
        CGA_BLUE
    } else {
        leaf_color(&rect.name, rect.is_directory, idx)
    };
    let fg = if selected {
        Color::White
    } else {
        Color::Rgb(235, 235, 235)
    };

    let w = area.width as usize;
    let h = area.height as usize;

    // Build rows of text, each exactly `w` chars wide so the background
    // fills every cell cleanly with no wrapping artefacts.
    let rows = leaf_rows(&rect.name, rect.size, w, h);

    let style = Style::default().fg(fg).bg(bg);

    for (row_idx, row_text) in rows.iter().enumerate() {
        if row_idx >= h {
            break;
        }
        let row_area = Rect::new(area.x, area.y + row_idx as u16, area.width, 1);
        frame.render_widget(
            Paragraph::new(row_text.as_str()).style(style),
            row_area,
        );
    }

    // Fill any remaining rows with blank coloured cells
    for row_idx in rows.len()..h {
        let row_area = Rect::new(area.x, area.y + row_idx as u16, area.width, 1);
        frame.render_widget(
            Paragraph::new(pad_row("", w).as_str()).style(style),
            row_area,
        );
    }
}

/// Build one string per row for a leaf rectangle.  Each string is exactly
/// `w` chars wide (space-padded) so the background colour fills cleanly.
fn leaf_rows(name: &str, size: u64, w: usize, h: usize) -> Vec<String> {
    if w == 0 || h == 0 {
        return Vec::new();
    }

    let mut rows: Vec<String> = Vec::new();

    if h >= 2 && w >= 6 {
        // Row 0: name (truncated)
        rows.push(pad_row(&trunc(name, w), w));
        // Row 1: size
        let s = crate::format_size(size);
        rows.push(pad_row(&trunc(&s, w), w));
    } else if w >= 3 {
        // Single row: name
        rows.push(pad_row(&trunc(name, w), w));
    } else if w >= 1 {
        // Tiny: first char(s)
        let t: String = name.chars().take(w).collect();
        rows.push(pad_row(&t, w));
    }

    rows
}

/// Pad (or truncate) `s` to exactly `w` characters.
fn pad_row(s: &str, w: usize) -> String {
    let char_count = s.chars().count();
    if char_count >= w {
        s.chars().take(w).collect()
    } else {
        let mut out = s.to_string();
        for _ in 0..(w - char_count) {
            out.push(' ');
        }
        out
    }
}

/// Truncate string to `max` display chars, adding … if needed.
fn trunc(s: &str, max: usize) -> String {
    let char_count = s.chars().count();
    if char_count <= max {
        s.to_string()
    } else if max <= 1 {
        s.chars().take(max).collect()
    } else {
        let t: String = s.chars().take(max - 1).collect();
        format!("{}\u{2026}", t)
    }
}
