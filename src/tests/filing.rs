//! Filing, from the map.

use super::*;

// ---- filing, from the map ----

/// Pick the message of the map with `subject`.
pub(super) fn pick_subject(app: &mut App, subject: &str) {
    let s = app.map.snapshot.clone().unwrap();
    let i = s
        .dots
        .iter()
        .position(|d| d.subject == subject)
        .unwrap_or_else(|| panic!("no {subject}"));
    app.map.cursor = Some(s.dots[i].key.clone());
}

#[test]
fn keys_mean_the_same_on_the_map_as_in_the_list() {
    let mut app = demo_map();
    // r answers the message picked on the map, not the list's.
    pick_subject(&mut app, "Aims page, third pass");
    press(&mut app, &["r"]);
    let text = app.compose.as_ref().expect("written").buffer.text();
    assert!(
        text.contains("Subject: Re: Aims page, third pass"),
        "{text}"
    );
    press(&mut app, &["<c-c>", "<c-k>"]);
    // e archives it, as it archives in the list.
    press(&mut app, &["e"]);
    let (_, inbox) = dot_labels(&app, "Aims page, third pass");
    assert!(!inbox, "{}", app.status);
    // ⌘L, or Super+L, back to the list.
    app.key(KeyEvent::new(KeyCode::Char('l'), KeyModifiers::SUPER));
    assert_eq!(app.map.mode, crate::map::Mode::List);
    let first = app.shown[0].summary.subject.clone();
    press(&mut app, &["e"]);
    assert!(
        app.status.starts_with("archived, out of the Inbox"),
        "{}",
        app.status
    );
    assert!(app.shown.iter().all(|m| m.summary.subject != first));
}

#[test]
fn the_newest_file_of_several_as_new_is_the_first_by_name() {
    let dir = tempfile::tempdir().unwrap();
    let at = std::time::SystemTime::now() - Duration::from_secs(60);
    for name in ["b.txt", "a.txt", "c.txt"] {
        let path = dir.path().join(name);
        std::fs::write(&path, b"x").unwrap();
        std::fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(at)
            .unwrap();
    }
    let p = crate::pick::Picker::open(dir.path().to_path_buf());
    assert_eq!(
        p.current().map(|e| e.name().to_string()).as_deref(),
        Some("a.txt")
    );
}

#[test]
fn space_marks_in_the_picker_after_typing_too() {
    let dir = tempfile::tempdir().unwrap();
    // Each newer than the last, as a clock that tells them apart has it.
    let at = std::time::SystemTime::now() - Duration::from_secs(60);
    for (i, name) in ["one draft.txt", "two draft.txt", "notes.txt"]
        .iter()
        .enumerate()
    {
        let path = dir.path().join(name);
        std::fs::write(&path, b"x").unwrap();
        std::fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(at + Duration::from_secs(i as u64))
            .unwrap();
    }
    let mut p = crate::pick::Picker::open(dir.path().to_path_buf());
    for c in "draft".chars() {
        p.key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
    }
    assert_eq!(p.shown().len(), 3, "..: and the two drafts");
    p.key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE));
    p.key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE));
    assert_eq!(p.filter, "draft");
    let crate::pick::Picked::Take(files) = p.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
    else {
        panic!("none taken");
    };
    let names: Vec<_> = files
        .iter()
        .map(|f| f.file_name().unwrap().to_owned())
        .collect();
    assert_eq!(names, ["one draft.txt", "two draft.txt"]);
}

#[test]
fn a_search_on_the_map_lights_what_holds_it_and_n_goes_through_them() {
    let mut app = demo_map();
    press(&mut app, &["/", "biosketch"]);
    let s = app.map.snapshot.clone().unwrap();
    let found: Vec<usize> = (0..s.dots.len())
        .filter(|&i| app.map.hits.as_ref().unwrap().matched[i])
        .collect();
    assert_eq!(found.len(), 2, "Mira's, and your answer");
    assert!(
        found
            .iter()
            .all(|&i| s.dots[i].subject.to_lowercase().contains("biosketch"))
    );
    // The one picked is one found, as it is typed.
    assert!(found.contains(&app.map.picked().unwrap()));
    let screen = app::screen(&mut app, 150, 45);
    assert!(screen.contains("2 found for “biosketch”"), "{screen}");
    // ⏎ asks no server; n goes to the other, and round again.
    press(&mut app, &["<enter>"]);
    assert!(!app.searching);
    assert_eq!(app.map.mode, crate::map::Mode::Map);
    let first = app.map.picked().unwrap();
    press(&mut app, &["n"]);
    let second = app.map.picked().unwrap();
    assert_ne!(first, second);
    assert!(found.contains(&second));
    press(&mut app, &["n"]);
    assert_eq!(app.map.picked().unwrap(), first);
    // Of two words, both; and esc lets it go.
    press(&mut app, &["/", "<c-u>", "mira budget"]);
    assert_eq!(app.map.hits.as_ref().unwrap().count, 1);
    press(&mut app, &["<enter>", "<esc>"]);
    assert!(app.query.is_empty());
    assert!(app.map.hits.is_none());
}

#[test]
fn another_folder_listed_leaves_the_message_read_behind() {
    let mut app = demo();
    press(&mut app, &["<enter>"]);
    let subject = app.message().expect("read").subject.clone();
    let screen = app::screen(&mut app, 150, 40);
    assert!(screen.contains(&subject), "{screen}");
    // Sent, from the sidebar: the message of the Inbox is not beside it.
    while app.focus != app::Focus::Side {
        press(&mut app, &["h"]);
    }
    press(&mut app, &["j", "j"]);
    assert_eq!(app.folder_name(), "Sent");
    assert!(app.reading.is_none());
    let screen = app::screen(&mut app, 150, 40);
    assert!(!screen.contains(&subject), "{screen}");
}

#[test]
fn the_line_before_the_message_is_dragged_to_make_it_wider() {
    let mut app = demo();
    let divider = |app: &mut App| {
        let _ = app::screen(app, 150, 40);
        app.targets
            .iter()
            .find(|(_, t)| *t == app::Target::Divider)
            .map(|(r, _)| r.x + 1)
            .expect("a line to drag")
    };
    let x = divider(&mut app);
    app.mouse(mouse(MouseEventKind::Down(MouseButton::Left), x, 10));
    app.mouse(mouse(MouseEventKind::Drag(MouseButton::Left), x - 20, 10));
    app.mouse(mouse(MouseEventKind::Up(MouseButton::Left), x - 20, 10));
    assert!(!app.resizing);
    assert_eq!(divider(&mut app), x - 20, "the message twenty wider");
    // The map keeps its width too.
    press(&mut app, &["<m-g>"]);
    assert_eq!(divider(&mut app), x - 20);
    press(&mut app, &["<m-l>"]);
    // Not past what leaves the list room.
    app.mouse(mouse(MouseEventKind::Down(MouseButton::Left), x - 20, 10));
    app.mouse(mouse(MouseEventKind::Drag(MouseButton::Left), 0, 10));
    app.mouse(mouse(MouseEventKind::Up(MouseButton::Left), 0, 10));
    let narrowest = divider(&mut app);
    assert!(
        narrowest >= 24 + 24,
        "past the sidebar, the list 24 wide: {narrowest}"
    );
    // A column beside the line takes it too; < and > move it by four.
    app.mouse(mouse(
        MouseEventKind::Down(MouseButton::Left),
        narrowest + 1,
        10,
    ));
    assert!(app.resizing);
    app.mouse(mouse(
        MouseEventKind::Up(MouseButton::Left),
        narrowest + 1,
        10,
    ));
    // Each as wide as the screen it was pressed on: this one, 150 wide.
    let char_key = |c| KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE);
    app.key(char_key('>'));
    assert_eq!(divider(&mut app), narrowest + 4);
    app.key(char_key('<'));
    assert_eq!(divider(&mut app), narrowest);
}

/// How many of the group `id`'s messages the map shows.
pub(super) fn shown_of(app: &App, id: u32) -> usize {
    let s = app.map.snapshot.as_ref().unwrap();
    let shown = app.map.shown(jiff::Timestamp::now().as_second());
    let g = s.group(id).unwrap();
    g.members.iter().filter(|i| shown.contains(i)).count()
}

/// How many cells of the screen each cluster's block has, by its number,
/// as the clicks it takes say.
pub(super) fn block_areas(app: &App) -> std::collections::BTreeMap<u32, u32> {
    let mut areas = std::collections::BTreeMap::new();
    let mut seen = std::collections::BTreeSet::new();
    for &(r, t) in &app.targets {
        if let app::Target::Anchor(id) = t {
            for x in r.x..r.right() {
                if seen.insert((x, r.y)) {
                    *areas.entry(id).or_default() += 1;
                }
            }
        }
    }
    areas
}

#[test]
fn a_mosaic_gives_each_message_a_cell_and_each_cluster_a_block() {
    use crate::mosaic::Mosaic;
    // Three clusters along the map, 40, 15 and 5 messages each.
    let mut items = Vec::new();
    let centres = [[0.0, 0.0], [10.0, 1.0], [20.0, 0.0]];
    for (b, n) in [40, 15, 5].into_iter().enumerate() {
        for k in 0..n {
            let wobble = (k as f32 * 0.37).sin();
            let p = [
                centres[b][0] + wobble,
                centres[b][1] + (k as f32 * 0.11).cos(),
            ];
            items.push((items.len(), b, p));
        }
    }
    let m = Mosaic::new(&items, &centres, 2.0);
    assert!(m.cols * m.rows >= items.len());
    assert_eq!(m.cells.len(), items.len(), "every message tiled");
    let cell = |p: [f32; 2]| (p[0].floor() as i64, p[1].floor() as i64);
    let cells: std::collections::HashSet<(i64, i64)> = m.cells.iter().map(|c| cell(c.1)).collect();
    assert_eq!(cells.len(), items.len(), "one message to a cell");
    for &(i, p) in &m.cells {
        assert_eq!(m.at(p), Some(items[i].1), "each in its cluster's block");
        assert_eq!(m.held(p), Some(i), "its cell holds it");
    }
    let held = (0..m.cols)
        .flat_map(|x| (0..m.rows).map(move |y| [x as f32 + 0.5, y as f32 + 0.5]))
        .filter(|&p| m.held(p).is_some())
        .count();
    assert_eq!(held, items.len(), "the rest spare");
    // Each block one: each of its cells reached from one, step by step;
    // and as big as how many it holds, or a little bigger.
    let of = |b: usize, (x, y): (i64, i64)| m.at([x as f32 + 0.5, y as f32 + 0.5]) == Some(b);
    let mut middles = Vec::new();
    for (b, n) in [40, 15, 5].into_iter().enumerate() {
        let all: Vec<(i64, i64)> = (0..m.cols as i64)
            .flat_map(|x| (0..m.rows as i64).map(move |y| (x, y)))
            .filter(|&c| of(b, c))
            .collect();
        assert!(
            all.len() >= n && all.len() <= n + n.div_ceil(3) + 2,
            "{b}: {}",
            all.len()
        );
        let (mut seen, mut next) = (std::collections::HashSet::from([all[0]]), vec![all[0]]);
        while let Some((x, y)) = next.pop() {
            for c in [(x + 1, y), (x - 1, y), (x, y + 1), (x, y - 1)] {
                if of(b, c) && seen.insert(c) {
                    next.push(c);
                }
            }
        }
        assert_eq!(seen.len(), all.len(), "cluster {b} in one block");
        middles.push(all.iter().map(|c| c.0 as f32).sum::<f32>() / all.len() as f32);
    }
    // Beside those it lies by: left to right, as on the map.
    assert!(
        middles[0] < middles[1] && middles[1] < middles[2],
        "{middles:?}"
    );
    // The same map, the same mosaic.
    assert_eq!(Mosaic::new(&items, &centres, 2.0).cells, m.cells);
}

#[test]
fn farther_than_all_of_it_the_clusters_are_a_mosaic_each_a_block_as_big_as_it_holds() {
    let mut app = demo_map();
    let screen = app::screen(&mut app, 150, 45);
    assert!(app.map.tree.is_none());
    assert!(screen.contains("its clusters"), "{screen}");
    let s = app.map.snapshot.clone().unwrap();
    let with_mail = s.groups.iter().filter(|g| shown_of(&app, g.id) > 0).count();
    assert_eq!(app.map.blocks.len(), with_mail, "one block each");
    // The more it holds, the bigger, none in another's.
    let areas = block_areas(&app);
    assert_eq!(areas.len(), with_mail, "{areas:?}");
    // Together they fill the map.
    let canvas = map_canvas(&app);
    let filled: u32 = areas.values().sum();
    assert!(
        filled as f32 >= 0.7 * f32::from(canvas.width) * f32::from(canvas.height),
        "{filled} of {canvas:?}"
    );
    for (&a, &area_a) in &areas {
        for (&b, &area_b) in &areas {
            if shown_of(&app, a) >= 2 * shown_of(&app, b) + 2 {
                assert!(area_a > area_b, "{a}: {area_a} cells, {b}: {area_b}");
            }
        }
    }
    // The biggest named once, on its block, with how many it holds.
    let biggest = s
        .groups
        .iter()
        .max_by_key(|g| (shown_of(&app, g.id), std::cmp::Reverse(g.id)))
        .unwrap();
    assert_eq!(screen.matches(&biggest.name()).count(), 1, "{screen}");
    assert!(
        screen.contains(&format!("{} ", shown_of(&app, biggest.id))),
        "{screen}"
    );
    // The arrows go from block to block.
    let group = |app: &App| {
        let p = app.map.picked().unwrap();
        app.map.snapshot.as_ref().unwrap().group_of(p).unwrap().id
    };
    let first = group(&app);
    for key in ["l", "j", "h", "k"] {
        press(&mut app, &[key]);
        if group(&app) != first {
            break;
        }
    }
    let other = group(&app);
    assert_ne!(other, first, "on to another block");
    // ⏎ opens it, as its tree.
    press(&mut app, &["<enter>"]);
    let screen = app::screen(&mut app, 150, 45);
    assert_eq!(app.map.tree.as_ref().map(|t| t.id), Some(other));
    assert_eq!(app.map.focused, crate::map::Focused::Cluster(other));
    let name = s.group(other).unwrap().name();
    assert!(screen.contains(&format!("Map · {name}")), "{screen}");
    assert_eq!(group(&app), other, "a message of it picked");
    // 0, all of it again.
    press(&mut app, &["0"]);
    assert!(app.map.tree.is_none());
}

/// Where the map was drawn: the canvas clicks on it land in.
pub(super) fn map_canvas(app: &App) -> ratatui::layout::Rect {
    app.targets
        .iter()
        .find_map(|&(r, t)| (t == app::Target::Map).then_some(r))
        .unwrap()
}

#[test]
fn a_block_is_clicked_open_and_the_wheel_goes_out_to_the_clusters_and_back() {
    let mut app = demo_map();
    let _ = app::screen(&mut app, 150, 45);
    let canvas = map_canvas(&app);
    let (x, y) = (canvas.x + canvas.width / 2, canvas.y + canvas.height / 2);
    app.mouse(mouse(MouseEventKind::ScrollDown, x, y));
    assert!(app.map.tree.is_none(), "no farther than all of it");
    let _ = app::screen(&mut app, 150, 45);
    let (r, id) = app
        .targets
        .iter()
        .rev()
        .find_map(|&(r, t)| match t {
            app::Target::Anchor(id) => Some((r, id)),
            _ => None,
        })
        .unwrap();
    app.mouse(mouse(MouseEventKind::Down(MouseButton::Left), r.x, r.y));
    assert_eq!(app.map.tree.as_ref().map(|t| t.id), Some(id));
    let p = app.map.picked().unwrap();
    assert!(app.map.snapshot.as_ref().unwrap().picked_by(id, p));
    // Down to its threads: there the wheel goes along them.
    let now = jiff::Timestamp::now().as_second();
    while matches!(
        app.map.level(now),
        Some((_, _, crate::forest::Level::Mosaic(_)))
    ) {
        press(&mut app, &["<enter>"]);
    }
    let _ = app::screen(&mut app, 150, 45);
    let depth = app.map.tree_picked(now).unwrap().1.len();
    app.mouse(mouse(MouseEventKind::ScrollDown, x, y));
    assert_eq!(
        app.map.tree_picked(now).unwrap().1.len(),
        depth,
        "along, not out"
    );
    // Z out to the clusters.
    while app.map.tree.is_some() {
        press(&mut app, &["Z"]);
    }
}

#[test]
fn the_map_is_a_mosaic_alone_its_levels_gone_into_and_out_of_by_zooming() {
    let mut app = demo_map();
    let screen = app::screen(&mut app, 150, 45);
    assert!(app.map.tree.is_none());
    assert!(screen.contains("its clusters"), "{screen}");
    let group = |app: &App| {
        let s = app.map.snapshot.as_ref().unwrap();
        s.group_of(app.map.picked().unwrap()).unwrap().id
    };
    let now = jiff::Timestamp::now().as_second();
    let depth = |app: &mut App| app.map.tree_picked(now).map(|t| t.1.len());
    // z goes into the cluster picked, Grant, and deeper, into its parts;
    // Z out, a level at a time.
    let s = app.map.snapshot.clone().unwrap();
    let grant = s
        .groups
        .iter()
        .position(|g| g.label.as_deref() == Some("Grant"))
        .unwrap();
    app.map.cursor = Some(s.dots[s.central(grant).unwrap()].key.clone());
    let id = group(&app);
    press(&mut app, &["z"]);
    assert_eq!(app.map.tree.as_ref().map(|t| t.id), Some(id));
    assert_eq!(depth(&mut app), Some(1));
    press(&mut app, &["z"]);
    assert_eq!(depth(&mut app), Some(2));
    press(&mut app, &["Z"]);
    assert_eq!(depth(&mut app), Some(1));
    press(&mut app, &["Z"]);
    assert!(app.map.tree.is_none());
    // Farther than all of it: the clusters still, never dots.
    press(&mut app, &["Z", "Z"]);
    let screen = app::screen(&mut app, 150, 45);
    assert!(app.map.tree.is_none());
    assert!(screen.contains("its clusters"), "{screen}");
    assert!(!screen.contains("nearer"), "{screen}");
    // 0, from deep in, all of it.
    press(&mut app, &["z", "z", "0"]);
    assert!(app.map.tree.is_none());
    // The wheel: up over a block goes into it; down, out again, and no
    // farther than all of it.
    let _ = app::screen(&mut app, 150, 45);
    // The biggest other cluster, as it has parts to go into; a small one's
    // threads the wheel goes along.
    let s = app.map.snapshot.clone().unwrap();
    let (r, other) = app
        .targets
        .iter()
        .filter_map(|&(r, t)| match t {
            app::Target::Anchor(g) if g != id => Some((r, g)),
            _ => None,
        })
        .max_by_key(|&(_, g)| s.group(g).map_or(0, |g| g.members.len()))
        .unwrap();
    app.mouse(mouse(MouseEventKind::ScrollUp, r.x, r.y));
    assert_eq!(app.map.tree.as_ref().map(|t| t.id), Some(other));
    // Down over the map as it is drawn now, wherever the block was.
    let on_map = |app: &mut App| {
        let _ = app::screen(app, 150, 45);
        app.targets
            .iter()
            .find_map(|&(r, t)| matches!(t, app::Target::Map).then_some(r))
            .unwrap()
    };
    let r = on_map(&mut app);
    app.mouse(mouse(MouseEventKind::ScrollDown, r.x, r.y));
    assert!(app.map.tree.is_none());
    let r = on_map(&mut app);
    app.mouse(mouse(MouseEventKind::ScrollDown, r.x, r.y));
    assert!(app.map.tree.is_none());
    let screen = app::screen(&mut app, 150, 45);
    assert!(screen.contains("its clusters"), "{screen}");
}

#[test]
fn the_map_follows_the_message_picked_into_its_cluster() {
    let (mut app, grant) = grant_tree();
    let s = app.map.snapshot.clone().unwrap();
    // A message of another cluster picked, as a search's n picks one: its
    // cluster gone into, and it picked still.
    let other = s.groups.iter().position(|g| g.id != grant).unwrap();
    let i = s.central(other).unwrap();
    app.map.cursor = Some(s.dots[i].key.clone());
    let _ = app::screen(&mut app, 150, 45);
    assert_eq!(app.map.picked(), Some(i));
    assert_eq!(
        app.map.tree.as_ref().map(|t| t.id),
        Some(s.groups[other].id)
    );
    // In a cluster, the Inbox looked through: the one read is answered.
    press(&mut app, &["i"]);
    let rows = app.review_rows();
    press(&mut app, &["j", "j", "<enter>", "r"]);
    let text = app.compose.as_ref().unwrap().buffer.text();
    let reply = crate::compose::reply_subject(&s.dots[rows[2]].subject);
    assert!(text.contains(&format!("Subject: {reply}")), "{text}");
}

#[test]
fn a_message_read_is_let_go_once_the_map_has_gone_into_another_cluster() {
    let (mut app, grant) = grant_tree();
    let s = app.map.snapshot.clone().unwrap();
    // Read in another cluster, and out to every cluster: still read.
    let other = s.groups.iter().position(|g| g.id != grant).unwrap();
    let i = s.central(other).unwrap();
    let subject = s.dots[i].subject.clone();
    app.map.cursor = Some(s.dots[i].key.clone());
    read_on_map(&mut app);
    assert!(app::screen(&mut app, 150, 45).contains(&subject));
    app.map.zoom_all();
    assert!(app::screen(&mut app, 150, 45).contains(&subject));
    // Into the Grant: what was read there is out of it, and not shown.
    app.map.open_block(grant);
    let screen = app::screen(&mut app, 150, 45);
    assert!(!screen.contains(&subject), "{screen}");
    assert!(screen.contains("reads the message picked"), "{screen}");
    // One of the Grant's read is shown, as it is in what is shown.
    read_on_map(&mut app);
    let picked = app.map.picked().unwrap();
    let screen = app::screen(&mut app, 150, 45);
    assert!(screen.contains(&s.dots[picked].subject), "{screen}");
    assert!(!screen.contains("reads the message picked"), "{screen}");
}

/// The demo's map, its Grant opened as its tree, drawn.
pub(super) fn grant_tree() -> (App, u32) {
    let mut app = demo_map();
    press(&mut app, &["Z"]);
    let id = app
        .map
        .snapshot
        .as_ref()
        .unwrap()
        .groups
        .iter()
        .find(|g| g.label.as_deref() == Some("Grant"))
        .unwrap()
        .id;
    app.map.open_block(id);
    let _ = app::screen(&mut app, 150, 45);
    (app, id)
}

#[test]
fn a_cluster_opened_is_a_tree_of_its_parts_threads_and_messages_oldest_first() {
    let (mut app, id) = grant_tree();
    let now = jiff::Timestamp::now().as_second();
    let root = app.map.grown(now).unwrap();
    let s = app.map.snapshot.clone().unwrap();
    // Each message of it shown a leaf, once.
    let mut leaves = Vec::new();
    fn gather(b: &crate::forest::Branch, out: &mut Vec<usize>) {
        if b.is_leaf() {
            out.extend(&b.messages);
        }
        for c in &b.branches {
            gather(c, out);
        }
    }
    gather(&root, &mut leaves);
    let shown = app.map.shown(now);
    let mut members: Vec<usize> = s
        .group(id)
        .unwrap()
        .members
        .iter()
        .copied()
        .filter(|i| shown.contains(i))
        .collect();
    members.sort();
    leaves.sort();
    assert_eq!(leaves, members);
    // Parts, at least two of seventeen, and in each branch the oldest first,
    // and the branches by when they began: new mail comes at the end.
    assert!(root.branches.len() >= 2, "{root:?}");
    let when = |i: usize| s.dots[i].date.unwrap_or(s.dots[i].arrived);
    fn check(b: &crate::forest::Branch, when: &dyn Fn(usize) -> i64) {
        assert!(b.messages.windows(2).all(|w| when(w[0]) <= when(w[1])));
        let firsts: Vec<i64> = b.branches.iter().map(|c| when(c.messages[0])).collect();
        assert!(firsts.windows(2).all(|w| w[0] <= w[1]), "{firsts:?}");
        for c in &b.branches {
            check(c, when);
        }
    }
    check(&root, &when);
    // The parts hold, grown again.
    app.map.tree.as_mut().unwrap().depth = 1;
    let again = app.map.grown(now).unwrap();
    assert_eq!(*again, *root);
    let screen = app::screen(&mut app, 150, 45);
    assert!(screen.contains("Map · Grant ·"), "{screen}");
}

#[test]
fn a_big_cluster_is_split_into_parts_and_its_big_parts_again() {
    let (records, _) = made_mail(1500, 7);
    let mut index = Index::memory();
    index
        .cache(
            records
                .into_iter()
                .map(|r| Cached::Upsert(Box::new(r)))
                .collect(),
        )
        .unwrap();
    let s = crate::map::compute(crate::map::input(&index, Lens::Both, None, None, true));
    let all: Vec<usize> = (0..s.dots.len()).collect();
    let biggest = s.groups.iter().max_by_key(|g| g.members.len()).unwrap();
    let (root, _) = crate::forest::grow(&s, biggest.id, &all, None).unwrap();
    // Parts, each of two or more, none of fewer than twelve split; below
    // the finest, threads of messages.
    fn walk(b: &crate::forest::Branch, depth: usize, deepest: &mut usize, leaves: &mut Vec<usize>) {
        if b.is_leaf() {
            leaves.extend(&b.messages);
            return;
        }
        if b.parts {
            assert!(b.branches.len() >= 2, "{}", b.name);
            assert!(b.messages.len() >= 12, "{}: {}", b.name, b.messages.len());
            *deepest = (*deepest).max(depth + 1);
        } else {
            for thread in &b.branches {
                assert!(!thread.parts);
                assert!(
                    thread.branches.iter().all(|m| m.is_leaf()),
                    "{}",
                    thread.name
                );
            }
        }
        for c in &b.branches {
            walk(c, depth + 1, deepest, leaves);
        }
    }
    let (mut deepest, mut leaves) = (0, Vec::new());
    walk(&root, 0, &mut deepest, &mut leaves);
    leaves.sort();
    let mut members = biggest.members.clone();
    members.sort();
    assert_eq!(leaves, members, "each message once");
    assert!(deepest >= 2, "a big part split again: {deepest}");
}

#[test]
fn a_cluster_gone_into_is_a_mosaic_of_its_parts_and_a_part_its_threads() {
    let (mut app, _) = grant_tree();
    let now = jiff::Timestamp::now().as_second();
    let path = |app: &mut App| app.map.tree_picked(now).unwrap().1;
    let root = app.map.grown(now).unwrap();
    assert!(root.parts);
    let screen = app::screen(&mut app, 150, 45);
    assert!(screen.contains("Map · Grant ·"), "{screen}");
    assert_eq!(app.map.blocks.len(), root.branches.len(), "a block each");
    assert!(!screen.contains("├─"), "no tree yet\n{screen}");
    // The arrows go from part to part.
    let first = path(&mut app);
    assert_eq!(first.len(), 1);
    for key in ["l", "j", "h", "k"] {
        press(&mut app, &[key]);
        let _ = app::screen(&mut app, 150, 45);
        if path(&mut app) != first {
            break;
        }
    }
    let other = path(&mut app);
    assert_eq!(other.len(), 1);
    assert_ne!(other, first, "on to another part");
    // ⏎ into a part: its threads, as a tree from it, named above.
    press(&mut app, &["<enter>"]);
    let screen = app::screen(&mut app, 150, 45);
    assert_eq!(path(&mut app)[0], other[0]);
    assert_eq!(path(&mut app).len(), 2);
    let part = &root.branches[other[0]].name;
    assert!(screen.contains(&format!("Grant › {part}")), "{screen}");
    // h out of the threads, to the parts; esc out of them, to the clusters.
    press(&mut app, &["h"]);
    let _ = app::screen(&mut app, 150, 45);
    assert_eq!(path(&mut app), other);
    assert!(!app.map.blocks.is_empty());
    press(&mut app, &["<esc>"]);
    assert!(app.map.tree.is_none());
}

#[test]
fn the_tree_is_gone_along_into_and_out_of_by_keys_and_clicks() {
    let (mut app, _) = grant_tree();
    let now = jiff::Timestamp::now().as_second();
    let path = |app: &mut App| app.map.tree_picked(now).unwrap().1;
    let part = path(&mut app);
    assert_eq!(part.len(), 1, "a part picked");
    // ⏎ into it, its finer parts or its threads; esc out of it.
    press(&mut app, &["<enter>"]);
    let inside = path(&mut app);
    assert_eq!(inside.len(), 2);
    assert_eq!(inside[0], part[0]);
    press(&mut app, &["<esc>"]);
    assert_eq!(path(&mut app), part);
    // Down to a message with ⏎, and it is read.
    for _ in 0..4 {
        press(&mut app, &["<enter>"]);
    }
    let p = app.map.picked().unwrap();
    let subject = app.map.snapshot.as_ref().unwrap().dots[p].subject.clone();
    let screen = app::screen(&mut app, 150, 45);
    assert!(screen.contains(&subject), "{subject}\n{screen}");
    // Back out to the parts: a click on one picks it; on it again, goes
    // into it.
    app.map.tree.as_mut().unwrap().depth = 1;
    let _ = app::screen(&mut app, 150, 45);
    assert_eq!(path(&mut app).len(), 1);
    let row = app.map.branches.iter().position(|b| *b == vec![1]).unwrap();
    click_at(&mut app, app::Target::Branch(row));
    assert_eq!(path(&mut app), vec![1]);
    let _ = app::screen(&mut app, 150, 45);
    let row = app.map.branches.iter().position(|b| *b == vec![1]).unwrap();
    click_at(&mut app, app::Target::Branch(row));
    assert_eq!(path(&mut app).len(), 2, "into it");
    // esc out, and out again: the clusters.
    press(&mut app, &["<esc>", "<esc>"]);
    assert!(app.map.tree.is_none());
    press(&mut app, &["<enter>"]);
    assert!(app.map.tree.is_some());
    press(&mut app, &["<esc>"]);
    assert!(app.map.tree.is_none());
}

#[test]
fn the_clusters_mosaic_stays_as_it_is_while_moving_in_it() {
    let mut app = demo_map();
    press(&mut app, &["Z"]);
    let _ = app::screen(&mut app, 150, 45);
    let before = (app.map.blocks.clone(), block_areas(&app));
    press(&mut app, &["l", "j"]);
    let _ = app::screen(&mut app, 150, 45);
    assert_eq!(before, (app.map.blocks.clone(), block_areas(&app)));
}

#[test]
fn letterbox_opens_as_it_was_left_the_list_or_the_map_and_its_clusters() {
    let dir = tempfile::tempdir().unwrap();
    let look = dir.path().join("look");
    let mut app = demo_map();
    press(&mut app, &["Z"]);
    app.look_file = Some(look.clone());
    app.remember_look();
    assert_eq!(std::fs::read_to_string(&look).unwrap(), "map\n");
    let mut again = demo();
    again.look_file = Some(look.clone());
    again.recall_look();
    assert_eq!(again.map.mode, crate::map::Mode::Map);
    assert!(again.map.tree.is_none());
    // Left at the list, the list.
    press(&mut again, &["<m-l>"]);
    again.remember_look();
    let mut third = demo();
    third.look_file = Some(look);
    third.recall_look();
    assert_eq!(third.map.mode, crate::map::Mode::List);
    // None kept, none written: the tests' and the demo's.
    assert!(demo().look_file.is_none());
}

#[test]
fn the_message_read_opens_as_wide_as_it_was_left() {
    let dir = tempfile::tempdir().unwrap();
    let look = dir.path().join("look");
    let mut app = demo();
    app.look_file = Some(look.clone());
    let _ = app::screen(&mut app, 140, 40);
    // Made wider by a key: written down at once, not only on quitting.
    press(&mut app, &[">"]);
    let wide = app.reader_width.unwrap();
    let mut again = demo();
    again.look_file = Some(look.clone());
    again.recall_look();
    assert_eq!(again.reader_width, Some(wide));
    // Dragged: written down as the drag ends.
    let _ = app::screen(&mut app, 140, 40);
    let x = app.panes_right - wide;
    app.mouse(mouse(MouseEventKind::Down(MouseButton::Left), x, 10));
    app.mouse(mouse(MouseEventKind::Drag(MouseButton::Left), x - 6, 10));
    app.mouse(mouse(MouseEventKind::Up(MouseButton::Left), x - 6, 10));
    let dragged = app.reader_width.unwrap();
    assert_ne!(dragged, wide);
    let mut third = demo();
    third.look_file = Some(look.clone());
    third.recall_look();
    assert_eq!(third.reader_width, Some(dragged));
    // A file written before, of how it was looked at alone, still read.
    std::fs::write(&look, "list\n").unwrap();
    let mut old = demo();
    old.look_file = Some(look);
    old.recall_look();
    assert_eq!(old.reader_width, None);
}

#[test]
fn c_g_goes_back_as_esc_does_on_the_map_and_its_mosaic() {
    let mut app = demo_map();
    press(&mut app, &["Z", "<enter>"]);
    assert!(app.map.tree.is_some());
    press(&mut app, &["<c-g>"]);
    assert!(app.map.tree.is_none(), "out to the clusters");
    // A search let go.
    press(&mut app, &["/", "g", "r", "<c-g>"]);
    assert!(!app.searching);
    assert!(app.query.is_empty());
    // A dialog closed: the help, and what the map shows.
    for open in ["?", ","] {
        press(&mut app, &[open]);
        assert!(app.dialog.is_open(), "{open}");
        press(&mut app, &["<c-g>"]);
        assert!(!app.dialog.is_open(), "{open}");
    }
}

/// The labels of the map's message with `subject`, and whether it is in
/// the Inbox.
pub(super) fn dot_labels(app: &App, subject: &str) -> (Vec<String>, bool) {
    let s = app.map.snapshot.as_ref().unwrap();
    let d = s.dots.iter().find(|d| d.subject == subject).unwrap();
    (d.labels.clone(), d.system.inbox)
}

#[test]
fn m_moves_a_message_onto_a_cluster_and_files_it_under_its_label() {
    let mut app = demo_map();
    pick_subject(&mut app, "Aims page, third pass");
    press(&mut app, &["m"]);
    assert!(app.map.moving.is_some());
    assert!(app.status.contains("moving it"), "{}", app.status);
    // Onto the Grant's cluster, by a message of it, and ⏎.
    pick_subject(&mut app, "Letter of support, signed");
    press(&mut app, &["<enter>"]);
    assert!(app.map.moving.is_none());
    assert!(app.status.contains("under Grant"), "{}", app.status);
    let (labels, inbox) = dot_labels(&app, "Aims page, third pass");
    assert!(labels.contains(&"Grant".to_string()), "{labels:?}");
    assert!(!inbox, "out of the Inbox");
    // The server has it, the next pass says, and it is pending no more.
    assert!(app.index.lock().unwrap().keep.pending.is_empty());
    let s = app.map.snapshot.clone().unwrap();
    let aims = s
        .dots
        .iter()
        .find(|d| d.subject == "Aims page, third pass")
        .unwrap();
    assert!(aims.places.iter().any(|p| p.folder == "Archive"));
    // Its like are suggested, beside it, read: its cluster gone into, down
    // to it.
    read_on_map(&mut app);
    let screen = app::screen(&mut app, 150, 60);
    assert!(screen.contains("Would go to Grant too"), "{screen}");
}

#[test]
fn n_files_what_is_moving_under_a_new_label_named_by_its_commonest_word() {
    let mut app = demo_map();
    let s = app.map.snapshot.clone().unwrap();
    let builds: Vec<crate::index::Key> = s
        .dots
        .iter()
        .filter(|d| d.address == "ci@runner.example")
        .map(|d| d.key.clone())
        .collect();
    app.map.region = Some(crate::map::Region { keys: builds });
    press(&mut app, &["m", "N"]);
    let app::Dialog::Label(typed) = &app.dialog else {
        panic!("not asked: {}", app.status);
    };
    assert_eq!(typed, "Build");
    press(&mut app, &["s", "<enter>"]);
    assert!(
        app.status.contains("3 filed under Builds"),
        "{}",
        app.status
    );
    for subject in [
        "Build failed: letterbox main (clippy)",
        "Build passed: letterbox main",
        "Build failed: faba (test timeout)",
    ] {
        assert_eq!(dot_labels(&app, subject).0, ["Builds"]);
    }
    // A cluster that is a label now, its anchor said so.
    let s = app.map.snapshot.clone().unwrap();
    assert!(
        s.groups
            .iter()
            .any(|g| g.label.as_deref() == Some("Builds"))
    );
}

#[test]
fn filing_suggests_its_like_and_y_and_minus_take_them_or_not() {
    let mut app = demo_map();
    pick_subject(&mut app, "R01 resubmission: internal deadline 3 November");
    press(&mut app, &["m"]);
    // Onto the Grant's anchor, by a click.
    let _ = app::screen(&mut app, 150, 45);
    let s = app.map.snapshot.clone().unwrap();
    let grant = s
        .groups
        .iter()
        .find(|g| g.label.as_deref() == Some("Grant"))
        .unwrap()
        .id;
    click_at(&mut app, app::Target::Anchor(grant));
    assert!(app.status.contains("under Grant"), "{}", app.status);
    let suggested: Vec<String> = {
        let s = app.map.snapshot.clone().unwrap();
        app.suggestions()
            .iter()
            .map(|&(i, _)| s.dots[i].subject.clone())
            .collect()
    };
    assert!(
        suggested.contains(&"Budget justification: the sequencing line".to_string()),
        "{suggested:?}"
    );
    let first = suggested[0].clone();
    press(&mut app, &["y"]);
    assert!(dot_labels(&app, &first).0.contains(&"Grant".to_string()));
    let next = {
        let s = app.map.snapshot.clone().unwrap();
        s.dots[app.suggestions()[0].0].subject.clone()
    };
    press(&mut app, &["-"]);
    assert!(app.status.contains("suggested no more"), "{}", app.status);
    let s = app.map.snapshot.clone().unwrap();
    assert!(
        app.suggestions()
            .iter()
            .all(|&(i, _)| s.dots[i].subject != next)
    );
    assert_eq!(app.index.lock().unwrap().keep.not_it.len(), 1);
    press(&mut app, &["<esc>"]);
    assert!(app.map.suggesting.is_none());
}

#[test]
fn r_renames_the_label_picked_on_its_server() {
    let mut app = demo_map();
    pick_subject(&mut app, "Letter of support, signed");
    press(&mut app, &["R"]);
    let app::Dialog::RenameLabel(old, typed) = &app.dialog else {
        panic!("not asked: {}", app.status);
    };
    assert_eq!((old.as_str(), typed.as_str()), ("Grant", "Grant"));
    press(&mut app, &["<c-u>", "Funding", "<enter>"]);
    assert!(
        app.status.contains("renamed Grant to Funding"),
        "{}",
        app.status
    );
    let (labels, _) = dot_labels(&app, "Letter of support, signed");
    assert_eq!(labels, ["Projects", "Funding"]);
    // Outside the map, R renames an account, as it did.
    press(&mut app, &["<m-l>", "R"]);
    assert!(matches!(app.dialog, app::Dialog::Rename(0, _)));
}
