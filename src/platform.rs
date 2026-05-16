use eframe::egui::ColorImage;

#[cfg(windows)]
pub fn set_clipboard_image(img: ColorImage) -> Result<(), String> {
    use rayon::prelude::*;
    use windows::Win32::{
        Foundation::HANDLE,
        Graphics::Gdi::BI_RGB,
        System::{
            DataExchange::{CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData},
            Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE},
        },
    };

    let w = img.size[0] as i32;
    let h = img.size[1] as i32;
    let row_stride = (w * 3 + 3) & !3_i32;

    #[repr(C)]
    struct BitmapInfoHeader {
        size: u32,
        width: i32,
        height: i32,
        planes: u16,
        bit_count: u16,
        compression: u32,
        size_image: u32,
        x_pels: i32,
        y_pels: i32,
        clr_used: u32,
        clr_important: u32,
    }

    let header_size = std::mem::size_of::<BitmapInfoHeader>();
    let total = header_size + (row_stride * h) as usize;

    let mut pixel_data = vec![0u8; (row_stride * h) as usize];
    pixel_data
        .par_chunks_exact_mut(row_stride as usize)
        .enumerate()
        .for_each(|(y, drow)| {
            let src_y = h as usize - 1 - y;
            for x in 0..w as usize {
                let c = img.pixels[src_y * w as usize + x];
                drow[x * 3] = c.b();
                drow[x * 3 + 1] = c.g();
                drow[x * 3 + 2] = c.r();
            }
        });

    unsafe {
        let hmem = GlobalAlloc(GMEM_MOVEABLE, total)
            .map_err(|e| format!("Clipboard memory allocation failed: {e}"))?;
        let ptr = GlobalLock(hmem) as *mut u8;
        if ptr.is_null() {
            return Err("Clipboard memory lock failed".into());
        }

        let header = BitmapInfoHeader {
            size: header_size as u32,
            width: w,
            height: h,
            planes: 1,
            bit_count: 24,
            compression: BI_RGB.0,
            size_image: (row_stride * h) as u32,
            x_pels: 0,
            y_pels: 0,
            clr_used: 0,
            clr_important: 0,
        };
        std::ptr::copy_nonoverlapping(&header as *const _ as *const u8, ptr, header_size);
        std::ptr::copy_nonoverlapping(pixel_data.as_ptr(), ptr.add(header_size), pixel_data.len());

        let _ = GlobalUnlock(hmem);

        OpenClipboard(None).map_err(|e| format!("OpenClipboard failed: {e}"))?;
        EmptyClipboard().map_err(|e| format!("EmptyClipboard failed: {e}"))?;
        let handle = HANDLE(hmem.0 as *mut _);
        SetClipboardData(8, Some(handle)).map_err(|e| format!("SetClipboardData failed: {e}"))?;
        CloseClipboard().map_err(|e| format!("CloseClipboard failed: {e}"))?;
    }

    Ok(())
}

#[cfg(target_os = "macos")]
pub fn set_clipboard_image(img: ColorImage) -> Result<(), String> {
    let mut clipboard =
        arboard::Clipboard::new().map_err(|e| format!("Clipboard open failed: {e}"))?;
    let w = img.size[0];
    let h = img.size[1];
    let mut rgba = Vec::with_capacity(w * h * 4);
    for px in &img.pixels {
        rgba.push(px.r());
        rgba.push(px.g());
        rgba.push(px.b());
        rgba.push(px.a());
    }
    let data = arboard::ImageData {
        width: w,
        height: h,
        bytes: rgba.into(),
    };
    clipboard
        .set_image(data)
        .map_err(|e| format!("Clipboard image write failed: {e}"))
}

#[cfg(all(not(windows), not(target_os = "macos"), not(target_arch = "wasm32")))]
pub fn set_clipboard_image(_img: ColorImage) -> Result<(), String> {
    Err("Clipboard image copy is not implemented on this platform".into())
}

#[cfg(target_arch = "wasm32")]
pub async fn clipboard_set_web(img: &ColorImage) -> Result<(), String> {
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
        return (async () => {
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
            await navigator.clipboard.write([item]);
        })();
        "#,
    );
    let promise = func
        .call3(
            &wasm_bindgen::JsValue::NULL,
            &wasm_bindgen::JsValue::from(w),
            &wasm_bindgen::JsValue::from(h),
            &js_array,
        )
        .map_err(|e| format!("{:?}", e))?;
    wasm_bindgen_futures::JsFuture::from(js_sys::Promise::from(promise))
        .await
        .map(|_| ())
        .map_err(|e| format!("Clipboard write failed: {:?}", e))
}
