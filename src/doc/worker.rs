//! Tile rendering pool.
//!
//! MuPDF documents are not `Send`, so each worker opens its own copy. The UI
//! thread replaces the whole list of wanted tiles whenever the view changes;
//! workers always take the most important one first. Finished pixel buffers
//! are recycled through a return channel so steady-state scrolling does not
//! allocate.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;

use crossbeam_channel::{unbounded, Receiver, Sender};
use mupdf::pdf::PdfPage;
use mupdf::{ColorParams, Colorspace, Device, DisplayList, Image, Matrix, Pixmap, Rect};

use super::annots;
use super::{open_document, FixedDoc, OpenDoc, Reflow};

/// Edge length of a tile slot in pixels.
pub const TILE: u32 = 512;
/// Edge length of a preview slot in pixels.
pub const THUMB: u32 = 256;

const DISPLAY_LIST_CACHE: usize = 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TileKey {
    pub page: u32,
    /// `f32::to_bits` of the render scale (physical pixels per point).
    pub scale: u32,
    pub tx: u16,
    pub ty: u16,
}

impl TileKey {
    pub fn scale_f(&self) -> f32 {
        f32::from_bits(self.scale)
    }
}

pub struct TilePixels {
    pub key: TileKey,
    /// Valid pixels inside the slot (the rest is padding).
    pub w: u16,
    pub h: u16,
    /// RGBA8, `TILE * TILE * 4` bytes.
    pub data: Vec<u8>,
}

pub struct ThumbPixels {
    pub page: u32,
    pub w: u16,
    pub h: u16,
    /// RGBA8, `THUMB * THUMB * 4` bytes.
    pub data: Vec<u8>,
}

pub enum Rendered {
    Tile(TilePixels),
    Thumb(ThumbPixels),
    Failed { page: u32, error: String },
    OpenFailed(String),
}

#[derive(Default)]
struct Queues {
    tiles: VecDeque<TileKey>,
    thumbs: VecDeque<u32>,
    running_tiles: HashSet<TileKey>,
    running_thumbs: HashSet<u32>,
}

struct Shared {
    q: Mutex<Queues>,
    cv: Condvar,
    quit: AtomicBool,
}

pub struct Pool {
    shared: Arc<Shared>,
    pub rx: Receiver<Rendered>,
    recycle_tx: Sender<Vec<u8>>,
    handles: Vec<JoinHandle<()>>,
}

pub fn worker_count() -> usize {
    let cores = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(2);
    cores.saturating_sub(1).clamp(1, 4)
}

impl Pool {
    pub fn spawn(
        path: PathBuf,
        password: Option<String>,
        wake: Arc<dyn Fn() + Send + Sync>,
        workers: usize,
    ) -> Pool {
        Self::spawn_reflow(path, password, Reflow::default(), wake, workers)
    }

    /// Like `spawn`; EPUBs are laid out with `reflow` (every worker has to
    /// use the same layout).
    pub fn spawn_reflow(
        path: PathBuf,
        password: Option<String>,
        reflow: Reflow,
        wake: Arc<dyn Fn() + Send + Sync>,
        workers: usize,
    ) -> Pool {
        let shared = Arc::new(Shared {
            q: Mutex::new(Queues::default()),
            cv: Condvar::new(),
            quit: AtomicBool::new(false),
        });
        let (tx, rx) = unbounded::<Rendered>();
        let (recycle_tx, recycle_rx) = unbounded::<Vec<u8>>();
        let handles = (0..workers.max(1))
            .filter_map(|i| {
                let ctx = WorkerCtx {
                    shared: shared.clone(),
                    tx: tx.clone(),
                    recycle: recycle_rx.clone(),
                    wake: wake.clone(),
                    path: path.clone(),
                    password: password.clone(),
                    reflow,
                };
                std::thread::Builder::new()
                    .name(format!("mizu-render-{i}"))
                    .spawn(move || ctx.run())
                    .ok()
            })
            .collect();
        Pool {
            shared,
            rx,
            recycle_tx,
            handles,
        }
    }

    /// Replace the queues. Tiles that are already being rendered are not
    /// queued a second time. Order = priority.
    pub fn set_wanted(&self, tiles: Vec<TileKey>, thumbs: Vec<u32>) {
        let mut q = self.shared.q.lock().unwrap_or_else(|e| e.into_inner());
        q.tiles = tiles
            .into_iter()
            .filter(|k| !q.running_tiles.contains(k))
            .collect();
        q.thumbs = thumbs
            .into_iter()
            .filter(|p| !q.running_thumbs.contains(p))
            .collect();
        let any = !q.tiles.is_empty() || !q.thumbs.is_empty();
        drop(q);
        if any {
            self.shared.cv.notify_all();
        }
    }

    pub fn clear_wanted(&self) {
        self.set_wanted(Vec::new(), Vec::new());
    }

    /// Give a pixel buffer back so a worker can reuse it.
    pub fn recycle(&self, buf: Vec<u8>) {
        let _ = self.recycle_tx.send(buf);
    }
}

impl Drop for Pool {
    fn drop(&mut self) {
        self.shared.quit.store(true, Ordering::SeqCst);
        self.shared.cv.notify_all();
        for h in self.handles.drain(..) {
            let _ = h.join();
        }
    }
}

struct WorkerCtx {
    shared: Arc<Shared>,
    tx: Sender<Rendered>,
    recycle: Receiver<Vec<u8>>,
    wake: Arc<dyn Fn() + Send + Sync>,
    path: PathBuf,
    password: Option<String>,
    reflow: Reflow,
}

enum Job {
    Tile(TileKey),
    Thumb(u32),
}

struct CachedPage {
    list: DisplayList,
    /// Page bounds origin and size in page space.
    x0: f32,
    y0: f32,
    w: f32,
    h: f32,
}

impl WorkerCtx {
    fn send(&self, r: Rendered) {
        let _ = self.tx.send(r);
        (self.wake)();
    }

    fn next_job(&self) -> Option<Job> {
        let mut q = self.shared.q.lock().unwrap_or_else(|e| e.into_inner());
        loop {
            if self.shared.quit.load(Ordering::SeqCst) {
                return None;
            }
            if let Some(k) = q.tiles.pop_front() {
                q.running_tiles.insert(k);
                return Some(Job::Tile(k));
            }
            if let Some(p) = q.thumbs.pop_front() {
                q.running_thumbs.insert(p);
                return Some(Job::Thumb(p));
            }
            q = self.shared.cv.wait(q).unwrap_or_else(|e| e.into_inner());
        }
    }

    fn finish(&self, job: &Job) {
        let mut q = self.shared.q.lock().unwrap_or_else(|e| e.into_inner());
        match job {
            Job::Tile(k) => {
                q.running_tiles.remove(k);
            }
            Job::Thumb(p) => {
                q.running_thumbs.remove(p);
            }
        }
    }

    fn buffer(&self) -> Vec<u8> {
        let mut v = self
            .recycle
            .try_recv()
            .unwrap_or_else(|_| Vec::with_capacity((TILE * TILE * 4) as usize));
        v.clear();
        v.resize((TILE * TILE * 4) as usize, 0);
        v
    }

    fn run(self) {
        let doc = match open_document(&self.path, self.password.as_deref(), self.reflow) {
            Ok(d) => d,
            Err(e) => {
                self.send(Rendered::OpenFailed(e.to_string()));
                return;
            }
        };
        let cs = Colorspace::device_rgb();
        let Ok(mut pixmap) = Pixmap::new_with_w_h(&cs, TILE as i32, TILE as i32, true) else {
            self.send(Rendered::OpenFailed("cannot allocate render buffer".into()));
            return;
        };
        let mut cache: HashMap<u32, CachedPage> = HashMap::new();
        let mut lru: VecDeque<u32> = VecDeque::new();

        while let Some(job) = self.next_job() {
            let page = match &job {
                Job::Tile(k) => k.page,
                Job::Thumb(p) => *p,
            };
            let result = (|| -> Result<Rendered, String> {
                if let std::collections::hash_map::Entry::Vacant(slot) = cache.entry(page) {
                    let cp = build_page(&doc, page as usize)?;
                    slot.insert(cp);
                    lru.push_back(page);
                    while lru.len() > DISPLAY_LIST_CACHE {
                        if let Some(old) = lru.pop_front() {
                            cache.remove(&old);
                        }
                    }
                } else if let Some(pos) = lru.iter().position(|&p| p == page) {
                    lru.remove(pos);
                    lru.push_back(page);
                }
                let cp = cache.get(&page).ok_or("page cache miss")?;
                match &job {
                    Job::Tile(k) => {
                        let s = k.scale_f();
                        let (w, h) = render_into(
                            &mut pixmap,
                            cp,
                            s,
                            -(k.tx as f32) * TILE as f32,
                            -(k.ty as f32) * TILE as f32,
                        )?;
                        let mut data = self.buffer();
                        data.copy_from_slice(pixmap.samples());
                        extend_edges(
                            &mut data,
                            TILE as usize,
                            covered(cp.w * s, k.tx),
                            covered(cp.h * s, k.ty),
                        );
                        let vw =
                            (w as i64 - k.tx as i64 * TILE as i64).clamp(0, TILE as i64) as u16;
                        let vh =
                            (h as i64 - k.ty as i64 * TILE as i64).clamp(0, TILE as i64) as u16;
                        Ok(Rendered::Tile(TilePixels {
                            key: *k,
                            w: vw,
                            h: vh,
                            data,
                        }))
                    }
                    Job::Thumb(p) => {
                        let s = (THUMB as f32 / cp.w).min(THUMB as f32 / cp.h);
                        let (w, h) = render_into(&mut pixmap, cp, s, 0.0, 0.0)?;
                        let mut data = self.buffer();
                        data.copy_from_slice(pixmap.samples());
                        // The preview slot is THUMB pixels wide: pack the rows.
                        let (row, stride) = (THUMB as usize * 4, TILE as usize * 4);
                        for y in 1..THUMB as usize {
                            data.copy_within(y * stride..y * stride + row, y * row);
                        }
                        data.truncate(THUMB as usize * row);
                        extend_edges(
                            &mut data,
                            THUMB as usize,
                            covered(cp.w * s, 0),
                            covered(cp.h * s, 0),
                        );
                        Ok(Rendered::Thumb(ThumbPixels {
                            page: *p,
                            w: w.min(THUMB) as u16,
                            h: h.min(THUMB) as u16,
                            data,
                        }))
                    }
                }
            })();
            match result {
                Ok(r) => self.send(r),
                Err(error) => {
                    log::warn!("render page {page}: {error}");
                    self.send(Rendered::Failed { page, error });
                }
            }
            self.finish(&job);
        }
    }
}

fn build_page(doc: &OpenDoc, index: usize) -> Result<CachedPage, String> {
    if let OpenDoc::Fixed(f) = doc {
        return build_image_page(f, index);
    }
    let page = doc
        .doc()
        .ok_or("no document")?
        .load_page(index as i32)
        .map_err(|e| e.to_string())?;
    let (b, list) = if doc.is_pdf() {
        let mut page = PdfPage::try_from(page).map_err(|e| e.to_string())?;
        // mizu strokes are drawn by the GPU layer, never by MuPDF.
        annots::remove_mizu(&mut page).map_err(|e| e.to_string())?;
        let b = page.bounds().map_err(|e| e.to_string())?;
        (b, page.to_display_list(true).map_err(|e| e.to_string())?)
    } else {
        let b = page.bounds().map_err(|e| e.to_string())?;
        (b, page.to_display_list(true).map_err(|e| e.to_string())?)
    };
    Ok(CachedPage {
        list,
        x0: b.x0,
        y0: b.y0,
        w: b.width().max(1.0),
        h: b.height().max(1.0),
    })
}

/// A page of a fixed-layout book: its image, stretched to the page size.
fn build_image_page(f: &FixedDoc, index: usize) -> Result<CachedPage, String> {
    let p = f.book.pages.get(index).ok_or("no such page")?;
    let name = p.image.as_deref().ok_or("page has no image")?;
    let data = f.zip.read(name).map_err(|e| e.to_string())?;
    let image = Image::from_bytes(&data).map_err(|e| e.to_string())?;
    let (w, h) = (p.w.max(1.0), p.h.max(1.0));
    let mut list = DisplayList::new(Rect::new(0.0, 0.0, w, h)).map_err(|e| e.to_string())?;
    {
        let dev = Device::from_display_list(&mut list).map_err(|e| e.to_string())?;
        dev.fill_image(
            &image,
            &Matrix::new(w, 0.0, 0.0, h, 0.0, 0.0),
            1.0,
            ColorParams::default(),
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(CachedPage {
        list,
        x0: 0.0,
        y0: 0.0,
        w,
        h,
    })
}

/// How many pixels of tile `t` a page `len` pixels long covers completely.
fn covered(len: f32, t: u16) -> usize {
    (len - t as f32 * TILE as f32)
        .floor()
        .clamp(0.0, TILE as f32) as usize
}

/// Repeat the last completely covered column and row of a `side` x `side`
/// buffer over the rest of it. A page whose size in pixels is fractional ends
/// in a partly covered pixel that shows the white clear colour (a light line
/// along dark pages), and linear filtering samples the padding past the edge.
fn extend_edges(data: &mut [u8], side: usize, w: usize, h: usize) {
    let stride = side * 4;
    if (1..side).contains(&w) {
        for row in data.chunks_exact_mut(stride).take(h) {
            let (inside, rest) = row.split_at_mut(w * 4);
            let last: [u8; 4] = inside[inside.len() - 4..].try_into().unwrap();
            rest.as_chunks_mut::<4>().0.fill(last);
        }
    }
    if (1..side).contains(&h) {
        let (inside, rest) = data.split_at_mut(h * stride);
        let last = &inside[inside.len() - stride..];
        for row in rest.chunks_exact_mut(stride) {
            row.copy_from_slice(last);
        }
    }
}

/// Render the page at scale `s` into `pixmap` (white background), shifted by
/// `(dx, dy)` device pixels. Returns the full page size in pixels.
fn render_into(
    pixmap: &mut Pixmap,
    cp: &CachedPage,
    s: f32,
    dx: f32,
    dy: f32,
) -> Result<(u32, u32), String> {
    pixmap.clear_with(255).map_err(|e| e.to_string())?;
    let ctm = Matrix::new(s, 0.0, 0.0, s, -cp.x0 * s + dx, -cp.y0 * s + dy);
    let dev = Device::from_pixmap(pixmap).map_err(|e| e.to_string())?;
    let area = Rect::new(0.0, 0.0, TILE as f32, TILE as f32);
    cp.list.run(&dev, &ctm, area).map_err(|e| e.to_string())?;
    Ok(((cp.w * s).ceil() as u32, (cp.h * s).ceil() as u32))
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    /// A one-page PDF of `w` x `h` points drawing `content`.
    fn one_page(dir: &std::path::Path, w: f32, h: f32, content: &str) -> std::path::PathBuf {
        let path = dir.join("page.pdf");
        let pdf = format!(
            "%PDF-1.4\n1 0 obj << /Type /Catalog /Pages 2 0 R >> endobj\n\
             2 0 obj << /Type /Pages /Kids [3 0 R] /Count 1 >> endobj\n\
             3 0 obj << /Type /Page /Parent 2 0 R /MediaBox [0 0 {w} {h}] \
             /Contents 4 0 R >> endobj\n\
             4 0 obj << /Length {} >> stream\n{content}\nendstream endobj\n\
             trailer << /Root 1 0 R >>\n%%EOF\n",
            content.len()
        );
        std::fs::write(&path, pdf).unwrap();
        path
    }

    #[test]
    fn dark_page_has_no_light_edge() {
        // A black page 300.4 x 200.6 points: its last pixel column and row
        // are partly covered.
        let dir = tempfile::tempdir().unwrap();
        let path = one_page(dir.path(), 300.4, 200.6, "0 0 0 rg 0 0 300.4 200.6 re f");
        let pool = Pool::spawn(path, None, Arc::new(|| {}), 1);
        let key = TileKey {
            page: 0,
            scale: 1.0f32.to_bits(),
            tx: 0,
            ty: 0,
        };
        pool.set_wanted(vec![key], vec![]);
        let Rendered::Tile(t) = pool.rx.recv_timeout(Duration::from_secs(20)).unwrap() else {
            panic!("no tile");
        };
        assert_eq!((t.w, t.h), (301, 201));
        let px = |x: usize, y: usize| t.data[(y * TILE as usize + x) * 4];
        // The edge, and the padding that filtering reads past it.
        for x in 299..302 {
            assert_eq!(px(x, 100), 0, "column {x}");
        }
        for y in 199..202 {
            assert_eq!(px(100, y), 0, "row {y}");
        }
        assert_eq!(px(400, 400), 0, "corner padding");
    }

    #[test]
    fn preview_rows_are_thumb_wide() {
        // 200 x 400 points, black in the lower half: the preview is 128 x 256.
        let dir = tempfile::tempdir().unwrap();
        let path = one_page(dir.path(), 200.0, 400.0, "0 0 0 rg 0 0 200 200 re f");
        let pool = Pool::spawn(path, None, Arc::new(|| {}), 1);
        pool.set_wanted(vec![], vec![0]);
        let Rendered::Thumb(t) = pool.rx.recv_timeout(Duration::from_secs(20)).unwrap() else {
            panic!("no preview");
        };
        assert_eq!((t.w, t.h), (128, 256));
        assert_eq!(t.data.len(), (THUMB * THUMB * 4) as usize);
        let px = |x: usize, y: usize| t.data[(y * THUMB as usize + x) * 4];
        assert_eq!(px(10, 60), 255, "upper half");
        assert_eq!(px(10, 200), 0, "lower half");
        // Padding repeats the last column.
        assert_eq!(px(200, 60), 255);
        assert_eq!(px(200, 200), 0);
    }
}
