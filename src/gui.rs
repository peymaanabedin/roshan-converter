//! Compact, bilingual, fully native Windows interface for Roshan Converter.
#![cfg(target_os = "windows")]
#![allow(unsafe_op_in_unsafe_fn)]

use crate::engine::{convert_file_with_progress, media_kind, ConversionSettings, ConvertedImage, ImageFormat, MediaKind, VideoFormat};
use std::collections::HashSet;
use std::ffi::{c_void, OsString};
use std::mem::{size_of, zeroed};
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};
use std::ptr::{null, null_mut};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Graphics::Gdi::*;
use windows_sys::Win32::System::Com::*;
use windows_sys::Win32::System::LibraryLoader::*;
use windows_sys::Win32::System::SystemServices::{SS_CENTER, SS_LEFT, SS_RIGHT};
use windows_sys::Win32::UI::Controls::Dialogs::*;
use windows_sys::Win32::UI::Controls::*;
use windows_sys::Win32::UI::HiDpi::*;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::*;
use windows_sys::Win32::UI::Shell::*;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

const CLASS_NAME: &str = "RoshanHeicOfflineWindow";
const CREDIT: &str = "برنامه نویسی و توسعه توسط پیمان عابدین پور";
const WEBSITE: &str = "https://peymaanabedinpour.ir";
const TIMER_ID: usize = 1;
const ID_ADD: i32 = 101;
const ID_REMOVE: i32 = 102;
const ID_CLEAR: i32 = 103;
const ID_FOLDER: i32 = 104;
const ID_RESET: i32 = 105;
const ID_START: i32 = 106;
const ID_CANCEL: i32 = 107;
const ID_OPEN: i32 = 108;
const ID_META: i32 = 109;
const ID_LIST: i32 = 110;
const ID_OUTPUT: i32 = 111;
const ID_DETAIL: i32 = 112;
const ID_PROGRESS: i32 = 113;
const ID_ABOUT: i32 = 114;
const ID_EMPTY: i32 = 115;
const ID_LANGUAGE: i32 = 116;
const ID_IMAGE_FORMAT: i32 = 117;
const ID_VIDEO_FORMAT: i32 = 118;
const ID_IMAGE_LABEL: i32 = 119;
const ID_VIDEO_LABEL: i32 = 120;
const MODAL_REVIEW: i32 = 501;
const MODAL_CONVERT: i32 = 502;
const MODAL_CLOSE: i32 = 601;
const MODAL_WEBSITE: i32 = 602;

const fn rgb(r: u8, g: u8, b: u8) -> u32 { r as u32 | ((g as u32) << 8) | ((b as u32) << 16) }
const BG: u32 = rgb(248, 250, 251);
const WHITE: u32 = rgb(255, 255, 255);
const INK: u32 = rgb(24, 40, 47);
const MUTED: u32 = rgb(108, 123, 131);
const ACCENT: u32 = rgb(0, 113, 107);
const TINT: u32 = rgb(229, 244, 241);
const BORDER: u32 = rgb(220, 230, 233);
const ERROR: u32 = rgb(171, 48, 56);
const WARNING: u32 = rgb(151, 96, 13);

#[derive(Clone, Copy, PartialEq, Eq)]
enum Lang { En, Fa }
impl Lang {
    fn t<'a>(self, en: &'a str, fa: &'a str) -> &'a str { if self == Self::Fa { fa } else { en } }
    fn number(self, n: usize) -> String {
        if self == Self::En { return n.to_string(); }
        n.to_string().chars().map(|c| match c {
            '0'=>'۰', '1'=>'۱', '2'=>'۲', '3'=>'۳', '4'=>'۴', '5'=>'۵', '6'=>'۶', '7'=>'۷', '8'=>'۸', '9'=>'۹', x=>x,
        }).collect()
    }
    fn flags(self) -> u32 { if self == Self::Fa { DT_RIGHT | DT_RTLREADING } else { DT_LEFT } }
    fn title(self) -> &'static str { self.t("Roshan Converter", "روشن — مبدل عکس و ویدئو") }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Status { Pending, Working, Done, Warning, Failed, Canceled }
impl Status {
    fn ready(self) -> bool { matches!(self, Self::Pending | Self::Failed | Self::Canceled) }
    fn label(self, l: Lang) -> &'static str { match self {
        Self::Pending=>l.t("Ready", "آماده"), Self::Working=>l.t("Converting…", "در حال تبدیل…"),
        Self::Done=>l.t("Done", "انجام شد"), Self::Warning=>l.t("Done · note", "انجام شد · توضیح"),
        Self::Failed=>l.t("Failed", "ناموفق"), Self::Canceled=>l.t("Canceled", "متوقف شد"),
    }}
}
struct Job {
    input: PathBuf,
    key: String,
    kind: MediaKind,
    status: Status,
    progress: f32,
    outputs: Vec<ConvertedImage>,
    error: Option<String>,
}
#[derive(Default, Clone)]
struct AddReport { added: usize, images: usize, videos: usize, duplicates: usize, unsupported: usize, unavailable: usize }
enum Banner { Ready, Added(AddReport), Running, Stopping, Finished { canceled: bool, unexpected: bool }, Notice(String, String, u32) }
enum WorkerEvent { Started(usize), Progress(usize, f32), Converted(usize, Result<Vec<ConvertedImage>, String>), Finished }

#[derive(Default, Clone, Copy)]
struct Controls {
    add: HWND, remove: HWND, clear: HWND, folder: HWND, reset: HWND, start: HWND, cancel: HWND,
    open: HWND, metadata: HWND, list: HWND, output: HWND, detail: HWND, progress: HWND,
    about: HWND, empty: HWND, language: HWND, image_format: HWND, video_format: HWND,
    image_label: HWND, video_label: HWND,
}
impl Controls {
    fn all(self) -> [HWND; 20] { [self.add,self.remove,self.clear,self.folder,self.reset,self.start,self.cancel,
        self.open,self.metadata,self.list,self.output,self.detail,self.progress,self.about,self.empty,self.language,
        self.image_format,self.video_format,self.image_label,self.video_label] }
}
#[derive(Default, Clone, Copy)]
struct Fonts { body: HFONT, small: HFONT, title: HFONT, section: HFONT, drop: HFONT }
struct App {
    hwnd: HWND, instance: HINSTANCE, dpi: u32, lang: Lang, controls: Controls, fonts: Fonts,
    background: HBRUSH, white: HBRUSH, row_images: HIMAGELIST, jobs: Vec<Job>,
    image_format: ImageFormat, video_format: VideoFormat, output_dir: Option<PathBuf>, last_output_dir: Option<PathBuf>,
    busy: bool, closing: bool, dialog_open: bool, cancel: Arc<AtomicBool>,
    receiver: Option<mpsc::Receiver<WorkerEvent>>, worker: Option<JoinHandle<()>>,
    total: usize, completed: usize, successes: usize, failures: usize, output_count: usize,
    banner: Banner, startup_error: Option<String>,
}
impl App {
    unsafe fn new(instance: HINSTANCE) -> Self { Self {
        hwnd:null_mut(), instance, dpi:96, lang:Lang::En, controls:Controls::default(), fonts:Fonts::default(),
        background:CreateSolidBrush(BG), white:CreateSolidBrush(WHITE), row_images:0, jobs:Vec::new(),
        image_format:ImageFormat::Jpg, video_format:VideoFormat::Mp4, output_dir:None, last_output_dir:None,
        busy:false, closing:false, dialog_open:false, cancel:Arc::new(AtomicBool::new(false)), receiver:None, worker:None,
        total:0, completed:0, successes:0, failures:0, output_count:0, banner:Banner::Ready, startup_error:None,
    }}
}
impl Drop for App {
    fn drop(&mut self) {
        self.cancel.store(true,Ordering::Relaxed);
        if let Some(worker)=self.worker.take() { let _=worker.join(); }
        unsafe {
            for font in [self.fonts.body,self.fonts.small,self.fonts.title,self.fonts.section,self.fonts.drop] {
                if !font.is_null() { DeleteObject(font); }
            }
            if !self.background.is_null() { DeleteObject(self.background); }
            if !self.white.is_null() { DeleteObject(self.white); }
            if self.row_images!=0 { ImageList_Destroy(self.row_images); }
        }
    }
}
struct ComApartment;
impl Drop for ComApartment { fn drop(&mut self) { unsafe { CoUninitialize(); } } }

fn wide(text:&str)->Vec<u16> { text.encode_utf16().chain(Some(0)).collect() }
fn native_text(text:&str)->Vec<u16> {
    // Explicit paragraph direction preserves Persian on English Windows. Paths
    // already isolated as LTR are left alone, even when a filename is Persian.
    let text=text.split('\n').map(|line| {
        if !line.starts_with('\u{202a}') && line.chars().any(|c| ('\u{0600}'..='\u{06ff}').contains(&c)) {
            format!("\u{202b}{line}\u{202c}")
        } else { line.to_owned() }
    }).collect::<Vec<_>>().join("\n");
    wide(&text)
}
fn path_wide(path:&Path)->Vec<u16> { path.as_os_str().encode_wide().chain(Some(0)).collect() }
fn display_path(path:&Path)->String { format!("\u{202a}{}\u{202c}",path.display()) }
fn scale(dpi:u32,value:i32)->i32 { ((value as i64*dpi as i64+48)/96) as i32 }
fn size_text(bytes:u64)->String {
    if bytes==0 { String::new() } else if bytes>=1_073_741_824 { format!("{:.2} GB",bytes as f64/1_073_741_824.) }
    else if bytes>=1_048_576 { format!("{:.1} MB",bytes as f64/1_048_576.) } else { format!("{:.0} KB",bytes as f64/1024.) }
}
fn localized_issue(l:Lang,text:&str)->String {
    if l==Lang::En { return text.into(); }
    let exact=[
        ("Conversion canceled","تبدیل متوقف شد"),
        ("The input file is empty","فایل ورودی خالی است"),
        ("Choose a file, not a folder","یک فایل انتخاب کنید؛ پوشه قابل تبدیل نیست"),
        ("No images were produced","هیچ خروجی قابل استفاده‌ای تولید نشد"),
        ("Supported files: HEIC, PNG, JPG, WebP, MP4, MOV, AVI and WebM","فرمت‌های پشتیبانی‌شده: HEIC، PNG، JPG، WebP، MP4، MOV، AVI و WebM"),
        ("The image exceeds the 512 MiB input limit; it was not resized.","حجم عکس از سقف ۵۱۲ مگابایت بیشتر است؛ ابعاد عکس تغییر نکرد."),
        ("Not enough memory to read this image","حافظهٔ کافی برای خواندن این عکس وجود ندارد"),
        ("The input image is too large","عکس ورودی بیش از اندازه بزرگ است"),
        ("Image dimensions exceed the 200 megapixel limit; the image was not resized.","ابعاد عکس از سقف ۲۰۰ مگاپیکسل بیشتر است؛ عکس کوچک نشده است."),
        ("The decoded image exceeds the memory limit","حافظهٔ مورد نیاز این عکس از حد مجاز بیشتر است"),
        ("The decoder returned an invalid pixel buffer","دادهٔ پیکسل‌های خوانده‌شده معتبر نیست"),
        ("The file contents are not JPG, PNG or WebP","محتوای فایل با JPG، PNG یا WebP مطابقت ندارد"),
        ("JPG supports at most 65,535 pixels per side; the image was not resized.","هر ضلع JPG حداکثر ۶۵٬۵۳۵ پیکسل است؛ ابعاد عکس تغییر نکرد."),
        ("WebP supports at most 16,383 pixels per side; choose PNG for this image.","هر ضلع WebP حداکثر ۱۶٬۳۸۳ پیکسل است؛ برای این عکس PNG را انتخاب کنید."),
        ("HEIC encoding produced no output","ساخت HEIC هیچ خروجی‌ای تولید نکرد"),
        ("The HEIC container has no usable still images or exceeds the 1,000-image limit","فایل HEIC عکس قابل استفاده ندارد یا تعداد عکس‌های آن از سقف ۱٬۰۰۰ بیشتر است"),
        ("HDR was tone-mapped to standard SDR; the HDR brightness range is not retained.","روشنایی HDR به SDR استاندارد تبدیل شد؛ گسترهٔ روشنایی HDR حفظ نمی‌شود."),
        ("Transparency was composited onto white because JPG does not support alpha.","بخش‌های شفاف روی زمینهٔ سفید قرار گرفتند؛ JPG شفافیت ندارد."),
        ("The source has more than 8 bits per channel; this conversion uses 8-bit color.","عمق رنگ منبع بیش از ۸ بیت در هر کانال است؛ این تبدیل از رنگ ۸ بیتی استفاده می‌کند."),
        ("The source has more than 8 bits per channel; this output format uses 8-bit color.","عمق رنگ منبع بیش از ۸ بیت در هر کانال است؛ فرمت خروجی از رنگ ۸ بیتی استفاده می‌کند."),
        ("The source color description was incomplete; a matching default profile was used.","اطلاعات رنگ منبع ناقص بود؛ پروفایل پیش‌فرض متناسب استفاده شد."),
        ("The decoder reported a warning; please inspect the result.","خواندن عکس با هشدار همراه بود؛ لطفاً نتیجه را بررسی کنید."),
        ("Some source metadata exceeded the size limit and was omitted.","بخشی از اطلاعات جانبی از حد مجاز بزرگ‌تر بود و منتقل نشد."),
        ("The source color profile was invalid and was replaced.","پروفایل رنگ منبع معتبر نبود و جایگزین شد."),
        ("A grayscale color profile was converted to sRGB.","پروفایل رنگ خاکستری به sRGB تبدیل شد."),
        ("The color profile could not be read; sRGB was assumed.","پروفایل رنگ خوانده نشد؛ فضای رنگ sRGB در نظر گرفته شد."),
        ("The EXIF metadata could not be read and was omitted.","اطلاعات EXIF خوانده نشد و منتقل نشد."),
        ("Animated PNG input: only the default still frame was exported.","ورودی PNG متحرک است؛ فقط تصویر ثابت پیش‌فرض ذخیره شد."),
        ("Animated WebP input: only the first still frame was exported.","ورودی WebP متحرک است؛ فقط اولین تصویر ثابت ذخیره شد."),
        ("Invalid EXIF metadata was omitted","اطلاعات EXIF نامعتبر بود و منتقل نشد"),
        ("EXIF metadata exceeds the output limit and was omitted","حجم اطلاعات EXIF از سقف خروجی بیشتر بود و منتقل نشد"),
        ("Incomplete EXIF metadata was omitted","اطلاعات EXIF ناقص بود و منتقل نشد"),
        ("Malformed EXIF metadata was omitted","ساختار اطلاعات EXIF خراب بود و منتقل نشد"),
        ("EXIF metadata is too complex and was omitted","ساختار اطلاعات EXIF بیش از حد پیچیده بود و منتقل نشد"),
        ("The source ICC profile was invalid; remaining PNG color data or an sRGB default were used.","پروفایل ICC منبع نامعتبر بود؛ اطلاعات رنگ PNG یا sRGB پیش‌فرض استفاده شد."),
        ("Invalid or repeated PNG cICP color chunk; transcoding was stopped to avoid incorrect color.","اطلاعات رنگ cICP در PNG نامعتبر یا تکراری است؛ تبدیل برای جلوگیری از رنگ نادرست متوقف شد."),
        ("HDR PNG transcoding is not supported. Choose PNG output to retain the original HDR file.","تبدیل PNG دارای HDR پشتیبانی نمی‌شود؛ خروجی PNG را برای حفظ فایل اصلی HDR انتخاب کنید."),
        ("A grayscale ICC profile does not match the decoded RGB pixels.","پروفایل ICC خاکستری با پیکسل‌های RGB عکس مطابقت ندارد."),
        ("This image uses a CMYK or other non-RGB ICC profile. A color-managed RGB source is required.","پروفایل رنگ این عکس CMYK یا غیر RGB است؛ ابتدا یک منبع RGB با رنگ صحیح تهیه کنید."),
        ("This video has more than the supported 32 audio tracks","این ویدئو بیش از ۳۲ مسیر صوتی دارد و پشتیبانی نمی‌شود"),
        ("The video dimensions could not be read","ابعاد ویدئو خوانده نشد"),
        ("WebM Opus supports up to eight channels; this track was not downmixed.","صدای Opus در WebM حداکثر ۸ کانال دارد؛ تعداد کانال‌های این صدا کاهش داده نشد."),
        ("The audio-track count changed unexpectedly; the result was not published.","تعداد مسیرهای صوتی به‌طور غیرمنتظره تغییر کرد؛ خروجی ذخیره نشد."),
        ("The video encoder produced an empty file","تبدیل ویدئو یک فایل خالی تولید کرد"),
        ("The first main video stream was converted; additional video views were omitted.","اولین مسیر اصلی تصویر تبدیل شد؛ نماهای ویدئویی دیگر منتقل نشدند."),
        ("Subtitles, attachments and auxiliary data streams were not included in the converted video.","زیرنویس‌ها، پیوست‌ها و داده‌های جانبی به ویدئوی خروجی منتقل نشدند."),
        ("HDR video was tone-mapped to standard SDR for this output codec.","روشنایی ویدئوی HDR برای این خروجی به SDR استاندارد تبدیل شد."),
        ("Odd dimensions were preserved using H.264 4:4:4; some hardware players may require a software player.","ابعاد فرد با H.264 و نمونه‌برداری ۴:۴:۴ حفظ شد؛ بعضی دستگاه‌ها به پخش‌کنندهٔ نرم‌افزاری نیاز دارند."),
        ("AVI uses lossless FFV1 to retain the odd dimensions; a compatible player is required.","برای حفظ ابعاد فرد، AVI از FFV1 بدون اتلاف استفاده می‌کند؛ پخش‌کنندهٔ سازگار لازم است."),
        ("Video transparency is not preserved when the video codec must be changed.","هنگام تغییر کدک ویدئو، شفافیت آن حفظ نمی‌شود."),
        ("The output duration differs from the source container duration; please check the result.","مدت خروجی با مدت ثبت‌شدهٔ منبع تفاوت دارد؛ لطفاً نتیجه را بررسی کنید."),
        ("Bundled video engine size verification failed","اندازهٔ موتور ویدئوی همراه برنامه تأیید نشد"),
        ("Bundled video engine integrity verification failed","صحت موتور ویدئوی همراه برنامه تأیید نشد"),
        ("Cannot reserve a private directory for the offline video engine","ایجاد پوشهٔ موقت اختصاصی برای موتور ویدئوی آفلاین ممکن نشد"),
        ("Video inspection reader failed","خواندن اطلاعات ویدئو ناموفق بود"),
        ("An internal conversion error occurred. The original file is unchanged.","یک خطای داخلی در تبدیل رخ داد. فایل اصلی تغییر نکرد."),
    ];
    for (en,fa) in exact { if text.trim_end_matches('.')==en.trim_end_matches('.') { return fa.into(); } }
    let prefixes=[
        ("Cannot open the input file:","فایل ورودی باز نشد:"),("Cannot read the input file:","فایل ورودی خوانده نشد:"),
        ("Cannot read this HEIC file:","فایل HEIC خوانده نشد:"),("Cannot decode this image:","خواندن پیکسل‌های عکس ناموفق بود:"),
        ("Cannot recognize this image:","فرمت عکس شناسایی نشد:"),("Cannot read this image:","عکس خوانده نشد:"),
        ("Cannot write to the output folder:","نوشتن در پوشهٔ خروجی ممکن نشد:"),
        ("Cannot embed the color profile:","ذخیرهٔ پروفایل رنگ ممکن نشد:"),("Cannot embed EXIF metadata:","ذخیرهٔ اطلاعات EXIF ممکن نشد:"),
        ("JPG encoding failed:","ساخت JPG ناموفق بود:"),("PNG encoding failed:","ساخت PNG ناموفق بود:"),
        ("WebP encoding failed:","ساخت WebP ناموفق بود:"),("HEIC encoding failed:","ساخت HEIC ناموفق بود:"),
        ("Cannot write HEIC output:","نوشتن خروجی HEIC ممکن نشد:"),("Could not finish writing the output:","نوشتن خروجی کامل نشد:"),
        ("Could not save the output:","ذخیرهٔ خروجی ممکن نشد:"),("Could not publish the output:","ذخیرهٔ نهایی خروجی ممکن نشد:"),
        ("The video inspection data is invalid:","اطلاعات خوانده‌شدهٔ ویدئو معتبر نیست:"),
        ("Cannot inspect this video:","بررسی این ویدئو ممکن نشد:"),("Video conversion failed:","تبدیل ویدئو ناموفق بود:"),
        ("Cannot prepare the bundled video engine:","آماده‌سازی موتور ویدئوی همراه ممکن نشد:"),
        ("Cannot unpack the video engine:","بازکردن بستهٔ موتور ویدئو ممکن نشد:"),
        ("Cannot prepare the offline video engine:","آماده‌سازی موتور ویدئوی آفلاین ممکن نشد:"),
        ("Cannot start the bundled video engine:","اجرای موتور ویدئوی همراه ممکن نشد:"),
        ("Cannot manage the video worker safely:","ادارهٔ امن پردازش ویدئو ممکن نشد:"),
        ("Could not create the color profile:","ساخت پروفایل رنگ ممکن نشد:"),
        ("Could not composite transparency:","ترکیب بخش‌های شفاف با زمینه ممکن نشد:"),
        ("Cannot preserve PNG cICP color:","حفظ رنگ cICP در PNG ممکن نشد:"),
        ("Output dimensions changed unexpectedly", "ابعاد خروجی به‌طور غیرمنتظره تغییر کرد و خروجی ذخیره نشد؛ جزئیات:"),
    ];
    for (en,fa) in prefixes { if let Some(rest)=text.strip_prefix(en) { return format!("{fa}\n\u{202a}{}\u{202c}",rest.trim_start()); } }
    if let Some(rest)=text.strip_prefix("Image ") {
        if let Some((position,error))=rest.split_once(": ") {
            if let Some((index,count))=position.split_once(" of ") {
                if let (Ok(index),Ok(count))=(index.parse::<usize>(),count.parse::<usize>()) {
                    return format!("عکس {} از {}: {}",l.number(index),l.number(count),localized_issue(l,error));
                }
            }
        }
    }
    text.into()
}
unsafe fn app_ptr(hwnd:HWND)->*mut App { GetWindowLongPtrW(hwnd,GWLP_USERDATA) as *mut App }
unsafe fn caption(hwnd:HWND,text:&str) { let text=native_text(text); SetWindowTextW(hwnd,text.as_ptr()); }
unsafe fn invalidate(app:*mut App) { InvalidateRect((*app).hwnd,null(),0); }
unsafe fn notice(app:*mut App,en:String,fa:String,color:u32) { (*app).banner=Banner::Notice(en,fa,color); invalidate(app); }

pub fn run()->Result<(),String> { unsafe {
    let result=CoInitializeEx(null(),COINIT_APARTMENTTHREADED as u32|COINIT_DISABLE_OLE1DDE as u32);
    if result<0 { return Err(format!("Cannot initialize Windows COM ({result:#x}).")); }
    let _com=ComApartment;
    SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    let common=INITCOMMONCONTROLSEX {dwSize:size_of::<INITCOMMONCONTROLSEX>() as u32,dwICC:ICC_LISTVIEW_CLASSES|ICC_PROGRESS_CLASS|ICC_STANDARD_CLASSES};
    if InitCommonControlsEx(&common)==0 { return Err("Cannot initialize Windows controls.".into()); }
    let instance=GetModuleHandleW(null()); let class_name=wide(CLASS_NAME);
    let mut icon=LoadIconW(instance,1usize as *const u16);
    if icon.is_null() { icon=LoadIconW(null_mut(),IDI_APPLICATION); }
    let class=WNDCLASSW {style:CS_HREDRAW|CS_VREDRAW,lpfnWndProc:Some(window_proc),hInstance:instance,hIcon:icon,
        hCursor:LoadCursorW(null_mut(),IDC_ARROW),hbrBackground:null_mut(),lpszClassName:class_name.as_ptr(),..zeroed()};
    if RegisterClassW(&class)==0 { return Err(format!("Cannot register the window: {}",GetLastError())); }
    let mut app=Box::new(App::new(instance)); let ptr=app.as_mut() as *mut App;
    let dpi=GetDpiForSystem().max(96); let mut work:RECT=zeroed();
    SystemParametersInfoW(SPI_GETWORKAREA,0,&mut work as *mut _ as *mut c_void,0);
    let aw=(work.right-work.left).max(800); let ah=(work.bottom-work.top).max(600);
    let width=scale(dpi,940).min(aw-24); let height=scale(dpi,650).min(ah-24);
    let title=wide(Lang::En.title());
    let hwnd=CreateWindowExW(WS_EX_CONTROLPARENT,class_name.as_ptr(),title.as_ptr(),WS_OVERLAPPEDWINDOW|WS_CLIPCHILDREN,
        work.left+(aw-width)/2,work.top+(ah-height)/2,width,height,null_mut(),null_mut(),instance,ptr as *mut c_void);
    if hwnd.is_null() { return Err(app.startup_error.take().unwrap_or_else(||format!("Cannot create the window: {}",GetLastError()))); }
    ShowWindow(hwnd,SW_SHOW); UpdateWindow(hwnd);
    let arguments:Vec<PathBuf>=std::env::args_os().skip(1).map(PathBuf::from).filter(|p|media_kind(p).is_some()).collect();
    if !arguments.is_empty() { let report=add_files(ptr,arguments); show_drop_report(ptr,&report); }
    let mut msg:MSG=zeroed();
    loop {
        let result=GetMessageW(&mut msg,null_mut(),0,0);
        if result<0 { return Err(format!("Cannot read window messages: {}",GetLastError())); }
        if result==0 { break; }
        let mut handled=false;
        if msg.message==WM_KEYDOWN && IsWindowEnabled(hwnd)!=0 {
            let ctrl=GetKeyState(VK_CONTROL as i32)<0;
            let id=if ctrl && msg.wParam==b'O' as usize {Some(ID_ADD)}
                else if ctrl && msg.wParam==VK_RETURN as usize {Some(ID_START)}
                else if msg.wParam==VK_DELETE as usize && GetFocus()==app.controls.list {Some(ID_REMOVE)}
                else if msg.wParam==VK_ESCAPE as usize && app.busy {Some(ID_CANCEL)}
                else if msg.wParam==VK_F1 as usize {Some(ID_ABOUT)} else {None};
            if let Some(id)=id {SendMessageW(hwnd,WM_COMMAND,id as usize,0);handled=true;}
        }
        if !handled && IsDialogMessageW(hwnd,&msg)==0 {TranslateMessage(&msg);DispatchMessageW(&msg);}
    }
    Ok(())
}}

unsafe extern "system" fn window_proc(hwnd:HWND,message:u32,wparam:WPARAM,lparam:LPARAM)->LRESULT {
    if message==WM_NCCREATE {
        let create=&*(lparam as *const CREATESTRUCTW); let app=create.lpCreateParams as *mut App;
        (*app).hwnd=hwnd; SetWindowLongPtrW(hwnd,GWLP_USERDATA,app as isize); return 1;
    }
    let app=app_ptr(hwnd); if app.is_null() {return DefWindowProcW(hwnd,message,wparam,lparam);}
    match message {
        WM_CREATE=>{
            (*app).dpi=GetDpiForWindow(hwnd).max(96);
            if !create_controls(app) {return -1;}
            set_fonts(app); translate_ui(app); layout(app); refresh_controls(app); DragAcceptFiles(hwnd,1);
            if SetTimer(hwnd,TIMER_ID,80,None)==0 {(*app).startup_error=Some("Cannot start queue monitoring.".into());return -1;}
            0
        }
        WM_COMMAND=>{
            let id=(wparam&0xffff) as i32; let event=(wparam>>16) as u32;
            if event==BN_CLICKED {command(app,id);}
            else if event==CBN_SELCHANGE && !(*app).busy {
                if id==ID_IMAGE_FORMAT {
                    let index=SendMessageW((*app).controls.image_format,CB_GETCURSEL,0,0) as usize;
                    if let Some(format)=[ImageFormat::Heic,ImageFormat::Png,ImageFormat::Jpg,ImageFormat::Webp].get(index) {
                        if (*app).image_format!=*format {(*app).image_format=*format;settings_changed(app,Some(MediaKind::Image));}
                    }
                } else if id==ID_VIDEO_FORMAT {
                    let index=SendMessageW((*app).controls.video_format,CB_GETCURSEL,0,0) as usize;
                    if let Some(format)=[VideoFormat::Mp4,VideoFormat::Mov,VideoFormat::Avi,VideoFormat::Webm].get(index) {
                        if (*app).video_format!=*format {(*app).video_format=*format;settings_changed(app,Some(MediaKind::Video));}
                    }
                }
                update_all_rows(app); update_details(app);
            }
            0
        }
        WM_DROPFILES=>{
            let drop=wparam as HDROP; let count=DragQueryFileW(drop,u32::MAX,null_mut(),0); let mut paths=Vec::with_capacity(count as usize);
            for index in 0..count {
                let length=DragQueryFileW(drop,index,null_mut(),0);let mut buffer=vec![0u16;length as usize+1];
                if DragQueryFileW(drop,index,buffer.as_mut_ptr(),buffer.len() as u32)>0 {paths.push(PathBuf::from(OsString::from_wide(&buffer[..length as usize])));}
            }
            DragFinish(drop);
            if (*app).busy {
                let l=(*app).lang;
                modal(app,l.t("Conversion in progress","تبدیل در حال انجام است"),l.t("Finish or cancel this batch first","ابتدا این صف را تمام یا متوقف کنید"),
                    l.t("These dropped files were not added. You can add them when this batch stops.","این فایل‌ها به صف اضافه نشدند. بعد از پایان یا توقف صف، دوباره آن‌ها را رها کنید."),
                    &[(MODAL_CLOSE,l.t("OK","باشه"))],MODAL_CLOSE,None);
            } else {let report=add_files(app,paths);show_drop_report(app,&report);}
            0
        }
        WM_TIMER if wparam==TIMER_ID=>{poll_worker(app);0}
        WM_SIZE=>{layout(app);0}
        WM_DPICHANGED=>{
            (*app).dpi=(wparam&0xffff) as u32;set_fonts(app);
            let rect=&*(lparam as *const RECT);
            SetWindowPos(hwnd,null_mut(),rect.left,rect.top,rect.right-rect.left,rect.bottom-rect.top,SWP_NOACTIVATE|SWP_NOZORDER);
            layout(app);0
        }
        WM_GETMINMAXINFO=>{
            let info=&mut *(lparam as *mut MINMAXINFO);
            info.ptMinTrackSize.x=scale((*app).dpi,780); info.ptMinTrackSize.y=scale((*app).dpi,590);0
        }
        WM_PAINT=>{paint(app);0}
        WM_ERASEBKGND=>1,
        WM_DRAWITEM=>{draw_button(app,&*(lparam as *const DRAWITEMSTRUCT));1}
        WM_CTLCOLORSTATIC|WM_CTLCOLOREDIT|WM_CTLCOLORBTN=>{
            let dc=wparam as HDC;let control=lparam as HWND;SetBkMode(dc,TRANSPARENT as i32);
            SetTextColor(dc,if control==(*app).controls.empty {MUTED}else{INK});
            if control==(*app).controls.metadata||control==(*app).controls.output||control==(*app).controls.empty {
                SetBkColor(dc,WHITE);(*app).white as isize
            } else {SetBkColor(dc,BG);(*app).background as isize}
        }
        WM_NOTIFY=>notify_control(app,lparam),
        WM_LBUTTONUP=>{
            let x=(lparam as u32&0xffff) as i16 as i32;let y=((lparam as u32>>16)&0xffff) as i16 as i32;
            let mut rect:RECT=zeroed();GetClientRect(hwnd,&mut rect);let dpi=(*app).dpi;
            if !(*app).busy&&x>=scale(dpi,18)&&x<=rect.right-scale(dpi,18)&&y>=scale(dpi,72)&&y<=scale(dpi,158) {pick_files(app);}0
        }
        WM_CLOSE=>{
            if (*app).busy {
                if (*app).closing||(*app).dialog_open {return 0;}
                let l=(*app).lang;
                let action=modal(app,l.t("Close Roshan","بستن روشن"),l.t("Cancel conversion and close?","تبدیل متوقف و برنامه بسته شود؟"),
                    l.t("Completed outputs are kept. The current conversion will stop safely.","خروجی‌های کامل‌شده حفظ می‌شوند. تبدیل جاری به‌صورت امن متوقف خواهد شد."),
                    &[(MODAL_CLOSE,l.t("Keep converting","ادامهٔ تبدیل")),(MODAL_CONVERT,l.t("Cancel and close","توقف و بستن"))],MODAL_CLOSE,None);
                if action==MODAL_CONVERT {
                    if (*app).busy {(*app).closing=true;request_stop(app);}else{DestroyWindow(hwnd);}
                }
            } else {DestroyWindow(hwnd);}0
        }
        WM_DESTROY=>{KillTimer(hwnd,TIMER_ID);DragAcceptFiles(hwnd,0);(*app).cancel.store(true,Ordering::Relaxed);PostQuitMessage(0);0}
        WM_NCDESTROY=>{SetWindowLongPtrW(hwnd,GWLP_USERDATA,0);DefWindowProcW(hwnd,message,wparam,lparam)}
        _=>DefWindowProcW(hwnd,message,wparam,lparam),
    }
}

unsafe fn create_control(app:*mut App,class:&str,text:&str,id:i32,style:u32,ex:u32)->HWND {
    let class=wide(class);let text=native_text(text);
    CreateWindowExW(ex,class.as_ptr(),text.as_ptr(),WS_CHILD|WS_VISIBLE|style,0,0,1,1,(*app).hwnd,id as usize as HMENU,(*app).instance,null())
}
unsafe fn create_controls(app:*mut App)->bool {
    let button=|app:*mut App,id|create_control(app,"BUTTON","",id,WS_TABSTOP|BS_OWNERDRAW as u32,0);
    let mut c=Controls::default();
    c.add=button(app,ID_ADD);c.remove=button(app,ID_REMOVE);c.clear=button(app,ID_CLEAR);
    c.folder=button(app,ID_FOLDER);c.reset=button(app,ID_RESET);c.start=button(app,ID_START);c.cancel=button(app,ID_CANCEL);
    c.open=button(app,ID_OPEN);c.about=button(app,ID_ABOUT);c.language=button(app,ID_LANGUAGE);
    c.metadata=create_control(app,"BUTTON","",ID_META,WS_TABSTOP|BS_AUTOCHECKBOX as u32|BS_LEFT as u32,0);
    c.output=create_control(app,"EDIT","",ID_OUTPUT,WS_TABSTOP|ES_READONLY as u32|ES_AUTOHSCROLL as u32,0);
    c.detail=create_control(app,"EDIT","",ID_DETAIL,WS_TABSTOP|ES_READONLY as u32|ES_MULTILINE as u32|ES_AUTOVSCROLL as u32|WS_VSCROLL,0);
    c.image_label=create_control(app,"STATIC","",ID_IMAGE_LABEL,SS_LEFT,0);
    c.image_format=create_control(app,"COMBOBOX","",ID_IMAGE_FORMAT,WS_TABSTOP|CBS_DROPDOWNLIST as u32|CBS_HASSTRINGS as u32|WS_VSCROLL,0);
    c.video_label=create_control(app,"STATIC","",ID_VIDEO_LABEL,SS_LEFT,0);
    c.video_format=create_control(app,"COMBOBOX","",ID_VIDEO_FORMAT,WS_TABSTOP|CBS_DROPDOWNLIST as u32|CBS_HASSTRINGS as u32|WS_VSCROLL,0);
    c.list=create_control(app,"SysListView32","",ID_LIST,WS_TABSTOP|LVS_REPORT|LVS_SHOWSELALWAYS|LVS_NOSORTHEADER|LVS_SHAREIMAGELISTS,WS_EX_CLIENTEDGE);
    c.progress=create_control(app,"msctls_progress32","",ID_PROGRESS,PBS_SMOOTH,0);
    c.empty=create_control(app,"STATIC","",ID_EMPTY,SS_CENTER,0);
    (*app).controls=c;
    if c.all().iter().any(|x|x.is_null()) {
        let ids=[ID_ADD,ID_REMOVE,ID_CLEAR,ID_FOLDER,ID_RESET,ID_START,ID_CANCEL,ID_OPEN,ID_META,ID_LIST,ID_OUTPUT,ID_DETAIL,ID_PROGRESS,ID_ABOUT,ID_EMPTY,ID_LANGUAGE,ID_IMAGE_FORMAT,ID_VIDEO_FORMAT,ID_IMAGE_LABEL,ID_VIDEO_LABEL];
        let missing:Vec<i32>=c.all().iter().zip(ids).filter_map(|(h,id)|if h.is_null(){Some(id)}else{None}).collect();
        (*app).startup_error=Some(format!("Cannot create native controls {missing:?} (Windows error {}).",GetLastError()));return false;
    }
    SendMessageW(c.metadata,BM_SETCHECK,BST_CHECKED as usize,0);
    for format in ["HEIC","PNG","JPG","WebP"] {let text=wide(format);SendMessageW(c.image_format,CB_ADDSTRING,0,text.as_ptr() as isize);}
    for format in ["MP4","MOV","AVI","WebM"] {let text=wide(format);SendMessageW(c.video_format,CB_ADDSTRING,0,text.as_ptr() as isize);}
    SendMessageW(c.image_format,CB_SETCURSEL,2,0);SendMessageW(c.video_format,CB_SETCURSEL,0,0);
    SendMessageW(c.list,LVM_SETEXTENDEDLISTVIEWSTYLE,0,(LVS_EX_FULLROWSELECT|LVS_EX_DOUBLEBUFFER|LVS_EX_LABELTIP) as isize);
    SendMessageW(c.list,LVM_SETBKCOLOR,0,WHITE as isize);SendMessageW(c.list,LVM_SETTEXTBKCOLOR,0,WHITE as isize);SendMessageW(c.list,LVM_SETTEXTCOLOR,0,INK as isize);
    let theme=wide("Explorer");SetWindowTheme(c.list,theme.as_ptr(),null());
    for index in 0..4 {
        let mut column:LVCOLUMNW=zeroed();column.mask=LVCF_TEXT|LVCF_WIDTH|LVCF_FMT|LVCF_SUBITEM;
        let mut text=wide("");column.pszText=text.as_mut_ptr();column.cx=120;column.iSubItem=index;
        SendMessageW(c.list,LVM_INSERTCOLUMNW,index as usize,&column as *const _ as isize);
    }
    SendMessageW(c.progress,PBM_SETRANGE32,0,1000);SendMessageW(c.progress,PBM_SETBARCOLOR,0,ACCENT as isize);
    true
}
unsafe fn font(dpi:u32,height:i32,weight:i32)->HFONT {
    let face=wide("Segoe UI");CreateFontW(-scale(dpi,height),0,0,0,weight,0,0,0,DEFAULT_CHARSET as u32,
        OUT_DEFAULT_PRECIS as u32,CLIP_DEFAULT_PRECIS as u32,CLEARTYPE_QUALITY as u32,DEFAULT_PITCH as u32|FF_DONTCARE as u32,face.as_ptr())
}
unsafe fn set_fonts(app:*mut App) {
    let dpi=(*app).dpi;let fa=(*app).lang==Lang::Fa;let old=(*app).fonts;
    let f=Fonts{body:font(dpi,if fa{16}else{14},FW_NORMAL as i32),small:font(dpi,if fa{13}else{12},FW_NORMAL as i32),
        title:font(dpi,25,FW_SEMIBOLD as i32),section:font(dpi,15,FW_SEMIBOLD as i32),drop:font(dpi,if fa{19}else{18},FW_SEMIBOLD as i32)};
    (*app).fonts=f;let c=(*app).controls;
    for control in c.all() {SendMessageW(control,WM_SETFONT,f.body as usize,1);}
    for control in [c.detail,c.image_label,c.video_label,c.metadata] {SendMessageW(control,WM_SETFONT,f.small as usize,1);}
    SendMessageW(c.image_format,CB_SETITEMHEIGHT,usize::MAX,scale(dpi,24) as isize);
    SendMessageW(c.video_format,CB_SETITEMHEIGHT,usize::MAX,scale(dpi,24) as isize);
    let images=ImageList_Create(1,scale(dpi,32),ILC_COLOR32,1,1);
    if images!=0 {let previous=SendMessageW(c.list,LVM_SETIMAGELIST,LVSIL_SMALL as usize,images as isize) as HIMAGELIST;(*app).row_images=images;if previous!=0{ImageList_Destroy(previous);}}
    for font in [old.body,old.small,old.title,old.section,old.drop] {if !font.is_null(){DeleteObject(font);}}
}
unsafe fn set_alignment(hwnd:HWND,l:Lang,is_edit:bool) {
    let mut style=GetWindowLongPtrW(hwnd,GWL_STYLE) as u32;
    if is_edit {style&=!(ES_LEFT as u32|ES_CENTER as u32|ES_RIGHT as u32);if l==Lang::Fa{style|=ES_RIGHT as u32;}}
    else {style&=!(SS_LEFT|SS_CENTER|SS_RIGHT);style|=if l==Lang::Fa{SS_RIGHT}else{SS_LEFT};}
    SetWindowLongPtrW(hwnd,GWL_STYLE,style as isize);
    let mut ex=GetWindowLongPtrW(hwnd,GWL_EXSTYLE) as u32;ex&=!WS_EX_RTLREADING;if l==Lang::Fa{ex|=WS_EX_RTLREADING;}
    SetWindowLongPtrW(hwnd,GWL_EXSTYLE,ex as isize);InvalidateRect(hwnd,null(),1);
}
unsafe fn translate_ui(app:*mut App) {
    let l=(*app).lang;let c=(*app).controls;caption((*app).hwnd,l.title());
    for (control,en,fa) in [(c.add,"Add files","انتخاب فایل"),(c.remove,"Remove","حذف"),(c.clear,"Clear","پاک‌سازی"),
        (c.folder,"Browse…","پوشه…"),(c.reset,"Reset","اولیه"),(c.open,"Output folder","پوشهٔ خروجی"),
        (c.about,"About","درباره"),(c.metadata,"Keep metadata","حفظ اطلاعات فایل"),(c.image_label,"Images →","عکس ←"),
        (c.video_label,"Videos →","ویدئو ←")]
    {caption(control,l.t(en,fa));}
    caption(c.language,if l==Lang::En{"فارسی"}else{"English"});
    caption(c.empty,l.t("Your files will appear here","فایل‌ها اینجا نمایش داده می‌شوند"));
    let mut style=GetWindowLongPtrW(c.metadata,GWL_STYLE) as u32;
    style&=!(BS_RIGHTBUTTON as u32|BS_LEFT as u32|BS_RIGHT as u32);
    style|=if l==Lang::Fa{BS_RIGHTBUTTON as u32|BS_RIGHT as u32}else{BS_LEFT as u32};
    SetWindowLongPtrW(c.metadata,GWL_STYLE,style as isize);
    set_alignment(c.output,l,true);set_alignment(c.detail,l,true);set_alignment(c.image_label,l,false);set_alignment(c.video_label,l,false);
    let headings=[l.t("File","فایل"),l.t("Status","وضعیت"),l.t("Output","خروجی"),l.t("Size","حجم")];
    for (index,heading) in headings.iter().enumerate() {
        let mut text=native_text(heading);let mut column:LVCOLUMNW=zeroed();
        column.mask=LVCF_TEXT|LVCF_FMT;column.pszText=text.as_mut_ptr();column.fmt=if l==Lang::Fa{LVCFMT_RIGHT as i32}else{LVCFMT_LEFT as i32};
        SendMessageW(c.list,LVM_SETCOLUMNW,index,&column as *const _ as isize);
    }
    let order=if l==Lang::En{[0i32,1,2,3]}else{[3i32,2,1,0]};
    SendMessageW(c.list,LVM_SETCOLUMNORDERARRAY,4,order.as_ptr() as isize);
    update_output_label(app);update_all_rows(app);update_details(app);refresh_controls(app);layout(app);
}
unsafe fn update_output_label(app:*mut App) {
    let text=(*app).output_dir.as_ref().map(|path|display_path(path)).unwrap_or_else(||(*app).lang.t("Converted folder beside each original","پوشهٔ Converted کنار هر فایل اصلی").into());
    caption((*app).controls.output,&text);
}
unsafe fn media_visibility(app:*mut App)->(bool,bool) {
    let empty=(*app).jobs.is_empty();
    (empty||(*app).jobs.iter().any(|job|job.kind==MediaKind::Image),empty||(*app).jobs.iter().any(|job|job.kind==MediaKind::Video))
}
unsafe fn layout(app:*mut App) {
    let c=(*app).controls;if c.list.is_null(){return;}
    let dpi=(*app).dpi.max(96);let l=(*app).lang;let mut rect:RECT=zeroed();GetClientRect((*app).hwnd,&mut rect);
    let w=(rect.right as i64*96/dpi as i64) as i32;let h=(rect.bottom as i64*96/dpi as i64) as i32;
    let place=|control:HWND,x:i32,y:i32,width:i32,height:i32|{
        let x=if l==Lang::Fa{w-x-width}else{x};
        MoveWindow(control,scale(dpi,x),scale(dpi,y),scale(dpi,width.max(1)),scale(dpi,height.max(1)),1);
    };
    place(c.about,w-88,22,70,30);place(c.language,w-174,22,76,30);
    place(c.add,w-152,97,116,36);
    place(c.remove,w-188,172,76,30);place(c.clear,w-104,172,86,30);
    let (images,videos)=media_visibility(app);
    ShowWindow(c.image_label,if images{SW_SHOW}else{SW_HIDE});ShowWindow(c.image_format,if images{SW_SHOW}else{SW_HIDE});
    ShowWindow(c.video_label,if videos{SW_SHOW}else{SW_HIDE});ShowWindow(c.video_format,if videos{SW_SHOW}else{SW_HIDE});
    let mut format_x=155;
    if images {place(c.image_label,format_x,179,59,19);place(c.image_format,format_x+64,171,83,160);format_x+=167;}
    if videos {place(c.video_label,format_x,179,59,19);place(c.video_format,format_x+64,171,88,160);}
    let list_height=(h-414).max(108);
    place(c.list,18,214,w-36,list_height);place(c.empty,40,214+list_height/2-2,w-80,28);
    place(c.detail,22,h-191,w-44,34);
    place(c.output,30,h-122,w-386,24);place(c.folder,w-346,h-132,80,32);place(c.reset,w-258,h-132,64,32);
    place(c.metadata,w-181,h-128,148,26);
    place(c.progress,18,h-79,w-36,5);
    place(c.start,w-164,h-63,146,35);place(c.cancel,w-259,h-63,85,35);place(c.open,18,h-63,138,35);
    let fixed=scale(dpi,145+179+92);let available=scale(dpi,w-36)-GetSystemMetricsForDpi(SM_CXVSCROLL,dpi)-scale(dpi,5);
    for (index,width) in [available-fixed,scale(dpi,145),scale(dpi,179),scale(dpi,92)].into_iter().enumerate() {
        SendMessageW(c.list,LVM_SETCOLUMNWIDTH,index,width.max(scale(dpi,70)) as isize);
    }
    RedrawWindow((*app).hwnd,null(),null_mut(),RDW_INVALIDATE|RDW_ERASE|RDW_ALLCHILDREN);
}
unsafe fn draw_text(dc:HDC,font:HFONT,text:&str,rect:RECT,color:u32,flags:u32) {
    let old=SelectObject(dc,font);let text=native_text(text);let mut rect=rect;
    SetBkMode(dc,TRANSPARENT as i32);SetTextColor(dc,color);
    DrawTextW(dc,text.as_ptr(),(text.len()-1) as i32,&mut rect,flags|DT_NOPREFIX|DT_END_ELLIPSIS);
    SelectObject(dc,old);
}
unsafe fn panel(dc:HDC,rect:RECT,fill:u32,border:u32,radius:i32) {
    let brush=CreateSolidBrush(fill);let pen=CreatePen(PS_SOLID,1,border);let ob=SelectObject(dc,brush);let op=SelectObject(dc,pen);
    RoundRect(dc,rect.left,rect.top,rect.right,rect.bottom,radius,radius);SelectObject(dc,op);SelectObject(dc,ob);DeleteObject(pen);DeleteObject(brush);
}
unsafe fn banner_text(app:*mut App)->(String,u32) {
    let l=(*app).lang;
    match &(*app).banner {
        Banner::Ready=>(l.t("Fully offline. Your originals stay untouched.","کاملاً آفلاین؛ فایل‌های اصلی حفظ می‌شوند.").into(),MUTED),
        Banner::Added(r)=>{
            let mut text=if l==Lang::En{format!("{} added",r.added)}else{format!("{} فایل اضافه شد",l.number(r.added))};
            if r.duplicates>0 {text.push_str(&if l==Lang::En{format!(" · {} already queued",r.duplicates)}else{format!(" · {} تکراری",l.number(r.duplicates))});}
            if r.unsupported+r.unavailable>0 {text.push_str(&if l==Lang::En{format!(" · {} skipped",r.unsupported+r.unavailable)}else{format!(" · {} مورد اضافه نشد",l.number(r.unsupported+r.unavailable))});}
            (text,if r.unsupported+r.unavailable>0{WARNING}else{MUTED})
        }
        Banner::Running=>{
            (if l==Lang::En{format!("{} / {} files · {} outputs · {} failed",(*app).completed,(*app).total,(*app).output_count,(*app).failures)}
                else{format!("{} از {} فایل · {} خروجی · {} خطا",l.number((*app).completed),l.number((*app).total),l.number((*app).output_count),l.number((*app).failures))},ACCENT)
        }
        Banner::Stopping=>(l.t("Canceling… completed outputs are kept.","در حال توقف… خروجی‌های کامل‌شده حفظ می‌شوند.").into(),WARNING),
        Banner::Finished{canceled,unexpected}=>{
            let heading=if *unexpected{l.t("Stopped","متوقف شد")}else if *canceled{l.t("Canceled","متوقف شد")}else{l.t("Complete","تمام شد")};
            (if l==Lang::En{format!("{heading} · {} completed · {} outputs · {} failed",(*app).successes,(*app).output_count,(*app).failures)}
                else{format!("{heading} · {} موفق · {} خروجی · {} خطا",l.number((*app).successes),l.number((*app).output_count),l.number((*app).failures))},
                if *unexpected||(*app).failures>0{WARNING}else{ACCENT})
        }
        Banner::Notice(en,fa,color)=>(l.t(en,fa).into(),*color),
    }
}
unsafe fn paint(app:*mut App) {
    let mut ps:PAINTSTRUCT=zeroed();let dc=BeginPaint((*app).hwnd,&mut ps);let mut client:RECT=zeroed();GetClientRect((*app).hwnd,&mut client);FillRect(dc,&client,(*app).background);
    let dpi=(*app).dpi;let l=(*app).lang;let f=(*app).fonts;
    let w=(client.right as i64*96/dpi as i64) as i32;let h=(client.bottom as i64*96/dpi as i64) as i32;
    let r=|x:i32,y:i32,width:i32,height:i32|{let x=if l==Lang::Fa{w-x-width}else{x};RECT{left:scale(dpi,x),top:scale(dpi,y),right:scale(dpi,x+width),bottom:scale(dpi,y+height)}};
    draw_text(dc,f.title,l.t("Roshan","روشن"),r(18,13,150,33),INK,l.flags()|DT_SINGLELINE);
    draw_text(dc,f.small,l.t("Photo & video converter","تبدیل عکس و ویدئو"),r(19,46,240,19),MUTED,l.flags()|DT_SINGLELINE);
    panel(dc,r(w-253,24,66,26),TINT,TINT,scale(dpi,13));
    draw_text(dc,f.small,l.t("Offline","آفلاین"),r(w-253,24,66,26),ACCENT,DT_CENTER|DT_SINGLELINE|DT_VCENTER);
    panel(dc,r(18,72,w-36,86),WHITE,BORDER,scale(dpi,14));
    draw_text(dc,f.drop,l.t("Drop photos or videos","عکس یا ویدئو را اینجا رها کنید"),r(34,88,w-218,29),INK,l.flags()|DT_SINGLELINE|DT_VCENTER);
    draw_text(dc,f.small,"HEIC · PNG · JPG · WebP   /   MP4 · MOV · AVI · WebM",r(34,121,w-218,18),MUTED,l.flags()|DT_SINGLELINE);
    let queue=if l==Lang::En{format!("Queue · {}",(*app).jobs.len())}else{format!("صف · {}",l.number((*app).jobs.len()))};
    draw_text(dc,f.section,&queue,r(18,174,125,26),INK,l.flags()|DT_SINGLELINE|DT_VCENTER);
    panel(dc,r(18,h-151,w-36,62),WHITE,BORDER,scale(dpi,12));
    draw_text(dc,f.small,l.t("Destination","محل ذخیره"),r(30,h-145,w-386,18),MUTED,l.flags()|DT_SINGLELINE);
    let (banner,color)=banner_text(app);
    draw_text(dc,f.small,&banner,r(18,h-23,w-36,19),color,l.flags()|DT_SINGLELINE|DT_VCENTER);
    EndPaint((*app).hwnd,&ps);
}
unsafe fn draw_button(app:*mut App,item:&DRAWITEMSTRUCT) {
    if item.CtlType!=ODT_BUTTON{return;}
    let disabled=item.itemState&ODS_DISABLED!=0;let pressed=item.itemState&ODS_SELECTED!=0;
    let primary=item.CtlID==ID_START as u32||item.CtlID==ID_ADD as u32;
    let (fill,border,ink)=if disabled{(rgb(237,241,243),rgb(237,241,243),rgb(161,172,179))}
        else if primary{(if pressed{rgb(0,86,82)}else{ACCENT},ACCENT,WHITE)}
        else{(if pressed{TINT}else{WHITE},BORDER,INK)};
    let card=matches!(item.CtlID as i32,ID_ADD|ID_FOLDER|ID_RESET);
    FillRect(item.hDC,&item.rcItem,if card{(*app).white}else{(*app).background});
    panel(item.hDC,item.rcItem,fill,border,scale((*app).dpi,10));
    let len=GetWindowTextLengthW(item.hwndItem);let mut buf=vec![0u16;len as usize+1];GetWindowTextW(item.hwndItem,buf.as_mut_ptr(),buf.len() as i32);
    let text=String::from_utf16_lossy(&buf[..len as usize]);
    let compact=(*app).lang==Lang::Fa&&matches!(item.CtlID as i32,ID_FOLDER|ID_RESET|ID_CLEAR);
    draw_text(item.hDC,if compact{(*app).fonts.small}else{(*app).fonts.body},&text,item.rcItem,ink,DT_CENTER|DT_SINGLELINE|DT_VCENTER);
    if item.itemState&ODS_FOCUS!=0&&!disabled {let mut focus=item.rcItem;let margin=scale((*app).dpi,4);InflateRect(&mut focus,-margin,-margin);DrawFocusRect(item.hDC,&focus);}
}

unsafe fn command(app:*mut App,id:i32) {
    match id {
        ID_ADD if !(*app).busy=>pick_files(app),
        ID_REMOVE if !(*app).busy=>remove_selected(app),
        ID_CLEAR if !(*app).busy=>{(*app).jobs.clear();(*app).banner=Banner::Ready;rebuild_rows(app);update_details(app);refresh_controls(app);layout(app);}
        ID_FOLDER if !(*app).busy=>pick_folder(app),
        ID_RESET if !(*app).busy=>{if (*app).output_dir.take().is_some(){settings_changed(app,None);}update_output_label(app);refresh_controls(app);}
        ID_META if !(*app).busy=>settings_changed(app,None),
        ID_START if !(*app).busy=>start_conversion(app), ID_CANCEL if (*app).busy=>request_stop(app),
        ID_OPEN=>open_output(app), ID_ABOUT=>show_about(app),
        ID_LANGUAGE=>{(*app).lang=if (*app).lang==Lang::En{Lang::Fa}else{Lang::En};set_fonts(app);translate_ui(app);}
        _=>{}
    }
}
unsafe fn modal(app:*mut App,title:&str,instruction:&str,content:&str,buttons:&[(i32,&str)],default:i32,expanded:Option<&str>)->i32 {
    if (*app).dialog_open{return IDCANCEL;}
    (*app).dialog_open=true;
    let title=native_text(title);let instruction=native_text(instruction);let content=native_text(content);
    let button_texts:Vec<Vec<u16>>=buttons.iter().map(|(_,text)|native_text(text)).collect();
    let native_buttons:Vec<TASKDIALOG_BUTTON>=buttons.iter().enumerate().map(|(i,(id,_))|TASKDIALOG_BUTTON{nButtonID:*id,pszButtonText:button_texts[i].as_ptr()}).collect();
    let expanded=expanded.map(native_text);let more=native_text((*app).lang.t("Format notes & shortcuts","نکته‌های فرمت و میان‌برها"));
    let less=native_text((*app).lang.t("Hide details","بستن جزئیات"));
    let mut config:TASKDIALOGCONFIG=zeroed();config.cbSize=size_of::<TASKDIALOGCONFIG>() as u32;config.hwndParent=(*app).hwnd;config.hInstance=(*app).instance;
    config.dwFlags=TDF_POSITION_RELATIVE_TO_WINDOW|TDF_ALLOW_DIALOG_CANCELLATION|if (*app).lang==Lang::Fa{TDF_RTL_LAYOUT}else{0};
    config.pszWindowTitle=title.as_ptr();config.pszMainInstruction=instruction.as_ptr();config.pszContent=content.as_ptr();
    config.cButtons=native_buttons.len() as u32;config.pButtons=native_buttons.as_ptr();config.nDefaultButton=default;config.cxWidth=310;
    if let Some(text)=expanded.as_ref(){config.pszExpandedInformation=text.as_ptr();config.pszExpandedControlText=less.as_ptr();config.pszCollapsedControlText=more.as_ptr();}
    let mut chosen=IDCANCEL;let result=TaskDialogIndirect(&config,&mut chosen,null_mut(),null_mut());
    if result<0 {
        let flags=if (*app).lang==Lang::Fa{MB_RTLREADING|MB_RIGHT}else{0};
        MessageBoxW((*app).hwnd,content.as_ptr(),title.as_ptr(),MB_OK|MB_ICONINFORMATION|flags);chosen=default;
        // A fallback message never starts a conversion or opens the website.
        if chosen==MODAL_CONVERT||chosen==MODAL_WEBSITE{chosen=MODAL_REVIEW;}
    }
    (*app).dialog_open=false;chosen
}
unsafe fn show_about(app:*mut App) {
    let l=(*app).lang;
    let content=format!("{}\n\n{}\n\npeymaanabedinpour.ir\n\n{}",
        l.t("A compact, offline image and video converter.","تبدیل‌کنندهٔ جمع‌وجور و آفلاین عکس و ویدئو."),CREDIT,
        l.t("Built with Rust · version 2.0.0\nYour files are processed on this device.","ساخته‌شده با Rust · نسخهٔ ۲.۰.۰\nفایل‌ها روی همین دستگاه پردازش می‌شوند."));
    let extra=l.t(
        "Images: HEIC / HEIF, PNG, JPG / JPEG, WebP\nVideos: MP4, MOV, AVI, WebM\n\nImage and video outputs are selected independently, so one queue can contain both. Original image dimensions are kept. Conversion uses high-quality settings; JPEG and video re-encoding are not lossless. Some formats cannot retain HDR, transparency, animation, or every metadata tag.\n\nKeep metadata may preserve location / GPS information. Turn it off when you do not want that information carried into supported outputs.\n\nOutputs are saved in a Converted folder beside each original, or the folder you choose. Existing files are never overwritten.\n\nCtrl+O: add files · Ctrl+Enter: convert · Delete: remove selected rows · Esc: cancel · F1: about\n\nOpen-source notices and corresponding source are provided with the source package. Application: GPL-3.0-or-later.",
        "عکس: HEIC / HEIF، PNG، JPG / JPEG، WebP\nویدئو: MP4، MOV، AVI، WebM\n\nفرمت خروجی عکس و ویدئو جدا انتخاب می‌شود و یک صف می‌تواند هر دو را داشته باشد. ابعاد اصلی عکس حفظ می‌شود. تبدیل با تنظیمات باکیفیت انجام می‌شود؛ JPEG و بازکدگذاری ویدئو بدون اتلاف نیستند. بعضی فرمت‌ها HDR، شفافیت، حرکت یا تمام اطلاعات جانبی را نگه نمی‌دارند.\n\nگزینهٔ حفظ اطلاعات ممکن است موقعیت مکانی و GPS را هم نگه دارد. اگر انتقال این اطلاعات را نمی‌خواهید، آن را خاموش کنید.\n\nخروجی‌ها در پوشهٔ Converted کنار فایل اصلی یا پوشهٔ انتخابی شما ذخیره می‌شوند. فایل موجود بازنویسی نمی‌شود.\n\nCtrl+O: انتخاب فایل · Ctrl+Enter: تبدیل · Delete: حذف از صف · Esc: توقف · F1: درباره\n\nکد منبع و مجوزهای اجزای متن‌باز در بستهٔ کد منبع ارائه شده‌اند. مجوز برنامه: GPL-3.0-or-later."
    );
    let chosen=modal(app,l.t("About Roshan","دربارهٔ روشن"),"Roshan Converter",&content,
        &[(MODAL_CLOSE,l.t("Close","بستن")),(MODAL_WEBSITE,l.t("Visit website","باز کردن وب‌سایت"))],MODAL_CLOSE,Some(extra));
    if chosen==MODAL_WEBSITE {let url=wide(WEBSITE);let verb=wide("open");ShellExecuteW((*app).hwnd,verb.as_ptr(),url.as_ptr(),null(),null(),SW_SHOWNORMAL);}
}
unsafe fn show_drop_report(app:*mut App,r:&AddReport) {
    let l=(*app).lang;
    let heading=if l==Lang::En{format!("{} file{} added",r.added,if r.added==1{""}else{"s"})}else{format!("{} فایل اضافه شد",l.number(r.added))};
    let mut lines=Vec::new();
    if r.added>0 {
        if r.images>0 {lines.push(if l==Lang::En{format!("{} image{} → {}",r.images,if r.images==1{""}else{"s"},(*app).image_format.label())}
            else{format!("{} عکس ← {}",l.number(r.images),(*app).image_format.label())});}
        if r.videos>0 {lines.push(if l==Lang::En{format!("{} video{} → {}",r.videos,if r.videos==1{""}else{"s"},(*app).video_format.label())}
            else{format!("{} ویدئو ← {}",l.number(r.videos),(*app).video_format.label())});}
    }
    if r.duplicates>0 {lines.push(if l==Lang::En{format!("{} already in the queue",r.duplicates)}else{format!("{} فایل از قبل در صف بود",l.number(r.duplicates))});}
    if r.unsupported>0 {lines.push(if l==Lang::En{format!("{} unsupported file{} skipped",r.unsupported,if r.unsupported==1{""}else{"s"})}else{format!("{} فرمت پشتیبانی‌نشده اضافه نشد",l.number(r.unsupported))});}
    if r.unavailable>0 {lines.push(if l==Lang::En{format!("{} unavailable file{} skipped",r.unavailable,if r.unavailable==1{""}else{"s"})}else{format!("{} فایل در دسترس نبود",l.number(r.unavailable))});}
    lines.push(String::new());lines.push(l.t("Save to:","محل ذخیره:").into());
    lines.push((*app).output_dir.as_ref().map(|p|display_path(p)).unwrap_or_else(||l.t("Converted folder beside each original","پوشهٔ Converted کنار هر فایل اصلی").into()));
    lines.push(String::new());lines.push(l.t("Review the queue to change output formats or the destination.","برای تغییر فرمت خروجی یا محل ذخیره، صف را بررسی کنید.").into());
    let mut buttons=vec![(MODAL_REVIEW,l.t("Review queue","بررسی صف"))];
    if r.added>0 {buttons.push((MODAL_CONVERT,l.t("Convert now","تبدیل اکنون")));}
    let chosen=modal(app,l.t("Files added","افزودن فایل‌ها"),&heading,&lines.join("\n"),&buttons,MODAL_REVIEW,None);
    if chosen==MODAL_CONVERT&&!(*app).busy {start_conversion(app);}
}

unsafe fn pick_files(app:*mut App) {
    if (*app).dialog_open||(*app).busy{return;}
    (*app).dialog_open=true;let l=(*app).lang;let mut buffer=vec![0u16;1_048_576];
    let filter=if l==Lang::En {
        "Supported media\0*.heic;*.heif;*.png;*.jpg;*.jpeg;*.webp;*.mp4;*.mov;*.avi;*.webm\0Images\0*.heic;*.heif;*.png;*.jpg;*.jpeg;*.webp\0Videos\0*.mp4;*.mov;*.avi;*.webm\0All files\0*.*\0\0"
    } else {
        "عکس و ویدئوی پشتیبانی‌شده\0*.heic;*.heif;*.png;*.jpg;*.jpeg;*.webp;*.mp4;*.mov;*.avi;*.webm\0عکس‌ها\0*.heic;*.heif;*.png;*.jpg;*.jpeg;*.webp\0ویدئوها\0*.mp4;*.mov;*.avi;*.webm\0همهٔ فایل‌ها\0*.*\0\0"
    }.encode_utf16().collect::<Vec<_>>();
    let title=native_text(l.t("Choose photos or videos","عکس یا ویدئو را انتخاب کنید"));
    let mut dialog:OPENFILENAMEW=zeroed();dialog.lStructSize=size_of::<OPENFILENAMEW>() as u32;dialog.hwndOwner=(*app).hwnd;
    dialog.lpstrFilter=filter.as_ptr();dialog.nFilterIndex=1;dialog.lpstrFile=buffer.as_mut_ptr();dialog.nMaxFile=buffer.len() as u32;dialog.lpstrTitle=title.as_ptr();
    dialog.Flags=OFN_EXPLORER|OFN_ALLOWMULTISELECT|OFN_FILEMUSTEXIST|OFN_PATHMUSTEXIST|OFN_HIDEREADONLY|OFN_NOCHANGEDIR|OFN_DONTADDTORECENT;
    let accepted=GetOpenFileNameW(&mut dialog)!=0;let error=if accepted{0}else{CommDlgExtendedError()};(*app).dialog_open=false;
    if !accepted {if error!=0 {notice(app,format!("File selection failed ({error}). Try a smaller selection or drag and drop."),format!("انتخاب فایل ناموفق بود ({error}). تعداد کمتری انتخاب کنید یا فایل‌ها را رها کنید."),ERROR);}return;}
    let mut parts=Vec::new();let mut start=0;
    while start<buffer.len()&&buffer[start]!=0 {let len=buffer[start..].iter().position(|&c|c==0).unwrap_or(buffer.len()-start);parts.push(OsString::from_wide(&buffer[start..start+len]));start+=len+1;}
    let paths=if parts.len()==1{vec![PathBuf::from(parts.remove(0))]}else if parts.len()>1{let dir=PathBuf::from(parts.remove(0));parts.into_iter().map(|name|dir.join(name)).collect()}else{Vec::new()};
    add_files(app,paths);
}
unsafe fn pick_folder(app:*mut App) {
    if (*app).dialog_open||(*app).busy{return;}
    (*app).dialog_open=true;let title=native_text((*app).lang.t("Choose one destination for every converted file.","یک پوشه برای ذخیرهٔ تمام فایل‌های تبدیل‌شده انتخاب کنید."));
    let mut display=vec![0u16;260];let mut dialog:BROWSEINFOW=zeroed();dialog.hwndOwner=(*app).hwnd;dialog.pszDisplayName=display.as_mut_ptr();dialog.lpszTitle=title.as_ptr();
    dialog.ulFlags=BIF_RETURNONLYFSDIRS|BIF_NEWDIALOGSTYLE|BIF_EDITBOX;let pidl=SHBrowseForFolderW(&dialog);(*app).dialog_open=false;if pidl.is_null(){return;}
    let mut buffer=vec![0u16;32768];let ok=SHGetPathFromIDListEx(pidl,buffer.as_mut_ptr(),buffer.len() as u32,0)!=0;CoTaskMemFree(pidl as *const c_void);
    if !ok {notice(app,"Cannot read that folder. Choose a local filesystem folder.".into(),"مسیر پوشه خوانده نشد. یک پوشهٔ عادی روی دستگاه انتخاب کنید.".into(),ERROR);return;}
    let len=buffer.iter().position(|&c|c==0).unwrap_or(buffer.len());let directory=PathBuf::from(OsString::from_wide(&buffer[..len]));
    if (*app).output_dir.as_ref()!=Some(&directory){(*app).output_dir=Some(directory);settings_changed(app,None);}update_output_label(app);refresh_controls(app);
}
unsafe fn settings_changed(app:*mut App,kind:Option<MediaKind>) {
    if (*app).busy{return;}
    // Changing a target reuses the queue. Published output files stay on disk;
    // only the affected rows' previous result records are reset.
    let mut count=0usize;
    for job in &mut (*app).jobs {
        if kind.is_some_and(|kind|job.kind!=kind){continue;}
        if job.status!=Status::Pending{count+=1;}
        job.status=Status::Pending;job.progress=0.;job.outputs.clear();job.error=None;
    }
    if count>0 {
        let l=(*app).lang;
        notice(app,format!("Settings changed · {count} files ready to convert again"),
            format!("تنظیمات تغییر کرد · {} فایل آمادهٔ تبدیل دوباره",l.number(count)),ACCENT);
        SendMessageW((*app).controls.progress,PBM_SETPOS,0,0);
    }
    update_all_rows(app);update_details(app);refresh_controls(app);
}
unsafe fn add_files(app:*mut App,paths:Vec<PathBuf>)->AddReport {
    let mut report=AddReport::default();if (*app).busy{return report;}
    let mut known:HashSet<String>=(*app).jobs.iter().map(|job|job.key.clone()).collect();
    for input in paths {
        let Some(kind)=media_kind(&input) else {report.unsupported+=1;continue;};
        if !input.is_file(){report.unavailable+=1;continue;}
        let canonical=std::fs::canonicalize(&input).unwrap_or_else(|_|input.clone());let key=canonical.to_string_lossy().to_lowercase();
        if !known.insert(key.clone()){report.duplicates+=1;continue;}
        if kind==MediaKind::Image{report.images+=1;}else{report.videos+=1;}
        (*app).jobs.push(Job{input,key,kind,status:Status::Pending,progress:0.,outputs:Vec::new(),error:None});report.added+=1;
    }
    (*app).banner=Banner::Added(report.clone());rebuild_rows(app);update_details(app);refresh_controls(app);layout(app);report
}

fn row_values(job:&Job,l:Lang,image_format:ImageFormat,video_format:VideoFormat)->[String;4] {
    let name=job.input.file_name().unwrap_or(job.input.as_os_str()).to_string_lossy().into_owned();
    let status=if job.status==Status::Working&&job.progress>0. {format!("{} {}%",l.t("Converting","تبدیل"),l.number((job.progress.clamp(0.,1.)*100.).round() as usize))}else{job.status.label(l).into()};
    let bytes=job.outputs.iter().fold(0u64,|sum,item|sum.saturating_add(item.bytes));
    let target=if let Some(first)=job.outputs.first(){first.path.extension().and_then(|x|x.to_str()).unwrap_or("").to_ascii_uppercase()}
        else if job.kind==MediaKind::Image{image_format.label().into()}else{video_format.label().into()};
    let output=if job.outputs.len()>1{format!("{} · {} {}",target,l.number(job.outputs.len()),l.t("files","فایل"))}
        else if let Some(first)=job.outputs.first().filter(|i|i.width>0&&i.height>0){format!("{} · {} × {}",target,first.width,first.height)}else{target};
    [name,status,output,size_text(bytes)]
}
unsafe fn row_strings(app:*mut App,index:usize)->Option<[String;4]> {
    (&(*app).jobs).get(index).map(|job|row_values(job,(*app).lang,(*app).image_format,(*app).video_format))
}
unsafe fn update_row(app:*mut App,index:usize) {
    let Some(values)=row_strings(app,index)else{return;};let list=(*app).controls.list;
    for (column,value) in values.iter().enumerate(){let mut text=native_text(value);let mut item:LVITEMW=zeroed();item.iSubItem=column as i32;item.pszText=text.as_mut_ptr();SendMessageW(list,LVM_SETITEMTEXTW,index,&item as *const _ as isize);}
}
unsafe fn update_all_rows(app:*mut App) {for index in 0..(*app).jobs.len(){update_row(app,index);}}
unsafe fn rebuild_rows(app:*mut App) {
    let list=(*app).controls.list;let rows:Vec<[String;4]>=(0..(*app).jobs.len()).filter_map(|i|row_strings(app,i)).collect();
    SendMessageW(list,WM_SETREDRAW,0,0);SendMessageW(list,LVM_DELETEALLITEMS,0,0);
    for (index,values) in rows.iter().enumerate(){let mut text=native_text(&values[0]);let mut item:LVITEMW=zeroed();item.mask=LVIF_TEXT;item.iItem=index as i32;item.pszText=text.as_mut_ptr();SendMessageW(list,LVM_INSERTITEMW,0,&item as *const _ as isize);
        for (column,value) in values.iter().enumerate().skip(1){let mut text=native_text(value);item.iSubItem=column as i32;item.pszText=text.as_mut_ptr();SendMessageW(list,LVM_SETITEMTEXTW,index,&item as *const _ as isize);}}
    SendMessageW(list,WM_SETREDRAW,1,0);InvalidateRect(list,null(),1);ShowWindow((*app).controls.empty,if rows.is_empty(){SW_SHOW}else{SW_HIDE});
}
unsafe fn selected_indices(list:HWND)->Vec<usize> {
    if list.is_null() { return Vec::new(); }
    let mut indices=Vec::new();let mut previous:isize=-1;
    loop{previous=SendMessageW(list,LVM_GETNEXTITEM,previous as usize,LVNI_SELECTED as isize);if previous<0{break;}indices.push(previous as usize);}indices
}
unsafe fn remove_selected(app:*mut App) {
    let indices=selected_indices((*app).controls.list);if indices.is_empty(){return;}
    for index in indices.into_iter().rev(){if index<(*app).jobs.len(){(*app).jobs.remove(index);}}
    (*app).banner=Banner::Ready;rebuild_rows(app);update_details(app);refresh_controls(app);layout(app);
}
fn job_details(job:&Job,l:Lang)->String {
    let mut lines=vec![format!("{}",job.status.label(l)),display_path(&job.input)];
    if let Some(error)=job.error.as_ref(){lines.push(format!("{} {}",l.t("Error:","خطا:"),localized_issue(l,error)));}
    for output in &job.outputs {
        lines.push(l.t("Output:","خروجی:").into());lines.push(display_path(&output.path));
        for warning in &output.warnings{lines.push(format!("{} {}",l.t("Note:","توضیح:"),localized_issue(l,warning)));}
    }
    if job.status==Status::Pending{lines.push(l.t("Choose an output format, then convert.","فرمت خروجی را انتخاب کنید و تبدیل را بزنید.").into());}
    else if job.status==Status::Canceled{lines.push(l.t("Choose Convert to retry this file.","برای ادامه، تبدیل را بزنید.").into());}
    lines.join("\r\n")
}
unsafe fn update_details(app:*mut App) {
    let selected=selected_indices((*app).controls.list);
    let text=selected.first().and_then(|&i|(&(*app).jobs).get(i)).map(|job|job_details(job,(*app).lang))
        .unwrap_or_else(||(*app).lang.t("Select a file for details. Double-click to expand.","برای جزئیات یک فایل را انتخاب کنید؛ با دوبار کلیک، پنجرهٔ کامل باز می‌شود.").into());
    let detail=(*app).controls.detail;caption(detail,&text);ShowScrollBar(detail,SB_VERT,(text.lines().count()>2) as i32);refresh_controls(app);
}
unsafe fn notify_control(app:*mut App,lparam:LPARAM)->LRESULT {
    if lparam==0{return 0;}let header=&*(lparam as *const NMHDR);if header.idFrom!=ID_LIST as usize{return 0;}
    if header.code==LVN_ITEMCHANGED {update_details(app);}
    else if header.code==NM_DBLCLK {
        let selected=selected_indices((*app).controls.list);let text=selected.first().and_then(|&i|(&(*app).jobs).get(i)).map(|job|job_details(job,(*app).lang));
        if let Some(text)=text{let l=(*app).lang;modal(app,l.t("File details","جزئیات فایل"),l.t("Conversion details","جزئیات تبدیل"),&text,&[(MODAL_CLOSE,l.t("Close","بستن"))],MODAL_CLOSE,None);}
    } else if header.code==NM_CUSTOMDRAW {
        let draw=&mut *(lparam as *mut NMLVCUSTOMDRAW);
        if draw.nmcd.dwDrawStage==CDDS_PREPAINT{return CDRF_NOTIFYITEMDRAW as isize;}
        if draw.nmcd.dwDrawStage==CDDS_ITEMPREPAINT{return CDRF_NOTIFYSUBITEMDRAW as isize;}
        if draw.nmcd.dwDrawStage==(CDDS_ITEMPREPAINT|CDDS_SUBITEM) {
            let status=(&(*app).jobs).get(draw.nmcd.dwItemSpec).map(|job|job.status);
            draw.clrTextBk=if draw.nmcd.dwItemSpec%2==0{WHITE}else{rgb(250,252,252)};
            draw.clrText=if draw.iSubItem==1{match status{Some(Status::Done)|Some(Status::Working)=>ACCENT,Some(Status::Warning)=>WARNING,Some(Status::Failed)=>ERROR,_=>MUTED}}else{INK};
            return CDRF_DODEFAULT as isize;
        }
    }0
}
unsafe fn refresh_controls(app:*mut App) {
    let c=(*app).controls;let busy=(*app).busy;let any=!(*app).jobs.is_empty();let ready=(*app).jobs.iter().any(|job|job.status.ready());
    let selected=!selected_indices(c.list).is_empty();let chosen=(*app).output_dir.is_some();let open=(*app).last_output_dir.is_some()||(*app).output_dir.as_ref().is_some_and(|p|p.is_dir());
    let stopping=(*app).cancel.load(Ordering::Relaxed);let l=(*app).lang;
    for (control,enabled) in [(c.add,!busy),(c.remove,!busy&&selected),(c.clear,!busy&&any),(c.folder,!busy),(c.reset,!busy&&chosen),
        (c.metadata,!busy),(c.image_format,!busy),(c.video_format,!busy),(c.start,!busy&&ready),(c.cancel,busy&&!stopping),(c.open,open)]{EnableWindow(control,enabled as i32);}
    caption(c.start,if busy{l.t("Converting…","در حال تبدیل…")}else{l.t("Convert","تبدیل")});
    caption(c.cancel,if busy&&stopping{l.t("Stopping…","توقف…")}else{l.t("Cancel","توقف")});invalidate(app);
}

unsafe fn start_conversion(app:*mut App) {
    let tasks:Vec<(usize,PathBuf)>=(*app).jobs.iter().enumerate().filter(|(_,job)|job.status.ready()).map(|(index,job)|(index,job.input.clone())).collect();
    if tasks.is_empty(){return;}
    let preserve_metadata=SendMessageW((*app).controls.metadata,BM_GETCHECK,0,0)==BST_CHECKED as isize;
    let settings=ConversionSettings{output_dir:(*app).output_dir.clone(),preserve_metadata,image_format:(*app).image_format,video_format:(*app).video_format};
    (*app).cancel.store(false,Ordering::Relaxed);let cancel=(*app).cancel.clone();let (sender,receiver)=mpsc::channel();
    (*app).total=tasks.len();(*app).completed=0;(*app).successes=0;(*app).failures=0;(*app).output_count=0;
    (*app).busy=true;(*app).receiver=Some(receiver);(*app).banner=Banner::Running;
    for (index,_) in &tasks {if let Some(job)=(&mut (*app).jobs).get_mut(*index){job.status=Status::Pending;job.progress=0.;job.error=None;}update_row(app,*index);}
    SendMessageW((*app).controls.progress,PBM_SETPOS,0,0);refresh_controls(app);
    match thread::Builder::new().name("roshan-media-converter".into()).spawn(move||{
        for (index,input) in tasks {
            if cancel.load(Ordering::Relaxed){break;}
            if sender.send(WorkerEvent::Started(index)).is_err(){return;}
            let mut last=Instant::now()-Duration::from_millis(150);let mut last_percent=-1i32;
            let result=std::panic::catch_unwind(std::panic::AssertUnwindSafe(||convert_file_with_progress(&input,&settings,&cancel,|value|{
                if !value.is_finite(){return;}let value=value.clamp(0.,1.);let percent=(value*100.) as i32;
                if percent!=last_percent&&(value>=1.||last.elapsed()>=Duration::from_millis(120)){
                    let _=sender.send(WorkerEvent::Progress(index,value));last=Instant::now();last_percent=percent;
                }
            }))).unwrap_or_else(|_|Err("An internal conversion error occurred. The original file is unchanged.".into()));
            if sender.send(WorkerEvent::Converted(index,result)).is_err(){return;}
        }
        let _=sender.send(WorkerEvent::Finished);
    }) {
        Ok(worker)=>(*app).worker=Some(worker),
        Err(error)=>{(*app).busy=false;(*app).receiver=None;notice(app,format!("Cannot start conversion: {error}"),format!("شروع تبدیل ممکن نشد: {error}"),ERROR);refresh_controls(app);}
    }
}
unsafe fn request_stop(app:*mut App) {
    if !(*app).busy{return;}(*app).cancel.store(true,Ordering::Relaxed);(*app).banner=Banner::Stopping;refresh_controls(app);
}
unsafe fn set_progress(app:*mut App,fraction:f32) {
    let overall=if (*app).total==0{0.}else{((*app).completed as f32+fraction)/(*app).total as f32};
    SendMessageW((*app).controls.progress,PBM_SETPOS,(overall.clamp(0.,1.)*1000.).round() as usize,0);
}
unsafe fn poll_worker(app:*mut App) {
    let mut events=Vec::new();let mut disconnected=false;
    if let Some(receiver)=(*app).receiver.as_ref(){loop{match receiver.try_recv(){Ok(event)=>events.push(event),Err(mpsc::TryRecvError::Empty)=>break,Err(mpsc::TryRecvError::Disconnected)=>{disconnected=true;break;}}}}
    let mut finished=false;
    for event in events {match event {
        WorkerEvent::Started(index)=>{if let Some(job)=(&mut (*app).jobs).get_mut(index){job.status=Status::Working;job.progress=0.;}update_row(app,index);SendMessageW((*app).controls.list,LVM_ENSUREVISIBLE,index,0);update_details(app);}
        WorkerEvent::Progress(index,value)=>{if let Some(job)=(&mut (*app).jobs).get_mut(index){if job.status==Status::Working{job.progress=value;}}update_row(app,index);set_progress(app,value);}
        WorkerEvent::Converted(index,result)=>{accept_result(app,index,result);update_row(app,index);set_progress(app,0.);update_details(app);if !(*app).cancel.load(Ordering::Relaxed){(*app).banner=Banner::Running;invalidate(app);}}
        WorkerEvent::Finished=>finished=true,
    }}
    if finished||(disconnected&&(*app).busy){finish_conversion(app,!finished);}
}
unsafe fn accept_result(app:*mut App,index:usize,result:Result<Vec<ConvertedImage>,String>) {
    let result=result.and_then(|outputs|if outputs.is_empty(){Err("No images were produced".into())}else{Ok(outputs)});
    match result {
        Ok(outputs)=>{
            (*app).completed+=1;(*app).successes+=1;(*app).output_count+=outputs.len();
            if let Some(folder)=outputs.first().and_then(|output|output.path.parent()){(*app).last_output_dir=Some(folder.to_path_buf());}
            let warning=outputs.iter().any(|output|!output.warnings.is_empty());
            if let Some(job)=(&mut (*app).jobs).get_mut(index){job.status=if warning{Status::Warning}else{Status::Done};job.progress=1.;job.outputs=outputs;job.error=None;}
        }
        Err(error)=>{
            let canceled=(*app).cancel.load(Ordering::Relaxed)||error.eq_ignore_ascii_case("Conversion canceled");
            if !canceled{(*app).completed+=1;(*app).failures+=1;}
            if let Some(job)=(&mut (*app).jobs).get_mut(index){job.status=if canceled{Status::Canceled}else{Status::Failed};job.error=if canceled{None}else{Some(error)};}
        }
    }
}
unsafe fn finish_conversion(app:*mut App,unexpected:bool) {
    (*app).busy=false;(*app).receiver=None;if let Some(worker)=(*app).worker.take(){let _=worker.join();}
    let canceled=(*app).cancel.load(Ordering::Relaxed);
    let mut changed=Vec::new();for (index,job) in (*app).jobs.iter_mut().enumerate(){if job.status==Status::Pending||job.status==Status::Working{job.status=Status::Canceled;changed.push(index);}}
    for index in changed{update_row(app,index);}
    (*app).banner=Banner::Finished{canceled,unexpected};update_details(app);refresh_controls(app);if (*app).closing{DestroyWindow((*app).hwnd);}
}
unsafe fn open_output(app:*mut App) {
    let selected=selected_indices((*app).controls.list);
    let selected_folder=selected.first().and_then(|&i|(&(*app).jobs).get(i)).and_then(|job|job.outputs.first()).and_then(|output|output.path.parent()).map(Path::to_path_buf);
    let folder=selected_folder.or_else(||(*app).last_output_dir.clone()).or_else(||(*app).output_dir.clone());let Some(folder)=folder else{return;};
    let path=path_wide(&folder);let verb=wide("open");let result=ShellExecuteW((*app).hwnd,verb.as_ptr(),path.as_ptr(),null(),null(),SW_SHOWNORMAL);
    if result as isize<=32{notice(app,format!("Cannot open output folder: {}",folder.display()),format!("باز کردن پوشهٔ خروجی ممکن نشد: {}",folder.display()),ERROR);}
}
