//! The clusters as a mosaic, as paperstack shows its themes: each message
//! a cell of its own on a grid, each cluster a block of them, beside the
//! clusters it lies by on the map, as recursive bisection partitions a
//! graph. The clusters are split in two halves by where they lie, the grid
//! in two as many cells as each half has messages, and again, till each
//! cluster has its block; then its messages alike, till each has its cell.
//! None hides another, and a block is as big as how many it holds.

/// A mosaic: its grid, each cell's block, and each message's cell.
pub struct Mosaic {
    pub cols: usize,
    pub rows: usize,
    /// The block each cell is in, if any, row by row.
    of: Vec<Option<usize>>,
    /// The message each cell holds, if any, row by row.
    holds: Vec<Option<usize>>,
    /// Each message tiled, and the middle of its cell, across and down.
    pub cells: Vec<(usize, [f32; 2])>,
}

impl Mosaic {
    /// `items`, each a message, its block and where it lies on the map,
    /// tiled on a grid `aspect` times as wide as high; each block where
    /// `centres`, by its number, says it lies.
    pub fn new(items: &[(usize, usize, [f32; 2])], centres: &[[f32; 2]], aspect: f32) -> Mosaic {
        let mut by: Vec<Vec<(usize, [f32; 2])>> = vec![Vec::new(); centres.len()];
        for &(i, b, p) in items {
            if let Some(of) = by.get_mut(b) {
                of.push((i, p));
            }
        }
        let n: usize = by.iter().map(Vec::len).sum();
        // Cells to spare, a fifth more, so cuts go straight across.
        let room = n + n.div_ceil(5);
        let cols = ((room as f32 * aspect.max(f32::MIN_POSITIVE)).sqrt().ceil() as usize).max(1);
        let rows = room.div_ceil(cols);
        let mut out = Mosaic {
            cols,
            rows,
            of: vec![None; cols * rows],
            holds: vec![None; cols * rows],
            cells: Vec::with_capacity(n),
        };
        let mut cells: Vec<[f32; 2]> = (0..rows)
            .flat_map(|r| (0..cols).map(move |c| [c as f32 + 0.5, r as f32 + 0.5]))
            .collect();
        let mut tiles: Vec<(Tile, usize, [f32; 2])> = by
            .iter()
            .enumerate()
            .filter(|(_, of)| !of.is_empty())
            .map(|(b, of)| (Tile::Block(b), of.len(), centres[b]))
            .collect();
        out.tile(&mut tiles, &mut cells, &by);
        out.cells.sort_by_key(|c| c.0);
        out
    }

    /// `tiles`, each as many messages as it weighs and lying where it
    /// does, put in `cells`, as many as they weigh or more: split in two
    /// by where they lie along the way the cells are wider, as are the
    /// cells. A block's cells, those spare too, are its own; its messages,
    /// those `by` lists.
    fn tile(
        &mut self,
        tiles: &mut [(Tile, usize, [f32; 2])],
        cells: &mut [[f32; 2]],
        by: &[Vec<(usize, [f32; 2])>],
    ) {
        match *tiles {
            [] => return,
            // The cell in the middle of those left to it.
            [(Tile::Message(i), _, _)] => {
                let m = mean(cells);
                let d = |c: &[f32; 2]| (c[0] - m[0]).hypot(c[1] - m[1]);
                if let Some(&c) = cells.iter().min_by(|a, b| d(a).total_cmp(&d(b))) {
                    let at = self.index(c);
                    self.holds[at] = Some(i);
                    self.cells.push((i, c));
                }
                return;
            }
            [(Tile::Block(b), _, _)] => {
                for &c in cells.iter() {
                    let at = self.index(c);
                    self.of[at] = Some(b);
                }
                let mut messages: Vec<(Tile, usize, [f32; 2])> = by[b]
                    .iter()
                    .map(|&(i, p)| (Tile::Message(i), 1, p))
                    .collect();
                self.tile(&mut messages, cells, by);
                return;
            }
            _ => {}
        }
        let extent = |k: usize| {
            let (lo, hi) = cells.iter().fold((f32::MAX, f32::MIN), |(lo, hi), c| {
                (lo.min(c[k]), hi.max(c[k]))
            });
            hi - lo
        };
        let wide = usize::from(extent(1) > extent(0));
        let total: usize = tiles.iter().map(|t| t.1).sum();
        let len = cells.len();
        let order = |tiles: &mut [(Tile, usize, [f32; 2])], cells: &mut [[f32; 2]], k: usize| {
            tiles.sort_by(|a, b| a.2[k].total_cmp(&b.2[k]).then(a.0.cmp(&b.0)));
            cells.sort_by(|a, b| a[k].total_cmp(&b[k]).then(a[1 - k].total_cmp(&b[1 - k])));
        };
        // The cut, as `(way, tiles, cells)`: straight, between two lines of
        // cells, with room on each side for its messages; the halves as
        // even, and the cells as near the messages' share, as may be, the
        // wider way first. Blocks are cut either way, so each is one; a
        // block's messages, the wider way alone.
        let mut best: Option<(f64, usize, usize, usize)> = None;
        let ways = if matches!(tiles[0].0, Tile::Block(_)) {
            2
        } else {
            1
        };
        for k in [wide, 1 - wide].into_iter().take(ways) {
            order(tiles, cells, k);
            let lines: Vec<usize> = (1..len)
                .filter(|&i| cells[i][k] != cells[i - 1][k])
                .collect();
            let mut w = 0;
            for cut in 1..tiles.len() {
                w += tiles[cut - 1].1;
                for &share in lines.iter().filter(|&&s| s >= w && len - s >= total - w) {
                    let off = (share as f64 / len as f64 - w as f64 / total as f64).abs()
                        + 0.5 * w.abs_diff(total - w) as f64 / total as f64
                        + if k == wide { 0.0 } else { 0.05 };
                    if best.is_none_or(|b| off < b.0) {
                        best = Some((off, k, cut, share));
                    }
                }
            }
        }
        // None straight: the halves as even, cut within a line.
        let (k, cut, share) = best.map_or_else(
            || {
                order(tiles, cells, wide);
                let (mut cut, mut left, mut off) = (1, 0, usize::MAX);
                let mut w = 0;
                for (i, t) in tiles[..tiles.len() - 1].iter().enumerate() {
                    w += t.1;
                    if w.abs_diff(total - w) < off {
                        (cut, left, off) = (i + 1, w, w.abs_diff(total - w));
                    }
                }
                let even = (len as f64 * left as f64 / total as f64).round() as usize;
                (wide, cut, even.clamp(left, len - (total - left)))
            },
            |(_, k, cut, share)| (k, cut, share),
        );
        order(tiles, cells, k);
        let (a, b) = tiles.split_at_mut(cut);
        let (ca, cb) = cells.split_at_mut(share);
        self.tile(a, ca, by);
        self.tile(b, cb, by);
    }

    /// The block holding the place `p`, across and down, if any.
    pub fn at(&self, p: [f32; 2]) -> Option<usize> {
        *self.of.get(self.place(p)?)?
    }

    /// The message whose cell holds the place `p`, if any.
    pub fn held(&self, p: [f32; 2]) -> Option<usize> {
        *self.holds.get(self.place(p)?)?
    }

    /// The cell holding the place `p`, row by row, if any.
    fn place(&self, p: [f32; 2]) -> Option<usize> {
        let (c, r) = (p[0].floor(), p[1].floor());
        (c >= 0.0 && r >= 0.0 && (c as usize) < self.cols)
            .then(|| r as usize * self.cols + c as usize)
    }

    /// The cell whose middle is `c`, row by row.
    fn index(&self, c: [f32; 2]) -> usize {
        c[1] as usize * self.cols + c[0] as usize
    }
}

/// What `Mosaic::tile` puts in cells: a block's messages, or a message.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Tile {
    Block(usize),
    Message(usize),
}

/// The middle of `points`.
fn mean(points: &[[f32; 2]]) -> [f32; 2] {
    let n = points.len().max(1) as f32;
    let (x, y) = points
        .iter()
        .fold((0.0, 0.0), |(x, y), p| (x + p[0], y + p[1]));
    [x / n, y / n]
}
