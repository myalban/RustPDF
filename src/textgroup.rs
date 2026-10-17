//! Grouping of the text pieces of a PDF (often one object per word) into lines then paragraphs.

use crate::geom::{R4, r4_union};

/// A raw piece of text of the PDF (a text object).
#[derive(Clone, Debug)]
pub struct Run {
    pub idx: usize,
    pub text: String,
    pub font: String,
    pub size: f32,
    pub color: [u8; 3],
    /// Baseline ordinate.
    pub base: f32,
    pub rect: R4,
    /// Leader dots / rules: never merged.
    pub leader: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Align {
    Left,
    Center,
    Right,
    Justify,
}

/// A line of text: several pieces of the same style on the same baseline.
#[derive(Clone, Debug)]
pub struct Line {
    pub runs: Vec<Run>,
    pub rect: R4,
    pub text: String,
    pub base: f32,
    /// Line belonging to a tabular layout (table of contents, table…): never merged.
    pub single: bool,
}

impl Line {
    pub fn first(&self) -> &Run {
        &self.runs[0]
    }
    pub fn members(&self) -> Vec<usize> {
        self.runs.iter().map(|r| r.idx).collect()
    }
}

/// An editable text block: one or several lines.
#[derive(Clone, Debug)]
pub struct Block {
    pub lines: Vec<Line>,
    pub rect: R4,
    /// Text of the block (the lines of a paragraph are joined by a space).
    pub text: String,
    pub align: Align,
}

impl Block {
    pub fn members(&self) -> Vec<usize> {
        self.lines.iter().flat_map(|l| l.members()).collect()
    }
    pub fn leader_idx(&self) -> usize {
        self.lines[0].runs[0].idx
    }
    pub fn first_run(&self) -> &Run {
        &self.lines[0].runs[0]
    }
}

fn same_style(a: &Run, b: &Run) -> bool {
    a.font == b.font && a.color == b.color && (a.size - b.size).abs() <= 0.3
}

/// Groups the pieces of a same line (same font, size, color and baseline).
pub fn group_lines(mut runs: Vec<Run>) -> Vec<Line> {
    runs.sort_by(|a, b| a.rect[0].partial_cmp(&b.rect[0]).unwrap_or(std::cmp::Ordering::Equal));
    let mut groups: Vec<Vec<Run>> = vec![];
    let mut rects: Vec<R4> = vec![];
    for r in runs {
        let mut target = None;
        if !r.leader {
            for (bi, g) in groups.iter().enumerate() {
                let f = &g[0];
                if f.leader || !same_style(f, &r) || (f.base - r.base).abs() > 0.3 * r.size.max(1.0) {
                    continue;
                }
                let gap = r.rect[0] - rects[bi][2];
                if gap > -r.size && gap < r.size * 1.0 {
                    target = Some(bi);
                }
            }
        }
        match target {
            Some(bi) => {
                rects[bi] = r4_union(&rects[bi], &r.rect);
                groups[bi].push(r);
            }
            None => {
                rects.push(r.rect);
                groups.push(vec![r]);
            }
        }
    }
    let mut lines: Vec<Line> = groups
        .into_iter()
        .zip(rects)
        .map(|(g, rect)| {
            let mut text = String::new();
            let mut prev_right: Option<f32> = None;
            for r in &g {
                if let Some(pr) = prev_right {
                    if r.rect[0] - pr > 0.15 * r.size && !text.ends_with(' ') && !r.text.starts_with(' ') {
                        text.push(' ');
                    }
                }
                text.push_str(&r.text);
                prev_right = Some(r.rect[2]);
            }
            let base = g[0].base;
            Line { runs: g, rect, text, base, single: false }
        })
        .collect();
    mark_tabular(&mut lines);
    lines
}

/// Marks as "single" the lines followed, on the same baseline, by a leader or
/// another very close text (tab, table cell).
fn mark_tabular(lines: &mut [Line]) {
    let snapshot: Vec<(R4, f32, bool, f32)> =
        lines.iter().map(|l| (l.rect, l.base, l.first().leader, l.first().size)).collect();
    for (i, l) in lines.iter_mut().enumerate() {
        let size = l.first().size.max(1.0);
        if l.first().leader {
            l.single = true;
            continue;
        }
        for (j, (r, base, leader, _)) in snapshot.iter().enumerate() {
            if i == j || (base - l.base).abs() > 0.3 * size {
                continue;
            }
            let gap = r[0] - l.rect[2];
            if gap >= -0.5 * size && (*leader && gap < 2.0 * size || gap < 1.5 * size) {
                l.single = true;
                break;
            }
        }
    }
}

/// Merges the consecutive lines of a same paragraph.
pub fn group_paragraphs(lines: Vec<Line>) -> Vec<Block> {
    let n = lines.len();
    // next[i] = candidate next line (below, same style, aligned)
    let mut next: Vec<Option<usize>> = vec![None; n];
    let mut has_prev = vec![false; n];
    for i in 0..n {
        let li = &lines[i];
        if li.single || li.first().leader {
            continue;
        }
        let size = li.first().size.max(1.0);
        let mut best: Option<(usize, f32)> = None;
        for j in 0..n {
            if i == j || lines[j].single || has_prev[j] {
                continue;
            }
            let lj = &lines[j];
            if !same_style(li.first(), lj.first()) {
                continue;
            }
            let pitch = li.base - lj.base;
            if pitch < 0.8 * size || pitch > 2.2 * size {
                continue;
            }
            let (wi, wj) = (li.rect[2] - li.rect[0], lj.rect[2] - lj.rect[0]);
            let overlap = li.rect[2].min(lj.rect[2]) - li.rect[0].max(lj.rect[0]);
            if overlap < 0.4 * wi.min(wj) {
                continue;
            }
            let left_ok = (li.rect[0] - lj.rect[0]).abs() <= 2.0 || (lj.rect[0] - li.rect[0]).abs() <= 3.5 * size;
            let right_ok = (li.rect[2] - lj.rect[2]).abs() <= 2.0;
            let center_ok = ((li.rect[0] + li.rect[2]) - (lj.rect[0] + lj.rect[2])).abs() <= 4.0;
            if !(left_ok || right_ok || center_ok) {
                continue;
            }
            if best.map(|(_, p)| pitch < p).unwrap_or(true) {
                best = Some((j, pitch));
            }
        }
        if let Some((j, _)) = best {
            next[i] = Some(j);
            has_prev[j] = true;
        }
    }
    // Chains: start from the lines without a predecessor
    let mut chains: Vec<Vec<usize>> = vec![];
    let mut used = vec![false; n];
    for s in 0..n {
        if has_prev[s] || used[s] {
            continue;
        }
        let mut c = vec![s];
        used[s] = true;
        let mut cur = s;
        while let Some(nx) = next[cur] {
            if used[nx] {
                break;
            }
            used[nx] = true;
            c.push(nx);
            cur = nx;
        }
        chains.push(c);
    }
    for (i, u) in used.iter().enumerate() {
        if !*u {
            chains.push(vec![i]);
        }
    }
    // Cut the chains at paragraph ends: a "short" line (that does not reach the right margin
    // of the chain) ends the paragraph.
    let mut blocks = vec![];
    for chain in chains {
        let size = lines[chain[0]].first().size.max(1.0);
        let max_right = chain.iter().map(|&i| lines[i].rect[2]).fold(f32::MIN, f32::max);
        let min_left = chain.iter().map(|&i| lines[i].rect[0]).fold(f32::MAX, f32::min);
        // A line may stop before the margin by at most the length of a long word (≈ 28 % of the width).
        let slack = (3.0 * size).max(0.28 * (max_right - min_left));
        let mut cur: Vec<usize> = vec![];
        for (k, &i) in chain.iter().enumerate() {
            cur.push(i);
            let is_last = k + 1 == chain.len();
            let full = lines[i].rect[2] >= max_right - slack;
            // the last line of a chain of at least two lines may be short; otherwise cut
            if is_last || !full {
                blocks.push(make_block(&lines, std::mem::take(&mut cur)));
            }
        }
    }
    blocks
}

fn make_block(lines: &[Line], idxs: Vec<usize>) -> Block {
    let ls: Vec<Line> = idxs.iter().map(|&i| lines[i].clone()).collect();
    let mut rect = ls[0].rect;
    for l in &ls[1..] {
        rect = r4_union(&rect, &l.rect);
    }
    let mut text = String::new();
    for (k, l) in ls.iter().enumerate() {
        if k > 0 {
            if !text.ends_with('-') {
                text.push(' ');
            }
        }
        text.push_str(l.text.trim());
    }
    let align = guess_align(&ls);
    Block { lines: ls, rect, text, align }
}

fn guess_align(ls: &[Line]) -> Align {
    if ls.len() < 2 {
        return Align::Left;
    }
    let tol = 2.0;
    let left0 = ls[0].rect[0];
    let right0 = ls[0].rect[2];
    let all_left = ls.iter().all(|l| (l.rect[0] - left0).abs() <= tol);
    let all_right = ls.iter().all(|l| (l.rect[2] - right0).abs() <= tol);
    let c0 = ls[0].rect[0] + ls[0].rect[2];
    let all_center = ls.iter().all(|l| ((l.rect[0] + l.rect[2]) - c0).abs() <= 2.0 * tol);
    let inner_right = ls[..ls.len() - 1].iter().all(|l| (l.rect[2] - right0).abs() <= tol);
    if all_left && (all_right || inner_right) && ls.len() >= 3 {
        Align::Justify
    } else if all_left {
        Align::Left
    } else if all_right {
        Align::Right
    } else if all_center {
        Align::Center
    } else {
        Align::Left
    }
}

/// Lines then paragraphs from the raw pieces.
pub fn group(runs: Vec<Run>) -> Vec<Block> {
    group_paragraphs(group_lines(runs))
}

/// Grouping by lines only (one block per line).
pub fn group_as_lines(runs: Vec<Run>) -> Vec<Block> {
    group_lines(runs).into_iter().map(|l| make_block(&[l], vec![0])).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(idx: usize, text: &str, x0: f32, x1: f32, base: f32, color: [u8; 3], leader: bool) -> Run {
        Run { idx, text: text.into(), font: "Calibri".into(), size: 12.0, color, base, rect: [x0, base - 3.0, x1, base + 9.0], leader }
    }

    #[test]
    fn words_of_a_line_become_one_line() {
        let runs = vec![
            run(0, "HOW", 50.0, 110.0, 700.0, [0, 0, 0], false),
            run(1, "TO READ", 114.0, 175.0, 700.0, [0, 0, 0], false),
            run(2, "THIS", 179.0, 195.0, 700.0, [0, 0, 0], false),
            run(3, "BOOK ?", 199.0, 240.0, 700.0, [0, 0, 0], false),
            run(4, "...............", 244.0, 480.0, 700.0, [0, 0, 0], true),
            run(5, "11", 500.0, 515.0, 700.0, [0, 0, 0], false),
            run(6, "Title", 50.0, 90.0, 700.0, [255, 60, 0], false),
        ];
        let l = group_lines(runs);
        let texts: Vec<_> = l.iter().map(|o| o.text.as_str()).collect();
        assert!(texts.contains(&"HOW TO READ THIS BOOK ?"), "{texts:?}");
        assert_eq!(l.len(), 4, "{texts:?}");
        let main = l.iter().find(|o| o.runs.len() == 4).unwrap();
        assert_eq!(main.members(), vec![0, 1, 2, 3]);
        assert!(main.single, "followed by leader dots: tabular line");
    }

    #[test]
    fn paragraph_of_three_lines_merges() {
        let mk = |i: usize, base: f32, last: bool| {
            let right = if last { 300.0 } else { 520.0 };
            vec![
                run(i * 10, "Lorem ipsum", 72.0, 150.0, base, [0, 0, 0], false),
                run(i * 10 + 1, "dolor sit amet", 154.0, right, base, [0, 0, 0], false),
            ]
        };
        let mut runs = mk(0, 700.0, false);
        runs.extend(mk(1, 686.0, false));
        runs.extend(mk(2, 672.0, true));
        let blocks = group(runs);
        assert_eq!(blocks.len(), 1, "{:?}", blocks.iter().map(|b| &b.text).collect::<Vec<_>>());
        assert_eq!(blocks[0].lines.len(), 3);
        assert!(blocks[0].text.starts_with("Lorem ipsum dolor sit amet Lorem"));
        assert_eq!(blocks[0].align, Align::Justify);
    }

    #[test]
    fn toc_rows_stay_separate() {
        let mut runs = vec![];
        for (k, base) in [700.0_f32, 686.0, 672.0, 658.0].iter().enumerate() {
            runs.push(run(k * 10, "CHAPTER TITLE", 72.0, 200.0 + k as f32 * 3.0, *base, [0, 0, 0], false));
            runs.push(run(k * 10 + 1, "..................", 204.0, 500.0, *base, [0, 0, 0], true));
            runs.push(run(k * 10 + 2, "12", 505.0, 520.0, *base, [0, 0, 0], false));
        }
        let blocks = group(runs);
        let titles: Vec<_> = blocks.iter().filter(|b| b.text.starts_with("CHAPTER")).collect();
        assert_eq!(titles.len(), 4, "each table-of-contents line stays alone");
    }

    #[test]
    fn two_columns_do_not_merge_across() {
        let mut runs = vec![];
        for (k, base) in [700.0_f32, 686.0, 672.0].iter().enumerate() {
            runs.push(run(k * 10, "left column long text", 72.0, 280.0, *base, [0, 0, 0], false));
            runs.push(run(k * 10 + 1, "right column long text", 320.0, 528.0, *base, [0, 0, 0], false));
        }
        let blocks = group(runs);
        assert_eq!(blocks.len(), 2, "{:?}", blocks.iter().map(|b| &b.text).collect::<Vec<_>>());
        assert!(blocks.iter().all(|b| b.lines.len() == 3));
    }

    #[test]
    fn different_style_breaks_paragraph() {
        let runs = vec![
            run(0, "Bold title", 72.0, 300.0, 700.0, [0, 0, 0], false),
            run(1, "normal text", 72.0, 300.0, 686.0, [255, 0, 0], false),
        ];
        assert_eq!(group(runs).len(), 2);
    }
}
