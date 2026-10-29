//! Squarified treemap layout (Bruls, Huizing, van Wijk): rectangles whose
//! areas match the sizes, kept as close to squares as the sizes allow.

use super::{NodeId, NodeKind, UsageTree};

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Self { x, y, w, h }
    }

    pub fn area(&self) -> f32 {
        self.w.max(0.0) * self.h.max(0.0)
    }

    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x && x < self.x + self.w && y >= self.y && y < self.y + self.h
    }
}

/// Rectangles for `sizes` (largest first) that together fill `area`, in the
/// same order. Each one's area is its share of the total.
pub fn squarify(sizes: &[u64], area: Rect) -> Vec<Rect> {
    let mut out = vec![Rect::new(area.x, area.y, 0.0, 0.0); sizes.len()];
    let total: u64 = sizes.iter().sum();
    if total == 0 || area.area() <= 0.0 {
        return out;
    }
    let scale = f64::from(area.w) * f64::from(area.h) / total as f64;
    let areas: Vec<f64> = sizes.iter().map(|&s| s as f64 * scale).collect();
    let (mut x, mut y, mut w, mut h) = (
        f64::from(area.x),
        f64::from(area.y),
        f64::from(area.w),
        f64::from(area.h),
    );
    let mut i = 0;
    while i < areas.len() {
        // Lay a row along the shorter side, adding items while that makes
        // its worst aspect ratio better.
        let side = w.min(h);
        let mut end = i + 1;
        let mut sum = areas[i];
        let mut worst = worst_ratio(&areas[i..end], sum, side);
        while end < areas.len() {
            let next = sum + areas[end];
            let ratio = worst_ratio(&areas[i..=end], next, side);
            if ratio > worst {
                break;
            }
            sum = next;
            worst = ratio;
            end += 1;
        }
        let last = end == areas.len();
        if w >= h {
            // A column on the left; the last one takes what is left.
            let thick = if last { w } else { sum / h };
            let mut at = y;
            for k in i..end {
                let len = if k + 1 == end {
                    y + h - at
                } else {
                    areas[k] / thick
                };
                out[k] = rect(x, at, thick, len);
                at += len;
            }
            x += thick;
            w -= thick;
        } else {
            // A row at the top.
            let thick = if last { h } else { sum / w };
            let mut at = x;
            for k in i..end {
                let len = if k + 1 == end {
                    x + w - at
                } else {
                    areas[k] / thick
                };
                out[k] = rect(at, y, len, thick);
                at += len;
            }
            y += thick;
            h -= thick;
        }
        i = end;
    }
    out
}

fn rect(x: f64, y: f64, w: f64, h: f64) -> Rect {
    Rect::new(x as f32, y as f32, w.max(0.0) as f32, h.max(0.0) as f32)
}

/// The worst aspect ratio of a row of `row` (largest first, summing to `sum`)
/// laid along `side`.
fn worst_ratio(row: &[f64], sum: f64, side: f64) -> f64 {
    let (max, min) = (row[0], row[row.len() - 1]);
    if min <= 0.0 || sum <= 0.0 {
        return f64::INFINITY;
    }
    let (s2, w2) = (sum * sum, side * side);
    (w2 * max / s2).max(s2 / (w2 * min))
}

/// How many of `sizes` (largest first) get a tile of their own: those of at
/// least `min_share` of the total, `max` at most. The rest are shown as one
/// "Other" tile (never a single item: that one gets its own tile).
pub fn split_small(sizes: &[u64], min_share: f64, max: usize) -> usize {
    let total: u64 = sizes.iter().sum();
    let min = total as f64 * min_share;
    let keep = sizes
        .iter()
        .take(max)
        .take_while(|&&s| s > 0 && s as f64 >= min)
        .count();
    let left = sizes[keep..].iter().filter(|&&s| s > 0).count();
    if left == 1 {
        keep + 1
    } else {
        keep
    }
}

/// How deep and how fine the treemap is.
#[derive(Debug, Clone, Copy)]
pub struct LayoutOptions {
    /// Levels of folders shown inside each other (1: only the items of the folder).
    pub levels: u8,
    /// Height of the strip at the top of a folder with its items inside (for its name).
    pub header: f32,
    /// Space around the items inside a folder.
    pub pad: f32,
    /// Items smaller than this (in square pixels) go into "Other".
    pub min_area: f32,
    /// Most tiles of their own per folder.
    pub max_tiles: usize,
}

impl Default for LayoutOptions {
    fn default() -> Self {
        Self {
            levels: 3,
            header: 16.0,
            pad: 2.0,
            min_area: 200.0,
            max_tiles: 200,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TileItem {
    Node(NodeId),
    /// The small items of a folder, together.
    Other {
        parent: NodeId,
        bytes: u64,
        count: usize,
    },
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Tile {
    pub item: TileItem,
    pub rect: Rect,
    /// 0 for the items of the folder shown, 1 for theirs, …
    pub depth: u8,
    /// Which of the shown folder's items this tile is in (for colours).
    pub top: usize,
    /// Its items are drawn inside it (below the header strip).
    pub nested: bool,
}

impl UsageTree {
    /// The treemap of folder `id` in `area`: parents come before the tiles
    /// inside them, so the last tile under a point is the innermost one.
    pub fn layout(&self, id: NodeId, area: Rect, opts: &LayoutOptions) -> Vec<Tile> {
        let mut out = Vec::new();
        self.layout_into(id, area, 0, None, opts, &mut out);
        out
    }

    fn layout_into(
        &self,
        id: NodeId,
        area: Rect,
        depth: u8,
        top: Option<usize>,
        opts: &LayoutOptions,
        out: &mut Vec<Tile>,
    ) {
        let kids: Vec<NodeId> = self
            .largest_children(id)
            .into_iter()
            .filter(|&k| self.node(k).bytes > 0)
            .collect();
        if kids.is_empty() || area.area() <= 0.0 {
            return;
        }
        let sizes: Vec<u64> = kids.iter().map(|&k| self.node(k).bytes).collect();
        let min_share = f64::from(opts.min_area) / f64::from(area.area());
        let keep = split_small(&sizes, min_share, opts.max_tiles);
        let mut weights = sizes[..keep].to_vec();
        let other: u64 = sizes[keep..].iter().sum();
        if other > 0 {
            weights.push(other);
        }
        for (i, r) in squarify(&weights, area).into_iter().enumerate() {
            let top = top.unwrap_or(i);
            if i == keep {
                out.push(Tile {
                    item: TileItem::Other {
                        parent: id,
                        bytes: other,
                        count: kids.len() - keep,
                    },
                    rect: r,
                    depth,
                    top,
                    nested: false,
                });
                continue;
            }
            let node = kids[i];
            let inner = Rect::new(
                r.x + opts.pad,
                r.y + opts.header,
                r.w - 2.0 * opts.pad,
                r.h - opts.header - opts.pad,
            );
            let nested = self.node(node).kind == NodeKind::Folder
                && depth + 1 < opts.levels
                && inner.w >= 8.0
                && inner.h >= 8.0
                && inner.area() >= opts.min_area * 2.0;
            out.push(Tile {
                item: TileItem::Node(node),
                rect: r,
                depth,
                top,
                nested,
            });
            if nested {
                self.layout_into(node, inner, depth + 1, Some(top), opts, out);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::ROOT;
    use super::*;

    const EPS: f32 = 0.05;

    fn overlap(a: &Rect, b: &Rect) -> f32 {
        let w = (a.x + a.w).min(b.x + b.w) - a.x.max(b.x);
        let h = (a.y + a.h).min(b.y + b.h) - a.y.max(b.y);
        w.max(0.0) * h.max(0.0)
    }

    fn inside(r: &Rect, area: &Rect) -> bool {
        r.x >= area.x - EPS
            && r.y >= area.y - EPS
            && r.x + r.w <= area.x + area.w + EPS
            && r.y + r.h <= area.y + area.h + EPS
    }

    /// Fills the area, areas proportional to the sizes, no overlaps.
    fn check(sizes: &[u64], area: Rect) {
        let rects = squarify(sizes, area);
        assert_eq!(rects.len(), sizes.len());
        let total: u64 = sizes.iter().sum();
        let covered: f32 = rects.iter().map(Rect::area).sum();
        assert!(
            (covered - area.area()).abs() < area.area() * 1e-4,
            "fills the area"
        );
        for (r, &s) in rects.iter().zip(sizes) {
            assert!(inside(r, &area), "{r:?} is outside {area:?}");
            let want = area.area() * s as f32 / total as f32;
            assert!(
                (r.area() - want).abs() <= want * 1e-3 + 0.5,
                "{r:?} for {s}"
            );
        }
        for (i, a) in rects.iter().enumerate() {
            for b in &rects[i + 1..] {
                assert!(overlap(a, b) < 0.5, "{a:?} overlaps {b:?}");
            }
        }
    }

    #[test]
    fn squarify_fills_the_rect_proportionally() {
        check(&[6, 6, 4, 3, 2, 2, 1], Rect::new(0.0, 0.0, 600.0, 400.0));
        check(&[1], Rect::new(10.0, 20.0, 30.0, 40.0));
        check(&[5, 5], Rect::new(0.0, 0.0, 100.0, 300.0));
        // Many uneven sizes (a simple pseudo-random sequence).
        let mut x: u64 = 12345;
        let mut sizes: Vec<u64> = (0..300)
            .map(|_| {
                x = x
                    .wrapping_mul(6364136223846793005)
                    .wrapping_add(1442695040888963407);
                (x >> 40) % 100_000 + 1
            })
            .collect();
        sizes.sort_unstable_by(|a, b| b.cmp(a));
        check(&sizes, Rect::new(5.0, 5.0, 1200.0, 700.0));
    }

    #[test]
    fn equal_sizes_make_squares() {
        let rects = squarify(&[1, 1, 1, 1], Rect::new(0.0, 0.0, 100.0, 100.0));
        for r in rects {
            assert!(
                (r.w - 50.0).abs() < EPS && (r.h - 50.0).abs() < EPS,
                "{r:?}"
            );
        }
    }

    #[test]
    fn nothing_to_lay_out() {
        assert!(squarify(&[], Rect::new(0.0, 0.0, 10.0, 10.0)).is_empty());
        let r = squarify(&[0, 0], Rect::new(0.0, 0.0, 10.0, 10.0));
        assert!(r.iter().all(|r| r.area() == 0.0));
    }

    #[test]
    fn tiny_items_are_grouped() {
        assert_eq!(split_small(&[1000, 500, 3, 2, 1], 0.01, 100), 2);
        // A single small one keeps its own tile.
        assert_eq!(split_small(&[1000, 500, 3], 0.01, 100), 3);
        assert_eq!(split_small(&[10, 10, 10, 10], 0.01, 2), 2);
        assert_eq!(split_small(&[], 0.01, 10), 0);
    }

    /// root: big (folder: x 600, y 300), file 100, and 50 files of 1 byte.
    fn sample() -> UsageTree {
        let mut t = UsageTree::new("/r".into());
        let big = t.add(ROOT, "big", NodeKind::Folder, 0, 0);
        let x = t.add(big, "x", NodeKind::File, 600, 1);
        let y = t.add(big, "y", NodeKind::File, 300, 1);
        t.grow(big, 900, 2);
        let _ = (x, y);
        t.add(ROOT, "file", NodeKind::File, 100, 1);
        for i in 0..50 {
            t.add(ROOT, &format!("tiny{i}"), NodeKind::File, 1, 1);
        }
        t.grow(ROOT, 150, 51);
        t
    }

    #[test]
    fn nested_layout() {
        let t = sample();
        assert_eq!(t.node(ROOT).bytes, 1050);
        let area = Rect::new(0.0, 0.0, 200.0, 150.0);
        let opts = LayoutOptions::default();
        let tiles = t.layout(ROOT, area, &opts);
        let top: Vec<_> = tiles.iter().filter(|t| t.depth == 0).collect();
        // big, file and the 50 tiny files as "Other".
        assert_eq!(top.len(), 3);
        assert!(matches!(
            top[2].item,
            TileItem::Other {
                bytes: 50,
                count: 50,
                ..
            }
        ));
        let covered: f32 = top.iter().map(|t| t.rect.area()).sum();
        assert!((covered - area.area()).abs() < 1.0);
        // The folder's files are inside it, below its header.
        let big = top[0];
        assert!(big.nested);
        let inner: Vec<_> = tiles.iter().filter(|t| t.depth == 1).collect();
        assert_eq!(inner.len(), 2);
        for tile in inner {
            assert_eq!(tile.top, 0);
            assert!(inside(&tile.rect, &big.rect));
            assert!(tile.rect.y >= big.rect.y + opts.header - EPS);
        }
        // One level only: nothing nested.
        let flat = t.layout(ROOT, area, &LayoutOptions { levels: 1, ..opts });
        assert!(flat.iter().all(|t| t.depth == 0 && !t.nested));
    }
}
