#![windows_subsystem = "windows"]

use eframe::egui::{self, ColorImage, ScrollArea, TextureHandle, TextureOptions};
use pdf_oxide::{
    document::PdfDocument,
    rendering::{
        list_installed_font_families, render_page, render_page_fit, set_preferred_cjk_font,
        set_preferred_latin_font, RenderOptions,
    },
};
use std::sync::{
    atomic::{AtomicU32, Ordering},
    mpsc, Arc, Mutex,
};
use windows::Win32::{
    Graphics::Gdi::BI_RGB,
    System::{
        DataExchange::{CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData},
        Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE},
    },
};

// ---------------------------------------------------------------------------
// App state
// ---------------------------------------------------------------------------

struct PageSlot {
    tex: Option<TextureHandle>,
}

struct App {
    pdf_bytes:       Option<Arc<Vec<u8>>>,
    page_count:      u32,
    pages:           Vec<PageSlot>,
    thumb_size:      u32,
    page_aspect:     f32,
    dpi:             f32,
    n_threads:       usize,
    cjk_font:        String,
    latin_font:      String,
    installed_fonts: Vec<String>,
    status:          String,
    tx:              mpsc::Sender<(usize, ColorImage)>,
    rx:              Arc<Mutex<mpsc::Receiver<(usize, ColorImage)>>>,
    render_gen:      Arc<AtomicU32>,
}

fn settings_path() -> Option<std::path::PathBuf> {
    let appdata = std::env::var("APPDATA").ok()?;
    let dir = std::path::Path::new(&appdata).join("PDFPageCopier");
    std::fs::create_dir_all(&dir).ok()?;
    Some(dir.join("settings.json"))
}

fn json_get(json: &str, key: &str) -> String {
    let needle = format!("\"{}\":\"", key);
    let start = match json.find(&needle) {
        Some(i) => i + needle.len(),
        None => return String::new(),
    };
    let rest = &json[start..];
    let end = rest.find('"').unwrap_or(rest.len());
    rest[..end].to_string()
}

fn json_esc(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

// Returns (cjk_font, latin_font)
fn load_font_settings() -> (String, String) {
    let path = match settings_path() {
        Some(p) => p,
        None => return (String::new(), String::new()),
    };
    let text = std::fs::read_to_string(&path).unwrap_or_default();
    (json_get(&text, "cjk_font"), json_get(&text, "latin_font"))
}

fn save_font_settings(cjk: &str, latin: &str) {
    let Some(path) = settings_path() else { return; };
    let _ = std::fs::write(
        path,
        format!(
            "{{\"cjk_font\":\"{}\",\"latin_font\":\"{}\"}}",
            json_esc(cjk),
            json_esc(latin)
        ),
    );
}

fn build_doc(bytes: &[u8]) -> Result<PdfDocument, String> {
    PdfDocument::from_bytes(bytes.to_vec()).map_err(|e| e.to_string())
}

fn setup_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
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
                    v.push("jp".to_owned());
                }
            }
            break;
        }
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
        let (cjk_font, latin_font) = load_font_settings();
        set_preferred_cjk_font(&cjk_font);
        set_preferred_latin_font(&latin_font);
        // Enumerate installed fonts once at startup (queries fontdb — takes ~10ms cold).
        let installed_fonts = list_installed_font_families();
        Self {
            pdf_bytes: None,
            page_count: 0,
            pages: Vec::new(),
            thumb_size: 180,
            page_aspect: 0.707,
            dpi: 300.0,
            n_threads,
            cjk_font,
            latin_font,
            installed_fonts,
            status: "Drop a PDF file onto this window".into(),
            tx,
            rx: Arc::new(Mutex::new(rx)),
            render_gen: Arc::new(AtomicU32::new(0)),
        }
    }

    fn load_pdf(&mut self, path: &str) {
        match self.do_load(path) {
            Ok(n) => {
                let name = path.rsplit(['/', '\\']).next().unwrap_or(path);
                self.status = format!("{name}  ({n} pages)  —  click a page to copy");
            }
            Err(e) => self.status = format!("Error: {e}"),
        }
    }

    fn do_load(&mut self, path: &str) -> Result<u32, String> {
        let data = std::fs::read(path).map_err(|e| e.to_string())?;
        let bytes = Arc::new(data);

        let doc = build_doc(&bytes)?;
        let n = doc.page_count().map_err(|e| e.to_string())? as u32;

        // Scan all pages for average aspect ratio (metadata only, fast)
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
        self.page_aspect = if count > 0 { total_aspect / count as f32 } else { 0.707 };

        self.page_count = n;
        self.pages = (0..n).map(|_| PageSlot { tex: None }).collect();
        self.pdf_bytes = Some(bytes);
        self.spawn_render(n, self.thumb_size);
        Ok(n)
    }

    fn spawn_render(&self, page_count: u32, thumb_size: u32) {
        let bytes = match &self.pdf_bytes {
            Some(b) => Arc::clone(b),
            None => return,
        };

        let gen        = self.render_gen.fetch_add(1, Ordering::Relaxed) + 1;
        let render_gen = Arc::clone(&self.render_gen);
        let tx         = self.tx.clone();
        let next_page  = Arc::new(Mutex::new(0u32));

        let n_threads = std::thread::available_parallelism()
            .map(|p| p.get().saturating_sub(1).max(1))
            .unwrap_or(1);

        for _ in 0..n_threads {
            let bytes     = Arc::clone(&bytes);
            let tx        = tx.clone();
            let next_page = Arc::clone(&next_page);
            let rgen      = Arc::clone(&render_gen);

            std::thread::spawn(move || {
                let doc = match build_doc(&bytes) {
                    Ok(d) => d,
                    Err(_) => return,
                };

                loop {
                    if rgen.load(Ordering::Relaxed) != gen { break; }

                    let i = {
                        let mut guard = next_page.lock().unwrap();
                        if *guard >= page_count { break; }
                        let i = *guard;
                        *guard += 1;
                        i
                    };

                    if let Ok(img) = pdf_render(&doc, i as usize, thumb_size as f32, None) {
                        if tx.send((i as usize, img)).is_err() { break; }
                    }
                }
            });
        }
    }

    fn copy_page(&mut self, page_num: usize) {
        let bytes = match &self.pdf_bytes {
            Some(b) => Arc::clone(b),
            None => return,
        };
        let dpi = self.dpi;

        match build_doc(&bytes) {
            Ok(doc) => match pdf_render(&doc, page_num, 0.0, Some(dpi)) {
                Ok(img) => {
                    let w = img.size[0];
                    let h = img.size[1];
                    clipboard_set(&img);
                    self.status = format!("Page {} copied  ({w}×{h} px @ {dpi} DPI)", page_num + 1);
                }
                Err(e) => self.status = format!("Error copying page {}: {e}", page_num + 1),
            },
            Err(e) => self.status = format!("Error: {e}"),
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

    if dpi.is_none() {
        sharpen_rgba(&mut pixels, w, h);
    }

    Ok(ColorImage { size: [w, h], pixels })
}

fn sharpen_rgba(pixels: &mut Vec<egui::Color32>, w: usize, h: usize) {
    if w < 3 || h < 3 { return; }
    let src = pixels.clone();
    #[rustfmt::skip]
    const K: [f32; 9] = [0.0, -0.5, 0.0, -0.5, 3.0, -0.5, 0.0, -0.5, 0.0];
    for y in 1..h - 1 {
        for x in 1..w - 1 {
            let (mut r, mut g, mut b) = (0.0f32, 0.0f32, 0.0f32);
            for ky in 0..3usize {
                for kx in 0..3usize {
                    let k = K[ky * 3 + kx];
                    if k == 0.0 { continue; }
                    let px = src[(y + ky - 1) * w + (x + kx - 1)];
                    r += px.r() as f32 * k;
                    g += px.g() as f32 * k;
                    b += px.b() as f32 * k;
                }
            }
            pixels[y * w + x] = egui::Color32::from_rgb(
                r.clamp(0.0, 255.0) as u8,
                g.clamp(0.0, 255.0) as u8,
                b.clamp(0.0, 255.0) as u8,
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Clipboard  (CF_DIB / 24-bit BGR, bottom-up)
// ---------------------------------------------------------------------------

fn clipboard_set(img: &ColorImage) {
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

// ---------------------------------------------------------------------------
// egui App
// ---------------------------------------------------------------------------

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let dropped = ctx.input(|i| i.raw.dropped_files.clone());
        for f in dropped {
            if let Some(p) = &f.path {
                if p.extension().map_or(false, |e| e == "pdf") {
                    self.load_pdf(&p.to_string_lossy());
                }
            }
        }

        {
            let rx = self.rx.lock().unwrap();
            let mut n = 0;
            while let Ok((i, img)) = rx.try_recv() {
                if i < self.pages.len() {
                    self.pages[i].tex = Some(
                        ctx.load_texture(format!("p{i}"), img, TextureOptions::LINEAR)
                    );
                }
                n += 1;
                if n >= 4 { break; }
            }
        }
        if self.page_count > 0 && self.pages.iter().any(|p| p.tex.is_none()) {
            ctx.request_repaint();
        }

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label("DPI:");
                ui.add(egui::DragValue::new(&mut self.dpi)
                    .range(72.0..=600.0f32).speed(1.0).suffix(" dpi"));
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

                // --- CJK font selector ---
                ui.label("CJK:");
                let old_cjk = self.cjk_font.clone();
                let cjk_label = if self.cjk_font.is_empty() {
                    "自動".to_owned()
                } else {
                    self.cjk_font.clone()
                };
                egui::ComboBox::from_id_salt("cjk_font")
                    .selected_text(cjk_label)
                    .width(160.0)
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut self.cjk_font, String::new(), "自動");
                        for name in &self.installed_fonts {
                            ui.selectable_value(&mut self.cjk_font, name.clone(), name.as_str());
                        }
                    });
                if self.cjk_font != old_cjk {
                    set_preferred_cjk_font(&self.cjk_font);
                    save_font_settings(&self.cjk_font, &self.latin_font);
                    if self.pdf_bytes.is_some() {
                        for p in &mut self.pages { p.tex = None; }
                        self.spawn_render(self.page_count, self.thumb_size);
                    }
                }

                ui.add_space(8.0);

                // --- Latin/general font selector ---
                ui.label("英数:");
                let old_latin = self.latin_font.clone();
                let latin_label = if self.latin_font.is_empty() {
                    "自動".to_owned()
                } else {
                    self.latin_font.clone()
                };
                egui::ComboBox::from_id_salt("latin_font")
                    .selected_text(latin_label)
                    .width(160.0)
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut self.latin_font, String::new(), "自動");
                        for name in &self.installed_fonts {
                            ui.selectable_value(
                                &mut self.latin_font,
                                name.clone(),
                                name.as_str(),
                            );
                        }
                    });
                if self.latin_font != old_latin {
                    set_preferred_latin_font(&self.latin_font);
                    save_font_settings(&self.cjk_font, &self.latin_font);
                    if self.pdf_bytes.is_some() {
                        for p in &mut self.pages { p.tex = None; }
                        self.spawn_render(self.page_count, self.thumb_size);
                    }
                }
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
                                        ui.add(
                                            egui::Image::new(
                                                egui::load::SizedTexture::new(tex.id(), disp))
                                            .sense(egui::Sense::click())
                                        )
                                    } else {
                                        let (rect, resp) = ui.allocate_exact_size(
                                            egui::vec2(cell_w.min(thumb_f), thumb_f),
                                            egui::Sense::click(),
                                        );
                                        ui.painter().rect_filled(
                                            rect, 4.0, egui::Color32::from_gray(50));
                                        resp
                                    };
                                    if resp.clicked() { clicked = Some(i); }
                                    resp.on_hover_text(format!("Page {} — click to copy", i + 1));
                                    ui.label(
                                        egui::RichText::new(format!("p.{}", i + 1))
                                            .color(egui::Color32::GRAY).size(11.0));
                                },
                            );
                        }
                    });
                }
            });

            if let Some(i) = clicked {
                self.copy_page(i);
            }
        });
    }
}

// ---------------------------------------------------------------------------
// main
// ---------------------------------------------------------------------------

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
