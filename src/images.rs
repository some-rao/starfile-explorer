//! İkon ve küçük resimleri arka planda üretip doku olarak önbelleğe alır.
//! Görüntülerin kendisi platform modülünden (`shell::shell_image`) gelir.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Condvar, Mutex, OnceLock};

#[derive(Clone)]
pub enum Img {
    Pending,
    Ready(egui::TextureHandle),
    Failed,
}

/// `key` boşsa açılış ön isteğidir: sonuç `EARLY` tablosuna yazılır
struct Req {
    key: String,
    path: PathBuf,
    size: i32,
    icon_only: bool,
}

type Done = (String, Option<egui::ColorImage>);
type EarlyKey = (PathBuf, i32, bool);

enum Early {
    /// Üretiliyor; arayüz bu arada isterse sonucu bu anahtarla kanala gönder
    Pending(Option<String>),
    Done(Option<egui::ColorImage>),
}

/// Arka plan iş parçacıkları süreç başına bir kez başlar. `start` pencere ve GPU
/// hazırlanırken çağrılır; Windows kabuğunun soğuk açılışı (COM, ikon işleyicileri)
/// ve ilk ekranın ikonları pencere beklenirken biter.
struct Workers {
    queue: Arc<(Mutex<Vec<Req>>, Condvar)>,
    tx: Sender<Done>,
    rx: Mutex<Option<Receiver<Done>>>,
}

static WORKERS: OnceLock<Workers> = OnceLock::new();
static EARLY: Mutex<Option<HashMap<EarlyKey, Early>>> = Mutex::new(None);

/// Bitince yeniden çizim istenecek arayüz bağlamı (pencere hazır olunca atanır)
pub static UI_CTX: OnceLock<egui::Context> = OnceLock::new();

pub fn repaint() {
    if let Some(ctx) = UI_CTX.get() {
        ctx.request_repaint();
    }
}

fn produce(req: &Req) -> Option<egui::ColorImage> {
    // Küçük resim: önce kabuk sağlayıcısı (Explorer önbelleği, video ve ofis
    // dosyaları dahil), olmazsa fotoğrafı kendimiz küçültürüz. Böylece kabuk
    // sağlayıcısının üretemediği (çevrimdışı OneDrive, bazı PNG/JPG) resimler
    // Galeri'de boş ikon yerine gerçek küçük resimle görünür.
    if !req.icon_only {
        for attempt in 0..3 {
            if let Some(img) = crate::shell::shell_image(&req.path, req.size, false) {
                return Some(img);
            }
            if attempt < 2 {
                std::thread::sleep(std::time::Duration::from_millis(30));
            }
        }
        if let Some(img) = decode_photo(&req.path, req.size) {
            return Some(img);
        }
    }
    // Simge: birkaç kez dene, sonra pes et
    for attempt in 0..3 {
        if let Some(img) = crate::shell::shell_image(&req.path, req.size, true) {
            return Some(img);
        }
        if attempt < 2 {
            std::thread::sleep(std::time::Duration::from_millis(30));
        }
    }
    None
}

/// Kabuk küçük resim üretemediğinde desteklenen fotoğraf biçimlerini kendimiz
/// çözeriz. Tam boy görüntü bellekte açıldığı için büyük dosyalarda sınır koyarız.
fn decode_photo(path: &Path, size: i32) -> Option<egui::ColorImage> {
    const MAX_BYTES: u64 = 48 << 20;
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .unwrap_or_default();
    if !matches!(
        ext.as_str(),
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "tif" | "tiff"
    ) {
        return None;
    }
    let meta = std::fs::metadata(path).ok()?;
    if meta.len() > MAX_BYTES {
        return None;
    }
    let img = image::open(path).ok()?;
    let side = size.clamp(16, 1024) as u32;
    let img = img.thumbnail(side, side).to_rgba8();
    let (w, h) = img.dimensions();
    Some(egui::ColorImage::from_rgba_unmultiplied(
        [w as usize, h as usize],
        img.as_raw(),
    ))
}

fn workers() -> &'static Workers {
    WORKERS.get_or_init(|| {
        let queue: Arc<(Mutex<Vec<Req>>, Condvar)> =
            Arc::new((Mutex::new(Vec::new()), Condvar::new()));
        let (tx, rx): (Sender<Done>, _) = channel();
        for _ in 0..4 {
            let q = queue.clone();
            let tx = tx.clone();
            std::thread::spawn(move || {
                crate::shell::init_worker_thread();
                loop {
                    let req = {
                        let (m, cv) = &*q;
                        let mut g = m.lock().unwrap();
                        loop {
                            if let Some(r) = g.pop() {
                                break r;
                            }
                            g = cv.wait(g).unwrap();
                        }
                    };
                    let img = produce(&req);
                    let key = if req.key.is_empty() {
                        let mut early = EARLY.lock().unwrap();
                        let Some(map) = early.as_mut() else { continue };
                        let k = (req.path, req.size, req.icon_only);
                        match map.get_mut(&k) {
                            Some(Early::Pending(Some(_))) => {
                                let Some(Early::Pending(Some(key))) = map.remove(&k) else {
                                    unreachable!()
                                };
                                key
                            }
                            Some(e) => {
                                *e = Early::Done(img);
                                continue;
                            }
                            None => continue, // açılış bitti, artık istenmiyor
                        }
                    } else {
                        req.key
                    };
                    if tx.send((key, img)).is_err() {
                        return;
                    }
                    repaint();
                }
            });
        }
        Workers {
            queue,
            tx,
            rx: Mutex::new(Some(rx)),
        }
    })
}

/// İş parçacıklarını başlatır (zaten başladıysa bir şey yapmaz)
pub fn start() {
    workers();
}

/// Açılışta ilk ekranda istenecek görüntüleri (yol, piksel, yalnızca ikon) önceden üretir.
/// Arayüz aynı isteği yapınca sonuç beklemeden kullanılır.
pub fn prefetch(items: Vec<EarlyKey>) {
    let mut reqs = Vec::with_capacity(items.len());
    {
        let mut early = EARLY.lock().unwrap();
        let map = early.get_or_insert_with(HashMap::new);
        for (path, size, icon_only) in items {
            if map
                .insert((path.clone(), size, icon_only), Early::Pending(None))
                .is_none()
            {
                reqs.push(Req {
                    key: String::new(),
                    path,
                    size,
                    icon_only,
                });
            }
        }
    }
    // LIFO kuyruk: ilk öğe en önce işlensin diye ters sırayla ve dibe ekle
    reqs.reverse();
    let (m, cv) = &*workers().queue;
    m.lock().unwrap().splice(0..0, reqs);
    cv.notify_all();
}

/// Açılış bitti: kullanılmayan ön sonuçları bırak. Üretimi süren ve arayüzün beklediği
/// istekler (Pending(Some)) iş parçacığı bitirince teslim edilir.
pub fn end_prefetch() {
    let mut early = EARLY.lock().unwrap();
    if let Some(map) = early.as_mut() {
        map.retain(|_, e| matches!(e, Early::Pending(Some(_))));
        if map.is_empty() {
            *early = None;
        }
    }
}

/// Önbellekteki görüntülerin toplam piksel belleği (GPU/paylaşımlı bellek) üst sınırı.
/// Aşılınca en uzun süredir çizilmeyenler bırakılır; ekrandakiler hiç atılmaz.
const BUDGET_BYTES: usize = 160 << 20;

struct Slot {
    img: Img,
    /// En son istendiği kare
    used: u64,
}

/// Arka plan iş parçacıklarında ikon/küçük resim üretip doku olarak önbelleğe alır.
/// İstekler LIFO işlenir: hızlı kaydırmada ekrandaki öğeler önce gelir.
/// Piksel olarak aynı görüntüler (ör. binlerce standart klasör ikonu) tek dokuyu paylaşır.
pub struct ImageCache {
    textures: HashMap<String, Slot>,
    /// İçerik özeti → doku ve bayt boyutu
    unique: HashMap<u64, (egui::TextureHandle, usize)>,
    unique_bytes: usize,
    frame: u64,
    queue: Arc<(Mutex<Vec<Req>>, Condvar)>,
    rx: Receiver<Done>,
    tx: Sender<Done>,
}

impl ImageCache {
    pub fn new(ctx: &egui::Context) -> ImageCache {
        let _ = UI_CTX.set(ctx.clone());
        let w = workers();
        let rx =
            w.rx.lock()
                .unwrap()
                .take()
                .expect("ImageCache yalnızca bir kez oluşturulur");
        ImageCache {
            textures: HashMap::new(),
            unique: HashMap::new(),
            unique_bytes: 0,
            frame: 0,
            queue: w.queue.clone(),
            rx,
            tx: w.tx.clone(),
        }
    }

    pub fn get(&mut self, key: &str, path: &Path, size: i32, icon_only: bool) -> Img {
        if let Some(t) = self.textures.get_mut(key) {
            t.used = self.frame;
            return t.img.clone();
        }
        self.textures.insert(
            key.to_string(),
            Slot {
                img: Img::Pending,
                used: self.frame,
            },
        );
        if self.take_early(key, path, size, icon_only) {
            return Img::Pending;
        }
        let (m, cv) = &*self.queue;
        m.lock().unwrap().push(Req {
            key: key.to_string(),
            path: path.to_path_buf(),
            size,
            icon_only,
        });
        cv.notify_one();
        Img::Pending
    }

    /// Açılışta önceden istenmiş bir görüntüyse onu kullanır; hazırsa bir sonraki
    /// `poll`'da, değilse üretilince gelir. Yeni istek gerekmez.
    fn take_early(&mut self, key: &str, path: &Path, size: i32, icon_only: bool) -> bool {
        let mut early = EARLY.lock().unwrap();
        let Some(map) = early.as_mut() else {
            return false;
        };
        let k = (path.to_path_buf(), size, icon_only);
        match map.get_mut(&k) {
            Some(Early::Pending(slot @ None)) => {
                *slot = Some(key.to_string());
                true
            }
            Some(Early::Done(_)) => {
                let Some(Early::Done(img)) = map.remove(&k) else {
                    unreachable!()
                };
                let _ = self.tx.send((key.to_string(), img));
                true
            }
            _ => false,
        }
    }

    /// Yüklenmeyi bekleyen görüntü var mı (açılışta pencereyi göstermek için)
    #[cfg_attr(not(windows), allow(dead_code))]
    pub fn busy(&self) -> bool {
        self.textures
            .values()
            .any(|t| matches!(t.img, Img::Pending))
    }

    /// Bekleyen (henüz işlenmemiş) istekleri atar; klasör değişince eski istekler beklemesin.
    /// Açılış ön istekleri kuyrukta kalır; ilgili sekme/görünüm hâlâ isteyebilir.
    pub fn drop_pending(&mut self) {
        let mut q = self.queue.0.lock().unwrap();
        let mut keep = Vec::new();
        for r in q.drain(..) {
            if r.key.is_empty() {
                keep.push(r);
            } else {
                self.textures.remove(&r.key);
            }
        }
        *q = keep;
    }

    /// Kuyrukta üretilmeyi bekleyen görüntü sayısı (geçici tanı için).
    pub fn pending(&self) -> usize {
        self.queue.0.lock().unwrap().len()
    }

    pub fn poll(&mut self, ctx: &egui::Context) {
        self.frame += 1;
        while let Ok((key, img)) = self.rx.try_recv() {
            let img = match img {
                Some(i) => Img::Ready(self.texture_for(ctx, &key, i)),
                None => Img::Failed,
            };
            self.textures.insert(
                key,
                Slot {
                    img,
                    used: self.frame,
                },
            );
        }
        if self.unique_bytes > BUDGET_BYTES {
            self.evict();
        }
    }

    /// Aynı piksellere sahip bir doku varsa onu paylaşır, yoksa yükler
    fn texture_for(
        &mut self,
        ctx: &egui::Context,
        key: &str,
        img: egui::ColorImage,
    ) -> egui::TextureHandle {
        use std::hash::{BuildHasher, Hash, Hasher};
        let mut h = egui::ahash::RandomState::with_seeds(1, 2, 3, 4).build_hasher();
        img.size.hash(&mut h);
        img.as_raw().hash(&mut h);
        let digest = h.finish();
        if let Some((tex, _)) = self.unique.get(&digest) {
            return tex.clone();
        }
        let bytes = img.pixels.len() * 4;
        let tex = ctx.load_texture(key, img, egui::TextureOptions::LINEAR);
        self.unique.insert(digest, (tex.clone(), bytes));
        self.unique_bytes += bytes;
        tex
    }

    /// En uzun süredir çizilmeyenleri bütçenin %75'ine inene dek bırakır
    fn evict(&mut self) {
        let mut order: Vec<(u64, String)> = self
            .textures
            .iter()
            .filter(|(_, s)| {
                s.used < self.frame.saturating_sub(1) && !matches!(s.img, Img::Pending)
            })
            .map(|(k, s)| (s.used, k.clone()))
            .collect();
        order.sort_unstable();
        let target = BUDGET_BYTES / 4 * 3;
        for chunk in order.chunks(64) {
            for (_, k) in chunk {
                self.textures.remove(k);
            }
            self.prune_unique();
            if self.unique_bytes <= target {
                break;
            }
        }
    }

    /// Artık hiçbir anahtarın göstermediği dokuları bırakır
    fn prune_unique(&mut self) {
        let live: std::collections::HashSet<egui::TextureId> = self
            .textures
            .values()
            .filter_map(|s| {
                if let Img::Ready(t) = &s.img {
                    Some(t.id())
                } else {
                    None
                }
            })
            .collect();
        let mut freed = 0;
        self.unique.retain(|_, (t, b)| {
            let keep = live.contains(&t.id());
            if !keep {
                freed += *b;
            }
            keep
        });
        self.unique_bytes -= freed;
    }
}
