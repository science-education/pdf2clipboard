#![windows_subsystem = "windows"]

use eframe::egui::{self, ColorImage, ScrollArea, TextureHandle, TextureOptions};
use pdf_oxide::{
    document::PdfDocument,
    rendering::{render_page, render_page_fit, RenderOptions},
};
use std::sync::{
    atomic::AtomicU32,
};
#[cfg(not(target_arch = "wasm32"))]
use std::sync::atomic::Ordering;
use std::sync::{mpsc, Arc, Mutex};

#[cfg(windows)]
use windows::Win32::{
    Graphics::Gdi::BI_RGB,
    System::{
        DataExchange::{CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData},
        Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE},
    },
};

#[cfg(target_arch = "wasm32")]
use wasm_bindgen::JsCast;

// ---------------------------------------------------------------------------
// App state
// ---------------------------------------------------------------------------

struct PageSlot {
    tex:         Option<TextureHandle>,
    img:         Option<ColorImage>,
    is_full_res: bool,
}

struct App {
    pdf_bytes:       Option<Arc<[u8]>>,
    cached_doc:      Option<PdfDocument>,
    copied_page:     Option<usize>,
    page_count:      u32,
    pages:           Vec<PageSlot>,
    thumb_size:      u32,
    page_aspect:     f32,
    dpi:             f32,
    n_threads:       usize,
    cache_full_res:  bool,
    status:          String,
    tx:              mpsc::Sender<(usize, ColorImage, bool)>,
    rx:              Arc<Mutex<mpsc::Receiver<(usize, ColorImage, bool)>>>,
    #[allow(dead_code)]
    render_gen:      Arc<AtomicU32>,
}

fn build_doc(bytes: &[u8]) -> Result<PdfDocument, String> {
    PdfDocument::from_bytes(bytes.to_vec()).map_err(|e| e.to_string())
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
            fonts.font_data.insert("jp".to_owned(), egui::FontData::from_owned(data));
            for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
                if let Some(v) = fonts.families.get_mut(&family) {
                    v.insert(0, "jp".to_owned());
                }
            }
            break;
        }
    }

    #[cfg(target_arch = "wasm32")]
    {
        // On Web, we can't access local system fonts.
        // The user should bundle a font or we use the default.
        // For demonstration, we could use include_bytes! here if a font was available.
        log::warn!("Japanese fonts are not bundled for Web. CJK text may not render correctly in UI.");
    }

    ctx.set_fonts(fonts);
}

impl App {
    fn new(cc: &eframe::CreationContext) -> Self {
        setup_fonts(&cc.egui_ctx);
        let (tx, rx) = mpsc::channel();
        let n_threads = std::thread::available_parallelism()
            .map(|p| p.get().saturating_sub(1).max(1))
            .unwrap_or(1);
        Self {
            pdf_bytes: None,
            cached_doc: None,
            copied_page: None,
            page_count: 0,
            pages: Vec::new(),
            thumb_size: 180,
            page_aspect: 0.707,
            dpi: 180.0,
            n_threads,
            cache_full_res: false,
            status: "Drop a PDF file onto this window".into(),
            tx,
            rx: Arc::new(Mutex::new(rx)),
            render_gen: Arc::new(AtomicU32::new(0)),
        }
    }

    fn load_pdf_path(&mut self, path: &str) {
        let _ = path;
        #[cfg(not(target_arch = "wasm32"))]
        {
            match std::fs::read(path) {
                Ok(data) => self.load_pdf_bytes(data.into(), Some(path)),
                Err(e) => self.status = format!("Error: {e}"),
            }
        }
        #[cfg(target_arch = "wasm32")]
        {
             self.status = "Direct file path access is not supported on Web. Use drag-and-drop.".into();
        }
    }

    fn load_pdf_bytes(&mut self, bytes: Arc<[u8]>, name: Option<&str>) {
        match self.do_load(bytes) {
            Ok(n) => {
                let display_name = name.map(|p| p.rsplit(['/', '\\']).next().unwrap_or(p)).unwrap_or("PDF");
                self.status = format!("{display_name}  ({n} pages)  —  click a page to copy");
            }
            Err(e) => self.status = format!("Error: {e}"),
        }
    }

    fn do_load(&mut self, bytes: Arc<[u8]>) -> Result<u32, String> {
        // Move heavy analysis to a separate thread with a large stack for stability
        let (tx_load, rx_load) = std::sync::mpsc::channel();
        let bytes_for_thread = Arc::clone(&bytes);
        
        let _ = std::thread::Builder::new()
            .name("pdf-loader".into())
            .stack_size(16 * 1024 * 1024)
            .spawn(move || {
                let res = (|| -> Result<(u32, f32, PdfDocument), String> {
                    let doc = build_doc(&bytes_for_thread)?;
                    let n = doc.page_count().map_err(|e| e.to_string())? as u32;

                    // Scan all pages for average aspect ratio
                    let mut total_aspect = 0.0f32;
                    let mut count = 0u32;
                    for i in 0..n as usize {
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
                    let aspect = if count > 0 { total_aspect / count as f32 } else { 0.707 };
                    Ok((n, aspect, doc))
                })();
                let _ = tx_load.send(res);
            });

        match rx_load.recv().unwrap_or(Err("Loader thread died".into())) {
            Ok((n, aspect, doc)) => {
                self.page_aspect = aspect;
                self.page_count = n;
                self.cached_doc = Some(doc);
                self.copied_page = None;
                self.pages = (0..n).map(|_| PageSlot { tex: None, img: None, is_full_res: false }).collect();
                self.pdf_bytes = Some(bytes);
                self.spawn_render(n, self.thumb_size);
                Ok(n)
            }
            Err(e) => Err(e),
        }
    }

    fn spawn_render(&self, _page_count: u32, _thumb_size: u32) {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let bytes = match &self.pdf_bytes {
                Some(b) => Arc::clone(b),
                None => return,
            };
            let page_count = _page_count;

            let gen         = self.render_gen.fetch_add(1, Ordering::Relaxed) + 1;
            let render_gen  = Arc::clone(&self.render_gen);
            let tx          = self.tx.clone();
            let next_page      = Arc::new(Mutex::new(0u32));
            let next_page_full = Arc::new(Mutex::new(0u32));

            let cache_full = self.cache_full_res;
            for _i in 0..self.n_threads {
                let bytes     = Arc::clone(&bytes);
                let tx        = tx.clone();
                let next_page = Arc::clone(&next_page);
                let next_page_full = Arc::clone(&next_page_full);
                let rgen      = Arc::clone(&render_gen);
                let dpi       = self.dpi;

                let builder = std::thread::Builder::new()
                    .name(format!("render-{}", _i))
                    .stack_size(16 * 1024 * 1024); // 16MB stack

                let _ = builder.spawn(move || {
                let doc = match build_doc(&bytes) {
                    Ok(d) => d,
                    Err(_) => return,
                };

                // Pass 1: Quick thumbnails (36 DPI) - updates UI
                loop {
                    if rgen.load(Ordering::Relaxed) != gen { return; }
                    let i = {
                        let mut guard = next_page.lock().unwrap();
                        if *guard >= page_count { break; }
                        let i = *guard;
                        *guard += 1;
                        i
                    };
                    if let Ok(img) = pdf_render(&doc, i as usize, 0.0, Some(36.0)) {
                        if tx.send((i as usize, img, false)).is_err() { return; }
                    }
                }

                // Pass 2: High-quality (dpi) - internal cache only (if enabled)
                if cache_full {
                    loop {
                        if rgen.load(Ordering::Relaxed) != gen { return; }
                        let i = {
                            let mut guard = next_page_full.lock().unwrap();
                            if *guard >= page_count { break; }
                            let i = *guard;
                            *guard += 1;
                            i
                        };
                        if let Ok(img) = pdf_render(&doc, i as usize, 0.0, Some(dpi)) {
                            if tx.send((i as usize, img, true)).is_err() { return; }
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

    fn copy_page(&mut self, page_num: usize) {
        // --- WASM: use cached thumbnail image (instant, no re-rendering) ---
        #[cfg(target_arch = "wasm32")]
        {
            if page_num < self.pages.len() {
                if let Some(ref img) = self.pages[page_num].img {
                    let w = img.size[0];
                    let h = img.size[1];
                    clipboard_set_web(img);
                    let status_suffix = if self.pages[page_num].is_full_res { "" } else { " (low-res preview)" };
                    self.status = format!("Page {} copied{status_suffix}  ({w}×{h} px)", page_num + 1);
                    return;
                }
            }
            self.status = format!("Page {} not yet rendered", page_num + 1);
            return;
        }

        // --- Native: use cached high-DPI image if ready, otherwise fallback ---
        #[cfg(not(target_arch = "wasm32"))]
        {
            if page_num < self.pages.len() {
                if let Some(ref img) = self.pages[page_num].img {
                    if self.pages[page_num].is_full_res {
                        let w = img.size[0];
                        let h = img.size[1];
                        #[cfg(windows)]
                        clipboard_set_windows(img);
                        
                        let dpi = self.dpi;
                        self.status = format!("Page {} copied  ({w}×{h} px) (@ {dpi} DPI)", page_num + 1);
                        return;
                    }
                }
            }

            // Fallback: synchronous render with large stack if cache is missing
                let dpi = self.dpi;
                let bytes = match &self.pdf_bytes {
                    Some(b) => Arc::clone(b),
                    None => return,
                };

                let (tx_sync, rx_sync) = std::sync::mpsc::channel();
                let _ = std::thread::Builder::new()
                    .stack_size(16 * 1024 * 1024)
                    .spawn(move || {
                        let res = (|| {
                            let doc = build_doc(&bytes)?;
                            pdf_render(&doc, page_num, 0.0, Some(dpi))
                        })();
                        let _ = tx_sync.send(res);
                    });

                match rx_sync.recv().unwrap_or(Err("Thread died".into())) {
                    Ok(img) => {
                        let w = img.size[0];
                        let h = img.size[1];
                        #[cfg(windows)]
                        clipboard_set_windows(&img);
                        self.status = format!("Page {} copied  ({w}×{h} px @ {dpi} DPI)", page_num + 1);
                    }
                    Err(e) => self.status = format!("Error copying page {}: {e}", page_num + 1),
                }
        }
    }
}

// ---------------------------------------------------------------------------
// PDF rendering  (pdf_oxide + tiny-skia)
// ---------------------------------------------------------------------------

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
    let mut pixels: Vec<egui::Color32> = rendered.data.chunks_exact(4)
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

    Ok(ColorImage { size: [w, h], pixels })
}

// ---------------------------------------------------------------------------
// Clipboard  (CF_DIB / 24-bit BGR, bottom-up)
// ---------------------------------------------------------------------------

#[cfg(windows)]
fn clipboard_set_windows(img: &ColorImage) {
    let w = img.size[0] as i32;
    let h = img.size[1] as i32;
    let row_stride = (w * 3 + 3) & !3_i32;

    #[repr(C)]
    struct BIH {
        size: u32, width: i32, height: i32,
        planes: u16, bit_count: u16,
        compression: u32, size_image: u32,
        x_pels: i32, y_pels: i32,
        clr_used: u32, clr_important: u32,
    }

    let bih_sz = std::mem::size_of::<BIH>();
    let total  = bih_sz + (row_stride * h) as usize;

    unsafe {
        let Ok(hmem) = GlobalAlloc(GMEM_MOVEABLE, total) else { return; };
        let ptr = GlobalLock(hmem) as *mut u8;
        if ptr.is_null() { return; }

        let bih = BIH {
            size: bih_sz as u32, width: w, height: h,
            planes: 1, bit_count: 24,
            compression: BI_RGB.0, size_image: (row_stride * h) as u32,
            x_pels: 0, y_pels: 0, clr_used: 0, clr_important: 0,
        };
        std::ptr::copy_nonoverlapping(&bih as *const _ as *const u8, ptr, bih_sz);

        let px  = img.pixels.as_ptr();
        let dst = ptr.add(bih_sz);
        for y in 0..h as usize {
            let src_y = h as usize - 1 - y;
            let drow  = dst.add(y * row_stride as usize);
            for x in 0..w as usize {
                let c = *px.add(src_y * w as usize + x);
                let d = drow.add(x * 3);
                *d        = c.b();
                *d.add(1) = c.g();
                *d.add(2) = c.r();
            }
        }
        let _ = GlobalUnlock(hmem);

        if OpenClipboard(None).is_ok() {
            let _ = EmptyClipboard();
            let handle = windows::Win32::Foundation::HANDLE(hmem.0 as *mut _);
            let _ = SetClipboardData(8, Some(handle));
            let _ = CloseClipboard();
        }
    }
}

#[cfg(target_arch = "wasm32")]
fn clipboard_set_web(img: &ColorImage) {
    let w = img.size[0] as u32;
    let h = img.size[1] as u32;
    let mut rgba_data = Vec::with_capacity((w * h * 4) as usize);
    for px in &img.pixels {
        rgba_data.push(px.r());
        rgba_data.push(px.g());
        rgba_data.push(px.b());
        rgba_data.push(px.a());
    }

    let js_array = js_sys::Uint8Array::from(rgba_data.as_slice());

    let func = js_sys::Function::new_with_args(
        "w, h, arr",
        r#"
        try {
            const canvas = document.createElement('canvas');
            canvas.width = w;
            canvas.height = h;
            const ctx = canvas.getContext('2d');
            const imgData = ctx.createImageData(w, h);
            imgData.data.set(arr);
            ctx.putImageData(imgData, 0, 0);

            const blobPromise = new Promise((resolve, reject) => {
                canvas.toBlob((blob) => {
                    if (blob) resolve(blob);
                    else reject(new Error("Canvas toBlob failed"));
                }, 'image/png');
            });

            const item = new ClipboardItem({ 'image/png': blobPromise });
            navigator.clipboard.write([item]).then(() => {
                console.log("Image copied to clipboard successfully via Wasm!");
            }).catch((err) => {
                console.error("Clipboard write failed (check browser permissions or user activation timeout):", err);
            });
        } catch(e) {
            console.error("Web clipboard copy exception:", e);
        }
        "#
    );
    let _ = func.call3(&wasm_bindgen::JsValue::NULL, &wasm_bindgen::JsValue::from(w), &wasm_bindgen::JsValue::from(h), &js_array);
}

// ---------------------------------------------------------------------------
// egui App
// ---------------------------------------------------------------------------

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let dropped = ctx.input(|i| i.raw.dropped_files.clone());
        for f in dropped {
            if let Some(bytes) = f.bytes {
                self.load_pdf_bytes(Arc::clone(&bytes), Some(&f.name));
            } else if let Some(p) = &f.path {
                if p.extension().map_or(false, |e| e == "pdf") {
                    self.load_pdf_path(&p.to_string_lossy());
                }
            }
        }

        #[cfg(target_arch = "wasm32")]
        if self.pdf_bytes.is_some() {
             // Wasm single-threaded rendering "driver"
             // Process one page per frame to keep UI responsive
             self.drive_render_wasm(ctx);
        }

        {
            let rx = self.rx.lock().unwrap();
            let mut n_recv = 0;
            while let Ok((i, img, is_full)) = rx.try_recv() {
                if i < self.pages.len() {
                    // Only update texture if it's the low-res pass (is_full == false)
                    // or if we somehow don't have a texture yet.
                    if !is_full || self.pages[i].tex.is_none() {
                        self.pages[i].tex = Some(
                            ctx.load_texture(format!("p{i}"), img.clone(), TextureOptions::LINEAR)
                        );
                    }
                    self.pages[i].img = Some(img);
                    self.pages[i].is_full_res = is_full;
                }
                n_recv += 1;
                if n_recv >= 4 { break; }
            }
        }
        // Repaint if we still have pages that haven't even finished pass 1
        if self.page_count > 0 && self.pages.iter().any(|p| p.tex.is_none()) {
            ctx.request_repaint();
        }

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label("DPI:");
                ui.add(egui::DragValue::new(&mut self.dpi)
                    .range(72.0..=600.0f32).speed(1.0).suffix(" dpi"));
                if ui.checkbox(&mut self.cache_full_res, "Pre-cache (High-res)").changed() {
                    if self.cache_full_res && self.pdf_bytes.is_some() {
                        self.spawn_render(self.page_count, self.thumb_size);
                    }
                }
                ui.add_space(16.0);
                ui.label("Thumbnail:");
                let old = self.thumb_size;
                ui.add(egui::DragValue::new(&mut self.thumb_size)
                    .range(80..=500u32).speed(1.0).suffix(" px"));
                if self.thumb_size != old && self.pdf_bytes.is_some() {
                    for p in &mut self.pages { p.tex = None; }
                    self.spawn_render(self.page_count, self.thumb_size);
                }
                ui.add_space(16.0);
                ui.label(
                    egui::RichText::new(format!("スレッド数：{}", self.n_threads))
                        .color(egui::Color32::GRAY)
                );
                ui.add_space(16.0);

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(egui::RichText::new(&self.status).color(egui::Color32::GRAY));
                });
            });
            ui.separator();

            if self.page_count == 0 {
                ui.centered_and_justified(|ui| {
                    ui.label(
                        egui::RichText::new("Drop a PDF file onto this window")
                            .color(egui::Color32::GRAY).size(20.0)
                    );
                });
                return;
            }

            let mut clicked: Option<usize> = None;
            let n = self.page_count as usize;

            let thumb_f = self.thumb_size as f32;
            let avail_w = ui.available_width();
            let cell_w  = (thumb_f * self.page_aspect).max(20.0);
            let cols    = (avail_w / cell_w).floor().max(1.0) as usize;
            let gap     = if cols > 1 {
                ((avail_w - cols as f32 * cell_w) / (cols - 1) as f32).max(0.0)
            } else {
                0.0
            };
            let cell_h  = thumb_f + 20.0;

            ScrollArea::vertical().show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                for row in 0..n.div_ceil(cols) {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = gap;
                        for col in 0..cols {
                            let i = row * cols + col;
                            if i >= n { break; }
                            ui.allocate_ui_with_layout(
                                egui::vec2(cell_w, cell_h),
                                egui::Layout::top_down(egui::Align::Center),
                                |ui| {
                                    let resp = if let Some(tex) = &self.pages[i].tex {
                                        let [iw, ih] = tex.size();
                                        let scale = (cell_w / iw as f32)
                                            .min(thumb_f / ih as f32);
                                        let disp = egui::vec2(iw as f32 * scale, ih as f32 * scale);
                                        let resp = ui.add(
                                            egui::Image::new(
                                                egui::load::SizedTexture::new(tex.id(), disp))
                                            .sense(egui::Sense::click())
                                        );
                                        if self.copied_page == Some(i) {
                                            let rect = resp.rect.shrink(2.0);
                                            ui.painter().rect_stroke(rect, 0.0, egui::Stroke::new(4.0, egui::Color32::RED));
                                        }
                                        resp
                                    } else {
                                        let (rect, resp) = ui.allocate_exact_size(
                                            egui::vec2(cell_w.min(thumb_f), thumb_f),
                                            egui::Sense::click(),
                                        );
                                        ui.painter().rect_filled(
                                            rect, 4.0, egui::Color32::from_gray(50));
                                        if self.copied_page == Some(i) {
                                            let rect = rect.shrink(2.0);
                                            ui.painter().rect_stroke(rect, 4.0, egui::Stroke::new(4.0, egui::Color32::RED));
                                        }
                                        resp
                                    };
                                    if resp.clicked() { clicked = Some(i); }
                                    resp.on_hover_text(format!("Page {} — click to copy", i + 1));
                                    let label_color = if self.pages[i].is_full_res {
                                        egui::Color32::BLACK
                                    } else {
                                        egui::Color32::GRAY
                                    };
                                    ui.label(
                                        egui::RichText::new(format!("p.{}", i + 1))
                                            .color(label_color).size(11.0));
                                },
                            );
                        }
                    });
                }
            });

            if let Some(i) = clicked {
                self.copied_page = Some(i);
                self.copy_page(i);
            }
        });
    }
}

impl App {
    #[cfg(target_arch = "wasm32")]
    fn drive_render_wasm(&self, ctx: &egui::Context) {
        let page_count = self.page_count;
        let bytes = match &self.pdf_bytes {
            Some(b) => Arc::clone(b),
            None => return,
        };

        // Pass 1: Find first page that has NO texture (render at 36 DPI)
        let mut target_idx = None;
        for i in 0..page_count as usize {
            if self.pages[i].tex.is_none() {
                target_idx = Some(i);
                break;
            }
        }

        if let Some(i) = target_idx {
            let doc = match build_doc(&bytes) { Ok(d) => d, Err(_) => return };
            if let Ok(img) = pdf_render(&doc, i, 0.0, Some(36.0)) {
                let _ = self.tx.send((i, img, false));
                ctx.request_repaint();
                return;
            }
        }

        // Pass 2: Find first page that is not yet full res (internal cache update)
        if self.cache_full_res {
            let mut target_high = None;
            for i in 0..page_count as usize {
                if !self.pages[i].is_full_res {
                    target_high = Some(i);
                    break;
                }
            }

            if let Some(i) = target_high {
                let doc = match build_doc(&bytes) { Ok(d) => d, Err(_) => return };
                let dpi = self.dpi.min(150.0);
                if let Ok(img) = pdf_render(&doc, i, 0.0, Some(dpi)) {
                    let _ = self.tx.send((i, img, true));
                    ctx.request_repaint();
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// main
// ---------------------------------------------------------------------------

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    let opts = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("PDF Page Copier")
            .with_inner_size([960.0, 720.0])
            .with_drag_and_drop(true),
        ..Default::default()
    };
    eframe::run_native(
        "PDF Page Copier",
        opts,
        Box::new(|cc| Ok(Box::new(App::new(cc)))),
    ).unwrap();
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
