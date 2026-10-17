//! Bundled, verified offline child tools. No PATH lookup, downloads, or shell commands.
use crate::engine::{check_cancel,CANCELED};
use std::{fs,path::PathBuf,io::{BufRead,BufReader,Read},process::{Child,Command,Stdio},
    sync::{atomic::AtomicBool,mpsc,Mutex,OnceLock},thread,time::{Duration,Instant}};

#[cfg(windows)]
mod embedded { include!(concat!(env!("OUT_DIR"),"/video_payloads.rs")); }
struct Runtime {directory:PathBuf,ffmpeg:PathBuf,ffprobe:PathBuf,owned:bool}
impl Drop for Runtime {fn drop(&mut self){if self.owned{let _=fs::remove_dir_all(&self.directory);}}}
static RUNTIME:OnceLock<Mutex<Option<Runtime>>>=OnceLock::new();

#[cfg(windows)]
fn unpack(dir:&std::path::Path,name:&str,data:&[u8],expected_size:u64,expected_hash:&[u8;32])->Result<PathBuf,String> {
    use flate2::read::GzDecoder;use sha2::{Digest,Sha256};use std::io::Write;
    if data.is_empty() || expected_size==0 {return Err("The embedded video engine is missing from this development build.".into());}
    let path=dir.join(name);let mut file=fs::OpenOptions::new().write(true).create_new(true).open(&path).map_err(|e|format!("Cannot prepare the bundled video engine: {e}"))?;
    let mut reader=GzDecoder::new(data).take(expected_size+1);let mut buffer=[0u8;128*1024];let mut hash=Sha256::new();let mut written=0;
    loop{let n=reader.read(&mut buffer).map_err(|e|format!("Cannot unpack the video engine: {e}"))?;if n==0{break;}written+=n as u64;
        if written>expected_size{return Err("Bundled video engine size verification failed".into());}hash.update(&buffer[..n]);file.write_all(&buffer[..n]).map_err(|e|e.to_string())?;}
    if written!=expected_size || hash.finalize().as_slice()!=expected_hash{return Err("Bundled video engine integrity verification failed".into());}
    file.sync_all().map_err(|e|e.to_string())?;Ok(path)
}
fn initialize()->Result<Runtime,String> {
    #[cfg(windows)]{
        for attempt in 0..100 {
            let t=std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos();
            let directory=std::env::temp_dir().join(format!("RoshanConverter-{}-{t:x}-{attempt}",std::process::id()));
            match fs::create_dir(&directory){Ok(())=>{},Err(e) if e.kind()==std::io::ErrorKind::AlreadyExists=>continue,Err(e)=>return Err(format!("Cannot prepare the offline video engine: {e}"))}
            let mut runtime=Runtime{ffmpeg:PathBuf::new(),ffprobe:PathBuf::new(),directory,owned:true};
            runtime.ffmpeg=unpack(&runtime.directory,"ffmpeg.exe",embedded::FFMPEG_GZ,embedded::FFMPEG_SIZE,&embedded::FFMPEG_SHA)?;
            runtime.ffprobe=unpack(&runtime.directory,"ffprobe.exe",embedded::FFPROBE_GZ,embedded::FFPROBE_SIZE,&embedded::FFPROBE_SHA)?;
            return Ok(runtime);
        }
        Err("Cannot reserve a private directory for the offline video engine".into())
    }
    #[cfg(not(windows))]{
        let directory=std::env::var_os("ROSHAN_FFMPEG_DIR").map(PathBuf::from).unwrap_or_else(||PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("video-native/linux/bin"));
        let ffmpeg=directory.join("ffmpeg");let ffprobe=directory.join("ffprobe");
        if !ffmpeg.is_file() || !ffprobe.is_file(){return Err("Build the local offline video tools or set ROSHAN_FFMPEG_DIR".into());}
        Ok(Runtime{directory,ffmpeg,ffprobe,owned:false})
    }
}
pub fn command(probe:bool)->Result<Command,String> {
    let mut runtime=RUNTIME.get_or_init(||Mutex::new(None)).lock().unwrap_or_else(|e|e.into_inner());
    if runtime.is_none(){*runtime=Some(initialize()?);}
    let runtime=runtime.as_ref().unwrap();let mut cmd=Command::new(if probe{&runtime.ffprobe}else{&runtime.ffmpeg});
    cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    #[cfg(windows)]{use std::os::windows::process::CommandExt;cmd.creation_flags(0x08000000);}
    Ok(cmd)
}
pub fn cleanup_runtime(){if let Some(runtime)=RUNTIME.get(){runtime.lock().unwrap_or_else(|e|e.into_inner()).take();}}

struct Process {child:Child,#[cfg(windows)]job:windows_sys::Win32::Foundation::HANDLE}
impl Process {
    fn spawn(cmd:&mut Command)->Result<Self,String> {
        #[allow(unused_mut)]
        let mut child=cmd.spawn().map_err(|e|format!("Cannot start the bundled video engine: {e}"))?;
        #[cfg(windows)]unsafe{
            use std::os::windows::io::AsRawHandle;use windows_sys::Win32::{Foundation::CloseHandle,System::JobObjects::*};
            let job=CreateJobObjectW(std::ptr::null(),std::ptr::null());
            let mut info:JOBOBJECT_EXTENDED_LIMIT_INFORMATION=std::mem::zeroed();info.BasicLimitInformation.LimitFlags=JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            if job.is_null() || SetInformationJobObject(job,JobObjectExtendedLimitInformation,(&info as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),std::mem::size_of_val(&info) as u32)==0 || AssignProcessToJobObject(job,child.as_raw_handle())==0 {
                let error=std::io::Error::last_os_error();let _=child.kill();let _=child.wait();if !job.is_null(){CloseHandle(job);}
                return Err(format!("Cannot manage the video worker safely: {error}"));
            }
            Ok(Self{child,job})
        }
        #[cfg(not(windows))]{Ok(Self{child})}
    }
}
impl Drop for Process {
    fn drop(&mut self){let _=self.child.kill();let _=self.child.wait();#[cfg(windows)]unsafe{windows_sys::Win32::Foundation::CloseHandle(self.job);}}
}
fn bounded_read(mut reader:impl Read,max:usize)->Vec<u8> {
    let mut result=Vec::new();let mut buffer=[0u8;8192];
    loop {match reader.read(&mut buffer){Ok(0)|Err(_)=>break,Ok(n)=>{let keep=n.min(max.saturating_sub(result.len()));result.extend_from_slice(&buffer[..keep]);}}}
    result
}
pub fn capture(mut cmd:Command,cancel:&AtomicBool)->Result<Vec<u8>,String> {
    check_cancel(cancel)?;let mut process=Process::spawn(&mut cmd)?;let stdout=process.child.stdout.take().unwrap();let stderr=process.child.stderr.take().unwrap();
    let out=thread::spawn(move||bounded_read(stdout,2*1024*1024));let err=thread::spawn(move||bounded_read(stderr,32*1024));let start=Instant::now();
    let status=loop {
        if check_cancel(cancel).is_err(){let _=process.child.kill();let _=process.child.wait();let _=out.join();let _=err.join();return Err(CANCELED.into());}
        if start.elapsed()>Duration::from_secs(120){let _=process.child.kill();let _=process.child.wait();let _=out.join();let _=err.join();return Err("Video inspection timed out".into());}
        if let Some(status)=process.child.try_wait().map_err(|e|e.to_string())?{break status;}thread::sleep(Duration::from_millis(40));
    };
    let output=out.join().map_err(|_|"Video inspection reader failed")?;let errors=err.join().unwrap_or_default();
    if !status.success(){return Err(format!("Cannot inspect this video: {}",String::from_utf8_lossy(&errors).trim()));}Ok(output)
}
pub fn transcode(mut cmd:Command,duration:f64,cancel:&AtomicBool,progress:&mut impl FnMut(f32))->Result<(),String> {
    check_cancel(cancel)?;let mut process=Process::spawn(&mut cmd)?;let stdout=process.child.stdout.take().unwrap();let stderr=process.child.stderr.take().unwrap();
    let(tx,rx)=mpsc::channel();let out=thread::spawn(move||{for line in BufReader::new(stdout).lines().map_while(Result::ok){
        if let Some(value)=line.strip_prefix("out_time_us="){if let Ok(time)=value.parse::<i64>(){let _=tx.send(time);}}
    }});let err=thread::spawn(move||bounded_read(stderr,32*1024));
    let status=loop {
        if check_cancel(cancel).is_err(){let _=process.child.kill();let _=process.child.wait();let _=out.join();let _=err.join();return Err(CANCELED.into());}
        while let Ok(time)=rx.try_recv(){if duration>0.0{progress((time as f64/1_000_000.0/duration).clamp(0.0,0.985) as f32);}}
        if let Some(status)=process.child.try_wait().map_err(|e|e.to_string())?{break status;}thread::sleep(Duration::from_millis(40));
    };
    let _=out.join();let errors=err.join().unwrap_or_default();
    if !status.success(){let detail=String::from_utf8_lossy(&errors);return Err(format!("Video conversion failed: {}",if detail.trim().is_empty(){"the codec rejected this input"}else{detail.trim()}));}
    check_cancel(cancel)?;Ok(())
}
