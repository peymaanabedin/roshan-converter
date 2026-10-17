use crate::{color,metadata,engine::{self,check_cancel,ConversionSettings,ConvertedImage,ImageFormat},safe_output::{stem,Temporary}};
use image::{DynamicImage,ImageDecoder,ImageEncoder,ImageReader};
use jpeg_encoder::{ColorType,Encoder,SamplingFactor};
use std::{ffi::{c_char,c_void,CStr},fs::File,io::{BufWriter,Cursor,Read,Write},path::Path,ptr,sync::atomic::AtomicBool};
const MAX_INPUT_BYTES:u64=512*1024*1024;
#[repr(C)]
struct RhImage {width:u32,height:u32,pixels:*mut u8,pixels_len:usize,icc:*mut u8,icc_len:usize,exif:*mut u8,exif_len:usize,flags:u32}
impl RhImage {
    fn new()->Self {Self {width:0,height:0,pixels:ptr::null_mut(),pixels_len:0,icc:ptr::null_mut(),icc_len:0,exif:ptr::null_mut(),exif_len:0,flags:0}}
    fn pixels(&self)->&[u8] {if self.pixels.is_null(){&[]}else{unsafe{std::slice::from_raw_parts(self.pixels,self.pixels_len)}}}
    fn icc(&self)->&[u8] {if self.icc.is_null(){&[]}else{unsafe{std::slice::from_raw_parts(self.icc,self.icc_len)}}}
    fn exif(&self)->&[u8] {if self.exif.is_null(){&[]}else{unsafe{std::slice::from_raw_parts(self.exif,self.exif_len)}}}
}
impl Drop for RhImage {fn drop(&mut self){unsafe{rh_free_image(self);}}}
#[repr(C)]
struct RhBuffer {data:*mut u8,len:usize,flags:u32}
impl Drop for RhBuffer {fn drop(&mut self){unsafe{rh_free_buffer(self);}}}
extern "C" {
    fn rh_open(data:*const u8,len:usize,error:*mut c_char,error_len:usize)->*mut c_void;
    fn rh_count(context:*mut c_void)->i32;
    fn rh_decode(context:*mut c_void,index:i32,image:*mut RhImage,error:*mut c_char,error_len:usize)->i32;
    fn rh_decode_rgba(context:*mut c_void,index:i32,image:*mut RhImage,error:*mut c_char,error_len:usize)->i32;
    fn rh_free_image(image:*mut RhImage);
    fn rh_close(context:*mut c_void);
    fn rh_version()->*const c_char;
    fn rh_encode_heic(pixels:*const u8,pixels_len:usize,width:u32,height:u32,channels:u32,icc:*const u8,icc_len:usize,exif:*const u8,exif_len:usize,out:*mut RhBuffer,error:*mut c_char,error_len:usize)->i32;
    fn rh_free_buffer(buffer:*mut RhBuffer);
}
struct Container {raw:*mut c_void,_input:Vec<u8>}
impl Drop for Container {fn drop(&mut self){unsafe{rh_close(self.raw);}}}
fn native_error(buf:&[c_char])->String {String::from_utf8_lossy(&buf.iter().take_while(|&&x|x!=0).map(|&x|x as u8).collect::<Vec<_>>()).into_owned()}
pub fn decoder_version()->String {unsafe{let p=rh_version();if p.is_null(){"libheif".into()}else{CStr::from_ptr(p).to_string_lossy().into_owned()}}}
fn read_file(input:&Path)->Result<Vec<u8>,String> {
    let file=File::open(input).map_err(|e|format!("Cannot open the input file: {e}"))?;let meta=file.metadata().map_err(|e|e.to_string())?;
    if !meta.is_file(){return Err("Choose a file, not a folder".into());}if meta.len()==0{return Err("The input file is empty".into());}
    if meta.len()>MAX_INPUT_BYTES{return Err("The image exceeds the 512 MiB input limit; it was not resized.".into());}
    let mut bytes=Vec::new();bytes.try_reserve(meta.len() as usize).map_err(|_|"Not enough memory to read this image")?;
    file.take(MAX_INPUT_BYTES+1).read_to_end(&mut bytes).map_err(|e|format!("Cannot read the input file: {e}"))?;
    if bytes.len() as u64>MAX_INPUT_BYTES{return Err("The input image is too large".into());}Ok(bytes)
}
fn validate_dimensions(w:u32,h:u32)->Result<(),String> {
    if w==0 || h==0 || w as u64*h as u64>200_000_000 {Err("Image dimensions exceed the 200 megapixel limit; the image was not resized.".into())}else{Ok(())}
}
fn load_heic(bytes:Vec<u8>)->Result<Container,String> {
    let mut error=[0;1024];let raw=unsafe{rh_open(bytes.as_ptr(),bytes.len(),error.as_mut_ptr(),error.len())};
    if raw.is_null(){return Err(format!("Cannot read this HEIC file: {}",native_error(&error)));}Ok(Container{raw,_input:bytes})
}
fn native_warnings(flags:u32)->Vec<String> {
    let mut out=Vec::new();for (bit,text) in [
        (1,"HDR was tone-mapped to standard SDR; the HDR brightness range is not retained."),
        (2,"Transparency was composited onto white because JPG does not support alpha."),
        (4,"The source has more than 8 bits per channel; this conversion uses 8-bit color."),
        (8,"The source color description was incomplete; a matching default profile was used."),
        (16,"The decoder reported a warning; please inspect the result."),
        (64,"Some source metadata exceeded the size limit and was omitted."),
        (128,"The source color profile was invalid and was replaced."),
        (256,"A grayscale color profile was converted to sRGB.")
    ]{if flags&bit!=0{out.push(text.into());}}out
}
struct Decoded {image:DynamicImage,icc:Vec<u8>,exif:Vec<u8>,warnings:Vec<String>}
fn decode_heic(context:&Container,index:i32,target:ImageFormat)->Result<Decoded,String> {
    let mut native=RhImage::new();let mut error=[0;1024];let rgb=target==ImageFormat::Jpg;
    let ok=unsafe{if rgb{rh_decode(context.raw,index,&mut native,error.as_mut_ptr(),error.len())}else{rh_decode_rgba(context.raw,index,&mut native,error.as_mut_ptr(),error.len())}};
    if ok==0{return Err(format!("Cannot decode this image: {}",native_error(&error)));}validate_dimensions(native.width,native.height)?;
    let channels=if rgb{3}else{4};if native.pixels_len!=native.width as usize*native.height as usize*channels || native.pixels.is_null(){return Err("The decoder returned an invalid pixel buffer".into());}
    let mut image=if rgb{DynamicImage::ImageRgb8(image::RgbImage::from_raw(native.width,native.height,native.pixels().to_vec()).ok_or("Invalid RGB dimensions")?)}
        else{DynamicImage::ImageRgba8(image::RgbaImage::from_raw(native.width,native.height,native.pixels().to_vec()).ok_or("Invalid RGBA dimensions")?)};
    if native.flags&32==0{if let Some(o)=metadata::orientation(native.exif()).and_then(|x|image::metadata::Orientation::from_exif(x as u8)){image.apply_orientation(o);}}
    Ok(Decoded{image,icc:native.icc().to_vec(),exif:native.exif().to_vec(),warnings:native_warnings(native.flags)})
}
fn decode_standard(bytes:&[u8])->Result<(Decoded,ImageFormat),String> {
    let mut reader=ImageReader::new(Cursor::new(bytes)).with_guessed_format().map_err(|e|format!("Cannot recognize this image: {e}"))?;
    let actual=match reader.format(){Some(image::ImageFormat::Jpeg)=>ImageFormat::Jpg,Some(image::ImageFormat::Png)=>ImageFormat::Png,Some(image::ImageFormat::WebP)=>ImageFormat::Webp,_=>return Err("The file contents are not JPG, PNG or WebP".into())};
    let mut limits=image::Limits::default();limits.max_image_width=Some(100_000);limits.max_image_height=Some(100_000);limits.max_alloc=Some(1024*1024*1024);reader.limits(limits);
    let mut decoder=reader.into_decoder().map_err(|e|format!("Cannot read this image: {e}"))?;let(w,h)=decoder.dimensions();validate_dimensions(w,h)?;
    if decoder.total_bytes()>1024*1024*1024{return Err("The decoded image exceeds the memory limit".into());}
    let mut warnings=Vec::new();
    let icc=match decoder.icc_profile(){Ok(x)=>x.unwrap_or_default(),Err(_)=>{warnings.push("The color profile could not be read; sRGB was assumed.".into());Vec::new()}};
    let exif=match decoder.exif_metadata(){Ok(x)=>x.unwrap_or_default(),Err(_)=>{warnings.push("The EXIF metadata could not be read and was omitted.".into());Vec::new()}};
    let orientation=decoder.orientation().unwrap_or(image::metadata::Orientation::NoTransforms);
    let mut image=DynamicImage::from_decoder(decoder).map_err(|e|format!("Cannot decode this image: {e}"))?;image.apply_orientation(orientation);
    Ok((Decoded{image,icc,exif,warnings},actual))
}
fn encode(decoded:Decoded,input:&Path,settings:&ConversionSettings,index:usize,count:usize,cancel:&AtomicBool)->Result<ConvertedImage,String> {
    check_cancel(cancel)?;let Decoded{mut image,icc,exif:raw_exif,mut warnings}=decoded;
    let(width,height)=(image.width(),image.height());let target=settings.image_format;
    if target==ImageFormat::Jpg && (width>65535 || height>65535){return Err("JPG supports at most 65,535 pixels per side; the image was not resized.".into());}
    if target==ImageFormat::Webp && (width>16383 || height>16383){return Err("WebP supports at most 16,383 pixels per side; choose PNG for this image.".into());}
    let depth=image.color().bits_per_pixel()/image.color().channel_count() as u16;
    if depth>8 && target!=ImageFormat::Png{warnings.push("The source has more than 8 bits per channel; this output format uses 8-bit color.".into());}
    let exif=if settings.preserve_metadata && !raw_exif.is_empty(){
        let normalized=if target==ImageFormat::Jpg{metadata::normalize(&raw_exif,width,height)}else{metadata::normalize_with_limit(&raw_exif,width,height,16*1024*1024)};
        match normalized{Ok(x)=>x,Err(e)=>{warnings.push(e.into());Vec::new()}}
    }else{Vec::new()};
    if target==ImageFormat::Jpg && image.color().has_alpha(){let rgba=image.to_rgba8();if rgba.pixels().any(|p|p.0[3]!=255){
        let rgb=color::flatten_rgba(rgba.as_raw(),width,height,&icc)?;image=DynamicImage::ImageRgb8(image::RgbImage::from_raw(width,height,rgb).ok_or("Invalid composited image size")?);
        warnings.push("Transparency was composited onto white because JPG does not support alpha.".into());}}
    let directory=engine::output_directory(input,settings);let(temporary,file)=Temporary::create(&directory).map_err(|e|format!("Cannot write to the output folder: {e}"))?;
    let mut writer=BufWriter::with_capacity(128*1024,file);
    match target {
        ImageFormat::Jpg=>{
            let rgb=image.to_rgb8();let mut encoder=Encoder::new(&mut writer,100);encoder.set_sampling_factor(SamplingFactor::F_1_1);encoder.set_optimized_huffman_tables(true);
            if !icc.is_empty(){encoder.add_icc_profile(&icc).map_err(|e|format!("Cannot embed the color profile: {e}"))?;}
            if !exif.is_empty(){encoder.add_exif_metadata(&exif).map_err(|e|format!("Cannot embed EXIF metadata: {e}"))?;}
            encoder.encode(rgb.as_raw(),width as u16,height as u16,ColorType::Rgb).map_err(|e|format!("JPG encoding failed: {e}"))?;
        },
        ImageFormat::Png=>{
            image=if depth>8{if image.color().has_alpha(){DynamicImage::ImageRgba16(image.to_rgba16())}else{DynamicImage::ImageRgb16(image.to_rgb16())}}
                else if image.color().has_alpha(){DynamicImage::ImageRgba8(image.to_rgba8())}else{DynamicImage::ImageRgb8(image.to_rgb8())};
            let mut encoder=image::codecs::png::PngEncoder::new_with_quality(&mut writer,image::codecs::png::CompressionType::Best,image::codecs::png::FilterType::Adaptive);
            encoder.set_icc_profile(icc).map_err(|e|format!("Cannot embed the color profile: {e}"))?;
            if !exif.is_empty(){encoder.set_exif_metadata(exif).map_err(|e|format!("Cannot embed EXIF metadata: {e}"))?;}
            encoder.write_image(image.as_bytes(),width,height,image.color().into()).map_err(|e|format!("PNG encoding failed: {e}"))?;
        },
        ImageFormat::Webp=>{
            image=if image.color().has_alpha(){DynamicImage::ImageRgba8(image.to_rgba8())}else{DynamicImage::ImageRgb8(image.to_rgb8())};
            let mut encoder=image::codecs::webp::WebPEncoder::new_lossless(&mut writer);encoder.set_icc_profile(icc).map_err(|e|format!("Cannot embed the color profile: {e}"))?;
            if !exif.is_empty(){encoder.set_exif_metadata(exif).map_err(|e|format!("Cannot embed EXIF metadata: {e}"))?;}
            encoder.write_image(image.as_bytes(),width,height,image.color().into()).map_err(|e|format!("WebP encoding failed: {e}"))?;
        },
        ImageFormat::Heic=>{
            image=if image.color().has_alpha(){DynamicImage::ImageRgba8(image.to_rgba8())}else{DynamicImage::ImageRgb8(image.to_rgb8())};
            let mut out=RhBuffer{data:ptr::null_mut(),len:0,flags:0};let mut error=[0;1024];
            if unsafe{rh_encode_heic(image.as_bytes().as_ptr(),image.as_bytes().len(),width,height,image.color().channel_count() as u32,icc.as_ptr(),icc.len(),exif.as_ptr(),exif.len(),&mut out,error.as_mut_ptr(),error.len())}==0{return Err(format!("HEIC encoding failed: {}",native_error(&error)));}
            if out.data.is_null() || out.len==0{return Err("HEIC encoding produced no output".into());}
            writer.write_all(unsafe{std::slice::from_raw_parts(out.data,out.len)}).map_err(|e|format!("Cannot write HEIC output: {e}"))?;
        }
    }
    check_cancel(cancel)?;writer.flush().map_err(|e|format!("Could not finish writing the output: {e}"))?;writer.get_ref().sync_all().map_err(|e|format!("Could not save the output: {e}"))?;
    let bytes=writer.get_ref().metadata().map_err(|e|e.to_string())?.len();drop(writer);let mut base=stem(input);if count>1{base.push(format!("_{:02}",index+1));}
    let path=temporary.publish(&directory,&base,target.extension()).map_err(|e|format!("Could not publish the output: {e}"))?;
    Ok(ConvertedImage{path,width,height,bytes,warnings})
}
pub fn convert(input:&Path,settings:&ConversionSettings,cancel:&AtomicBool,progress:&mut impl FnMut(f32))->Result<Vec<ConvertedImage>,String> {
    let bytes=read_file(input)?;progress(0.05);
    let declared=ImageFormat::parse(input.extension().and_then(|x|x.to_str()).unwrap_or(""));
    if declared==Some(ImageFormat::Heic){
        let context=load_heic(bytes)?;let count=unsafe{rh_count(context.raw)};
        if count<=0 || count>1000{return Err("The HEIC container has no usable still images or exceeds the 1,000-image limit".into());}
        let mut outputs:Vec<ConvertedImage>=Vec::new();let mut failures=Vec::new();
        for index in 0..count {
            if let Err(e)=check_cancel(cancel){if outputs.is_empty(){return Err(e);}outputs[0].warnings.push(e);break;}
            let result=decode_heic(&context,index,settings.image_format).and_then(|d|{
                if index==0 && settings.image_format==ImageFormat::Heic && settings.preserve_metadata{return engine::copy_original(input,settings,"heic",d.image.width(),d.image.height(),cancel,progress);}
                encode(d,input,settings,index as usize,count as usize,cancel)
            });
            match result{Ok(image)=>outputs.push(image),Err(e)=>failures.push(format!("Image {} of {count}: {e}",index+1))}
            if settings.image_format==ImageFormat::Heic && settings.preserve_metadata{break;}progress((index+1) as f32/count as f32);
        }
        if outputs.is_empty(){return Err(failures.join("\n"));}if !failures.is_empty(){outputs[0].warnings.extend(failures);}Ok(outputs)
    }else{
        let(mut decoded,actual)=decode_standard(&bytes)?;
        if settings.preserve_metadata && actual==settings.image_format{return engine::copy_original(input,settings,actual.extension(),decoded.image.width(),decoded.image.height(),cancel,progress).map(|x|vec![x]);}
        let png=if actual==ImageFormat::Png{Some(color::png_color(&bytes))}else{None};
        if png.as_ref().is_some_and(|x|x.animation){decoded.warnings.push("Animated PNG input: only the default still frame was exported.".into());}
        if actual==ImageFormat::Webp && bytes.get(12..16)==Some(b"VP8X") && bytes.get(20).is_some_and(|x|x&2!=0){decoded.warnings.push("Animated WebP input: only the first still frame was exported.".into());}
        (decoded.image,decoded.icc)=color::prepare(decoded.image,decoded.icc,png.as_ref(),&mut decoded.warnings)?;
        progress(0.35);let output=encode(decoded,input,settings,0,1,cancel)?;progress(1.0);Ok(vec![output])
    }
}
