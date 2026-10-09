//! Which tiles are visible / wanted, and the cache that remembers them.
//!
//! The geometry here is pure (no GPU) so it can be unit tested.

use std::collections::HashMap;

use super::atlas::Slot;
use crate::doc::worker::{TileKey, THUMB, TILE};
use crate::view::layout::PageGeom;
use crate::view::Camera;

/// Screen position of a page's top-left corner, snapped to whole pixels so
/// that tiles are sampled 1:1 and text stays crisp.
pub fn page_origin(cam: &Camera, g: &PageGeom) -> [f32; 2] {
    let p = cam.doc_to_screen([g.x, g.y]);
    [p[0].round(), p[1].round()]
}

/// Size of the page in tile pixels at `tile_scale`.
pub fn page_px(g: &PageGeom, tile_scale: f32) -> (u32, u32) {
    (
        (g.w * tile_scale).ceil().max(1.0) as u32,
        (g.h * tile_scale).ceil().max(1.0) as u32,
    )
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TileRange {
    pub tx0: u32,
    pub tx1: u32,
    pub ty0: u32,
    pub ty1: u32,
}

/// Tiles of one page that intersect the viewport grown by `margin` pixels.
/// `ratio` = screen pixels per tile pixel (1.0 when tiles are current).
pub fn tile_range(
    origin: [f32; 2],
    ratio: f32,
    page_px: (u32, u32),
    viewport: [f32; 2],
    margin: f32,
) -> Option<TileRange> {
    let to_px = |screen: f32, o: f32| (screen - o) / ratio;
    let x0 = to_px(-margin, origin[0]).max(0.0);
    let y0 = to_px(-margin, origin[1]).max(0.0);
    let x1 = to_px(viewport[0] + margin, origin[0]).min(page_px.0 as f32);
    let y1 = to_px(viewport[1] + margin, origin[1]).min(page_px.1 as f32);
    if x1 <= x0 || y1 <= y0 {
        return None;
    }
    let t = TILE as f32;
    Some(TileRange {
        tx0: (x0 / t).floor() as u32,
        tx1: (((x1 / t).ceil() as u32).max(1)) - 1,
        ty0: (y0 / t).floor() as u32,
        ty1: (((y1 / t).ceil() as u32).max(1)) - 1,
    })
}

/// Everything the workers should render, most important first.
pub struct Wanted {
    pub tiles: Vec<TileKey>,
    pub thumbs: Vec<u32>,
}

/// Compute the wanted tiles for the current view.
///
/// Visible tiles come first (nearest to the screen centre first), then a
/// prefetch ring of one extra screen above and below.
pub fn wanted_tiles(
    cam: &Camera,
    pages: &[PageGeom],
    visible: std::ops::Range<usize>,
    tile_scale: f32,
    have: impl Fn(&TileKey) -> bool,
) -> Vec<TileKey> {
    let ratio = cam.scale() / tile_scale;
    let center = [cam.viewport[0] * 0.5, cam.viewport[1] * 0.5];
    let mut near: Vec<(f32, TileKey)> = Vec::new();
    let mut far: Vec<(f32, TileKey)> = Vec::new();
    let prefetch = cam.viewport[1];

    let first = visible.start.saturating_sub(3);
    let last = (visible.end + 3).min(pages.len());
    for (pi, g) in pages.iter().enumerate().take(last).skip(first) {
        let origin = page_origin(cam, g);
        let ppx = page_px(g, tile_scale);
        let Some(outer) = tile_range(origin, ratio, ppx, cam.viewport, prefetch) else {
            continue;
        };
        let inner = tile_range(origin, ratio, ppx, cam.viewport, 0.0);
        for ty in outer.ty0..=outer.ty1 {
            for tx in outer.tx0..=outer.tx1 {
                let key = TileKey {
                    page: pi as u32,
                    scale: tile_scale.to_bits(),
                    tx: tx as u16,
                    ty: ty as u16,
                };
                if have(&key) {
                    continue;
                }
                let cx = origin[0] + (tx as f32 + 0.5) * TILE as f32 * ratio;
                let cy = origin[1] + (ty as f32 + 0.5) * TILE as f32 * ratio;
                let d = (cx - center[0]).powi(2) + (cy - center[1]).powi(2);
                let is_visible = inner
                    .map(|r| tx >= r.tx0 && tx <= r.tx1 && ty >= r.ty0 && ty <= r.ty1)
                    .unwrap_or(false);
                if is_visible {
                    near.push((d, key));
                } else {
                    far.push((d, key));
                }
            }
        }
    }
    near.sort_by(|a, b| a.0.total_cmp(&b.0));
    far.sort_by(|a, b| a.0.total_cmp(&b.0));
    near.into_iter().chain(far).map(|(_, k)| k).collect()
}

/// Pages that should have a preview, nearest to `current` first.
pub fn wanted_thumbs(
    current: usize,
    page_count: usize,
    radius: usize,
    limit: usize,
    have: impl Fn(u32) -> bool,
) -> Vec<u32> {
    let mut out = Vec::new();
    if page_count == 0 {
        return out;
    }
    let current = current.min(page_count - 1);
    for d in 0..=radius {
        for p in [current.checked_sub(d), Some(current + d)]
            .into_iter()
            .flatten()
        {
            if p < page_count && !have(p as u32) && !out.contains(&(p as u32)) {
                out.push(p as u32);
                if out.len() >= limit {
                    return out;
                }
            }
        }
    }
    out
}

pub struct TileEntry {
    pub slot: Slot,
    pub w: u16,
    pub h: u16,
    pub last_used: u64,
}

pub struct ThumbEntry {
    pub slot: Slot,
    pub w: u16,
    pub h: u16,
    pub last_used: u64,
}

/// CPU-side index of what is on the GPU.
#[derive(Default)]
pub struct TileIndex {
    pub tiles: HashMap<TileKey, TileEntry>,
    pub by_page: HashMap<u32, Vec<TileKey>>,
    pub thumbs: HashMap<u32, ThumbEntry>,
    pub frame: u64,
}

impl TileIndex {
    pub fn insert(&mut self, key: TileKey, entry: TileEntry) {
        self.by_page.entry(key.page).or_default().push(key);
        self.tiles.insert(key, entry);
    }

    pub fn remove(&mut self, key: &TileKey) -> Option<TileEntry> {
        let e = self.tiles.remove(key)?;
        if let Some(v) = self.by_page.get_mut(&key.page) {
            v.retain(|k| k != key);
            if v.is_empty() {
                self.by_page.remove(&key.page);
            }
        }
        Some(e)
    }

    /// Least recently used tile that was not drawn this frame.
    pub fn lru_tile(&self) -> Option<TileKey> {
        self.tiles
            .iter()
            .filter(|(_, e)| e.last_used < self.frame)
            .min_by_key(|(_, e)| e.last_used)
            .map(|(k, _)| *k)
    }

    /// Preview farthest from `current` that was not drawn this frame.
    pub fn farthest_thumb(&self, current: usize) -> Option<u32> {
        self.thumbs
            .iter()
            .filter(|(_, e)| e.last_used < self.frame)
            .max_by_key(|(p, _)| (**p as i64 - current as i64).unsigned_abs())
            .map(|(p, _)| *p)
    }

    /// Scale of the cached tile set for `page` that is closest to `target`
    /// (but not equal to it), preferring sharper ones.
    pub fn best_fallback_scale(&self, page: u32, target_bits: u32) -> Option<u32> {
        let target = f32::from_bits(target_bits);
        let mut best: Option<(f32, u32)> = None;
        for k in self.by_page.get(&page)? {
            if k.scale == target_bits {
                continue;
            }
            let s = k.scale_f();
            // Distance in log space; upscaling a coarse tile looks worse than
            // downscaling a sharp one, so penalise coarse ones.
            let ratio = (s / target).ln();
            let score = if ratio >= 0.0 { ratio } else { -ratio * 1.5 };
            if best.map(|(b, _)| score < b).unwrap_or(true) {
                best = Some((score, k.scale));
            }
        }
        best.map(|(_, s)| s)
    }
}

pub fn thumb_scale(g: &PageGeom) -> f32 {
    (THUMB as f32 / g.w).min(THUMB as f32 / g.h)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::view::{Layout, ZoomMode};

    fn cam(offset_y: f32, zoom: f32) -> Camera {
        Camera {
            offset: [0.0, offset_y],
            zoom,
            dpr: 1.0,
            viewport: [1000.0, 800.0],
            mode: ZoomMode::Free,
        }
    }

    #[test]
    fn range_covers_visible_part_only() {
        // Page 600x2000 px, origin at the top-left, viewport 1000x800.
        let r = tile_range([0.0, 0.0], 1.0, (600, 2000), [1000.0, 800.0], 0.0).unwrap();
        assert_eq!((r.tx0, r.tx1), (0, 1));
        assert_eq!((r.ty0, r.ty1), (0, 1)); // 800 px -> tiles 0..=1
                                            // Scrolled so that the page top is 1000 px above the viewport.
        let r = tile_range([0.0, -1000.0], 1.0, (600, 2000), [1000.0, 800.0], 0.0).unwrap();
        assert_eq!((r.ty0, r.ty1), (1, 3)); // px 1000..1800 -> tiles 1,2,3
    }

    #[test]
    fn range_is_none_when_offscreen() {
        assert!(tile_range([0.0, 900.0], 1.0, (600, 600), [1000.0, 800.0], 0.0).is_none());
        assert!(tile_range([0.0, -700.0], 1.0, (600, 600), [1000.0, 800.0], 0.0).is_none());
    }

    #[test]
    fn exact_tile_boundary_does_not_add_a_tile() {
        // Visible px 0..512 must be exactly tile 0.
        let r = tile_range([0.0, 0.0], 1.0, (2000, 2000), [512.0, 512.0], 0.0).unwrap();
        assert_eq!((r.tx0, r.tx1, r.ty0, r.ty1), (0, 0, 0, 0));
    }

    #[test]
    fn stretched_tiles_account_for_ratio() {
        // Tiles rendered at half the current scale: each tile px is 2 screen px.
        let r = tile_range([0.0, 0.0], 2.0, (1000, 1000), [1024.0, 1024.0], 0.0).unwrap();
        assert_eq!((r.tx0, r.tx1), (0, 0));
    }

    #[test]
    fn wanted_tiles_are_sorted_by_distance_and_skip_cached() {
        let layout = Layout::new(&[(600.0, 800.0), (600.0, 800.0)]);
        let c = cam(0.0, 1.0);
        let vis = layout.visible(0.0, 800.0);
        let all = wanted_tiles(&c, &layout.pages, vis.clone(), 1.0, |_| false);
        assert!(!all.is_empty());
        // The first tile is the one closest to the centre (400, 400) -> (0,0)/(1,0)/(0,1)/(1,1)
        assert_eq!(all[0].page, 0);
        // Everything cached -> nothing wanted.
        let none = wanted_tiles(&c, &layout.pages, vis.clone(), 1.0, |_| true);
        assert!(none.is_empty());
        // No duplicates.
        let mut sorted = all.clone();
        sorted.sort_by_key(|k| (k.page, k.ty, k.tx));
        sorted.dedup();
        assert_eq!(sorted.len(), all.len());
    }

    #[test]
    fn visible_tiles_precede_prefetch() {
        let layout = Layout::new(&[(600.0, 3000.0)]);
        let c = cam(1000.0, 1.0);
        let vis = layout.visible(1000.0, 1800.0);
        let all = wanted_tiles(&c, &layout.pages, vis, 1.0, |_| false);
        // Visible rows: px 1000..1800 -> ty 1..=3. Prefetch adds ty 0 and 4,5.
        let first_prefetch = all.iter().position(|k| k.ty == 0 || k.ty >= 4).unwrap();
        assert!(all[..first_prefetch]
            .iter()
            .all(|k| (1..=3).contains(&k.ty)));
        assert!(!all[first_prefetch..]
            .iter()
            .any(|k| (1..=3).contains(&k.ty)));
    }

    #[test]
    fn thumbs_nearest_first() {
        let t = wanted_thumbs(5, 100, 10, 6, |_| false);
        assert_eq!(t[0], 5);
        assert_eq!(t.len(), 6);
        assert!(t.iter().all(|&p| (p as i32 - 5).abs() <= 3));
        let t = wanted_thumbs(0, 3, 10, 10, |p| p == 0);
        assert_eq!(t, vec![1, 2]);
        assert!(wanted_thumbs(0, 0, 10, 10, |_| false).is_empty());
    }

    #[test]
    fn fallback_prefers_closest_scale() {
        let mut idx = TileIndex::default();
        let mk = |s: f32| TileKey {
            page: 3,
            scale: s.to_bits(),
            tx: 0,
            ty: 0,
        };
        let entry = || TileEntry {
            slot: Slot { array: 0, layer: 0 },
            w: 1,
            h: 1,
            last_used: 0,
        };
        idx.insert(mk(1.0), entry());
        idx.insert(mk(4.0), entry());
        // Target 3.0: 4.0 (sharper, ln 1.33=0.29) beats 1.0 (0.51*1.5 penalised).
        assert_eq!(
            idx.best_fallback_scale(3, 3.0f32.to_bits()),
            Some(4.0f32.to_bits())
        );
        // Target 1.2: 1.0 is closest.
        assert_eq!(
            idx.best_fallback_scale(3, 1.2f32.to_bits()),
            Some(1.0f32.to_bits())
        );
        // The target scale itself is never a fallback.
        assert_eq!(
            idx.best_fallback_scale(3, 1.0f32.to_bits()),
            Some(4.0f32.to_bits())
        );
        assert_eq!(idx.best_fallback_scale(99, 1.0f32.to_bits()), None);
        idx.remove(&mk(4.0));
        assert_eq!(idx.by_page[&3].len(), 1);
    }

    #[test]
    fn lru_skips_tiles_used_this_frame() {
        let mut idx = TileIndex {
            frame: 10,
            ..Default::default()
        };
        let k1 = TileKey {
            page: 0,
            scale: 1,
            tx: 0,
            ty: 0,
        };
        let k2 = TileKey {
            page: 0,
            scale: 1,
            tx: 1,
            ty: 0,
        };
        let mk = |t| TileEntry {
            slot: Slot { array: 0, layer: 0 },
            w: 1,
            h: 1,
            last_used: t,
        };
        idx.insert(k1, mk(10)); // used this frame
        idx.insert(k2, mk(7));
        assert_eq!(idx.lru_tile(), Some(k2));
        idx.remove(&k2);
        assert_eq!(idx.lru_tile(), None);
    }

    #[test]
    fn page_origin_is_pixel_aligned() {
        let c = Camera {
            offset: [0.3, 10.7],
            zoom: 1.37,
            dpr: 1.0,
            viewport: [800.0, 600.0],
            mode: ZoomMode::Free,
        };
        let g = PageGeom {
            x: 5.5,
            y: 20.2,
            w: 100.0,
            h: 100.0,
        };
        let o = page_origin(&c, &g);
        assert_eq!(o[0], o[0].round());
        assert_eq!(o[1], o[1].round());
    }
}
