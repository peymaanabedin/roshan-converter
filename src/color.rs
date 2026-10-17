//! ICC profiles and color-correct alpha handling. All processing stays local.
use image::{DynamicImage, ImageBuffer, Rgb, Rgba};
use std::{ffi::{c_char, c_void}, ptr};

#[repr(C)]
struct Buffer { data: *mut u8, len: usize, flags: u32 }
impl Buffer {
    fn new() -> Self { Self { data: ptr::null_mut(), len: 0, flags: 0 } }
    fn bytes(&self) -> Vec<u8> {
        if self.data.is_null() { Vec::new() } else { unsafe { std::slice::from_raw_parts(self.data, self.len).to_vec() } }
    }
}
impl Drop for Buffer { fn drop(&mut self) { unsafe { rh_free_buffer(self); } } }
#[repr(C)]
struct XyY { x: f64, y: f64, big_y: f64 }
#[repr(C)]
struct Primaries { red: XyY, green: XyY, blue: XyY }

extern "C" {
    fn rh_srgb_profile(out: *mut Buffer, error: *mut c_char, error_len: usize) -> i32;
    fn rh_nclx_profile(primaries: u32, transfer: u32, matrix: u32, full_range: u32,
        out: *mut Buffer, error: *mut c_char, error_len: usize) -> i32;
    fn rh_flatten_rgba(pixels: *const u8, len: usize, width: u32, height: u32,
        icc: *const u8, icc_len: usize, out: *mut Buffer, error: *mut c_char, error_len: usize) -> i32;
    fn rh_free_buffer(out: *mut Buffer);
    fn cmsOpenProfileFromMem(bytes: *const c_void, size: u32) -> *mut c_void;
    fn cmsCloseProfile(profile: *mut c_void) -> i32;
    fn cmsGetColorSpace(profile: *mut c_void) -> u32;
    fn cmsCreate_sRGBProfile() -> *mut c_void;
    fn cmsBuildGamma(context: *mut c_void, gamma: f64) -> *mut c_void;
    fn cmsFreeToneCurve(curve: *mut c_void);
    fn cmsCreateRGBProfile(white: *const XyY, primaries: *const Primaries, curves: *const *mut c_void) -> *mut c_void;
    fn cmsSaveProfileToMem(profile: *mut c_void, bytes: *mut c_void, size: *mut u32) -> i32;
    fn cmsCreateTransform(input: *mut c_void, input_format: u32, output: *mut c_void,
        output_format: u32, intent: u32, flags: u32) -> *mut c_void;
    fn cmsDoTransform(transform: *mut c_void, input: *const c_void, output: *mut c_void, count: u32);
    fn cmsDeleteTransform(transform: *mut c_void);
}

fn message(error: &[c_char]) -> String {
    String::from_utf8_lossy(&error.iter().take_while(|&&x| x != 0).map(|&x| x as u8).collect::<Vec<_>>()).into_owned()
}
pub fn srgb_profile() -> Result<Vec<u8>, String> {
    let mut out = Buffer::new(); let mut error = [0; 1024];
    if unsafe { rh_srgb_profile(&mut out, error.as_mut_ptr(), error.len()) } == 0 {
        return Err(format!("Could not create the color profile: {}", message(&error)));
    }
    Ok(out.bytes())
}
pub fn flatten_rgba(pixels: &[u8], width: u32, height: u32, icc: &[u8]) -> Result<Vec<u8>, String> {
    let mut out = Buffer::new(); let mut error = [0; 1024];
    if unsafe { rh_flatten_rgba(pixels.as_ptr(), pixels.len(), width, height,
        icc.as_ptr(), icc.len(), &mut out, error.as_mut_ptr(), error.len()) } == 0 {
        return Err(format!("Could not composite transparency: {}", message(&error)));
    }
    Ok(out.bytes())
}

struct Profile(*mut c_void);
impl Drop for Profile { fn drop(&mut self) { unsafe { cmsCloseProfile(self.0); } } }
impl Profile {
    fn open(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < 128 || bytes.len() > 16 * 1024 * 1024 { return None; }
        let raw = unsafe { cmsOpenProfileFromMem(bytes.as_ptr().cast(), bytes.len() as u32) };
        (!raw.is_null()).then_some(Self(raw))
    }
    fn bytes(&self) -> Result<Vec<u8>, String> {
        let mut size = 0;
        if unsafe { cmsSaveProfileToMem(self.0, ptr::null_mut(), &mut size) } == 0 || size > 16 * 1024 * 1024 {
            return Err("Could not serialize the color profile".into());
        }
        let mut result = vec![0; size as usize];
        if unsafe { cmsSaveProfileToMem(self.0, result.as_mut_ptr().cast(), &mut size) } == 0 {
            return Err("Could not serialize the color profile".into());
        }
        result.truncate(size as usize); Ok(result)
    }
}

#[derive(Default, Clone)]
pub struct PngColor {
    pub gamma: Option<f64>,
    pub chromaticities: Option<[f64; 8]>,
    pub srgb: bool,
    pub hdr: bool,
    pub cicp: Option<[u8; 4]>,
    pub invalid_cicp: bool,
    pub animation: bool,
}
/// Reads only bounded color/animation chunks; the image decoder validates the PNG itself.
pub fn png_color(bytes: &[u8]) -> PngColor {
    let mut result = PngColor::default();
    if !bytes.starts_with(b"\x89PNG\r\n\x1a\n") { return result; }
    let mut at = 8usize;
    while let Some(head) = bytes.get(at..at.saturating_add(8)) {
        let len = u32::from_be_bytes(head[..4].try_into().unwrap()) as usize;
        let Some(end) = at.checked_add(12).and_then(|x| x.checked_add(len)) else { break; };
        if end > bytes.len() { break; }
        let data = &bytes[at + 8..at + 8 + len];
        match &head[4..8] {
            b"gAMA" if len == 4 => result.gamma = Some(u32::from_be_bytes(data.try_into().unwrap()) as f64 / 100_000.0),
            b"cHRM" if len == 32 => {
                let mut c = [0.0; 8];
                for (n, v) in data.chunks_exact(4).enumerate() { c[n] = u32::from_be_bytes(v.try_into().unwrap()) as f64 / 100_000.0; }
                result.chromaticities = Some(c);
            },
            b"sRGB" => result.srgb = true,
            b"cICP" => {
                if len != 4 || result.cicp.is_some() { result.invalid_cicp = true; }
                else {
                    let mut crc = u32::MAX;
                    for byte in &bytes[at + 4..at + 8 + len] {
                        crc ^= *byte as u32;
                        for _ in 0..8 { crc = (crc >> 1) ^ (0xedb88320u32 & (0u32.wrapping_sub(crc & 1))); }
                    }
                    if !crc != u32::from_be_bytes(bytes[at + 8 + len..end].try_into().unwrap()) {
                        result.invalid_cicp = true;
                    }
                    let values: [u8; 4] = data.try_into().unwrap();
                    result.hdr = matches!(values[1], 16 | 18);
                    result.cicp = Some(values);
                }
            },
            b"acTL" => result.animation = true,
            b"IDAT" | b"IEND" => break,
            _ => {}
        }
        at = end;
    }
    result
}

fn cicp_profile(codes: [u8; 4]) -> Result<Vec<u8>, String> {
    let mut out = Buffer::new(); let mut error = [0; 1024];
    if unsafe { rh_nclx_profile(codes[0] as u32, codes[1] as u32, codes[2] as u32, codes[3] as u32,
        &mut out, error.as_mut_ptr(), error.len()) } == 0 {
        return Err(format!("Cannot preserve PNG cICP color: {}", message(&error)));
    }
    Ok(out.bytes())
}

fn png_profile(info: &PngColor) -> Result<Vec<u8>, String> {
    if info.srgb || (info.gamma.is_none() && info.chromaticities.is_none()) { return srgb_profile(); }
    // PNG stores the encoding exponent; ICC needs the inverse (decoding) exponent.
    let exponent = 1.0 / info.gamma.unwrap_or(0.45455);
    if !exponent.is_finite() || !(0.1..=10.0).contains(&exponent) { return Err("Invalid PNG color gamma".into()); }
    let c = info.chromaticities.unwrap_or([0.3127,0.3290,0.6400,0.3300,0.3000,0.6000,0.1500,0.0600]);
    if c.iter().any(|x| !x.is_finite() || !(0.0..=1.0).contains(x)) || [1,3,5,7].iter().any(|&i| c[i] == 0.0) {
        return Err("Invalid PNG chromaticities".into());
    }
    let point = |x,y| XyY { x, y, big_y: 1.0 };
    let white = point(c[0],c[1]);
    let primaries = Primaries { red: point(c[2],c[3]), green: point(c[4],c[5]), blue: point(c[6],c[7]) };
    let curve = unsafe { cmsBuildGamma(ptr::null_mut(), exponent) };
    if curve.is_null() { return Err("Could not create the PNG color curve".into()); }
    let curves = [curve;3];
    let profile = unsafe { cmsCreateRGBProfile(&white, &primaries, curves.as_ptr()) };
    unsafe { cmsFreeToneCurve(curve); }
    if profile.is_null() { return Err("Could not create the PNG color profile".into()); }
    Profile(profile).bytes()
}

/// Preserves RGB ICC coordinates. Gray ICC data are transformed to sRGB before RGB encoding.
pub fn prepare(mut pixels: DynamicImage, mut icc: Vec<u8>, png: Option<&PngColor>, warnings: &mut Vec<String>)
    -> Result<(DynamicImage, Vec<u8>), String> {
    if png.is_some_and(|x| x.invalid_cicp) {
        return Err("Invalid or repeated PNG cICP color chunk; transcoding was stopped to avoid incorrect color.".into());
    }
    if png.is_some_and(|x| x.hdr) {
        return Err("HDR PNG transcoding is not supported. Choose PNG output to retain the original HDR file.".into());
    }
    // PNG Third Edition 11.3.2.6 gives cICP precedence over iCCP, sRGB,
    // gAMA and cHRM. Pixel coordinates already use that source RGB space;
    // attach a matching ICC without changing the samples or alpha.
    // https://www.w3.org/TR/png-3/#cICP-chunk
    if let Some(codes) = png.and_then(|info| info.cicp) {
        return Ok((pixels, cicp_profile(codes)?));
    }
    if icc.is_empty() {
        icc = if let Some(info) = png { png_profile(info)? } else { srgb_profile()? };
        return Ok((pixels, icc));
    }
    let Some(profile) = Profile::open(&icc) else {
        warnings.push("The source ICC profile was invalid; remaining PNG color data or an sRGB default were used.".into());
        let fallback = if let Some(info) = png { png_profile(info)? } else { srgb_profile()? };
        return Ok((pixels, fallback));
    };
    match unsafe { cmsGetColorSpace(profile.0) } {
        0x52474220 => {}, // RGB
        0x47524159 => { // GRAY
            let gray = matches!(pixels.color(), image::ColorType::L8 | image::ColorType::La8 | image::ColorType::L16 | image::ColorType::La16);
            if !gray { return Err("A grayscale ICC profile does not match the decoded RGB pixels.".into()); }
            let out = unsafe { cmsCreate_sRGBProfile() };
            if out.is_null() { return Err("Could not create the output color profile".into()); }
            let output_profile = Profile(out);
            let (w,h) = (pixels.width(),pixels.height());
            let alpha = pixels.color().has_alpha();
            let bits = pixels.color().bits_per_pixel() / pixels.color().channel_count() as u16;
            let bytes = if bits > 8 { 2 } else { 1 };
            let transform = unsafe { cmsCreateTransform(profile.0, (3<<16)|(1<<3)|bytes,
                out, (4<<16)|(3<<3)|bytes, 1, 0) };
            if transform.is_null() { return Err("Could not transform the grayscale color profile".into()); }
            if bytes == 2 {
                let input = pixels.to_luma16(); let mut rgb = vec![0u16; w as usize*h as usize*3];
                unsafe { cmsDoTransform(transform, input.as_ptr().cast(), rgb.as_mut_ptr().cast(), w*h); }
                if alpha {
                    let original = pixels.to_rgba16(); let mut rgba = Vec::with_capacity(w as usize*h as usize*4);
                    for (c,a) in rgb.chunks_exact(3).zip(original.pixels()) { rgba.extend_from_slice(c); rgba.push(a.0[3]); }
                    pixels = DynamicImage::ImageRgba16(ImageBuffer::<Rgba<u16>,_>::from_raw(w,h,rgba).ok_or("Invalid grayscale dimensions")?);
                } else { pixels = DynamicImage::ImageRgb16(ImageBuffer::<Rgb<u16>,_>::from_raw(w,h,rgb).ok_or("Invalid grayscale dimensions")?); }
            } else {
                let input = pixels.to_luma8(); let mut rgb = vec![0u8; w as usize*h as usize*3];
                unsafe { cmsDoTransform(transform, input.as_ptr().cast(), rgb.as_mut_ptr().cast(), w*h); }
                if alpha {
                    let original = pixels.to_rgba8(); let mut rgba = Vec::with_capacity(w as usize*h as usize*4);
                    for (c,a) in rgb.chunks_exact(3).zip(original.pixels()) { rgba.extend_from_slice(c); rgba.push(a.0[3]); }
                    pixels = DynamicImage::ImageRgba8(ImageBuffer::<Rgba<u8>,_>::from_raw(w,h,rgba).ok_or("Invalid grayscale dimensions")?);
                } else { pixels = DynamicImage::ImageRgb8(ImageBuffer::<Rgb<u8>,_>::from_raw(w,h,rgb).ok_or("Invalid grayscale dimensions")?); }
            }
            unsafe { cmsDeleteTransform(transform); }
            icc = output_profile.bytes()?;
        },
        _ => return Err("This image uses a CMYK or other non-RGB ICC profile. A color-managed RGB source is required.".into())
    }
    Ok((pixels,icc))
}
