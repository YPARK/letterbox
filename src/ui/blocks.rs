//! The clusters, or a cluster's parts, as a mosaic: each a block, boxed,
//! its messages each a tile.
use super::*;

/// A block of a mosaic: what a click on it is, what it is called and
/// holds, where it lies on the map, and how it is lit.
pub(super) struct Tile {
    /// Its number among the blocks drawn: a cluster's, or a part's place.
    pub(super) id: u32,
    pub(super) target: Target,
    pub(super) chip: &'static str,
    pub(super) name: String,
    pub(super) members: Vec<usize>,
    pub(super) centre: [f32; 2],
    pub(super) hue: f32,
    /// Its colour, where not its hue: as much of the Inbox goes there.
    pub(super) colour: Option<ratatui::style::Color>,
    /// Whether it is in what the map is narrowed to.
    pub(super) focus: bool,
    /// How much of the Inbox looked through goes there, and whether the
    /// one picked does.
    pub(super) going: usize,
    pub(super) bound: bool,
    /// What it says, only what is not yet filed shown.
    pub(super) unfiled: Option<&'static str>,
}

/// Farther than all of it, the clusters as a mosaic, as paperstack shows
/// its themes: each a block as big as how many of its messages are shown,
/// beside those it lies by on the map.
pub(super) fn cluster_mosaic(
    f: &mut Frame,
    app: &mut App,
    s: &crate::map::Snapshot,
    shown: &[usize],
    canvas: Rect,
    now: i64,
) {
    use crate::map::Focused;
    let Some(at) = laid_out(app, s) else {
        return;
    };
    let heading = app.heading();
    let mut tiles = Vec::new();
    for g in &s.groups {
        let members: Vec<usize> = g
            .members
            .iter()
            .copied()
            .filter(|i| shown.binary_search(i).is_ok())
            .collect();
        if members.is_empty() {
            continue;
        }
        // Where it lies: the middle of its largest part, a label's mail
        // lying, it may be, in several.
        let core: Vec<usize> = g
            .core
            .iter()
            .copied()
            .filter(|i| shown.binary_search(i).is_ok())
            .collect();
        let middle = if core.is_empty() { &members } else { &core };
        tiles.push(Tile {
            id: g.id,
            target: Target::Anchor(g.id),
            chip: if g.label.is_some() { "◆ " } else { "◇ " },
            name: g.name(),
            centre: middle_of(&at, middle),
            hue: g.hue,
            colour: heading.as_ref().map(|h| match h.heat(g.id) {
                heat if heat > 0.0 => heated(&app.theme, heat),
                _ => app.theme.rule,
            }),
            focus: matches!(app.map.focused, Focused::All)
                || app.map.focused == Focused::Cluster(g.id),
            going: heading
                .as_ref()
                .and_then(|h| h.to.get(&g.id))
                .map_or(0, crate::app::Going::all),
            bound: heading
                .as_ref()
                .is_some_and(|h| h.picked == Some(crate::app::Bound::To(g.id))),
            unfiled: app.map.unfiled_only.then_some(if g.label.is_some() {
                "would go here"
            } else {
                "no label yet"
            }),
            members,
        });
    }
    let picked = app.map.picked().and_then(|i| s.group_of(i)).map(|g| g.id);
    mosaic(f, app, s, canvas, &tiles, picked, None, now);
}

/// A cluster gone into, its parts the branch `above` is split in, as a
/// mosaic, the one the way `picked` goes to picked.
#[allow(clippy::too_many_arguments)]
pub(super) fn parts_mosaic(
    f: &mut Frame,
    app: &mut App,
    s: &crate::map::Snapshot,
    root: &crate::forest::Branch,
    picked: &[usize],
    above: &[usize],
    canvas: Rect,
    now: i64,
) {
    let (Some(at), Some(parent)) = (laid_out(app, s), root.at(above)) else {
        return;
    };
    let mut tiles = Vec::new();
    for (k, b) in parent.branches.iter().enumerate() {
        let row = app.map.branches.len();
        app.map.branches.push([above, &[k]].concat());
        // Its hue, the mean of its messages' by angle.
        let (sin, cos) = b.messages.iter().fold((0.0, 0.0), |(x, y), &i| {
            let h = s.hues[i].to_radians();
            (x + h.sin(), y + h.cos())
        });
        tiles.push(Tile {
            id: k as u32,
            target: Target::Branch(row),
            chip: "",
            name: b.name.clone(),
            members: b.messages.clone(),
            centre: middle_of(&at, &b.messages),
            hue: sin.atan2(cos).to_degrees().rem_euclid(360.0),
            colour: None,
            focus: true,
            going: 0,
            bound: false,
            unfiled: None,
        });
    }
    let id = app.map.tree.as_ref().map_or(0, |t| t.id);
    let picked = picked.last().map(|&k| k as u32);
    mosaic(
        f,
        app,
        s,
        canvas,
        &tiles,
        picked,
        Some((id, above.to_vec())),
        now,
    );
}

/// The middle of the messages `members` as laid out at `at`.
pub(super) fn middle_of(at: &[crate::model::Point], members: &[usize]) -> [f32; 2] {
    let k = members.len().max(1) as f32;
    members.iter().fold([0.0, 0.0], |m, &i| {
        [m[0] + at[i][0] / k, m[1] + at[i][1] / k]
    })
}

/// `tiles` as a mosaic, as paperstack shows its themes: each a block,
/// boxed, as big as how many messages it holds and beside those it lies
/// by on the map; each message a half of a cell of its block, bright where
/// it is unread, or found; on each, its name, how many it holds and what
/// is new in it. `picked` is the block picked; `level`, the cluster and
/// the way to the parts shown, if they are a cluster's.
#[allow(clippy::too_many_arguments)]
pub(super) fn mosaic(
    f: &mut Frame,
    app: &mut App,
    s: &crate::map::Snapshot,
    canvas: Rect,
    tiles: &[Tile],
    picked: Option<u32>,
    level: Option<(u32, Vec<usize>)>,
    now: i64,
) {
    use crate::mosaic::Mosaic;
    use crate::theme::mix;
    let t = app.theme;
    if tiles.is_empty() || canvas.width == 0 || canvas.height == 0 {
        return;
    }
    let query = app.query.clone();
    let found = app.map.hits(&query).map(|h| h.matched.clone());
    let heading = app.heading();
    // What each holds that is unread, new, and found.
    let count = |m: &[usize], keep: &dyn Fn(usize) -> bool| m.iter().filter(|&&i| keep(i)).count();
    let unread: Vec<usize> = tiles
        .iter()
        .map(|b| count(&b.members, &|i| !s.dots[i].flags.seen))
        .collect();
    let new: Vec<usize> = tiles
        .iter()
        .map(|b| count(&b.members, &|i| app.map.is_new(&s.dots[i], now)))
        .collect();
    let hits: Vec<Option<usize>> = tiles
        .iter()
        .map(|b| found.as_ref().map(|m| count(&b.members, &|i| m[i])))
        .collect();
    // What each holds that is marked, to act on all of it.
    let region: std::collections::HashSet<usize> = app
        .map
        .region
        .iter()
        .flat_map(|r| r.keys.iter().filter_map(|k| s.find(k)))
        .collect();
    let marks: Vec<usize> = tiles
        .iter()
        .map(|b| count(&b.members, &|i| region.contains(&i)))
        .collect();
    let lit: Vec<bool> = (0..tiles.len())
        .map(|k| tiles[k].focus && hits[k] != Some(0) && !(app.map.new_only && new[k] == 0))
        .collect();
    // Tiled again only when what is shown, or the canvas, has changed: a
    // message to a half of a cell, as square as a terminal's cells halved.
    let Some(at) = laid_out(app, s) else {
        return;
    };
    let items: Vec<(usize, usize, [f32; 2])> = tiles
        .iter()
        .enumerate()
        .flat_map(|(k, b)| {
            let at = &at;
            b.members.iter().map(move |&i| (i, k, at[i]))
        })
        .collect();
    let tiled = (
        s as *const crate::map::Snapshot as usize,
        items.iter().map(|it| it.0).collect::<Vec<_>>(),
        canvas.width,
        canvas.height,
        level,
        app.map.preview.as_ref().map(|p| p.as_ptr() as usize),
    );
    let mosaic = match &app.map.mosaic {
        Some((was, m)) if *was == tiled => m.clone(),
        _ => {
            let aspect = f32::from(canvas.width) / (2.0 * f32::from(canvas.height));
            let centres: Vec<[f32; 2]> = tiles.iter().map(|b| b.centre).collect();
            let m = Arc::new(Mosaic::new(&items, &centres, aspect));
            app.map.mosaic = Some((tiled, m.clone()));
            m
        }
    };
    // Each half cell's block, a row of them to each half row.
    let (w, h) = (usize::from(canvas.width), 2 * usize::from(canvas.height));
    let (cols, rows) = (mosaic.cols as f32, mosaic.rows as f32);
    let block_of: Vec<Option<usize>> = (0..h)
        .flat_map(|half| (0..w).map(move |col| (col, half)))
        .map(|(col, half)| {
            mosaic.at([
                (col as f32 + 0.5) / w as f32 * cols,
                (half as f32 + 0.5) / h as f32 * rows,
            ])
        })
        .collect();
    let block_at = |col: i64, half: i64| {
        ((0..w as i64).contains(&col) && (0..h as i64).contains(&half))
            .then(|| block_of[half as usize * w + col as usize])
            .flatten()
    };
    // Each message's own tile, read or not: where a cell of the grid
    // spans a few of the screen's, with a gutter at its right and foot;
    // where it spans less, each message unread, or, searching, found, at
    // least the half cell its middle is in.
    let lit_up = |i: usize| found.as_ref().map_or(!s.dots[i].flags.seen, |m| m[i]);
    let mut marked = vec![false; w * h];
    for &(i, p) in &mosaic.cells {
        let col = ((p[0] / cols * w as f32) as usize).min(w - 1);
        let half = ((p[1] / rows * h as f32) as usize).min(h - 1);
        if lit_up(i) {
            marked[half * w + col] = true;
        }
    }
    let grid = |col: i64, half: i64| {
        [
            ((col as f32 + 0.5) / w as f32 * cols).floor(),
            ((half as f32 + 0.5) / h as f32 * rows).floor(),
        ]
    };
    let gutters = w as f32 / cols >= 3.0 && h as f32 / rows >= 3.0;
    // Of a half cell: none, a message read, one lit up, or the one picked.
    let picked_one = app.map.picked();
    let tone = |col: i64, half: i64| -> u8 {
        if marked[half as usize * w + col as usize] {
            return 2;
        }
        let at = grid(col, half);
        let Some(i) = mosaic.held([at[0] + 0.5, at[1] + 0.5]) else {
            return 0;
        };
        if gutters && (grid(col + 1, half)[0] != at[0] || grid(col, half + 1)[1] != at[1]) {
            return 0;
        }
        match () {
            _ if Some(i) == picked_one => 3,
            _ if lit_up(i) => 2,
            _ => 1,
        }
    };
    // Each block's colour: its own, or its hue; grey where it is not lit.
    // Its cells in a faint pastel of it, less faint picked, its edge a box
    // in a paler one, and what is unread, or found, in the colour itself.
    let paper = t.ground();
    let strong: Vec<_> = tiles
        .iter()
        .zip(&lit)
        .map(|(b, &lit)| match b.colour {
            _ if !lit => t.rule,
            Some(c) => c,
            None => t.hue(b.hue),
        })
        .collect();
    let fills: Vec<_> = tiles
        .iter()
        .zip(&strong)
        .map(|(b, &c)| mix(c, paper, if picked == Some(b.id) { 0.6 } else { 0.82 }))
        .collect();
    let reads: Vec<_> = tiles
        .iter()
        .zip(&strong)
        .map(|(b, &c)| mix(c, paper, if picked == Some(b.id) { 0.45 } else { 0.7 }))
        .collect();
    let edges: Vec<_> = strong
        .iter()
        .zip(&marks)
        .map(|(&c, &m)| if m > 0 { t.title } else { mix(c, paper, 0.4) })
        .collect();
    let colour = |col: i64, half: i64| {
        let Some(b) = block_at(col, half) else {
            return t.page;
        };
        let inner = [(-1, 0), (1, 0), (0, -1), (0, 1)]
            .iter()
            .all(|(dx, dy)| block_at(col + dx, half + dy) == Some(b));
        match tone(col, half) {
            _ if !inner => edges[b],
            3 => t.accent,
            2 => strong[b],
            1 => reads[b],
            _ => fills[b],
        }
    };
    // Each block's bounds on the screen, and the runs of it on each row,
    // to be clicked.
    let mut bounds: Vec<Option<[u16; 4]>> = vec![None; tiles.len()];
    let mut runs: Vec<(Rect, usize)> = Vec::new();
    let buf = f.buffer_mut();
    for row in 0..canvas.height {
        let mut run: Option<(u16, u16, usize)> = None;
        for col in 0..canvas.width {
            let (c, half) = (i64::from(col), 2 * i64::from(row));
            let (top, low) = (colour(c, half), colour(c, half + 1));
            let (x, y) = (canvas.x + col, canvas.y + row);
            let cell = &mut buf[(x, y)];
            // A half beyond the blocks as the page, never in the
            // terminal's own colour for text, as a half block would be.
            match (top == t.page, low == t.page) {
                (true, true) => cell.set_symbol(" "),
                (true, false) => cell.set_symbol("▄").set_fg(low),
                _ => cell.set_symbol("▀").set_fg(top),
            };
            cell.set_bg(if top == t.page { t.page } else { low });
            let Some(b) = block_at(c, half + 1).or(block_at(c, half)) else {
                continue;
            };
            let r = bounds[b].get_or_insert([x, x, y, y]);
            *r = [r[0].min(x), r[1].max(x), r[2].min(y), r[3].max(y)];
            match &mut run {
                Some((_, end, rb)) if *rb == b && *end + 1 == x => *end = x,
                _ => {
                    if let Some((a, e, rb)) = run.take() {
                        runs.push((Rect::new(a, y, e - a + 1, 1), rb));
                    }
                    run = Some((x, x, b));
                }
            }
        }
        if let Some((a, e, rb)) = run {
            runs.push((Rect::new(a, canvas.y + row, e - a + 1, 1), rb));
        }
    }
    for (r, b) in runs {
        app.targets.push((r, tiles[b].target));
    }
    app.map.blocks = tiles
        .iter()
        .zip(&bounds)
        .filter_map(|(b, r)| {
            let [x0, x1, y0, y1] = (*r)?;
            let middle = |a: u16, z: u16| (f32::from(a) + f32::from(z) + 1.0) / 2.0;
            Some((
                b.id,
                [middle(x0, x1), middle(y0, y1)],
                f32::from(x1 - x0 + 1) / 2.0,
            ))
        })
        .collect();
    // On each block, inside its box, its name, then how many it holds and
    // what is new in it; on one line where there is not room for two.
    for (k, (b, r)) in tiles.iter().zip(&bounds).enumerate() {
        let Some([x0, x1, y0, y1]) = *r else {
            continue;
        };
        let (bw, bh) = (x1 - x0 + 1, y1 - y0 + 1);
        let width = if bw > 2 { bw - 2 } else { bw };
        if width < 3 {
            continue;
        }
        let (x, y) = (
            if bw > 2 { x0 + 1 } else { x0 },
            if bh > 2 { y0 + 1 } else { y0 },
        );
        let mut said = format!("{}", b.members.len());
        match hits[k] {
            _ if b.going > 0 => said.push_str(&format!(" ← {} of the Inbox", b.going)),
            _ if marks[k] == b.members.len() => said.push_str(" · all marked"),
            _ if marks[k] > 0 => said.push_str(&format!(" · {} marked", marks[k])),
            Some(h) if h > 0 => said.push_str(&format!(" · {h} found")),
            _ if b.unfiled.is_some() => said.push_str(&format!(" · {}", b.unfiled.unwrap_or(""))),
            _ if new[k] > 0 => said.push_str(&format!(" · {} new", new[k])),
            _ if unread[k] > 0 => said.push_str(&format!(" · {} unread", unread[k])),
            _ => {}
        }
        let lines = if y + 1 < y1 {
            vec![format!(" {}{} ", b.chip, b.name), format!(" {said} ")]
        } else {
            vec![format!(" {}{} {said} ", b.chip, b.name)]
        };
        let style = if b.bound {
            // Where the one picked goes.
            Style::new().fg(t.ground()).bg(t.flag).bold()
        } else if picked == Some(b.id) {
            Style::new().fg(t.title).bg(t.focus).bold()
        } else if heading.is_some() && b.going > 0 {
            Style::new().fg(t.flag).bg(fills[k]).bold()
        } else if !lit[k] {
            Style::new().fg(t.dim).bg(fills[k])
        } else if unread[k] > 0 {
            Style::new().fg(t.title).bg(fills[k]).bold()
        } else {
            Style::new().fg(t.soft).bg(fills[k])
        };
        for (row, line) in (y..).zip(&lines) {
            let text = clip(line, usize::from(width));
            let spot = Rect::new(x, row, text.width() as u16, 1);
            f.render_widget(Paragraph::new(Span::styled(text, style)), spot);
            app.targets.push((spot, b.target));
        }
    }
}
