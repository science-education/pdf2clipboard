#![windows_subsystem = "windows"]

mod platform;

use eframe::egui::{self, ColorImage, ScrollArea, TextureHandle, TextureOptions};
#[cfg(not(target_arch = "wasm32"))]
use pdf_oxide::{
    document::PdfDocument,
    rendering::{render_page, render_page_fit, RenderOptions},
};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{mpsc, Arc, Mutex};

#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::*;

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_name = render_pdf_page_js)]
    async fn render_pdf_page_js(pdf_data: js_sys::Uint8Array, page_num: u32, dpi: f32) -> JsValue;

    #[wasm_bindgen(js_name = get_pdf_info_js)]
    async fn get_pdf_info_js(pdf_data: js_sys::Uint8Array) -> JsValue;
}

#[cfg(target_arch = "wasm32")]
fn parse_js_image(val: JsValue) -> Result<ColorImage, String> {
    let width = js_sys::Reflect::get(&val, &"width".into())
        .map_err(|e| format!("{:?}", e))?
        .as_f64()
        .ok_or("no width")? as usize;
    let height = js_sys::Reflect::get(&val, &"height".into())
        .map_err(|e| format!("{:?}", e))?
        .as_f64()
        .ok_or("no height")? as usize;
    let data = js_sys::Reflect::get(&val, &"data".into()).map_err(|e| format!("{:?}", e))?;
    let uint8_data = js_sys::Uint8Array::new(&data);
    let mut pixels = vec![egui::Color32::from_rgba_unmultiplied(0, 0, 0, 0); width * height];
    uint8_data.copy_to(unsafe {
        std::slice::from_raw_parts_mut(pixels.as_mut_ptr() as *mut u8, pixels.len() * 4)
    });
    Ok(ColorImage {
        size: [width, height],
        pixels,
    })
}

#[cfg(target_arch = "wasm32")]
use wasm_bindgen::JsCast;

// ---------------------------------------------------------------------------
// Internationalization
// ---------------------------------------------------------------------------

struct Tr {
    #[allow(dead_code)]
    is_jp: bool,
    drop_pdf: &'static str,
    dpi: &'static str,
    thumbnail: &'static str,
    copying: &'static str,
    done: &'static str,
    click_to_copy: &'static str,
    url_prompt: &'static str,
    open_url: &'static str,
    downloading: &'static str,
    download_err: fn(&str) -> String,
    #[allow(dead_code)]
    pages_count: fn(u32) -> String,
    status_copy_done: fn(usize) -> String,
}

impl Tr {
    fn from_locale(locale: Option<String>) -> &'static Self {
        if let Some(l) = locale {
            if l.starts_with("ja") {
                return &TR_JP;
            }
        }
        &TR_EN
    }

    #[allow(dead_code)]
    fn status_not_yet(&self, p: usize) -> String {
        if self.is_jp {
            format!("ページ {} はまだレンダリングされていません", p)
        } else {
            format!("Page {} not yet rendered", p)
        }
    }
}

const TR_EN: Tr = Tr {
    is_jp: false,
    drop_pdf: "Drop a PDF file onto this window",
    dpi: "DPI:",
    thumbnail: "Preview:",
    copying: "Copying...",
    done: "Done",
    click_to_copy: "Click\n(Enter)\nto\ncopy",
    url_prompt: "URL:",
    open_url: "Load",
    downloading: "Downloading...",
    download_err: |e| format!("Download Error: {e}"),
    pages_count: |n| format!(" - {} pages", n),
    status_copy_done: |p| format!("Page {} copied to clipboard", p),
};

const TR_JP: Tr = Tr {
    is_jp: true,
    drop_pdf: "PDFファイルをここにドロップしてください",
    dpi: "解像度:",
    thumbnail: "プレビュー:",
    copying: "COPY中",
    done: "完了",
    click_to_copy: "クリック\n(Enter)\nすると\nコピー",
    url_prompt: "URL:",
    open_url: "開く",
    downloading: "ダウンロード中...",
    download_err: |e| format!("ダウンロード失敗: {e}"),
    pages_count: |n| format!(" - {} ページ", n),
    status_copy_done: |p| format!("ページ {} をクリップボードにコピー完了", p),
};

// ---------------------------------------------------------------------------
// App state
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Debug)]
enum CopyState {
    Idle,
    Copying,
    Done,
}

struct PageSlot {
    tex: Option<TextureHandle>,
    img: Option<ColorImage>,
    is_full_res: bool,
    copy_state: CopyState,
    render_error: Option<String>,
}

#[cfg(not(target_arch = "wasm32"))]
struct LoadedPdf {
    bytes: Arc<[u8]>,
    name: String,
    page_count: u32,
    aspect: f32,
}

enum AppMsg {
    Rendered {
        render_gen: u32,
        page: usize,
        img: ColorImage,
        is_full_res: bool,
    },
    #[allow(dead_code)]
    RenderFailed {
        render_gen: u32,
        page: usize,
        error: String,
    },
    #[cfg(not(target_arch = "wasm32"))]
    Loaded {
        doc_gen: u32,
        result: Result<LoadedPdf, String>,
    },
    #[cfg(target_arch = "wasm32")]
    PdfInfo {
        doc_gen: u32,
        page_count: u32,
        aspect: f32,
        name: String,
    },
}

enum CopyMsg {
    Done {
        doc_gen: u32,
        copy_gen: u32,
        page: usize,
    },
    Failed {
        doc_gen: u32,
        copy_gen: u32,
        page: usize,
        error: String,
    },
}

struct App {
    pdf_bytes: Option<Arc<[u8]>>,
    copied_page: Option<usize>,
    selected_page: Option<usize>,
    page_count: u32,
    pages: Vec<PageSlot>,
    thumb_size: u32,
    page_aspect: f32,
    dpi: f32,
    status: String,
    tr: &'static Tr,
    tx: mpsc::Sender<AppMsg>,
    rx: Arc<Mutex<mpsc::Receiver<AppMsg>>>,
    #[allow(dead_code)]
    copy_tx: mpsc::Sender<CopyMsg>,
    #[allow(dead_code)]
    copy_rx: Arc<Mutex<mpsc::Receiver<CopyMsg>>>,
    #[allow(dead_code)]
    render_gen: Arc<AtomicU32>,
    #[allow(dead_code)]
    doc_gen: Arc<AtomicU32>,
    #[allow(dead_code)]
    copy_gen: Arc<AtomicU32>,
    #[allow(dead_code)]
    egui_ctx: egui::Context,
    url_input: String,
    is_downloading: bool,
    download_tx: mpsc::Sender<Result<(Arc<[u8]>, String), String>>,
    download_rx: Arc<Mutex<mpsc::Receiver<Result<(Arc<[u8]>, String), String>>>>,
}

#[cfg(not(target_arch = "wasm32"))]
fn build_doc(bytes: &[u8]) -> Result<PdfDocument, String> {
    PdfDocument::from_bytes(bytes.to_vec()).map_err(|e| e.to_string())
}

#[cfg(not(target_arch = "wasm32"))]
fn load_pdf_in_background(bytes: Arc<[u8]>, name: String) -> Result<LoadedPdf, String> {
    eprintln!("Starting build_doc...");
    let doc = build_doc(&bytes)?;
    eprintln!("build_doc success, getting page count...");
    let page_count = doc.page_count().map_err(|e| e.to_string())? as u32;
    eprintln!("Page count: {}", page_count);

    let mut total_aspect = 0.0f32;
    let mut count = 0u32;
    for i in 0..page_count as usize {
        if let Ok(info) = doc.get_page_info(i) {
            let (pw, ph) = if info.rotation == 90 || info.rotation == 270 {
                (info.media_box.height, info.media_box.width)
            } else {
                (info.media_box.width, info.media_box.height)
            };
            if ph > 0.0 {
                total_aspect += pw / ph;
                count += 1;
            }
        }
    }
    let aspect = if count > 0 {
        total_aspect / count as f32
    } else {
        0.707
    };

    Ok(LoadedPdf {
        bytes,
        name,
        page_count,
        aspect,
    })
}

fn setup_fonts(ctx: &egui::Context) {
    #[allow(unused_mut)]
    let mut fonts = egui::FontDefinitions::default();

    #[cfg(windows)]
    for path in &[
        "C:/Windows/Fonts/meiryo.ttc",
        "C:/Windows/Fonts/yugothm.ttc",
        "C:/Windows/Fonts/msgothic.ttc",
        "C:/Windows/Fonts/YuGothM.ttc",
    ] {
        if let Ok(data) = std::fs::read(path) {
            fonts
                .font_data
                .insert("jp".to_owned(), egui::FontData::from_owned(data));
            for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
                if let Some(v) = fonts.families.get_mut(&family) {
                    v.insert(0, "jp".to_owned());
                }
            }
            break;
        }
    }

    #[cfg(target_os = "macos")]
    {
        let mut loaded = false;
        for path in &[
            "/System/Library/Fonts/PingFang.ttc",
            "/System/Library/Fonts/Hiragino Sans GB.ttc",
            "/System/Library/Fonts/Supplemental/Hiragino Sans GB W3.otf",
            "/System/Library/Fonts/Supplemental/Hiragino Sans GB W6.otf",
            "/Library/Fonts/Hiragino Sans W3.ttc",
        ] {
            if let Ok(data) = std::fs::read(path) {
                eprintln!("Loaded Japanese font from: {}", path);
                fonts
                    .font_data
                    .insert("jp".to_owned(), egui::FontData::from_owned(data));
                for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
                    if let Some(v) = fonts.families.get_mut(&family) {
                        v.insert(0, "jp".to_owned());
                    }
                }
                loaded = true;
                break;
            }
        }
        if !loaded {
            eprintln!("Warning: No Japanese fonts found on macOS at expected locations.");
        }
    }

    #[cfg(target_arch = "wasm32")]
    {
        log::info!("Loading bundled BIZ UD Gothic Japanese font for Wasm...");
        let font_bytes = include_bytes!("../assets/font.ttf");
        fonts.font_data.insert("jp".to_owned(), egui::FontData::from_static(font_bytes));
        for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
            if let Some(v) = fonts.families.get_mut(&family) {
                v.insert(0, "jp".to_owned());
            }
        }
    }

    ctx.set_fonts(fonts);
}

impl App {
    fn new(cc: &eframe::CreationContext) -> Self {
        setup_fonts(&cc.egui_ctx);
        let tr = Tr::from_locale(sys_locale::get_locale());
        let (tx, rx) = mpsc::channel();
        let (copy_tx, copy_rx) = mpsc::channel();
        let (download_tx, download_rx) = mpsc::channel();

        Self {
            pdf_bytes: None,
            copied_page: None,
            selected_page: None,
            page_count: 0,
            pages: Vec::new(),
            thumb_size: 200,
            page_aspect: 0.707,
            dpi: 300.0,
            status: tr.drop_pdf.into(),
            tr,
            tx,
            rx: Arc::new(Mutex::new(rx)),
            copy_tx,
            copy_rx: Arc::new(Mutex::new(copy_rx)),
            render_gen: Arc::new(AtomicU32::new(0)),
            doc_gen: Arc::new(AtomicU32::new(0)),
            copy_gen: Arc::new(AtomicU32::new(0)),
            egui_ctx: cc.egui_ctx.clone(),
            url_input: String::new(),
            is_downloading: false,
            download_tx,
            download_rx: Arc::new(Mutex::new(download_rx)),
        }
    }

    fn load_pdf_path(&mut self, path: &std::path::Path) {
        eprintln!("Loading PDF path: {:?}", path);
        #[allow(unused_variables)]
        let doc_gen = self.doc_gen.fetch_add(1, Ordering::Relaxed) + 1;
        self.render_gen.fetch_add(1, Ordering::Relaxed);
        self.copy_gen.fetch_add(1, Ordering::Relaxed);
        #[cfg(not(target_arch = "wasm32"))]
        {
            let tx = self.tx.clone();
            let ctx = self.egui_ctx.clone();
            let path = path.to_owned();
            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("PDF")
                .to_owned();
            self.status = format!("Loading {name}...");
            let spawn_res = std::thread::Builder::new()
                .name("pdf-loader".into())
                .stack_size(16 * 1024 * 1024)
                .spawn(move || {
                    let res = match std::fs::read(&path) {
                        Ok(data) => {
                            eprintln!("File read success: {} bytes", data.len());
                            load_pdf_in_background(data.into(), name)
                        }
                        Err(e) => {
                            eprintln!("File read error for {:?}: {}", path, e);
                            Err(format!("Error: {e}"))
                        }
                    };
                    let _ = tx.send(AppMsg::Loaded {
                        doc_gen,
                        result: res,
                    });
                    ctx.request_repaint();
                });
            if let Err(e) = spawn_res {
                self.status = format!("Error: {e}");
            }
        }
        #[cfg(target_arch = "wasm32")]
        {
            let _ = path;
            self.status =
                "Direct file path access is not supported on Web. Use drag-and-drop.".into();
        }
    }

    fn load_pdf_bytes(&mut self, bytes: Arc<[u8]>, name: Option<&str>) {
        eprintln!("Loading PDF bytes: {} bytes", bytes.len());
        let display_name = name
            .map(|p| p.rsplit(['/', '\\']).next().unwrap_or(p))
            .unwrap_or("PDF")
            .to_owned();
        self.status = format!("Loading {display_name}...");
        self.do_load(bytes, display_name);
    }

    fn fetch_url(&mut self, url: &str) {
        let url_trimmed = url.trim().to_string();
        if url_trimmed.is_empty() { return; }
        self.is_downloading = true;
        self.status = self.tr.downloading.into();

        let tx = self.download_tx.clone();
        let ctx = self.egui_ctx.clone();
        let file_name = url_trimmed.rsplit(['/', '\\', '?']).find(|s| !s.is_empty()).unwrap_or("URL_PDF").to_string();

        #[cfg(not(target_arch = "wasm32"))]
        {
            let req = ehttp::Request::get(&url_trimmed);
            ehttp::fetch(req, move |res| {
                match res {
                    Ok(response) if response.ok => {
                        let _ = tx.send(Ok((response.bytes.into(), file_name)));
                    }
                    Ok(response) => {
                        let _ = tx.send(Err(format!("HTTP {} {}", response.status, response.status_text)));
                    }
                    Err(e) => {
                        let _ = tx.send(Err(e));
                    }
                }
                ctx.request_repaint();
            });
        }

        #[cfg(target_arch = "wasm32")]
        {
            if url_trimmed.starts_with("http://localhost") || url_trimmed.starts_with("http://127.0.0.1") {
                let req = ehttp::Request::get(&url_trimmed);
                ehttp::fetch(req, move |res| {
                    match res {
                        Ok(response) if response.ok => { let _ = tx.send(Ok((response.bytes.into(), file_name))); }
                        Ok(response) => { let _ = tx.send(Err(format!("HTTP {} {}", response.status, response.status_text))); }
                        Err(e) => { let _ = tx.send(Err(e)); }
                    }
                    ctx.request_repaint();
                });
            } else {
                let encoded = urlencoding::encode(&url_trimmed);
                let proxies = vec![
                    format!("https://api.codetabs.com/v1/proxy/?quest={}", url_trimmed),
                    format!("https://corsproxy.io/?{}", encoded),
                    format!("https://api.allorigins.win/raw?url={}", encoded),
                ];
                Self::try_fetch_proxies(proxies, 0, tx, ctx, file_name);
            }
        }
    }

    #[cfg(target_arch = "wasm32")]
    fn try_fetch_proxies(
        proxies: Vec<String>,
        idx: usize,
        tx: mpsc::Sender<Result<(Arc<[u8]>, String), String>>,
        ctx: egui::Context,
        file_name: String,
    ) {
        if idx >= proxies.len() {
            let _ = tx.send(Err("すべてのプロキシサーバーで有効なPDFデータのダウンロードに失敗しました（サイトのアクセス制限等）。".into()));
            ctx.request_repaint();
            return;
        }

        let req = ehttp::Request::get(&proxies[idx]);
        let proxies_clone = proxies.clone();
        let tx_clone = tx.clone();
        let ctx_clone = ctx.clone();
        let file_name_clone = file_name.clone();

        ehttp::fetch(req, move |res| {
            match res {
                Ok(response) if response.ok => {
                    let bytes = &response.bytes;
                    // Check if response contains valid PDF magic number (%PDF)
                    let is_pdf = bytes.len() > 4 && (bytes.starts_with(b"%PDF") || bytes[..bytes.len().min(1024)].windows(4).any(|w| w == b"%PDF"));
                    if is_pdf {
                        let _ = tx_clone.send(Ok((bytes.to_vec().into(), file_name_clone)));
                        ctx_clone.request_repaint();
                        return;
                    }
                    // If not valid PDF, try next proxy
                    Self::try_fetch_proxies(proxies_clone, idx + 1, tx_clone, ctx_clone, file_name_clone);
                }
                _ => {
                    // Try next proxy
                    Self::try_fetch_proxies(proxies_clone, idx + 1, tx_clone, ctx_clone, file_name_clone);
                }
            }
        });
    }

    #[allow(unused)]
    fn do_load(&mut self, bytes: Arc<[u8]>, #[allow(unused_variables)] name: String) {
        if bytes.len() < 4 || (!bytes.starts_with(b"%PDF") && !bytes[..bytes.len().min(1024)].windows(4).any(|w| w == b"%PDF")) {
            self.status = "Error: 取得したデータが有効なPDFファイルではありません（サーバーによるアクセス制限等の可能性）".into();
            return;
        }
        #[allow(unused_variables)]
        let doc_gen = self.doc_gen.fetch_add(1, Ordering::Relaxed) + 1;
        self.render_gen.fetch_add(1, Ordering::Relaxed);
        self.copy_gen.fetch_add(1, Ordering::Relaxed);
        #[cfg(not(target_arch = "wasm32"))]
        {
            let tx = self.tx.clone();
            let ctx = self.egui_ctx.clone();

            let spawn_res = std::thread::Builder::new()
                .name("pdf-loader".into())
                .stack_size(16 * 1024 * 1024)
                .spawn(move || {
                    let res = load_pdf_in_background(bytes, name);
                    let _ = tx.send(AppMsg::Loaded {
                        doc_gen,
                        result: res,
                    });
                    ctx.request_repaint();
                });
            if let Err(e) = spawn_res {
                self.status = format!("Error: {e}");
            }
        }
        #[cfg(target_arch = "wasm32")]
        {
            let ctx = self.egui_ctx.clone();
            let tx = self.tx.clone();
            let bytes_js = js_sys::Uint8Array::from(&bytes[..]);
            let name_clone = name.clone();

            wasm_bindgen_futures::spawn_local(async move {
                let res = get_pdf_info_js(bytes_js).await;
                let n = js_sys::Reflect::get(&res, &"numPages".into())
                    .unwrap()
                    .as_f64()
                    .unwrap_or(0.0) as u32;
                let aspect = js_sys::Reflect::get(&res, &"avgAspect".into())
                    .unwrap()
                    .as_f64()
                    .unwrap_or(0.707) as f32;

                let _ = tx.send(AppMsg::PdfInfo {
                    doc_gen,
                    page_count: n,
                    aspect,
                    name: name_clone,
                });
                ctx.request_repaint();
            });
            self.pdf_bytes = Some(bytes);
        }
    }

    #[allow(dead_code)]
    fn spawn_render(&self, _page_count: u32, _thumb_size: u32) {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let bytes = match &self.pdf_bytes {
                Some(bytes) => Arc::clone(bytes),
                None => return,
            };
            let page_count = _page_count;
            let thumb_size = _thumb_size;

            let gen = self.render_gen.fetch_add(1, Ordering::Relaxed) + 1;
            // Receive rendered images
            let render_gen = Arc::clone(&self.render_gen);
            let tx = self.tx.clone();
            let next_page = Arc::new(Mutex::new(0u32));
            let n_threads = std::thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or(4)
                .saturating_sub(1)
                .clamp(2, 4);
            for _i in 0..n_threads {
                let bytes = Arc::clone(&bytes);
                let tx = tx.clone();
                let next_page = Arc::clone(&next_page);
                let rgen = Arc::clone(&render_gen);

                let builder = std::thread::Builder::new()
                    .name(format!("render-{}", _i))
                    .stack_size(16 * 1024 * 1024); // 16MB stack

                let _ = builder.spawn(move || {
                    let doc = match build_doc(&bytes) {
                        Ok(doc) => doc,
                        Err(e) => {
                            eprintln!("Failed to build PDF document in render thread: {e}");
                            return;
                        }
                    };

                    // Render thumbnails independently from copy DPI.
                    loop {
                        if rgen.load(Ordering::Relaxed) != gen {
                            return;
                        }
                        let i = {
                            let mut guard = next_page.lock().unwrap();
                            if *guard >= page_count {
                                break;
                            }
                            let i = *guard;
                            *guard += 1;
                            i
                        };
                        match pdf_render(&doc, i as usize, thumb_size as f32, None) {
                            Ok(img) => {
                                if tx
                                    .send(AppMsg::Rendered {
                                        render_gen: gen,
                                        page: i as usize,
                                        img,
                                        is_full_res: false,
                                    })
                                    .is_err()
                                {
                                    return;
                                }
                            }
                            Err(error) => {
                                eprintln!("Failed to render thumbnail page {}: {}", i + 1, error);
                                if tx
                                    .send(AppMsg::RenderFailed {
                                        render_gen: gen,
                                        page: i as usize,
                                        error,
                                    })
                                    .is_err()
                                {
                                    return;
                                }
                            }
                        }
                    }
                });
            }
        }

        #[cfg(target_arch = "wasm32")]
        {
            // On Web, we can't easily spawn threads.
            // For now, we perform rendering in a single-threaded background "loop"
            // using request_animation_frame or simply processing a few per frame.
            // A more robust way is to use Web Workers, but that requires more setup.
            log::info!("Starting single-threaded rendering for Wasm");
        }
    }

    fn copy_page(&mut self, ctx: &egui::Context, page_num: usize) {
        let _ = ctx;
        if page_num >= self.pages.len() {
            return;
        }
        let doc_gen = self.doc_gen.load(Ordering::Relaxed);
        let copy_gen = self.copy_gen.fetch_add(1, Ordering::Relaxed) + 1;

        // Reset Done state of previous pages
        for p in &mut self.pages {
            if p.copy_state == CopyState::Done {
                p.copy_state = CopyState::Idle;
            }
        }
        self.pages[page_num].copy_state = CopyState::Copying;

        #[cfg(target_arch = "wasm32")]
        {
            let bytes = match &self.pdf_bytes {
                Some(b) => Arc::clone(b),
                None => return,
            };
            let ctx = ctx.clone();
            let copy_tx = self.copy_tx.clone();
            let tr = self.tr;
            let page_num = page_num;
            let dpi = self.dpi;

            self.status = tr.copying.replace("\n", " ");
            wasm_bindgen_futures::spawn_local(async move {
                let bytes_js = js_sys::Uint8Array::from(&bytes[..]);
                let res = render_pdf_page_js(bytes_js, page_num as u32, dpi).await;
                let msg = match parse_js_image(res) {
                    Ok(img) => match platform::clipboard_set_web(&img).await {
                        Ok(()) => CopyMsg::Done {
                            doc_gen,
                            copy_gen,
                            page: page_num,
                        },
                        Err(error) => CopyMsg::Failed {
                            doc_gen,
                            copy_gen,
                            page: page_num,
                            error,
                        },
                    },
                    Err(error) => CopyMsg::Failed {
                        doc_gen,
                        copy_gen,
                        page: page_num,
                        error,
                    },
                };
                let _ = copy_tx.send(msg);
                ctx.request_repaint();
            });
            return;
        }

        // --- Native: use cached high-DPI image if ready, otherwise fallback ---
        #[cfg(not(target_arch = "wasm32"))]
        {
            if let Some(ref img) = self.pages[page_num].img {
                if self.pages[page_num].is_full_res {
                    let img_to_copy = img.clone();
                    let copy_tx = self.copy_tx.clone();
                    let ctx = ctx.clone();
                    std::thread::spawn(move || {
                        let copy_res = platform::set_clipboard_image(img_to_copy);
                        let msg = match copy_res {
                            Ok(()) => CopyMsg::Done {
                                doc_gen,
                                copy_gen,
                                page: page_num,
                            },
                            Err(error) => CopyMsg::Failed {
                                doc_gen,
                                copy_gen,
                                page: page_num,
                                error,
                            },
                        };
                        let _ = copy_tx.send(msg);
                        ctx.request_repaint();
                    });

                    self.selected_page = Some(page_num);
                    return;
                }
            }

            // Fallback: asynchronous render with large stack if cache is missing
            let dpi = self.dpi;
            let bytes = match &self.pdf_bytes {
                Some(bytes) => Arc::clone(bytes),
                None => return,
            };

            self.status = self.tr.copying.replace("\n", " ");
            let copy_tx = self.copy_tx.clone();
            let ctx = ctx.clone();

            let spawn_res = std::thread::Builder::new()
                .name(format!("copy-render-{}", page_num))
                .stack_size(16 * 1024 * 1024)
                .spawn(move || {
                    let res = (|| {
                        let doc = build_doc(&bytes)?;
                        pdf_render(&doc, page_num, 0.0, Some(dpi))
                    })();

                    match res {
                        Ok(img) => {
                            let copy_res = platform::set_clipboard_image(img);
                            let msg = match copy_res {
                                Ok(()) => CopyMsg::Done {
                                    doc_gen,
                                    copy_gen,
                                    page: page_num,
                                },
                                Err(error) => CopyMsg::Failed {
                                    doc_gen,
                                    copy_gen,
                                    page: page_num,
                                    error,
                                },
                            };
                            let _ = copy_tx.send(msg);
                            ctx.request_repaint();
                        }
                        Err(error) => {
                            let _ = copy_tx.send(CopyMsg::Failed {
                                doc_gen,
                                copy_gen,
                                page: page_num,
                                error,
                            });
                            ctx.request_repaint();
                        }
                    }
                });
            if let Err(e) = spawn_res {
                self.pages[page_num].copy_state = CopyState::Idle;
                self.status = format!("Error: {e}");
            }
        }
    }
}

// ---------------------------------------------------------------------------
// PDF rendering  (pdf_oxide + tiny-skia)
// ---------------------------------------------------------------------------

#[cfg(not(target_arch = "wasm32"))]
fn pdf_render(
    doc: &PdfDocument,
    page_num: usize,
    max_size: f32,
    dpi: Option<f32>,
) -> Result<ColorImage, String> {
    let rendered = if let Some(d) = dpi {
        render_page(doc, page_num, &RenderOptions::with_dpi(d as u32).as_raw())
            .map_err(|e| e.to_string())?
    } else {
        let sz = max_size as u32;
        render_page_fit(doc, page_num, sz, sz, &RenderOptions::default().as_raw())
            .map_err(|e| e.to_string())?
    };

    let w = rendered.width as usize;
    let h = rendered.height as usize;

    // tiny-skia outputs premultiplied RGBA. Un-premultiply to straight RGBA for egui.
    // For scanned PDFs with white background alpha is always 255, so this is fast.
    #[allow(unused_mut)]
    let mut pixels: Vec<egui::Color32> = rendered
        .data
        .chunks_exact(4)
        .map(|p| {
            let a = p[3];
            let (r, g, b) = if a == 0 {
                (0, 0, 0)
            } else if a == 255 {
                (p[0], p[1], p[2])
            } else {
                let af = a as f32 / 255.0;
                (
                    (p[0] as f32 / af).min(255.0) as u8,
                    (p[1] as f32 / af).min(255.0) as u8,
                    (p[2] as f32 / af).min(255.0) as u8,
                )
            };
            egui::Color32::from_rgba_unmultiplied(r, g, b, a)
        })
        .collect();

    // Fast sharpening for thumbnails. Copy rendering passes a DPI; thumbnails use fit size.
    if dpi.is_none() {
        sharpen_image(&mut pixels, w, h);
    }

    Ok(ColorImage {
        size: [w, h],
        pixels,
    })
}

#[allow(unused)]
fn sharpen_image(pixels: &mut [egui::Color32], w: usize, h: usize) {
    if w < 3 || h < 3 {
        return;
    }
    let original = pixels.to_vec();
    // Simple 3x3 sharpening kernel:
    // [ 0, -1,  0]
    // [-1,  5, -1]
    // [ 0, -1,  0]
    for y in 1..h - 1 {
        for x in 1..w - 1 {
            let idx = y * w + x;
            let center = original[idx];
            let top = original[(y - 1) * w + x];
            let bottom = original[(y + 1) * w + x];
            let left = original[y * w + x - 1];
            let right = original[y * w + x + 1];

            let sharpen = |c1: u8, c2: u8, c3: u8, c4: u8, c5: u8| -> u8 {
                let val = 5 * (c1 as i32) - (c2 as i32) - (c3 as i32) - (c4 as i32) - (c5 as i32);
                val.clamp(0, 255) as u8
            };

            pixels[idx] = egui::Color32::from_rgba_unmultiplied(
                sharpen(center.r(), top.r(), bottom.r(), left.r(), right.r()),
                sharpen(center.g(), top.g(), bottom.g(), left.g(), right.g()),
                sharpen(center.b(), top.b(), bottom.b(), left.b(), right.b()),
                center.a(),
            );
        }
    }
}

// ---------------------------------------------------------------------------
// egui App
// ---------------------------------------------------------------------------

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // --- WASM: On-demand thumbnail rendering (via PDF.js) ---
        #[cfg(target_arch = "wasm32")]
        {
            let mut target = None;
            for i in 0..self.pages.len() {
                if self.pages[i].img.is_none() {
                    target = Some(i);
                    break;
                }
            }
            if let Some(i) = target {
                // Set a placeholder to avoid re-triggering
                self.pages[i].img = Some(egui::ColorImage::new([1, 1], egui::Color32::TRANSPARENT));

                let tx = self.tx.clone();
                let ctx = ctx.clone();
                let bytes = Arc::clone(self.pdf_bytes.as_ref().unwrap());
                let render_gen = self.render_gen.load(Ordering::Relaxed);
                wasm_bindgen_futures::spawn_local(async move {
                    let bytes_js = js_sys::Uint8Array::from(&bytes[..]);
                    let res = render_pdf_page_js(bytes_js, i as u32, 100.0).await;
                    if let Ok(img) = parse_js_image(res) {
                        let _ = tx.send(AppMsg::Rendered {
                            render_gen,
                            page: i,
                            img,
                            is_full_res: false,
                        });
                        ctx.request_repaint();
                    }
                });
            }
        }
        let dropped = ctx.input(|i| i.raw.dropped_files.clone());
        if !dropped.is_empty() {
            for f in &dropped {
                eprintln!(
                    "File dropped: name={}, path={:?}, bytes={}",
                    f.name,
                    f.path,
                    f.bytes.as_ref().map(|b| b.len()).unwrap_or(0)
                );
            }
            let f = &dropped[0];
            if let Some(path) = &f.path {
                let ext = path
                    .extension()
                    .and_then(|e| e.to_str())
                    .unwrap_or("")
                    .to_ascii_lowercase();
                eprintln!("Extension detected: '{}'", ext);
                if ext == "pdf" {
                    self.load_pdf_path(path);
                } else {
                    eprintln!("Ignoring file with extension: '{}'", ext);
                    self.status = format!("Ignored non-PDF file: {}", f.name);
                }
            } else if let Some(bytes) = &f.bytes {
                eprintln!("Loading from dropped bytes ({} bytes)", bytes.len());
                self.load_pdf_bytes(bytes.to_vec().into(), Some(&f.name));
            } else {
                eprintln!("Dropped file has no path or bytes");
                self.status = format!("Dropped file has no path/data: {}", f.name);
            }
        }

        // Receive copy finished notifications
        if let Ok(rx) = self.copy_rx.try_lock() {
            while let Ok(msg) = rx.try_recv() {
                match msg {
                    CopyMsg::Done {
                        doc_gen,
                        copy_gen,
                        page: i,
                    } => {
                        if doc_gen == self.doc_gen.load(Ordering::Relaxed)
                            && copy_gen == self.copy_gen.load(Ordering::Relaxed)
                            && i < self.pages.len()
                        {
                            self.pages[i].copy_state = CopyState::Done;
                            self.copied_page = Some(i);
                            self.status = (self.tr.status_copy_done)(i + 1);
                        }
                    }
                    CopyMsg::Failed {
                        doc_gen,
                        copy_gen,
                        page: i,
                        error,
                    } => {
                        if doc_gen == self.doc_gen.load(Ordering::Relaxed)
                            && copy_gen == self.copy_gen.load(Ordering::Relaxed)
                            && i < self.pages.len()
                        {
                            self.pages[i].copy_state = CopyState::Idle;
                            self.status = format!("Error: {error}");
                        }
                    }
                }
            }
        }

        // Receive downloaded PDF bytes
        let mut downloaded_data = None;
        if let Ok(rx) = self.download_rx.try_lock() {
            if let Ok(res) = rx.try_recv() {
                downloaded_data = Some(res);
            }
        }
        if let Some(res) = downloaded_data {
            self.is_downloading = false;
            match res {
                Ok((bytes, name)) => {
                    self.load_pdf_bytes(bytes, Some(&name));
                }
                Err(e) => {
                    self.status = (self.tr.download_err)(&e);
                }
            }
        }

        {
            let rx = self.rx.lock().unwrap();
            let mut n_recv = 0;
            while let Ok(msg) = rx.try_recv() {
                match msg {
                    AppMsg::Rendered {
                        render_gen,
                        page: i,
                        img,
                        is_full_res: is_full,
                    } => {
                        if render_gen == self.render_gen.load(Ordering::Relaxed)
                            && i < self.pages.len()
                        {
                            // Only update texture if it's the low-res pass (is_full == false)
                            // or if we somehow don't have a texture yet.
                            if !is_full || self.pages[i].tex.is_none() {
                                self.pages[i].tex = Some(ctx.load_texture(
                                    format!("p{i}"),
                                    img.clone(),
                                    TextureOptions::LINEAR,
                                ));
                            }
                            #[cfg(target_arch = "wasm32")]
                            {
                                self.pages[i].img = Some(img.clone());
                            }
                            #[cfg(not(target_arch = "wasm32"))]
                            {
                                if is_full {
                                    self.pages[i].img = Some(img.clone());
                                }
                            }
                            self.pages[i].is_full_res = is_full;
                            self.pages[i].render_error = None;
                        }
                    }
                    AppMsg::RenderFailed {
                        render_gen,
                        page: i,
                        error,
                    } => {
                        if render_gen == self.render_gen.load(Ordering::Relaxed)
                            && i < self.pages.len()
                        {
                            self.pages[i].tex = None;
                            self.pages[i].render_error = Some(error.clone());
                            self.status = format!("Page {} render failed: {error}", i + 1);
                        }
                    }
                    #[cfg(not(target_arch = "wasm32"))]
                    AppMsg::Loaded {
                        doc_gen,
                        result: res,
                    } => {
                        if doc_gen != self.doc_gen.load(Ordering::Relaxed) {
                            n_recv += 1;
                            if n_recv >= 4 {
                                break;
                            }
                            continue;
                        }
                        match res {
                            Ok(loaded) => {
                                eprintln!("PDF loaded successfully: {} pages", loaded.page_count);
                                self.page_aspect = loaded.aspect;
                                self.page_count = loaded.page_count;
                                self.copied_page = None;
                                self.selected_page = Some(0);
                                self.pages = (0..loaded.page_count)
                                    .map(|_| PageSlot {
                                        tex: None,
                                        img: None,
                                        is_full_res: false,
                                        copy_state: CopyState::Idle,
                                        render_error: None,
                                    })
                                    .collect();
                                self.pdf_bytes = Some(loaded.bytes);
                                self.status = format!(
                                    "{}{}",
                                    loaded.name,
                                    (self.tr.pages_count)(loaded.page_count)
                                );
                                self.spawn_render(loaded.page_count, self.thumb_size);
                            }
                            Err(e) => {
                                eprintln!("PDF load error: {}", e);
                                self.status = e;
                            }
                        }
                    }
                    #[cfg(target_arch = "wasm32")]
                    AppMsg::PdfInfo {
                        doc_gen,
                        page_count,
                        aspect,
                        name,
                    } => {
                        if doc_gen != self.doc_gen.load(Ordering::Relaxed) {
                            n_recv += 1;
                            if n_recv >= 4 {
                                break;
                            }
                            continue;
                        }
                        self.page_count = page_count;
                        self.page_aspect = aspect;
                        self.selected_page = Some(0);
                        self.pages = (0..page_count)
                            .map(|_| PageSlot {
                                tex: None,
                                img: None,
                                is_full_res: false,
                                copy_state: CopyState::Idle,
                                render_error: None,
                            })
                            .collect();
                        self.status = format!("{}{}", name, (self.tr.pages_count)(page_count));
                    }
                }
                n_recv += 1;
                if n_recv >= 4 {
                    break;
                }
            }
        }
        // Repaint if we still have pages that haven't even finished pass 1
        if self.page_count > 0
            && self
                .pages
                .iter()
                .any(|p| p.tex.is_none() && p.render_error.is_none())
        {
            ctx.request_repaint();
        }

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.style_mut().spacing.item_spacing.y = 0.0;
                ui.label(self.tr.dpi);
                ui.add(
                    egui::DragValue::new(&mut self.dpi)
                        .range(72.0..=600.0f32)
                        .speed(1.0)
                        .suffix(" dpi"),
                );
                ui.add_space(16.0);
                ui.label(self.tr.thumbnail);
                let old = self.thumb_size;
                ui.add(
                    egui::DragValue::new(&mut self.thumb_size)
                        .range(80..=512u32)
                        .speed(8.0)
                        .suffix(" px"),
                );
                self.thumb_size = (self.thumb_size / 8) * 8;
                if self.thumb_size != old && self.pdf_bytes.is_some() {
                    // Do NOT clear textures or images! Let egui GPU scale dynamically in real-time.
                }
                ui.add_space(16.0);

                ui.label(self.tr.url_prompt);
                let url_resp = ui.add(egui::TextEdit::singleline(&mut self.url_input)
                    .hint_text("https://...")
                    .desired_width(180.0));
                if ui.button(self.tr.open_url).clicked() || (url_resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter))) {
                    self.fetch_url(&self.url_input.clone());
                }
                if self.is_downloading {
                    ui.spinner();
                }
                ui.add_space(16.0);
                ui.add_space(16.0);

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(egui::RichText::new(&self.status).color(egui::Color32::GRAY));
                });
            });
            ui.separator();

            if self.page_count == 0 {
                ui.centered_and_justified(|ui| {
                    ui.label(
                        egui::RichText::new(self.tr.drop_pdf)
                            .color(egui::Color32::GRAY)
                            .size(20.0),
                    );
                });
                return;
            }

            let mut clicked: Option<usize> = None;
            let n = self.page_count as usize;

            let thumb_f = self.thumb_size as f32;
            let avail_w = ui.available_width();
            let cell_w = (thumb_f * self.page_aspect).max(20.0);
            let cols = (avail_w / cell_w).floor().max(1.0) as usize;
            let gap = if cols > 1 {
                ((avail_w - cols as f32 * cell_w) / (cols - 1) as f32).max(0.0)
            } else {
                0.0
            };
            let cell_h = thumb_f + 20.0;

            // Keyboard navigation
            let mut scroll_to_selected = false;
            if self.page_count > 0 {
                let n = self.page_count as usize;
                let cur = self.selected_page.unwrap_or(0);

                ui.input(|i| {
                    if i.key_pressed(egui::Key::ArrowRight) {
                        let new_idx = (cur + 1).min(n - 1);
                        self.selected_page = Some(new_idx);
                        scroll_to_selected = true;
                    }
                    if i.key_pressed(egui::Key::ArrowLeft) {
                        let new_idx = cur.saturating_sub(1);
                        self.selected_page = Some(new_idx);
                        scroll_to_selected = true;
                    }
                    if i.key_pressed(egui::Key::ArrowDown) {
                        let new_idx = (cur + cols).min(n - 1);
                        self.selected_page = Some(new_idx);
                        scroll_to_selected = true;
                    }
                    if i.key_pressed(egui::Key::ArrowUp) {
                        let new_idx = cur.saturating_sub(cols);
                        self.selected_page = Some(new_idx);
                        scroll_to_selected = true;
                    }
                    if i.key_pressed(egui::Key::PageUp) {
                        let rows = (ui.available_height() / cell_h).floor().max(1.0) as usize;
                        self.selected_page = Some(cur.saturating_sub(cols * rows));
                        scroll_to_selected = true;
                    }
                    if i.key_pressed(egui::Key::PageDown) {
                        let rows = (ui.available_height() / cell_h).floor().max(1.0) as usize;
                        self.selected_page = Some((cur + cols * rows).min(n - 1));
                        scroll_to_selected = true;
                    }
                    if i.key_pressed(egui::Key::Home) {
                        self.selected_page = Some(0);
                        scroll_to_selected = true;
                    }
                    if i.key_pressed(egui::Key::End) {
                        self.selected_page = Some(n - 1);
                        scroll_to_selected = true;
                    }
                    if i.key_pressed(egui::Key::Enter) || i.key_pressed(egui::Key::Space) {
                        if let Some(idx) = self.selected_page {
                            clicked = Some(idx);
                        }
                    }
                });
            }

            let mut red_borders = Vec::new();
            let mut blue_border = None;
            let mut hover_rect = None;
            let pointer_moved = ui.input(|i| i.pointer.delta().length_sq() > 0.0);

            ScrollArea::vertical().show(ui, |ui| {
                // Increase scroll sensitivity (add extra scroll)
                let extra_scroll = ui.input(|i| i.smooth_scroll_delta);
                ui.scroll_with_delta(extra_scroll * 1.2);

                ui.spacing_mut().item_spacing.y = 0.0;
                for row in 0..n.div_ceil(cols) {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = gap;
                        for col in 0..cols {
                            let i = row * cols + col;
                            if i >= n {
                                break;
                            }
                            ui.allocate_ui_with_layout(
                                egui::vec2(cell_w, cell_h),
                                egui::Layout::top_down(egui::Align::Center),
                                |ui| {
                                    let (resp, rect) = if let Some(tex) = &self.pages[i].tex {
                                        let [iw, ih] = tex.size();
                                        let scale = (cell_w / iw as f32).min(thumb_f / ih as f32);
                                        let disp = egui::vec2(iw as f32 * scale, ih as f32 * scale);
                                        let resp = ui.add(
                                            egui::Image::new(egui::load::SizedTexture::new(
                                                tex.id(),
                                                disp,
                                            ))
                                            .sense(egui::Sense::click()),
                                        );
                                        let rect = resp.rect;
                                        (resp, rect)
                                    } else {
                                        let (rect, resp) = ui.allocate_exact_size(
                                            egui::vec2(cell_w.min(thumb_f), thumb_f),
                                            egui::Sense::click(),
                                        );
                                        let fill = if self.pages[i].render_error.is_some() {
                                            egui::Color32::from_rgb(70, 35, 35)
                                        } else {
                                            egui::Color32::from_gray(50)
                                        };
                                        ui.painter().rect_filled(rect, 4.0, fill);
                                        if self.pages[i].render_error.is_some() {
                                            ui.painter().text(
                                                rect.center(),
                                                egui::Align2::CENTER_CENTER,
                                                "Render\nerror",
                                                egui::FontId::proportional(13.0),
                                                egui::Color32::WHITE,
                                            );
                                        }
                                        (resp, rect)
                                    };

                                    if resp.clicked() {
                                        clicked = Some(i);
                                        self.selected_page = Some(i);
                                    }
                                    if resp.hovered() && pointer_moved {
                                        self.selected_page = Some(i);
                                    }

                                    // Selection/Focus highlight (Unified Mouse & Keyboard)
                                    let is_selected = self.selected_page == Some(i);

                                    if is_selected && self.pages[i].copy_state != CopyState::Done {
                                        // Unified Blue highlight
                                        ui.painter().rect_filled(
                                            rect,
                                            0.0,
                                            egui::Color32::from_rgba_unmultiplied(0, 120, 215, 100),
                                        );

                                        // Show guide text
                                        let txt = self.tr.click_to_copy;
                                        let font = egui::FontId::proportional(14.0);
                                        let center = rect.center();

                                        let mut job_w = egui::text::LayoutJob::simple(
                                            txt.to_string(),
                                            font.clone(),
                                            egui::Color32::WHITE,
                                            f32::INFINITY,
                                        );
                                        job_w.halign = egui::Align::Center;
                                        let mut job_b = egui::text::LayoutJob::simple(
                                            txt.to_string(),
                                            font.clone(),
                                            egui::Color32::BLACK,
                                            f32::INFINITY,
                                        );
                                        job_b.halign = egui::Align::Center;

                                        let galley_w = ui.fonts(|f| f.layout_job(job_w));
                                        let galley_b = ui.fonts(|f| f.layout_job(job_b));
                                        let text_pos = center - galley_w.rect.center().to_vec2();

                                        // Draw white outline for readability
                                        for off in [
                                            egui::vec2(-1.0, -1.0),
                                            egui::vec2(1.0, -1.0),
                                            egui::vec2(-1.0, 1.0),
                                            egui::vec2(1.0, 1.0),
                                        ] {
                                            ui.painter().galley(
                                                text_pos + off,
                                                galley_w.clone(),
                                                egui::Color32::WHITE,
                                            );
                                        }
                                        // Draw black main text
                                        ui.painter().galley(
                                            text_pos,
                                            galley_b,
                                            egui::Color32::BLACK,
                                        );
                                    }

                                    if is_selected {
                                        hover_rect = Some(rect);
                                    }

                                    // Status overlay text
                                    match self.pages[i].copy_state {
                                        CopyState::Copying => {
                                            ui.painter().text(
                                                rect.center(),
                                                egui::Align2::CENTER_CENTER,
                                                self.tr.copying,
                                                egui::FontId::proportional(20.0),
                                                egui::Color32::WHITE,
                                            );
                                        }
                                        CopyState::Done => {
                                            ui.painter().text(
                                                rect.center(),
                                                egui::Align2::CENTER_CENTER,
                                                self.tr.done,
                                                egui::FontId::proportional(20.0),
                                                egui::Color32::WHITE,
                                            );
                                            red_borders.push(rect);
                                        }
                                        CopyState::Idle => {}
                                    }

                                    // Page number overlay (bottom-center)
                                    let txt = format!("{}", i + 1);
                                    let font = egui::FontId::proportional(11.0);
                                    let galley = ui.painter().layout_no_wrap(
                                        txt,
                                        font,
                                        egui::Color32::BLACK,
                                    );
                                    let bg_rect = egui::Rect::from_center_size(
                                        rect.center_bottom() + egui::vec2(0.0, -8.0),
                                        galley.size() + egui::vec2(6.0, 2.0),
                                    );
                                    ui.painter().rect_filled(
                                        bg_rect,
                                        2.0,
                                        egui::Color32::WHITE.gamma_multiply(0.8),
                                    );
                                    ui.painter().galley(
                                        bg_rect.min + egui::vec2(3.0, 1.0),
                                        galley,
                                        egui::Color32::BLACK,
                                    );

                                    // Collect Blue border
                                    if self.selected_page == Some(i) {
                                        blue_border = Some(rect);
                                        if scroll_to_selected {
                                            ui.scroll_to_rect(rect, None);
                                        }
                                    }
                                },
                            );
                        }
                    });
                }

                // DRAW BORDERS AT THE END (ON TOP OF EVERYTHING)
                for r in red_borders {
                    ui.painter().rect_stroke(
                        r.shrink(2.0),
                        2.0,
                        egui::Stroke::new(4.0, egui::Color32::RED),
                    );
                }
                if let Some(r) = blue_border {
                    ui.painter().rect_stroke(
                        r.expand(2.0),
                        2.0,
                        egui::Stroke::new(3.0, egui::Color32::from_rgb(0, 120, 215)),
                    );
                }

                // Draw hover/selection border.
                if let Some(r) = hover_rect {
                    ui.painter().rect_stroke(
                        r.expand(2.0),
                        1.0,
                        egui::Stroke::new(
                            2.0,
                            egui::Color32::from_rgba_unmultiplied(0, 120, 215, 180),
                        ),
                    );
                }
            });

            if let Some(i) = clicked {
                self.copied_page = Some(i);
                self.copy_page(ctx, i);
            }
        });
    }
}

// ---------------------------------------------------------------------------
// main
// ---------------------------------------------------------------------------

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    let opts = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("PDF Page to Clipboard")
            .with_inner_size([960.0, 720.0])
            .with_drag_and_drop(true),
        ..Default::default()
    };
    eframe::run_native(
        "PDF Page to Clipboard",
        opts,
        Box::new(|cc| Ok(Box::new(App::new(cc)))),
    )
    .unwrap();
}

#[cfg(target_arch = "wasm32")]
fn main() {
    // Redirect log to console
    console_log::init_with_level(log::Level::Info).expect("error initializing logger");

    let web_options = eframe::WebOptions::default();

    wasm_bindgen_futures::spawn_local(async {
        let canvas_id = "the_canvas_id";
        let document = web_sys::window()
            .and_then(|win| win.document())
            .expect("Could not find window or document");
        let canvas = document
            .get_element_by_id(canvas_id)
            .and_then(|element| element.dyn_into::<web_sys::HtmlCanvasElement>().ok())
            .expect("Could not find canvas element");

        eframe::WebRunner::new()
            .start(
                canvas,
                web_options,
                Box::new(|cc| Ok(Box::new(App::new(cc)))),
            )
            .await
            .expect("failed to start eframe");
    });
}
