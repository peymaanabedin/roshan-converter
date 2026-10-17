use crate::{engine::{self,check_cancel,ConversionSettings,ConvertedImage,VideoFormat},safe_output::{stem,Temporary},video_tools};
use serde_json::Value;
use std::{path::{Path,PathBuf},sync::atomic::AtomicBool};

struct Probe {video:Value,audio:Vec<Value>,duration:f64,width:u32,height:u32,rotation:i64,video_count:usize,other_count:usize,format:String,brand:String}
fn number(value:&Value)->f64 {value.as_f64().or_else(||value.as_str().and_then(|x|x.parse().ok())).unwrap_or(0.0)}
fn text<'a>(v:&'a Value,key:&str)->&'a str {v.get(key).and_then(Value::as_str).unwrap_or("")}
fn absolute(path:&Path)->Result<PathBuf,String> {if path.is_absolute(){Ok(path.to_path_buf())}else{std::env::current_dir().map(|p|p.join(path)).map_err(|e|e.to_string())}}
fn inspect(input:&Path,cancel:&AtomicBool)->Result<Probe,String> {
    let mut command=video_tools::command(true)?;
    command.args(["-v","error","-protocol_whitelist","file,pipe","-probesize","20000000","-analyzeduration","10000000","-show_streams","-show_format","-of","json","-i"]).arg(absolute(input)?);
    let result=video_tools::capture(command,cancel)?;
    let data:Value=serde_json::from_slice(&result).map_err(|e|format!("The video inspection data is invalid: {e}"))?;
    let streams=data["streams"].as_array().ok_or("No media streams were found")?;
    let videos:Vec<_>=streams.iter().filter(|x|text(x,"codec_type")=="video" && x["disposition"]["attached_pic"].as_i64().unwrap_or(0)==0).collect();
    let video=(*videos.first().ok_or("The file does not contain a video stream")?).clone();
    let audio=streams.iter().filter(|x|text(x,"codec_type")=="audio").cloned().collect::<Vec<_>>();
    if audio.len()>32{return Err("This video has more than the supported 32 audio tracks".into());}
    let rotation=video["side_data_list"].as_array().and_then(|items|items.iter().find_map(|x|x.get("rotation").map(number)))
        .or_else(||video["tags"].get("rotate").map(number)).unwrap_or(0.0).round() as i64;
    let(mut width,mut height)=(video["width"].as_u64().unwrap_or(0) as u32,video["height"].as_u64().unwrap_or(0) as u32);
    if width==0 || height==0{return Err("The video dimensions could not be read".into());}
    if rotation.rem_euclid(180)==90{std::mem::swap(&mut width,&mut height);}
    let duration=number(&data["format"]["duration"]).max(number(&video["duration"]));
    Ok(Probe{video,audio,duration,width,height,rotation,video_count:videos.len(),other_count:streams.len().saturating_sub(videos.len()+streams.iter().filter(|x|text(x,"codec_type")=="audio").count()),
        format:text(&data["format"],"format_name").into(),brand:text(&data["format"]["tags"],"major_brand").into()})
}
fn video_compatible(target:VideoFormat,codec:&str)->bool {
    match target {VideoFormat::Mp4|VideoFormat::Mov=>matches!(codec,"h264"|"hevc"),VideoFormat::Avi=>matches!(codec,"mpeg4"|"mjpeg"|"ffv1"),VideoFormat::Webm=>matches!(codec,"vp8"|"vp9"|"av1")}
}
fn audio_compatible(target:VideoFormat,codec:&str)->bool {
    match target {VideoFormat::Mp4=>matches!(codec,"aac"|"mp3"|"alac"|"ac3"),VideoFormat::Mov=>matches!(codec,"aac"|"mp3"|"alac"|"ac3"|"pcm_s16le"|"pcm_s24le"|"pcm_s32le"),
        VideoFormat::Avi=>matches!(codec,"mp3"|"pcm_s16le"|"pcm_s24le"|"pcm_s32le"),VideoFormat::Webm=>matches!(codec,"opus"|"vorbis")}
}
fn same_container(source:&Probe,input:&Path,target:VideoFormat)->bool {
    if input.extension().and_then(|x|x.to_str()).and_then(VideoFormat::parse)!=Some(target){return false;}
    match target {VideoFormat::Mp4=>source.format.contains("mp4") && !source.brand.starts_with("qt"),VideoFormat::Mov=>source.format.contains("mov"),
        VideoFormat::Avi=>source.format=="avi",VideoFormat::Webm=>source.format.contains("webm") && video_compatible(target,text(&source.video,"codec_name")) && source.audio.iter().all(|x|audio_compatible(target,text(x,"codec_name")))}
}

pub fn convert(input:&Path,settings:&ConversionSettings,cancel:&AtomicBool,progress:&mut impl FnMut(f32))->Result<ConvertedImage,String> {
    check_cancel(cancel)?;
    let meta=input.metadata().map_err(|e|format!("Cannot open the input file: {e}"))?;
    if !meta.is_file(){return Err("Choose a file, not a folder".into());}if meta.len()==0{return Err("The input file is empty".into());}
    progress(0.01);let source=inspect(input,cancel)?;let target=settings.video_format;
    if settings.preserve_metadata && same_container(&source,input,target){return engine::copy_original(input,settings,target.extension(),source.width,source.height,cancel,progress);}
    let mut warnings=Vec::new();
    if source.video_count>1{warnings.push("The first main video stream was converted; additional video views were omitted.".into());}
    if source.other_count>0{warnings.push("Subtitles, attachments and auxiliary data streams were not included in the converted video.".into());}
    let rotation=source.rotation.rem_euclid(360);
    let copy_video=video_compatible(target,text(&source.video,"codec_name")) &&
        (rotation==0 || (settings.preserve_metadata && matches!(target,VideoFormat::Mp4|VideoFormat::Mov)));
    let hdr=matches!(text(&source.video,"color_transfer"),"smpte2084"|"arib-std-b67");
    let mut cmd=video_tools::command(false)?;
    cmd.args(["-hide_banner","-nostdin","-v","error","-xerror","-y","-protocol_whitelist","file,pipe","-probesize","20000000","-analyzeduration","10000000","-i"]).arg(absolute(input)?);
    cmd.args(["-map",&format!("0:{}",source.video["index"].as_u64().unwrap_or(0))]);
    for track in &source.audio{cmd.args(["-map",&format!("0:{}",track["index"].as_u64().unwrap_or(0))]);}
    if settings.preserve_metadata{cmd.args(["-map_metadata","0"]);}else{cmd.args(["-map_metadata","-1","-map_metadata:s","-1"]);}
    cmd.args(["-map_chapters",if settings.preserve_metadata && target!=VideoFormat::Avi{"0"}else{"-1"}]);
    if copy_video {
        cmd.args(["-c:v","copy"]);
        if matches!(target,VideoFormat::Mp4|VideoFormat::Mov) && text(&source.video,"codec_name")=="hevc"{cmd.args(["-tag:v","hvc1"]);}
    }else{
        let odd=source.width%2!=0 || source.height%2!=0;
        let pix_fmt=if odd && matches!(target,VideoFormat::Mp4|VideoFormat::Mov|VideoFormat::Avi){"yuv444p"}else{"yuv420p"};
        if hdr{
            // FFmpeg's decoded frame supplies the input transfer/primaries/matrix. The final
            // tags explicitly describe the new SDR output, never the old HDR signal.
            let filter=format!("zscale=t=linear:npl=100,format=gbrpf32le,zscale=p=bt709,tonemap=tonemap=hable:desat=0,zscale=t=bt709:m=bt709:r=tv,format={pix_fmt}");
            cmd.args(["-vf",&filter,"-color_primaries","bt709","-color_trc","bt709","-colorspace","bt709","-color_range","tv"]);
            warnings.push("HDR video was tone-mapped to standard SDR for this output codec.".into());
        }
        cmd.args(["-pix_fmt",pix_fmt]);
        match target {
            VideoFormat::Mp4|VideoFormat::Mov=>{
                cmd.args(["-c:v","libx264","-crf","16","-preset","medium"]);
                if odd{warnings.push("Odd dimensions were preserved using H.264 4:4:4; some hardware players may require a software player.".into());}
            },
            VideoFormat::Webm=>{cmd.args(["-c:v","libvpx-vp9","-crf","18","-b:v","0","-deadline","good","-cpu-used","2","-row-mt","1"]);},
            VideoFormat::Avi=>{
                if odd{cmd.args(["-c:v","ffv1","-level","3","-coder","1","-context","1"]);warnings.push("AVI uses lossless FFV1 to retain the odd dimensions; a compatible player is required.".into());}
                else{cmd.args(["-c:v","mpeg4","-q:v","2"]);}
            }
        }
        cmd.args(["-threads","4"]);
        // Keep original frame timestamps where the target container supports them.
        if target!=VideoFormat::Avi{cmd.args(["-fps_mode","passthrough"]);}
        if rotation!=0{cmd.args(["-metadata:s:v:0","rotate=0"]);}
        if text(&source.video,"pix_fmt").contains("yuva") || text(&source.video,"pix_fmt").contains("rgba") || source.video["tags"]["alpha_mode"].as_str()==Some("1"){
            warnings.push("Video transparency is not preserved when the video codec must be changed.".into());
        }
    }
    for (index,audio) in source.audio.iter().enumerate(){
        let codec=text(audio,"codec_name");let key=format!("-c:a:{index}");
        if audio_compatible(target,codec){cmd.args([&key,"copy"]);continue;}
        match target {
            VideoFormat::Mp4|VideoFormat::Mov=>{cmd.args([&key,"aac",&format!("-b:a:{index}"),"320k"]);},
            VideoFormat::Webm=>{
                let channels=audio["channels"].as_u64().unwrap_or(2);
                if channels>8{return Err("WebM Opus supports up to eight channels; this track was not downmixed.".into());}
                let bitrate=if channels>2{"384k"}else{"192k"};
                cmd.args([&key,"libopus",&format!("-b:a:{index}"),bitrate,&format!("-ar:a:{index}"),"48000"]);
                if channels>2{cmd.args([&format!("-mapping_family:a:{index}"),"1"]);}
            },
            VideoFormat::Avi=>{
                let bits=audio["bits_per_raw_sample"].as_str().and_then(|x|x.parse::<u32>().ok()).unwrap_or(16);
                cmd.args([key.as_str(),if bits>16{"pcm_s24le"}else{"pcm_s16le"}]);
            }
        }
    }
    if matches!(target,VideoFormat::Mp4|VideoFormat::Mov){cmd.args(["-movflags","+faststart"]);}
    cmd.args(["-progress","pipe:1","-stats_period","0.15","-nostats","-f",target.extension()]);
    let directory=engine::output_directory(input,settings);let(temp,file)=Temporary::create(&directory).map_err(|e|format!("Cannot write to the output folder: {e}"))?;drop(file);
    cmd.arg(absolute(&temp.path)?);
    video_tools::transcode(cmd,source.duration,cancel,progress)?;
    let output=inspect(&temp.path,cancel)?;
    if (output.width,output.height)!=(source.width,source.height){return Err(format!("Output dimensions changed unexpectedly ({}x{} to {}x{}); the result was not published.",source.width,source.height,output.width,output.height));}
    if output.audio.len()!=source.audio.len(){return Err("The audio-track count changed unexpectedly; the result was not published.".into());}
    if source.duration>0.0 && output.duration>0.0 && (source.duration-output.duration).abs()>0.5_f64.max(source.duration*0.005){warnings.push("The output duration differs from the source container duration; please check the result.".into());}
    check_cancel(cancel)?;let file=std::fs::OpenOptions::new().read(true).write(true).open(&temp.path).map_err(|e|e.to_string())?;let bytes=file.metadata().map_err(|e|e.to_string())?.len();file.sync_all().map_err(|e|e.to_string())?;drop(file);
    if bytes==0{return Err("The video encoder produced an empty file".into());}
    let path=temp.publish(&directory,&stem(input),target.extension()).map_err(|e|format!("Could not publish the output: {e}"))?;progress(1.0);
    Ok(ConvertedImage{path,width:output.width,height:output.height,bytes,warnings})
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]fn compatible_streams_can_be_copied_without_recompression(){
        assert!(video_compatible(VideoFormat::Mov,"h264"));assert!(video_compatible(VideoFormat::Mp4,"hevc"));
        assert!(!video_compatible(VideoFormat::Webm,"h264"));assert!(video_compatible(VideoFormat::Webm,"vp9"));
        assert!(audio_compatible(VideoFormat::Webm,"opus"));assert!(!audio_compatible(VideoFormat::Webm,"aac"));
    }
}
