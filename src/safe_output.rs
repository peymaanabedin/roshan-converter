use std::{ffi::OsString,fs::{self,File,OpenOptions},io,path::{Path,PathBuf},sync::atomic::{AtomicU64,Ordering}};
static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

pub(crate) fn stem(input: &Path) -> OsString {
    let name=input.file_stem().unwrap_or_default().to_string_lossy();
    let mut clean=String::new();
    let mut units=0;
    for ch in name.chars() {
        if units+ch.len_utf16()>180 { break; }
        let c=if ch.is_control() || "<>:\"/\\|?*".contains(ch) { '_' } else { ch };
        clean.push(c); units+=c.len_utf16();
    }
    let clean=clean.trim_matches(|c| c==' ' || c=='.');
    let upper=clean.split('.').next().unwrap_or("").to_ascii_uppercase();
    let reserved=matches!(upper.as_str(),"CON"|"PRN"|"AUX"|"NUL") ||
        (upper.len()==4 && (upper.starts_with("COM") || upper.starts_with("LPT")) && matches!(upper.as_bytes()[3],b'1'..=b'9'));
    if clean.is_empty() { "image".into() } else if reserved { format!("_{clean}").into() } else { clean.into() }
}

pub(crate) struct Temporary { pub(crate) path: PathBuf }
impl Drop for Temporary { fn drop(&mut self) { let _=remove_temporary(&self.path); } }

fn remove_temporary(path:&Path)->io::Result<()> {
    // A terminated Windows codec may briefly retain a filesystem handle after
    // its process is signaled. Retry only access/sharing/lock failures, with a
    // fixed upper bound; never touch any path except this reserved temporary.
    #[cfg(windows)]
    for attempt in 0..=20 {
        match fs::remove_file(path) {
            Ok(())=>return Ok(()),
            Err(error) if error.kind()==io::ErrorKind::NotFound=>return Ok(()),
            Err(error) if attempt<20 && matches!(error.raw_os_error(),Some(5|32|33))=>{
                std::thread::sleep(std::time::Duration::from_millis(50));
            },
            Err(error)=>return Err(error),
        }
    }
    #[cfg(not(windows))]
    return match fs::remove_file(path) {
        Err(error) if error.kind()==io::ErrorKind::NotFound=>Ok(()),
        result=>result,
    };
    #[cfg(windows)]
    unreachable!("bounded cleanup loop always returns on its final attempt")
}
impl Temporary {
    pub(crate) fn create(dir: &Path) -> io::Result<(Self,File)> {
        fs::create_dir_all(dir)?;
        for _ in 0..100 {
            let n=TEMP_SEQUENCE.fetch_add(1,Ordering::Relaxed);
            let t=std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos();
            let path=dir.join(format!(".roshan-{}-{t:x}-{n}.partial",std::process::id()));
            match OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(file)=>return Ok((Self {path},file)),
                Err(e) if e.kind()==io::ErrorKind::AlreadyExists=>continue,
                Err(e)=>return Err(e),
            }
        }
        Err(io::Error::new(io::ErrorKind::AlreadyExists,"Cannot reserve a temporary file"))
    }
    pub(crate) fn publish(&self,dir:&Path,base:&OsString,extension:&str) -> io::Result<PathBuf> {
        for n in 1..=100_000 {
            let mut name=base.clone();
            if n>1 { name.push(format!(" ({n})")); }
            name.push(format!(".{extension}"));
            let path=dir.join(name);
            match publish_without_overwrite(&self.path,&path) {
                Ok(())=>return Ok(path),
                Err(e) if e.kind()==io::ErrorKind::AlreadyExists=>continue,
                Err(e)=>return Err(e),
            }
        }
        Err(io::Error::new(io::ErrorKind::AlreadyExists,"Too many files with the same name"))
    }
}

#[cfg(windows)]
fn publish_without_overwrite(from:&Path,to:&Path)->io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::{Foundation::{GetLastError,ERROR_ALREADY_EXISTS,ERROR_FILE_EXISTS},Storage::FileSystem::{MoveFileExW,MOVEFILE_WRITE_THROUGH}};
    let source:Vec<u16>=from.as_os_str().encode_wide().chain(Some(0)).collect();
    let target:Vec<u16>=to.as_os_str().encode_wide().chain(Some(0)).collect();
    let ok=unsafe { MoveFileExW(source.as_ptr(),target.as_ptr(),MOVEFILE_WRITE_THROUGH) };
    if ok!=0 { return Ok(()); }
    let error=unsafe {GetLastError()};
    if error==ERROR_ALREADY_EXISTS || error==ERROR_FILE_EXISTS { Err(io::Error::new(io::ErrorKind::AlreadyExists,"Destination exists")) }
    else { Err(io::Error::from_raw_os_error(error as i32)) }
}
#[cfg(not(windows))]
fn publish_without_overwrite(from:&Path,to:&Path)->io::Result<()> {
    fs::hard_link(from,to)
}


#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    fn temp_directory()->PathBuf { let n=TEMP_SEQUENCE.fetch_add(1,Ordering::Relaxed);let path=PathBuf::from("target/test-work").join(format!("safe-{}-{n}",std::process::id()));fs::create_dir_all(&path).unwrap();path }
    #[test]
    fn publish_never_replaces_existing_content_and_cleans_temporary() {
        let dir=temp_directory(); let original=dir.join("عکس.jpg"); fs::write(&original,b"keep-me").unwrap();
        let (temporary,mut file)=Temporary::create(&dir).unwrap(); file.write_all(b"new-image").unwrap(); drop(file);
        let temp_path=temporary.path.clone(); let output=temporary.publish(&dir,&OsString::from("عکس"),"jpg").unwrap(); drop(temporary);
        assert_eq!(fs::read(&original).unwrap(),b"keep-me"); assert_eq!(fs::read(&output).unwrap(),b"new-image");
        assert_eq!(output.file_name().unwrap(),"عکس (2).jpg"); assert!(!temp_path.exists()); fs::remove_dir_all(dir).unwrap();
    }
    #[cfg(windows)]
    #[test]
    fn cleanup_waits_for_a_transient_windows_file_lock() {
        use std::os::windows::fs::OpenOptionsExt;
        let dir=temp_directory();let (temporary,file)=Temporary::create(&dir).unwrap();drop(file);
        let path=temporary.path.clone();
        let lock=OpenOptions::new().read(true).share_mode(0).open(&path).unwrap();
        assert!(fs::remove_file(&path).is_err(),"the fixture must actually deny deletion");
        let unlock=std::thread::spawn(move||{std::thread::sleep(std::time::Duration::from_millis(150));drop(lock);});
        drop(temporary);unlock.join().unwrap();
        assert!(!path.exists(),"temporary deletion must retry until the short-lived lock closes");
        fs::remove_dir_all(dir).unwrap();
    }
}
