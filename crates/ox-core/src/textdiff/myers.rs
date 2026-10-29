//! Myers' O(ND) difference algorithm in linear space (the "middle snake"
//! divide and conquer from Myers' 1986 paper), on anything comparable.
//! After a deadline it stops looking for the shortest script: what is left
//! is then shown as removed and added as a whole.

use std::ops::Range;
use std::time::Instant;

/// One step of an edit script, with indexes into the old and new sequence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Edit {
    Same(usize, usize),
    Removed(usize),
    Added(usize),
}

/// The shortest edit script turning `old` into `new` (or a longer one, if
/// finding it takes past `deadline`). In each run of changes the removed
/// items come before the added ones.
pub fn diff<T: Eq>(old: &[T], new: &[T], deadline: Option<Instant>) -> Vec<Edit> {
    let max_d = max_d(old.len(), new.len());
    let mut vf = V::new(max_d);
    let mut vb = V::new(max_d);
    let mut out = Vec::with_capacity(old.len().max(new.len()));
    let mut ctx = Ctx {
        old,
        new,
        vf: &mut vf,
        vb: &mut vb,
        deadline,
        out: &mut out,
    };
    ctx.conquer(0..old.len(), 0..new.len());
    removed_first(out)
}

fn max_d(n: usize, m: usize) -> usize {
    (n + m).div_ceil(2) + 1
}

/// The furthest x reached on each diagonal k (−d ≤ k ≤ d).
struct V {
    offset: isize,
    v: Vec<usize>,
}

impl V {
    fn new(max_d: usize) -> Self {
        Self {
            offset: max_d as isize,
            v: vec![0; 2 * max_d + 1],
        }
    }
}

impl std::ops::Index<isize> for V {
    type Output = usize;
    fn index(&self, k: isize) -> &usize {
        &self.v[(k + self.offset) as usize]
    }
}

impl std::ops::IndexMut<isize> for V {
    fn index_mut(&mut self, k: isize) -> &mut usize {
        &mut self.v[(k + self.offset) as usize]
    }
}

struct Ctx<'a, T> {
    old: &'a [T],
    new: &'a [T],
    vf: &'a mut V,
    vb: &'a mut V,
    deadline: Option<Instant>,
    out: &'a mut Vec<Edit>,
}

impl<T: Eq> Ctx<'_, T> {
    fn prefix(&self, old: Range<usize>, new: Range<usize>) -> usize {
        self.old[old]
            .iter()
            .zip(&self.new[new])
            .take_while(|(a, b)| a == b)
            .count()
    }

    fn suffix(&self, old: Range<usize>, new: Range<usize>) -> usize {
        self.old[old]
            .iter()
            .rev()
            .zip(self.new[new].iter().rev())
            .take_while(|(a, b)| a == b)
            .count()
    }

    fn same(&mut self, old: usize, new: usize, n: usize) {
        self.out
            .extend((0..n).map(|i| Edit::Same(old + i, new + i)));
    }

    fn conquer(&mut self, mut old: Range<usize>, mut new: Range<usize>) {
        let p = self.prefix(old.clone(), new.clone());
        self.same(old.start, new.start, p);
        old.start += p;
        new.start += p;
        let s = self.suffix(old.clone(), new.clone());
        old.end -= s;
        new.end -= s;
        if old.is_empty() {
            self.out.extend(new.clone().map(Edit::Added));
        } else if new.is_empty() {
            self.out.extend(old.clone().map(Edit::Removed));
        } else {
            match self.middle_snake(old.clone(), new.clone()) {
                // A split that makes both halves smaller.
                Some((x, y))
                    if (x, y) != (old.start, new.start) && (x, y) != (old.end, new.end) =>
                {
                    self.conquer(old.start..x, new.start..y);
                    self.conquer(x..old.end, y..new.end);
                }
                _ => {
                    self.out.extend(old.clone().map(Edit::Removed));
                    self.out.extend(new.clone().map(Edit::Added));
                }
            }
        }
        self.same(old.end, new.end, s);
    }

    /// A point on a shortest path through `old` × `new`, found by searching
    /// from both ends until the paths meet. `None` past the deadline.
    fn middle_snake(&mut self, old: Range<usize>, new: Range<usize>) -> Option<(usize, usize)> {
        let (n, m) = (old.len(), new.len());
        let delta = n as isize - m as isize;
        let odd = delta & 1 == 1;
        self.vf[1] = 0;
        self.vb[1] = 0;
        let d_max = max_d(n, m) as isize;
        for d in 0..d_max {
            if self.deadline.is_some_and(|t| Instant::now() > t) {
                return None;
            }
            // Forward.
            for k in (-d..=d).rev().step_by(2) {
                let mut x = if k == -d || (k != d && self.vf[k - 1] < self.vf[k + 1]) {
                    self.vf[k + 1]
                } else {
                    self.vf[k - 1] + 1
                };
                let y = (x as isize - k) as usize;
                let (x0, y0) = (x, y);
                if x < n && y < m {
                    x += self.prefix(old.start + x..old.end, new.start + y..new.end);
                }
                self.vf[k] = x;
                if odd && (k - delta).abs() < d && self.vf[k] + self.vb[-(k - delta)] >= n {
                    return Some((old.start + x0, new.start + y0));
                }
            }
            // Backward.
            for k in (-d..=d).rev().step_by(2) {
                let mut x = if k == -d || (k != d && self.vb[k - 1] < self.vb[k + 1]) {
                    self.vb[k + 1]
                } else {
                    self.vb[k - 1] + 1
                };
                let mut y = (x as isize - k) as usize;
                if x < n && y < m {
                    let s = self.suffix(old.start..old.start + n - x, new.start..new.start + m - y);
                    x += s;
                    y += s;
                }
                self.vb[k] = x;
                if !odd && (k - delta).abs() <= d && self.vb[k] + self.vf[-(k - delta)] >= n {
                    return Some((old.start + n - x, new.start + m - y));
                }
            }
        }
        None
    }
}

/// `edits` with each run of changes reordered: removed first, then added.
fn removed_first(edits: Vec<Edit>) -> Vec<Edit> {
    let mut out = Vec::with_capacity(edits.len());
    let mut added = Vec::new();
    for e in edits {
        match e {
            Edit::Same(..) => {
                out.append(&mut added);
                out.push(e);
            }
            Edit::Removed(_) => out.push(e),
            Edit::Added(_) => added.push(e),
        }
    }
    out.append(&mut added);
    out
}
