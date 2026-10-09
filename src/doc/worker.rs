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
