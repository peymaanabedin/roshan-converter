use crate::{image_engine, safe_output::{stem,Temporary}, video_engine};
use std::{fs::File,io::{Read,Write},path::{Path,PathBuf},sync::atomic::{AtomicBool,Ordering}};

pub const CANCELED:&str="Conversion canceled";
#[derive(Clone,Copy,Debug,PartialEq,Eq)]
pub enum ImageFormat {Heic,Png,Jpg,Webp}
impl ImageFormat {
    pub const ALL:[Self;4]=[Self::Jpg,Self::Png,Self::Webp,Self::Heic];
    pub fn extension(self)->&'static str {match self {Self::Heic=>"heic",Self::Png=>"png",Self::Jpg=>"jpg",Self::Webp=>"webp"}}
    pub fn label(self)->&'static str {match self {Self::Heic=>"HEIC",Self::Png=>"PNG",Self::Jpg=>"JPG",Self::Webp=>"WebP"}}
    pub fn parse(s:&str)->Option<Self> {match s.trim_start_matches('.').to_ascii_lowercase().as_str() {"heic"|"heif"=>Some(Self::Heic),"png"=>Some(Self::Png),"jpg"|"jpeg"=>Some(Self::Jpg),"webp"=>Some(Self::Webp),_=>None}}
}
#[derive(Clone,Copy,Debug,PartialEq,Eq)]
pub enum VideoFormat {Mp4,Mov,Avi,Webm}
impl VideoFormat {
    pub const ALL:[Self;4]=[Self::Mp4,Self::Mov,Self::Avi,Self::Webm];
    pub fn extension(self)->&'static str {match self {Self::Mp4=>"mp4",Self::Mov=>"mov",Self::Avi=>"avi",Self::Webm=>"webm"}}
    pub fn label(self)->&'static str {match self {Self::Mp4=>"MP4",Self::Mov=>"MOV",Self::Avi=>"AVI",Self::Webm=>"WebM"}}
    pub fn parse(s:&str)->Option<Self> {match s.trim_start_matches('.').to_ascii_lowercase().as_str() {"mp4"=>Some(Self::Mp4),"mov"=>Some(Self::Mov),"avi"=>Some(Self::Avi),"webm"=>Some(Self::Webm),_=>None}}
}
#[derive(Clone,Copy,Debug,PartialEq,Eq)]
pub enum MediaKind {Image,Video}
pub fn media_kind(path:&Path)->Option<MediaKind> {
    let ext=path.extension()?.to_str()?;
    if ImageFormat::parse(ext).is_some() {Some(MediaKind::Image)} else if VideoFormat::parse(ext).is_some() {Some(MediaKind::Video)} else {None}
}
pub fn is_supported_path(path:&Path)->bool {media_kind(path).is_some()}
#[derive(Clone,Debug)]
pub struct ConversionSettings {
    pub output_dir:Option<PathBuf>,pub preserve_metadata:bool,pub image_format:ImageFormat,pub video_format:VideoFormat,
}
impl Default for ConversionSettings {
    fn default()->Self {Self {output_dir:None,preserve_metadata:true,image_format:ImageFormat::Jpg,video_format:VideoFormat::Mp4}}
}
#[derive(Clone,Debug)]
pub struct ConvertedImage {pub path:PathBuf,pub width:u32,pub height:u32,pub bytes:u64,pub warnings:Vec<String>}
pub fn decoder_version()->String {image_engine::decoder_version()}
pub(crate) fn check_cancel(cancel:&AtomicBool)->Result<(),String> {if cancel.load(Ordering::Relaxed) {Err(CANCELED.into())} else {Ok(())}}
pub(crate) fn output_directory(input:&Path,settings:&ConversionSettings)->PathBuf {
    settings.output_dir.clone().unwrap_or_else(||input.parent().unwrap_or_else(||Path::new(".")).join("Converted"))
}
pub(crate) fn copy_original(input:&Path,settings:&ConversionSettings,extension:&str,width:u32,height:u32,cancel:&AtomicBool,progress:&mut impl FnMut(f32))->Result<ConvertedImage,String> {
    let directory=output_directory(input,settings); let (temporary,mut destination)=Temporary::create(&directory).map_err(|e|format!("Cannot write to the output folder: {e}"))?;
    let mut source=File::open(input).map_err(|e|e.to_string())?; let total=source.metadata().map_err(|e|e.to_string())?.len();
    let mut buffer=vec![0;1024*1024]; let mut written=0;
    loop {check_cancel(cancel)?;let count=source.read(&mut buffer).map_err(|e|e.to_string())?;if count==0 {break;}
        destination.write_all(&buffer[..count]).map_err(|e|e.to_string())?;written+=count as u64;progress((written as f64/total.max(1) as f64) as f32);}
    destination.sync_all().map_err(|e|e.to_string())?;drop(destination);check_cancel(cancel)?;
    let path=temporary.publish(&directory,&stem(input),extension).map_err(|e|format!("Could not publish the output: {e}"))?;
    Ok(ConvertedImage {path,width,height,bytes:written,warnings:Vec::new()})
}
pub fn convert_file(input:&Path,settings:&ConversionSettings,cancel:&AtomicBool)->Result<Vec<ConvertedImage>,String> {
    convert_file_with_progress(input,settings,cancel,|_|{})
}
pub fn convert_file_with_progress(input:&Path,settings:&ConversionSettings,cancel:&AtomicBool,mut progress:impl FnMut(f32))->Result<Vec<ConvertedImage>,String> {
    check_cancel(cancel)?;
    match media_kind(input) {
        Some(MediaKind::Image)=>image_engine::convert(input,settings,cancel,&mut progress),
        Some(MediaKind::Video)=>video_engine::convert(input,settings,cancel,&mut progress).map(|x|vec![x]),
        None=>Err("Supported files: HEIC, PNG, JPG, WebP, MP4, MOV, AVI and WebM".into())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn classification_and_output_defaults() {
        for ext in ["heic","HEIF","PNG","JpEg","jpg","WEBP"] {assert_eq!(media_kind(Path::new(&format!("sample.{ext}"))),Some(MediaKind::Image));}
        for ext in ["MP4","mov","Avi","WEBM"] {assert_eq!(media_kind(Path::new(&format!("sample.{ext}"))),Some(MediaKind::Video));}
        assert_eq!(media_kind(Path::new("file.exe")),None);
        assert_eq!(output_directory(Path::new("photos/test.heic"),&ConversionSettings::default()),Path::new("photos/Converted"));
    }
}
