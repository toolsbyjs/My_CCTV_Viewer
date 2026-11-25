//! Window split layouts, taken from the original's window split table
//! (12 normal splits: 1, 4, 6, 8, 9, 13, 16, 20, 25, 36, 49, 64).

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Cell {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

#[derive(Clone, Debug)]
pub struct Split {
    pub count: usize,
    pub grid: u32,
    pub cells: Vec<Cell>,
}

/// (window count, grid size, big windows as (x, y, size))
const TABLE: &[(usize, u32, &[(u32, u32, u32)])] = &[
    (1, 1, &[]),
    (4, 2, &[]),
    (6, 3, &[(0, 0, 2)]),
    (8, 4, &[(0, 0, 3)]),
    (9, 3, &[]),
    (13, 4, &[(1, 1, 2)]),
    (16, 4, &[]),
    (20, 6, &[(0, 0, 3), (3, 0, 3)]),
    (25, 5, &[]),
    (36, 6, &[]),
    (49, 7, &[]),
    (64, 8, &[]),
];

pub const COUNTS: [usize; 12] = [1, 4, 6, 8, 9, 13, 16, 20, 25, 36, 49, 64];
pub const MAX_WINDOWS: usize = 64;
/// The original refuses playback layouts above 36 windows.
pub const MAX_PLAYBACK_WINDOWS: usize = 36;

pub fn split(count: usize) -> Split {
    let (count, grid, bigs) = TABLE.iter().copied().find(|(c, _, _)| *c == count).unwrap_or(TABLE[1]);
    let mut covered = vec![false; (grid * grid) as usize];
    let mut cells = Vec::new();
    // Windows are numbered in reading order of their top-left corner, as in the original.
    let mut starts: Vec<(u32, u32, u32)> = Vec::new();
    for &(x, y, s) in bigs {
        for yy in y..y + s {
            for xx in x..x + s {
                covered[(yy * grid + xx) as usize] = true;
            }
        }
        starts.push((x, y, s));
    }
    for y in 0..grid {
        for x in 0..grid {
            if !covered[(y * grid + x) as usize] {
                starts.push((x, y, 1));
            }
        }
    }
    starts.sort_by_key(|&(x, y, _)| (y, x));
    for (x, y, s) in starts {
        cells.push(Cell { x, y, w: s, h: s });
    }
    debug_assert_eq!(cells.len(), count);
    Split { count, grid, cells }
}

#[cfg(test)]
mod tests {
    #[test]
    fn counts_match() {
        for c in super::COUNTS {
            assert_eq!(super::split(c).cells.len(), c);
        }
    }
}
