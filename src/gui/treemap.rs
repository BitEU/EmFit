use crate::file_tree::{FileTree, NodeKey};
use std::sync::Arc;

// ============================================================================
// Data Structures
// ============================================================================

/// A single rectangle in the treemap.
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
    /// laid out inside it.
    pub children_rendered: bool,
}

/// Persistent state for the treemap view.
pub struct TreemapState {
    pub rects: Vec<TreemapRect>,
    pub selected: usize,
    pub breadcrumb: Vec<(NodeKey, String)>,
    pub current_key: NodeKey,
    pub canvas_w: f64,
    pub canvas_h: f64,
}

impl TreemapState {
    pub fn new() -> Self {
        Self {
            rects: Vec::new(),
            selected: 0,
            breadcrumb: Vec::new(),
            current_key: NodeKey::root(),
            canvas_w: 800.0,
            canvas_h: 600.0,
        }
    }

    pub fn set_canvas_size(&mut self, w: f32, h: f32) {
        self.canvas_w = w.max(10.0) as f64;
        self.canvas_h = h.max(10.0) as f64;
    }

    // ====================================================================
    // Build entry points
    // ====================================================================

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

    pub fn build_from_node(&mut self, tree: &FileTree, key: &NodeKey) {
        self.rects.clear();
        self.selected = 0;
        self.current_key = *key;
        self.layout_children(tree, key, 0.0, 0.0, 1.0, 1.0, 0);
        self.snap_selection();
    }

    // ====================================================================
    // Hierarchical squarified layout (identical algorithm to TUI)
    // ====================================================================

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

        let w_px = w * self.canvas_w;
        let h_px = h * self.canvas_h;

        if w_px < 4.0 || h_px < 4.0 || depth > MAX_DEPTH {
            return;
        }

        let children = tree.get_children(parent_key);
        let mut items: Vec<(String, u64, bool, NodeKey)> = Vec::new();

        for child in &children {
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
            if size > 0 {
                items.push((child.name.clone(), size, child.is_directory, child.key()));
            }
        }
        if items.is_empty() {
            return;
        }

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
        let w_px = w * self.canvas_w;
        let h_px = h * self.canvas_h;

        let can_nest = item.2 && h_px >= 40.0 && w_px >= 60.0 && depth < 8;

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
            let border = 2.0;
            let title_h = 16.0;
            let bx = border / self.canvas_w;
            let by_top = (border + title_h) / self.canvas_h;
            let by_bot = border / self.canvas_h;
            let inner_x = x + bx;
            let inner_y = y + by_top;
            let inner_w = w - 2.0 * bx;
            let inner_h = h - by_top - by_bot;

            if inner_w > 0.0 && inner_h > 0.0 {
                self.layout_children(tree, &item.3, inner_x, inner_y, inner_w, inner_h, depth + 1);
            }
        }
    }

    // ====================================================================
    // Navigation
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
