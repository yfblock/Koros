//! Boot-time physical memory region collector.
//!
//! Arch-specific code registers raw memory ranges here; `mm::init()` clips out
//! the kernel image and feeds the remainder to the buddy frame allocator.

use core::cmp::Ordering;

const MAX_REGIONS: usize = 32;
const PAGE: usize = 4096;

#[derive(Clone, Copy)]
struct Region {
    start: usize,
    end: usize,
}

/// Collects page-aligned physical memory ranges before buddy initialisation.
pub struct RegionCollector {
    regions: [Region; MAX_REGIONS],
    num: usize,
}

impl RegionCollector {
    pub const fn new() -> Self {
        Self {
            regions: [Region { start: 0, end: 0 }; MAX_REGIONS],
            num: 0,
        }
    }

    /// Record a half-open physical range `[start, end)`.
    pub fn add(&mut self, start: usize, end: usize) {
        if start >= end {
            return;
        }
        debug_assert!(start % PAGE == 0, "start {start:#x} not page-aligned");
        debug_assert!(end % PAGE == 0, "end {end:#x} not page-aligned");

        let new = Region { start, end };
        let mut i = 0;
        while i < self.num {
            match self.regions[i].start.cmp(&new.start) {
                Ordering::Less if self.regions[i].end >= new.start => {
                    if new.end > self.regions[i].end {
                        self.regions[i].end = new.end;
                        self.try_merge(i);
                    }
                    return;
                }
                Ordering::Equal | Ordering::Greater => break,
                _ => {}
            }
            i += 1;
        }

        if i > 0 && self.regions[i - 1].end == new.start {
            self.regions[i - 1].end = new.end;
            self.try_merge(i - 1);
            return;
        }

        self.insert(i, new);
    }

    pub fn each(&self, mut f: impl FnMut(usize, usize)) {
        for region in &self.regions[..self.num] {
            f(region.start, region.end);
        }
    }

    fn insert(&mut self, idx: usize, region: Region) {
        assert!(self.num < MAX_REGIONS, "RegionCollector OOM");
        self.regions.copy_within(idx..self.num, idx + 1);
        self.regions[idx] = region;
        self.num += 1;
    }

    fn try_merge(&mut self, i: usize) {
        if i + 1 < self.num && self.regions[i].end >= self.regions[i + 1].start {
            if self.regions[i + 1].end > self.regions[i].end {
                self.regions[i].end = self.regions[i + 1].end;
            }
            self.regions.copy_within(i + 2..self.num, i + 1);
            self.num -= 1;
        }
    }
}

/// Split `[start, end)` around `[hole_start, hole_end)` and invoke `f` per piece.
pub fn clip_region(
    start: usize,
    end: usize,
    hole_start: usize,
    hole_end: usize,
    mut f: impl FnMut(usize, usize),
) {
    if start >= end {
        return;
    }
    if hole_start >= hole_end || end <= hole_start || start >= hole_end {
        f(start, end);
        return;
    }
    if start < hole_start {
        f(start, hole_start);
    }
    if end > hole_end {
        f(hole_end, end);
    }
}
