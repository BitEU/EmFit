use crate::file_tree::{FileTree, NodeKey};
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Paragraph};
use std::sync::Arc;

/// A single rectangle in the treemap
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
    pub has_visible_children: bool,  // Don't render if children are shown inside
}

/// State for the treemap view
pub struct TreemapState {
    pub rects: Vec<TreemapRect>,
    pub selected: usize,
    pub breadcrumb: Vec<(NodeKey, String)>,
    pub current_key: NodeKey,
}

impl TreemapState {
    pub fn new() -> Self {
        Self {
            rects: Vec::new(),
            selected: 0,
            breadcrumb: Vec::new(),
            current_key: NodeKey::root(),
        }
    }

    /// Build treemap from the root of a set of trees (WizTree-style hierarchical layout)
    pub fn build_from_trees(&mut self, trees: &[Arc<FileTree>]) {
        self.rects.clear();
        self.selected = 0;
        self.breadcrumb.clear();
        self.current_key = NodeKey::root();

        for tree in trees {
            if let Some(root) = tree.root() {
                let drive = format!("{}:", tree.drive_letter);
                self.breadcrumb.push((root.key(), drive));
                
                // Layout children hierarchically within the available space
                self.layout_hierarchical(tree, &root.key(), 0.0, 0.0, 1.0, 1.0, 0);
            }
        }
    }

    /// Hierarchical treemap layout - shows files WITHIN parent directory boxes (like WizTree)
    fn layout_hierarchical(
        &mut self,
        tree: &FileTree,
        parent_key: &NodeKey,
        x: f64,
        y: f64,
        w: f64,
        h: f64,
        depth: usize,
    ) {
        // Minimum box size in screen units to be visible
        const MIN_WIDTH: f64 = 0.01;
        const MIN_HEIGHT: f64 = 0.01;
        const MAX_DEPTH: usize = 4; // Max recursion depth for nested layouts

        if w < MIN_WIDTH || h < MIN_HEIGHT || depth > MAX_DEPTH {
            return;
        }

        let children = tree.get_children(parent_key);
        let mut items: Vec<(String, u64, bool, NodeKey)> = Vec::new();

        for child in children {
            if child.name == "." || child.name == ".." {
                continue;
            }

            let size = if child.is_directory {
                if child.total_size > 0 {
                    child.total_size
                } else {
                    child.file_size
                }
            } else {
                child.file_size
            };

            // Skip tiny items that won't be visible
            if size > 0 {
                items.push((child.name.clone(), size, child.is_directory, child.key()));
            }
        }

        if items.is_empty() {
            return;
        }

        // Sort by size descending
        items.sort_by(|a, b| b.1.cmp(&a.1));

        // Limit items to prevent excessive detail at deep levels
        let limit = match depth {
            0 => 1000,
            1 => 500,
            2 => 300,
            3 => 150,
            _ => 75,
        };
        items.truncate(limit);

        // Layout items in this rectangle and recurse into directories
        self.layout_squarify_hierarchical(tree, &items, x, y, w, h, depth);
    }

    /// Build treemap for a specific directory node
    pub fn build_from_node(&mut self, tree: &FileTree, key: &NodeKey) {
        self.rects.clear();
        self.selected = 0;
        self.current_key = *key;

        // Layout children hierarchically
        self.layout_hierarchical(tree, key, 0.0, 0.0, 1.0, 1.0, 0);
    }

    /// Squarified treemap layout with hierarchical recursion into directories
    fn layout_squarify_hierarchical(
        &mut self,
        tree: &FileTree,
        items: &[(String, u64, bool, NodeKey)],
        x: f64,
        y: f64,
        w: f64,
        h: f64,
        depth: usize,
    ) {
        if items.is_empty() || w <= 0.001 || h <= 0.001 {
            return;
        }

        let total: f64 = items.iter().map(|i| i.1 as f64).sum();
        if total <= 0.0 {
            return;
        }

        self.squarify_recursive_hierarchical(tree, items, x, y, w, h, total, depth);
    }

    fn squarify_recursive_hierarchical(
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
        if items.is_empty() || w < 0.001 || h < 0.001 {
            return;
        }

        if items.len() == 1 {
            let item = &items[0];
            
            // Check if this directory will have visible children
            let will_recurse = item.2 && w > 0.02 && h > 0.02;
            
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
                has_visible_children: will_recurse,
            });

            // If it's a directory, recursively layout its children WITHIN this box
            if will_recurse {
                // Reserve some space for borders (1% padding for better space use)
                let padding = 0.01;
                let inner_x = x + w * padding;
                let inner_y = y + h * padding;
                let inner_w = w * (1.0 - 2.0 * padding);
                let inner_h = h * (1.0 - 2.0 * padding);
                
                if inner_w > 0.01 && inner_h > 0.01 {
                    self.layout_hierarchical(tree, &item.3, inner_x, inner_y, inner_w, inner_h, depth + 1);
                }
            }
            return;
        }

        // Determine layout direction
        let vertical = w >= h;
        let short_side = if vertical { h } else { w };
        let long_side = if vertical { w } else { h };

        // Find optimal split point
        let mut best_split = 1;
        let mut best_aspect = f64::MAX;
        let mut running_sum = 0.0;

        for i in 0..items.len() {
            running_sum += items[i].1 as f64;
            let fraction = running_sum / total;
            let strip_length = fraction * long_side;

            let mut worst = 0.0_f64;
            let mut strip_sum = 0.0;
            for j in 0..=i {
                strip_sum += items[j].1 as f64;
            }

            for j in 0..=i {
                let item_fraction = items[j].1 as f64 / strip_sum;
                let item_short = item_fraction * short_side;
                let aspect = if item_short > 0.0 && strip_length > 0.0 {
                    (strip_length / item_short).max(item_short / strip_length)
                } else {
                    f64::MAX
                };
                worst = worst.max(aspect);
            }

            if worst <= best_aspect {
                best_aspect = worst;
                best_split = i + 1;
            } else {
                break;
            }
        }

        // Layout the first strip
        let strip_items = &items[..best_split];
        let strip_total: f64 = strip_items.iter().map(|i| i.1 as f64).sum();
        let strip_fraction = strip_total / total;

        let (strip_x, strip_y, strip_w, strip_h) = if vertical {
            (x, y, strip_fraction * w, h)
        } else {
            (x, y, w, strip_fraction * h)
        };

        // Lay out items within the strip
        let mut pos = 0.0;
        for item in strip_items {
            let item_fraction = item.1 as f64 / strip_total;
            let (ix, iy, iw, ih) = if vertical {
                (strip_x, strip_y + pos * strip_h, strip_w, item_fraction * strip_h)
            } else {
                (strip_x + pos * strip_w, strip_y, item_fraction * strip_w, strip_h)
            };
            pos += item_fraction;

            // Check if this directory will have visible children
            let will_recurse = item.2 && iw > 0.02 && ih > 0.02;

            self.rects.push(TreemapRect {
                name: item.0.clone(),
                size: item.1,
                is_directory: item.2,
                depth,
                x: ix,
                y: iy,
                w: iw,
                h: ih,
                key: item.3,
                has_visible_children: will_recurse,
            });

            // If it's a directory, recursively layout children WITHIN this box
            if will_recurse {
                let padding = 0.01;
                let inner_x = ix + iw * padding;
                let inner_y = iy + ih * padding;
                let inner_w = iw * (1.0 - 2.0 * padding);
                let inner_h = ih * (1.0 - 2.0 * padding);
                
                if inner_w > 0.01 && inner_h > 0.01 {
                    self.layout_hierarchical(tree, &item.3, inner_x, inner_y, inner_w, inner_h, depth + 1);
                }
            }
        }

        // Recurse on remaining items
        if best_split < items.len() {
            let remaining = &items[best_split..];
            let remaining_total = total - strip_total;
            let (rx, ry, rw, rh) = if vertical {
                (x + strip_fraction * w, y, w * (1.0 - strip_fraction), h)
            } else {
                (x, y + strip_fraction * h, w, h * (1.0 - strip_fraction))
            };
            self.squarify_recursive_hierarchical(tree, remaining, rx, ry, rw, rh, remaining_total, depth);
        }
    }

    pub fn move_next(&mut self) {
        if !self.rects.is_empty() {
            self.selected = (self.selected + 1) % self.rects.len();
        }
    }

    pub fn move_prev(&mut self) {
        if !self.rects.is_empty() {
            self.selected = if self.selected == 0 {
                self.rects.len() - 1
            } else {
                self.selected - 1
            };
        }
    }

    pub fn selected_rect(&self) -> Option<&TreemapRect> {
        self.rects.get(self.selected)
    }
}

/// Color for treemap rectangles based on extension or directory
fn treemap_color(name: &str, is_directory: bool, index: usize) -> Color {
    if is_directory {
        // Different blue/teal shades for directories
        let colors = [
            Color::Rgb(30, 80, 120),
            Color::Rgb(40, 90, 110),
            Color::Rgb(50, 70, 130),
            Color::Rgb(35, 100, 100),
            Color::Rgb(60, 85, 115),
            Color::Rgb(45, 75, 140),
            Color::Rgb(55, 95, 105),
            Color::Rgb(25, 85, 125),
        ];
        colors[index % colors.len()]
    } else {
        // Color by extension
        let ext = name.rsplit('.').next().unwrap_or("").to_lowercase();
        match ext.as_str() {
            "exe" | "dll" | "sys" => Color::Rgb(140, 60, 60),
            "zip" | "rar" | "7z" | "gz" | "tar" => Color::Rgb(140, 100, 40),
            "mp4" | "avi" | "mkv" | "mov" | "wmv" => Color::Rgb(100, 40, 130),
            "mp3" | "wav" | "flac" | "ogg" => Color::Rgb(40, 130, 100),
            "jpg" | "jpeg" | "png" | "gif" | "bmp" => Color::Rgb(130, 80, 40),
            "pdf" | "doc" | "docx" | "xls" | "xlsx" => Color::Rgb(60, 100, 60),
            "txt" | "log" | "md" | "cfg" | "ini" => Color::Rgb(90, 90, 90),
            _ => {
                let hue = (index * 47 + 20) % 360;
                hsl_to_rgb(hue as f64, 0.4, 0.35)
            }
        }
    }
}

fn hsl_to_rgb(h: f64, s: f64, l: f64) -> Color {
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

    Color::Rgb(
        ((r + m) * 255.0) as u8,
        ((g + m) * 255.0) as u8,
        ((b + m) * 255.0) as u8,
    )
}

/// Draw the treemap visualization
pub fn draw_treemap(frame: &mut Frame, state: &TreemapState, area: Rect) {
    // Layout: breadcrumb bar (1) + treemap area + info bar (1)
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // Breadcrumb
            Constraint::Min(5),   // Treemap
            Constraint::Length(1), // Info bar
        ])
        .split(area);

    // Draw breadcrumb
    let breadcrumb_text: String = state
        .breadcrumb
        .iter()
        .map(|(_, name)| name.as_str())
        .collect::<Vec<_>>()
        .join(" > ");
    let breadcrumb = Paragraph::new(format!(" {} ", breadcrumb_text))
        .style(Style::default().fg(Color::White).bg(Color::Rgb(40, 40, 60)));
    frame.render_widget(breadcrumb, chunks[0]);

    // Draw treemap rectangles
    let map_area = chunks[1];
    let map_w = map_area.width as f64;
    let map_h = map_area.height as f64;

    for (i, rect) in state.rects.iter().enumerate() {
        // Skip rendering directories that have visible children inside them
        // (the children will render instead, preventing overlap)
        if rect.has_visible_children {
            continue;
        }
        
        let rx = map_area.x + (rect.x * map_w) as u16;
        let ry = map_area.y + (rect.y * map_h) as u16;
        let rw = ((rect.x + rect.w) * map_w) as u16 - (rect.x * map_w) as u16;
        let rh = ((rect.y + rect.h) * map_h) as u16 - (rect.y * map_h) as u16;

        if rw == 0 || rh == 0 {
            continue;
        }

        let cell_area = Rect::new(rx, ry, rw.min(map_area.width - rx + map_area.x), rh.min(map_area.height - ry + map_area.y));
        if cell_area.width == 0 || cell_area.height == 0 {
            continue;
        }

        let is_selected = i == state.selected;
        let bg = if is_selected {
            Color::Rgb(200, 200, 100)
        } else {
            treemap_color(&rect.name, rect.is_directory, i)
        };
        let fg = if is_selected {
            Color::Black
        } else {
            Color::White
        };

        // Smart text rendering based on available space
        let w = cell_area.width;
        let h = cell_area.height;
        
        // Determine what to show based on box size
        let content = if w >= 12 && h >= 2 {
            // Large box: show name + size
            let name_max = w.saturating_sub(1) as usize;
            let name = if rect.name.len() > name_max {
                &rect.name[..name_max]
            } else {
                &rect.name
            };
            let size_str = crate::format_size(rect.size);
            format!("{}\n{}", name, size_str)
        } else if w >= 8 && h >= 1 {
            // Medium box: show name only, truncated
            let max_chars = w.saturating_sub(1) as usize;
            if rect.name.len() > max_chars {
                rect.name[..max_chars].to_string()
            } else {
                rect.name.clone()
            }
        } else if w >= 3 && h >= 1 {
            // Small box: show first few chars
            let max_chars = (w as usize).min(rect.name.len());
            rect.name[..max_chars].to_string()
        } else {
            // Tiny box: no text
            String::new()
        };

        // Adaptive border rendering - only show borders for larger boxes
        if w >= 6 && h >= 3 {
            // Large enough for borders
            let border_style = if is_selected {
                Style::default().fg(Color::Yellow)
            } else {
                Style::default().fg(Color::Rgb(60, 60, 60))
            };
            
            let block = Block::default()
                .borders(Borders::ALL)
                .border_style(border_style);
            let inner = block.inner(cell_area);
            
            frame.render_widget(
                Paragraph::new("").style(Style::default().bg(bg)),
                cell_area,
            );
            frame.render_widget(block, cell_area);
            
            if inner.width > 0 && inner.height > 0 && !content.is_empty() {
                frame.render_widget(
                    Paragraph::new(content).style(Style::default().fg(fg).bg(bg)),
                    inner,
                );
            }
        } else if w >= 2 && h >= 1 {
            // Small box: just colored rectangle with text if it fits
            if is_selected && w >= 2 && h >= 1 {
                // Draw thin border for selection
                let block = Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::Yellow));
                frame.render_widget(block, cell_area);
            }
            
            if !content.is_empty() {
                frame.render_widget(
                    Paragraph::new(content).style(Style::default().fg(fg).bg(bg)),
                    cell_area,
                );
            } else {
                // Just fill with color
                frame.render_widget(
                    Paragraph::new("").style(Style::default().bg(bg)),
                    cell_area,
                );
            }
        } else {
            // Tiny box: just color, no text or borders
            frame.render_widget(
                Paragraph::new("").style(Style::default().bg(bg)),
                cell_area,
            );
        }
    }

    // Draw info bar
    let info_text = if let Some(rect) = state.selected_rect() {
        let icon = if rect.is_directory { "DIR" } else { "FILE" };
        format!(
            " [{} {} - {}] | Arrows/Tab:Navigate Enter:Drill Backspace:Up Esc:Close T:Toggle",
            icon,
            rect.name,
            crate::format_size(rect.size),
        )
    } else {
        " Treemap View | Arrows/Tab:Navigate Enter:Drill Backspace:Up Esc:Close".to_string()
    };

    let info = Paragraph::new(info_text)
        .style(Style::default().fg(Color::White).bg(Color::Rgb(0, 95, 135)));
    frame.render_widget(info, chunks[2]);
}
