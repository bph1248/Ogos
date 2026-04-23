use ogos_common::*;
use ogos_config as config;
use config::*;
use ogos_core::*;
use ogos_discord as discord;
use ogos_err::*;
use ogos_video as video;

use concat_string::*;
use crossbeam::sync::*;
use discord_rich_presence::*;
use eframe::{
    egui,
    egui_wgpu,
    wgpu
};
use indexmap::*;
use log::*;
use raw_window_handle::*;
use rayon::*;
use serde::*;
use std::{
    collections::*,
    f64::consts::PI,
    ffi::*,
    fmt::Write,
    fs::{self, *},
    io::Read,
    ops::*,
    path::*,
    process::*,
    rc::*,
    sync::*,
    thread,
    time::*
};
use tap::TapOptional;
use tokio::sync::oneshot::{self, error::*};
use range_compare::*;
use windows::{
    core::PWSTR,
    Win32::{
        Foundation::*,
        Graphics::Gdi::*,
        UI::{
            Shell::*,
            WindowsAndMessaging::*
        }
    }
};

const ASPECT_RATIO_3_2: f32 = 1.5;
const CELL_STROKE: egui::Stroke = egui::Stroke { width: 3.0, color: egui::Color32::from_rgb(250, 246, 235) };
const CHUNK_BYTE_COUNT: u64 = 500 * 1024;
const GRID_IMAGE_SPACING: egui::Vec2 = egui::vec2(30.0, 30.0);
const DETAILS_ENTRY_COUNT: usize = 64;
const FRAME_MARGIN: f32 = 15.0;
const IMAGE_EXTS: &[&str] = &["jpg", "jpeg", "png", "webp"];
const SEPARATOR_WIDTH: f32 = 2.0;

type AspectRatioV = f32;
type Residence = Range<usize>;

#[derive(Default, Serialize, Deserialize)]
struct Cache {
    #[serde(default)]
    grid_cell_size: egui::Vec2,
    #[serde(default)]
    details_cell_size: egui::Vec2,
    #[serde(default)]
    images: IndexSet<Rc<str>>,
    #[serde(default)]
    tags: Vec<Rc<str>>,
    entries: HashMap<PathBuf, CacheEntryInfo>
}

#[derive(Serialize, Deserialize)]
struct CacheEntryInfo {
    #[serde(rename = "image")]
    image_file_name_i: Option<usize>,
    sort_name: Option<Rc<str>>,
    hash: Option<Arc<str>>,
    #[serde(default)]
    tags: Vec<usize>
}

#[derive(Clone)]
struct DirEntryInfo {
    path: PathBuf,
    stem: String,
    file_kind: FileKind
}

struct FerryImageInfo {
    image_file_name: Arc<str>,
    expected_hash: Option<Arc<str>>,
    grid_entry_i: usize,
    grid_image_state_sx: oneshot::Sender<Result<(egui::ColorImage, Orientation), ()>>,
    details_image_state_sx: oneshot::Sender<Result<(egui::ColorImage, Orientation), ()>>,
    wait_ready: Option<WaitGroup>,
    poll_ready: Option<Arc<()>>
}

struct FerryImagesInfo<'a> {
    ctx: &'a egui::Context,
    thread_pool: &'a Arc<ThreadPool>,
    hash_sx: &'a mpsc::Sender<HashInfo>,
    image_dirs: &'static ImageDirs,
    base_image_kind: BaseImageKind,
    grid_cell_size: egui::Vec2,
    force_resize_grid_images: bool,
    details_cell_size: egui::Vec2,
    force_resize_details_images: bool,
    ferry_image_infos: Vec<FerryImageInfo>
}

struct GridEntryInfo {
    path: PathBuf,
    stem: Rc<str>,
    sort_name: Option<Rc<str>>,
    file_kind: FileKind,
    image_file_name_i: Option<usize>,
    hash: Option<Arc<str>>
}

struct HashInfo {
    grid_entry_i: usize,
    hash: Arc<str>
}

struct ImageDirs {
    base: PathBuf,
    grid: PathBuf,
    details: PathBuf
}

#[derive(Default)]
struct ImageStates {
    grid: ImageState,
    details: ImageState,
    ref_count: usize
}
impl ImageStates {
    fn is_none(&self) -> bool {
        matches!((&self.grid, &self.details), (ImageState::None, ImageState::None))
    }
}

struct QueueImageInfo {
    src_path: PathBuf,
    image_state_sx: oneshot::Sender<Result<(egui::ColorImage, Orientation), ()>>,
    resize: Option<ResizeImage>
}

#[derive(Default)]
struct Stream {
    load_first: HashSet<usize>,
    load_after: HashSet<usize>,
    drop: HashSet<usize>
}
impl Stream {
    fn with_flatten_load(mut self, residence: Range<usize>, visible: Range<usize>, grid_view: &[usize]) -> Self {
        for grid_view_i in residence {
            let grid_entry_i = grid_view[grid_view_i];
            let overlap = self.drop.remove(&grid_entry_i);

            if !overlap {
                if visible.contains(&grid_view_i) {
                    self.load_first.insert(grid_entry_i);
                } else {
                    self.load_after.insert(grid_entry_i);
                }
            }
        }

        self
    }

    fn with_flatten_drop(mut self, drop: Range<usize>, grid_view: &[usize]) -> Self {
        for grid_view_i in drop {
            let grid_entry_i = grid_view[grid_view_i];

            self.drop.insert(grid_entry_i);
        }

        self
    }
}

#[derive(Default)]
struct StreamView {
    load: Range<usize>,
    drop: Range<usize>
}
impl StreamView {
    fn with_load(mut self, load: Range<usize>) -> Self {
        self.load = load;

        self
    }

    fn with_drop(mut self, drop: Range<usize>) -> Self {
        self.drop = drop;

        self
    }

    fn flatten(self, grid_view: &[usize]) -> Stream {
        let load = self.load.map(|grid_view_i| grid_view[grid_view_i]).collect::<HashSet<_>>();
        let drop = self.drop.map(|grid_view_i| grid_view[grid_view_i]).collect::<HashSet<_>>();

        Stream { load_first: default!(), load_after: load, drop }
    }
}

struct ResizeImage {
    dst_path: PathBuf,
    dst_size: egui::Vec2
}

#[derive(Default)]
struct TagWinButtonMenuDeferred {
    is_open: bool,
    tag_op: Option<(Rc<str>, TagOp)>,
    stream: Option<Stream>
}

#[derive(Clone)]
enum BaseImageKind {
    Pick { path: PathBuf },
    Startup
}

#[derive(Default)]
enum ImageState {
    #[default]
    None,
    Pending(oneshot::Receiver<Result<(egui::ColorImage, Orientation), ()>>),
    Ready((egui::TextureHandle, Orientation)),
    Failed
}

pub enum Kind {
    Info { msg: String },
    MediaBrowser
}

#[derive(Clone, Copy, Debug)]
enum Orientation {
    Wide,
    Tall
}

enum TagOp {
    Rename,
    Remove
}

#[derive(Default)]
enum ViewKind {
    #[default]
    Grid,
    Details
}

#[derive(Default, Deserialize, PartialEq)]
enum Watching {
    Movie,
    #[default]
    TV,
    Words
}

trait AsOrientation {
    fn as_orientation(&self) -> Orientation;
}
impl AsOrientation for f32 {
    fn as_orientation(&self) -> Orientation {
        match *self <= ASPECT_RATIO_3_2 {
            true => Orientation::Wide,
            false => Orientation::Tall
        }
    }
}

fn add_image(ui: &mut egui::Ui, tex: &egui::TextureHandle, orientation: Orientation) -> egui::Response {
    let image = egui::Image::new(tex).sense(egui::Sense::click_and_drag()).fit_to_exact_size(tex.size_vec2());

    match orientation {
        Orientation::Wide => ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| ui.add(image)),
        Orientation::Tall => ui.with_layout(egui::Layout::top_down(egui::Align::Center), |ui| ui.add(image))
    }
    .inner
}

fn try_add_image(ui: &mut egui::Ui, image_state: &mut ImageState, tex_name: &str, label: &str) -> egui::Response {
    match image_state {
        ImageState::Ready((tex, orientation)) => add_image(ui, tex, *orientation),
        ImageState::Pending(rx) => {
            let recvd = match rx.try_recv() {
                Ok(res) => res,
                Err(TryRecvError::Empty) => return alloc_hover_response(ui),
                Err(TryRecvError::Closed) => Err(())
            };

            match recvd {
                Ok((color_image, orientation)) => {
                    let tex = ui.ctx().load_texture(tex_name, color_image, default!());
                    let resp = add_image(ui, &tex, orientation);
                    *image_state = ImageState::Ready((tex, orientation));

                    resp
                },
                Err(_) => {
                    let resp = add_label(ui, label);
                    *image_state = ImageState::Failed;

                    resp
                }
            }
        },
        ImageState::Failed => add_label(ui, label),
        _ => alloc_hover_response(ui)
        // _ => add_label(ui, label)
    }
}

fn add_label(ui: &mut egui::Ui, text: &str) -> egui::Response {
    ui.with_layout(egui::Layout::centered_and_justified(egui::Direction::LeftToRight), |ui| {
        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);

        let label = egui::Label::new(text);
        ui.add(label).on_hover_cursor(egui::CursorIcon::Default)
    })
    .inner
}

fn alloc_hover_response(ui: &mut egui::Ui) -> egui::Response {
    ui.allocate_response(ui.available_size(), egui::Sense::click())
}

fn sinc(x: f64) -> f64 {
    if x.abs() < 1e-6 {
        1.0
    } else {
        (PI * x).sin() / (PI * x)
    }
}

fn blackman(x: f64) -> f64 {
    let t = x.abs();

    if t >= 2.0 {
        0.0
    } else {
        let a = 2.0;
        let w = 0.42
            + 0.5 * (PI * t / a).cos()
            + 0.08 * (2.0 * PI * t / a).cos();

        sinc(t) * w
    }
}

fn blackman_filter() -> resize::Filter {
    resize::Filter::new(
        Box::new(|x: f32| -> f32 {
            blackman(f64::from(x)) as f32
        }),
        2.0
    )
}

fn get_aspect_ratio_v(width: f32, height: f32) -> AspectRatioV {
    height / width
}

fn load_rgba_image(path: &Path) -> ResVar<image::ImageBuffer<image::Rgba<u8>, Vec<u8>>> {
    let image = image::open(path)?;

    Ok(match image {
        image::DynamicImage::ImageRgba8(image) => image,
        _ => image.to_rgba8()
    })
}

fn ferry_base_image(src_path: &Path, dst_path: PathBuf, cell_size: egui::Vec2, image_state_sx: oneshot::Sender<Result<(egui::ColorImage, Orientation), ()>>) -> Res1<()> {
    fn inner(src_path: &Path, cell_size: egui::Vec2) -> Res1<(egui::ColorImage, Vec<u8>, AspectRatioV)> {
        use rgb::FromSlice;

        let src_image = load_rgba_image(src_path)?;
        let (src_width, src_height) = src_image.dimensions();
        let src_aspect_ratio_v = get_aspect_ratio_v(src_width as f32, src_height as f32);

        let (dst_width, dst_height) = if src_aspect_ratio_v <= ASPECT_RATIO_3_2 {
            (cell_size.x as usize, (cell_size.x * src_aspect_ratio_v).round() as usize) // Wide
        } else {
            ((cell_size.y / src_aspect_ratio_v).round() as usize, cell_size.y as usize) // Tall
        };

        let src_width = src_width as usize;
        let src_height = src_height as usize;
        let mut tmp_pixels = vec![0_u8; dst_width * src_height * 4];
        let mut dst_pixels = vec![0_u8; dst_width * dst_height * 4];

        let mut resizer = resize::new(
            src_width,
            src_height,
            dst_width,
            src_height,
            resize::Pixel::RGBA8,
            resize::Type::Custom(blackman_filter())
        )?;
        let src_pixels = src_image.as_raw();
        resizer.resize(src_pixels.as_rgba(), tmp_pixels.as_rgba_mut())?;

        let mut resizer = resize::new(
            dst_width,
            src_height,
            dst_width,
            dst_height,
            resize::Pixel::RGBA8,
            resize::Type::Custom(blackman_filter())
        )?;
        resizer.resize(tmp_pixels.as_rgba(), dst_pixels.as_rgba_mut())?;

        let color_image = egui::ColorImage::from_rgba_unmultiplied([dst_width, dst_height], dst_pixels.as_slice());

        Ok((color_image, dst_pixels, src_aspect_ratio_v))
    }

    match inner(src_path, cell_size) {
        Ok((image, pixels, aspect_ratio)) => {
            let image_size = image.size;
            image_state_sx.send(Ok((image, aspect_ratio.as_orientation()))).unwrap();

            let image_file = fs::File::create(dst_path)?;
            let encoder = image::codecs::webp::WebPEncoder::new_lossless(image_file);
            encoder.encode(pixels.as_slice(), image_size[0] as u32, image_size[1] as u32, image::ExtendedColorType::Rgba8)?;
        },
        Err(err) => {
            image_state_sx.send(Err(())).unwrap();

            Err(err)?;
        }
    }

    Ok(())
}

fn ferry_cached_image(path: PathBuf, image_state_sx: oneshot::Sender<Result<(egui::ColorImage, Orientation), ()>>) -> ResVar<()> {
    fn inner(path: PathBuf) -> ResVar<(egui::ColorImage, AspectRatioV)> {
        let image = load_rgba_image(path.as_path())?;
        let (width, height) = image.dimensions();
        let aspect_ratio_v = get_aspect_ratio_v(width as f32, height as f32);

        let pixels = image.as_raw();
        let color_image = egui::ColorImage::from_rgba_unmultiplied([width as usize, height as usize], pixels);

        Ok((color_image, aspect_ratio_v))
    }

    match inner(path) {
        Ok((image, aspect_ratio_v)) => image_state_sx.send(Ok((image, aspect_ratio_v.as_orientation()))).unwrap(),
        Err(err) => {
            image_state_sx.send(Err(())).unwrap();

            Err(err)?;
        }
    }

    Ok(())
}

fn queue_ferry_base_image(queue_sx: mpsc::Sender<QueueImageInfo>, src_path: PathBuf, dst_path: PathBuf, dst_size: egui::Vec2, image_state_sx: oneshot::Sender<Result<(egui::ColorImage, Orientation), ()>>) {
    queue_sx.send(
        QueueImageInfo {
            src_path,
            image_state_sx,
            resize: Some(ResizeImage {
                dst_path,
                dst_size
            })
        }
    )
    .unwrap();
}

fn queue_ferry_cached_image(queue_sx: mpsc::Sender<QueueImageInfo>, src_path: PathBuf, image_state_sx: oneshot::Sender<Result<(egui::ColorImage, Orientation), ()>>) {
    queue_sx.send(
        QueueImageInfo {
            src_path,
            image_state_sx,
            resize: None
        }
    )
    .unwrap();
}

fn ferry_images(info: FerryImagesInfo) {
    let FerryImagesInfo {
        ctx,
        thread_pool,
        hash_sx,
        image_dirs,
        base_image_kind,
        grid_cell_size,
        force_resize_grid_images,
        details_cell_size,
        force_resize_details_images,
        ferry_image_infos
    } = info;

    let handle_err = |err| {
        error!("{}: failed to ferry image: {}", module_path!(), err);
    };

    let (queue_sx, queue_rx) = mpsc::channel();
    for info in ferry_image_infos {
        let FerryImageInfo {
            image_file_name,
            expected_hash,
            grid_entry_i,
            grid_image_state_sx,
            details_image_state_sx,
            wait_ready,
            poll_ready
        } = info;

        let ctx = ctx.clone();
        let hash_sx = hash_sx.clone();
        let queue_sx = queue_sx.clone();
        let base_image_kind = base_image_kind.clone();

        thread_pool.spawn_fifo(move || {
            (|| -> Res<()> {
                let base_image_path = match base_image_kind {
                    BaseImageKind::Pick { path } => path,
                    BaseImageKind::Startup => image_dirs.base.join(image_file_name.as_ref())
                };
                let grid_image_path = image_dirs.grid.join(image_file_name.as_ref()).with_added_extension("webp");
                let details_image_path = image_dirs.details.join(image_file_name.as_ref()).with_added_extension("webp");

                // Compute hash on chunk
                let mut base_image_file = File::open(base_image_path.as_path())?;
                let mut hasher = blake3::Hasher::new();
                let mut chunk = [0_u8; CHUNK_BYTE_COUNT as usize];
                _ = base_image_file.read(&mut chunk)?;
                let hash = Arc::from(hasher.update(&chunk).finalize().to_hex().as_str());
                let hash_mismatches = expected_hash.is_none_or(|expected_hash| expected_hash != hash);

                match hash_mismatches {
                    true => {
                        hash_sx.send(HashInfo { grid_entry_i, hash }).unwrap();

                        ferry_base_image(base_image_path.as_path(), grid_image_path, grid_cell_size, grid_image_state_sx)?;
                        queue_ferry_base_image(queue_sx, base_image_path, details_image_path, details_cell_size, details_image_state_sx);
                    },
                    false => {
                        match force_resize_grid_images {
                            true => ferry_base_image(base_image_path.as_path(), grid_image_path, grid_cell_size, grid_image_state_sx)?,
                            false => ferry_cached_image(grid_image_path, grid_image_state_sx)?
                        }

                        match force_resize_details_images {
                            true => queue_ferry_base_image(queue_sx, base_image_path, details_image_path, details_cell_size, details_image_state_sx),
                            false => queue_ferry_cached_image(queue_sx, details_image_path, details_image_state_sx)
                        }
                    }
                }

                drop(wait_ready);
                drop(poll_ready);

                ctx.request_repaint();

                Ok(())
            })()
            .unwrap_or_else(handle_err)
        });
    }

    drop(queue_sx);
    let thread_pool = thread_pool.clone();
    thread::spawn(move || {
        for info in queue_rx {
            let QueueImageInfo {
                src_path,
                image_state_sx,
                resize
            } = info;

            thread_pool.spawn_fifo(move || {
                (|| -> Res<()> {
                    match resize {
                        Some(resize) => {
                            let ResizeImage { dst_path, dst_size } = resize;

                            ferry_base_image(src_path.as_path(), dst_path, dst_size, image_state_sx)?;
                        },
                        None => ferry_cached_image(src_path, image_state_sx)?
                    }

                    Ok(())
                })()
                .unwrap_or_else(handle_err);
            });
        }
    });
}

fn fix_background_brush(hnd: Win32WindowHandle) {
    fn make_colorref(r: u8, g: u8, b: u8) -> COLORREF {
        COLORREF(u32::from(r) | u32::from(g) << 8 | u32::from(b) << 16)
    }

    let hwnd = HWND(hnd.hwnd.get() as *mut c_void);
    (|| -> Res<()> {
        unsafe {
            let new_brush = CreateSolidBrush(make_colorref(27, 27, 27));
            let set = SetClassLongPtrW(hwnd, GCLP_HBRBACKGROUND, new_brush.0 as isize);

            if set == 0 {
                let maybe_err = GetLastError();

                let check = GetClassLongPtrW(hwnd, GCLP_HBRBACKGROUND).win32_core_ok()?;
                if check != new_brush.0 as usize {
                    maybe_err.ok()?;
                }
            }
        }

        Ok(())
    })()
    .unwrap_or_else(|err| {
        error!("{}: failed to set background brush: {}", module_path!(), err);
    });
}

fn get_default_handler(path: &Path) -> Res<PathBuf> { unsafe {
    let ext = path.get_file_ext()?;
    let ext = concat_string!(".", ext);
    let ext = ext.as_str().to_win_str();

    let mut buffer = [0_u16; MAX_PATH as usize];
    let path_str = PWSTR(buffer.as_mut_ptr());
    let mut path_len = buffer.len() as u32;

    AssocQueryStringW(ASSOCF_INIT_DEFAULTTOSTAR, ASSOCSTR_EXECUTABLE, *ext, None, Some(path_str), &mut path_len,).ok()?;

    let path_str = String::from_utf16(&buffer[..path_len as usize - 1])?;

    Ok(PathBuf::from(path_str))
} }

fn open_media(path: PathBuf, file_kind: FileKind, maintain_sample_rate: bool, override_glsl_shaders: bool, discord_info: Option<DiscordActivityInfo>, discord_display_kind: DiscordDisplayKind, error_sx: mpsc::Sender<String>) {
    thread::spawn(move || {
        (|| -> Res<()> {
            let ipc_client = discord_info.as_ref().map(|discord_info| -> Res<_> {
                let mut ipc_client = DiscordIpcClient::new(discord_info.app_id.as_str());

                discord::begin(&mut ipc_client, &discord_info.as_view(), discord_display_kind)?;

                Ok(ipc_client)
            })
            .transpose()?;

            match file_kind {
                FileKind::Vid => video::launch_mpv(&path, maintain_sample_rate.into(), override_glsl_shaders)?,
                _ => {
                    let handler = get_default_handler(&path)?;

                    let mut command = Command::new(handler);
                    command.arg(path);

                    output_command(&mut command)?;
                }
            }

            if let Some(mut ipc_client) = ipc_client {
                ipc_client.clear_activity()?;
                ipc_client.close()?;
            }

            Ok(())
        })()
        .unwrap_or_else(|err| {
            let msg = format!("{}: failure handling media: {}", module_path!(), err);

            error_sx.send(msg).unwrap();
        });
    });
}

fn populate_grid_view(view: &mut Vec<usize>, entries: &[GridEntryInfo], set: &BTreeSet<usize>) {
    view.clear();
    view.extend(set.iter().cloned());
    sort_grid_view(view, entries);
}

fn replace_dir_entries(entries: &mut Vec<DirEntryInfo>, dir: &Path) {
    (|| -> ResVar<()> {
        entries.clear();

        let read_dir = dir.read_dir()?;
        for dir_entry in read_dir {
            dir_entry.map_err(into!()).and_then(|dir_entry| -> Res<_> {
                let path = dir_entry.path();
                let stem = path.get_file_stem()?.to_string();
                let file_kind = path.get_file_kind()?;

                entries.push(DirEntryInfo { path, stem, file_kind });

                Ok(())
            })
            .unwrap_or_else(|err| error!("{}: failed to read dir entry: dir: {}: {}", module_path!(), dir.display(), err));
        }

        Ok(())
    })()
    .unwrap_or_else(|err| error!("{}: failed to read dir: {}: {}", module_path!(), dir.display(), err));
}

fn sort_grid_view(view: &mut [usize], entries: &[GridEntryInfo]) {
    view.sort_unstable_by(|a, b| {
        let entry_a = &entries[*a];
        let entry_b = &entries[*b];
        let name_a = entry_a.sort_name.as_deref().unwrap_or(entry_a.stem.as_ref());
        let name_b = entry_b.sort_name.as_deref().unwrap_or(entry_b.stem.as_ref());

        name_a.cmp(name_b)
    });
}

fn stroke_rect(ui: &mut egui::Ui, rect: egui::Rect) {
    ui.painter().rect_stroke(rect, 0.0, CELL_STROKE, egui::StrokeKind::Outside);
}

fn stroke_rect_painter(painter: egui::Painter, rect: egui::Rect) {
    painter.rect_stroke(rect, 0.0, CELL_STROKE, egui::StrokeKind::Outside);
}

fn to_discord_asset_name(s: impl AsRef<str>) -> String {
    s.as_ref().chars()
        .map(|c| {
            let c = c.to_ascii_lowercase();

            match c {
                '\'' | '.' | ' ' => '_',
                _ => c
            }
        })
        .collect()
}

struct Info {
    msg: String,
    checked_background_brush: bool,
    resized_viewport: bool
}
impl eframe::App for Info {
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        if !self.checked_background_brush && let win_hnd = frame.window_handle().unwrap() && let RawWindowHandle::Win32(hnd) = win_hnd.as_raw() {
            fix_background_brush(hnd);

            self.checked_background_brush = true;
        }

        if !self.resized_viewport {
            let screen_size = ctx.input(|i| i.viewport().monitor_size).unwrap();
            let init_win_size = screen_size.div(2.0).yx();
            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(init_win_size));

            let content_size = egui::CentralPanel::default()
                .show(ctx, |ui: &mut egui::Ui| {
                    ui.set_max_width(init_win_size.x);

                    Self::central_panel(self, ui)
                })
                .inner;

            let win_margins = ctx.style().spacing.window_margin.sum();
            let final_win_size = (content_size + win_margins + egui::vec2(5.0, 5.0))
                .min(init_win_size);

            let win_pos = egui::pos2(
                (screen_size.x - final_win_size.x) / 2.0,
                (screen_size.y - final_win_size.y) / 2.0
            );

            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(final_win_size));
            ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(win_pos));

            self.resized_viewport = true;

            return
        }

        egui::CentralPanel::default()
            .show(ctx, |ui: &mut egui::Ui| Self::central_panel(self, ui));
    }
}
impl Info {
    fn new(msg: String) -> Self {
        Self {
            msg,
            checked_background_brush: false,
            resized_viewport: false
        }
    }

    fn central_panel(&mut self, ui: &mut egui::Ui) -> egui::Vec2 {
        egui::ScrollArea::new([false, true])
            .auto_shrink(false)
            .show(ui, |ui| ui.label(&self.msg))
            .content_size
    }
}

struct MediaBrowser<'a> {
    thread_pool: Arc<rayon::ThreadPool>,
    image_dirs: &'static ImageDirs,
    images: IndexMap<Arc<str>, ImageStates>,
    hash_sx: mpsc::Sender<HashInfo>,
    hash_rx: mpsc::Receiver<HashInfo>,
    cache_path: PathBuf,
    cache: Cache,
    cached_images_to_remove: Vec<Rc<str>>,
    frame: egui::Frame,
    is_first_frame: bool,
    view_kind: ViewKind,
    grid_entries: Vec<GridEntryInfo>,
    grid_entry_i: usize,
    grid_cell_size: egui::Vec2,
    grid_cell_space: egui::Vec2,
    grid_scroll_offset: f32,
    /// Indices into [`grid_entries`]
    grid_view: Vec<usize>,
    grid_view_i: usize,
    grid_view_selected_i: Option<usize>,
    grid_view_poll_ready: Arc<()>,
    grid_view_entry_removed: bool,
    lookahead: usize,
    proximity: usize,
    residence: Range<usize>,
    animate: bool,
    sort_name_edit: String,
    tag_add_edit: String,
    /// Sets of indices into [`grid_entries`]
    tags: BTreeMap<Rc<str>, BTreeSet<usize>>,
    active_tag: Option<Rc<str>>,
    open_tag_win: bool,
    tag_win_button_menu: TagWinButtonMenuDeferred,
    tag_win_rename_edit: String,
    tag_win_stamp: Option<Instant>,
    tag_win_cursor_checked: bool,
    details_grid_entry_i: usize,
    details_dir_entries: Vec<DirEntryInfo>,
    details_button_resps: Vec<egui::Response>,
    details_cell_size: egui::Vec2,
    details_hovered_dir_entry_i: usize,
    details_levels: Vec<PathBuf>,
    scroll_multiplier: f32,
    maintain_sample_rate: bool,
    override_glsl_shaders: bool,
    enable_override_glsl_shaders_checkbox: bool,
    discord_app_ids: DiscordAppIds<'a>,
    discord_enabled: bool,
    discord_watching: Watching,
    discord_details: String,
    discord_state: String,
    discord_display_kind: DiscordDisplayKind,
    open_error_win: bool,
    error_sx: mpsc::Sender<String>,
    error_rx: mpsc::Receiver<String>,
    error_msg: String
}
impl<'a> eframe::App for MediaBrowser<'a> {
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        if self.is_first_frame {
            self.first_frame(ctx, frame);
            self.continue_update(ctx);
        } else {
            self.continue_update(ctx);
        }
    }
}
impl<'a> MediaBrowser<'a> {
    fn new(ctx: &egui::Context) -> Res<Self> {
        let config = config::get().read()?;
        let (media_dirs,
            grid_cell_width,
            details_cell_width,
            scroll_multiplier,
            lookahead,
            proximity
        ) = config.media_browser.as_ref()
            .map(|media_browser_config| {
                (
                    &media_browser_config.dirs,
                    media_browser_config.grid_cell_width.next_multiple_of(2) as f32,
                    media_browser_config.details_cell_width.next_multiple_of(2) as f32,
                    media_browser_config.scroll_multiplier,
                    media_browser_config.lookahead,
                    media_browser_config.proximity

                )
            })
            .ok_or(ErrVar::MissingConfigKey { name: config::MediaBrowser::NAME })?;
        let grid_cell_size = egui::vec2(grid_cell_width, grid_cell_width * ASPECT_RATIO_3_2);
        let grid_cell_space = grid_cell_size + GRID_IMAGE_SPACING;
        let details_cell_size = egui::vec2(details_cell_width, details_cell_width * ASPECT_RATIO_3_2);
        let discord_app_ids = config.discord.app_ids.clone();
        let discord_display_kind = config.discord.display_kind;
        let enable_override_glsl_shaders_checkbox = config.mpv.as_ref().map(|mpv_config| mpv_config.override_glsl_shaders.is_some()).unwrap_or(false);

        let thread_pool = Arc::new(rayon::ThreadPoolBuilder::new()
            .num_threads(thread::available_parallelism()?.get())
            .build()?);
        let (hash_sx, hash_rx) = mpsc::channel();

        let current_exe_dir = CURRENT_EXE_DIR.get().unwrap();
        let base_images_dir = current_exe_dir.join("images");
        let grid_images_dir = base_images_dir.join("grid");
        let details_images_dir = base_images_dir.join("details");
        let image_dirs = ImageDirs {
            base: base_images_dir,
            grid: grid_images_dir,
            details: details_images_dir
        };
        let image_dirs = Box::leak(Box::new(image_dirs));

        let cache_path = image_dirs.base.join("cache").with_extension("json");
        let cache_slc = fs::read(&cache_path)?;
        let mut cache: Cache = serde_json::from_slice(&cache_slc)?;
        let mut cached_images_to_remove = Vec::new();

        let tags = cache.tags.iter()
            .map(|tag| (tag.clone(), default!())) // Clone these - cache entries need to reference them later
            .collect::<BTreeMap::<Rc<str>, BTreeSet<_>>>();

        let mut images = image_dirs.base.read_dir()?
            .filter_map(|dir_entry| {
                dir_entry.map_err(ErrLoc::from).and_then(|dir_entry| -> Res<_> {
                    let path = dir_entry.path();
                    let file_name = path.get_file_name()?;
                    let file_kind = path.get_file_kind()?;

                    match file_kind {
                        FileKind::Image => Ok(Some((Arc::from(file_name), default!()))),
                        _ => Ok(None)
                    }
                })
                .transpose()
            })
            .collect::<Res<IndexMap::<Arc<str>, ImageStates>>>()?;

        let grid_entries = media_dirs.iter()
            .map(|dir| Path::new(dir).read_dir())
            .filter_map(|read_dir| match read_dir {
                Ok(read_dir) => Some(read_dir),
                Err(err) => {
                    error!("{}: failed to read dir: {}", module_path!(), err);

                    None
                }
            })
            .flatten()
            .filter_map(|dir_entry| {
                dir_entry.map_err(into!()).and_then(|dir_entry| -> Res<_> {
                    let path = dir_entry.path();

                    if let Some(ext) = path.extension() && ext == "ini" {
                        return Ok(None)
                    }

                    let stem = Rc::from(path.get_file_stem()?);
                    let file_kind = path.get_file_kind()?;

                    let try_get_image_file_name_i = |images: &mut IndexMap<Arc<str>, ImageStates>| {
                        for ext in IMAGE_EXTS {
                            let attempt = concat_string!(stem, ".", ext);

                            if let Some((image_file_name_i, _, state)) = images.get_full_mut(attempt.as_str()) {
                                state.ref_count += 1;

                                return Some(image_file_name_i)
                            }
                        }

                        None
                    };

                    let cache_entry_info = cache.entries.get_mut(&path);
                    let grid_entry_info = match cache_entry_info {
                        Some(info) => {
                            let sort_name = info.sort_name.clone();
                            let image_file_name_i = info.image_file_name_i
                                .and_then(|image_file_name_i| cache.images.get_index(image_file_name_i))
                                .and_then(|image_file_name| {
                                    images.get_full_mut(image_file_name.as_ref())
                                        // Entry was cached but its image was moved or deleted - remove cached image
                                        .tap_none(|| cached_images_to_remove.push(image_file_name.clone()))
                                })
                                .map(|(image_file_name_i, _, image_states)| {
                                    image_states.ref_count += 1;

                                    image_file_name_i
                                });
                            let hash = info.hash.clone();

                            GridEntryInfo { path, stem, sort_name, file_kind, image_file_name_i, hash }
                        },
                        None => {
                            let sort_name = None;
                            let image_file_name_i = try_get_image_file_name_i(&mut images);
                            let hash = None;

                            GridEntryInfo { path, stem, sort_name, file_kind, image_file_name_i, hash }
                        }
                    };

                    Ok(Some(grid_entry_info))
                })
                .unwrap_or_else(|err| {
                    error!("{}: failed to read dir entry: {}", module_path!(), err);

                    None
                })
            })
            .collect::<Vec<_>>();
        if grid_entries.is_empty() {
            Err(ErrVar::MissingEntries)?;
        }

        let mut grid_view = Vec::with_capacity(grid_entries.len());
        grid_view.extend(0..grid_entries.len());
        sort_grid_view(&mut grid_view, &grid_entries);

        drop(config);

        let frame = egui::Frame::central_panel(&ctx.style()).inner_margin(
            egui::Margin::symmetric(FRAME_MARGIN as i8, FRAME_MARGIN as i8)
        );

        let (error_sx, error_rx) = mpsc::channel();
        let error_msg = "".to_string();

        Ok(Self {
            thread_pool,
            image_dirs,
            images,
            hash_sx,
            hash_rx,
            cache_path,
            cache,
            cached_images_to_remove,
            frame,
            is_first_frame: true,
            view_kind: ViewKind::Grid,
            grid_entries,
            grid_entry_i: default!(),
            grid_cell_size,
            grid_cell_space,
            grid_scroll_offset: default!(),
            grid_view,
            grid_view_i: default!(),
            grid_view_selected_i: default!(),
            grid_view_poll_ready: default!(),
            grid_view_entry_removed: default!(),
            lookahead,
            proximity,
            residence: default!(),
            animate: true,
            sort_name_edit: default!(),
            tag_add_edit: default!(),
            tags,
            active_tag: default!(),
            open_tag_win: default!(),
            tag_win_button_menu: default!(),
            tag_win_rename_edit: default!(),
            tag_win_stamp: default!(),
            tag_win_cursor_checked: default!(),
            details_grid_entry_i: default!(),
            details_dir_entries: Vec::with_capacity(DETAILS_ENTRY_COUNT),
            details_button_resps: Vec::with_capacity(DETAILS_ENTRY_COUNT),
            details_cell_size,
            details_hovered_dir_entry_i: default!(),
            details_levels: Vec::with_capacity(16),
            scroll_multiplier,
            maintain_sample_rate: default!(),
            override_glsl_shaders: default!(),
            enable_override_glsl_shaders_checkbox,
            discord_app_ids,
            discord_enabled: default!(),
            discord_watching: default!(),
            discord_details: default!(),
            discord_state: default!(),
            discord_display_kind,
            open_error_win: false,
            error_sx,
            error_rx,
            error_msg
        })
    }

    fn first_frame(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        let visible_cell_count = self.reset_residence(ctx);

        let wait_group = WaitGroup::new();
        let ferry_image_infos = self.grid_view.iter().enumerate()
            .filter_map(|(grid_view_i, &grid_entry_i)| {
                let grid_entry_info = &self.grid_entries[grid_entry_i];

                // Fill tags
                if let Some(CacheEntryInfo { tags: tag_is, .. }) = self.cache.entries.get_mut(&grid_entry_info.path) {
                    for tag_i in tag_is {
                        let tag = &self.cache.tags[*tag_i];
                        let set = self.tags.get_mut(tag);

                        if let Some(set) = set {
                            set.insert(grid_entry_i);
                        }
                    }
                }

                if let Some(image_file_name_i) = grid_entry_info.image_file_name_i &&
                    grid_view_i < self.residence.end
                {
                    let (grid_image_state_sx, grid_image_state_rx) = oneshot::channel();
                    let (details_image_state_sx, details_image_state_rx) = oneshot::channel();
                    let (image_file_name, image_states) = self.images.get_index_mut(image_file_name_i).unwrap();

                    if image_states.is_none() {
                        image_states.grid = ImageState::Pending(grid_image_state_rx);
                        image_states.details = ImageState::Pending(details_image_state_rx);

                        return Some(FerryImageInfo {
                            image_file_name: image_file_name.clone(),
                            expected_hash: grid_entry_info.hash.clone(),
                            grid_entry_i,
                            grid_image_state_sx,
                            details_image_state_sx,
                            wait_ready: grid_entry_i.lt(&visible_cell_count).then_some(wait_group.clone()),
                            poll_ready: None
                        })
                    }
                }

                None
            })
            .collect::<Vec<_>>();

        let ferry_images_info = FerryImagesInfo {
            ctx,
            thread_pool: &self.thread_pool,
            hash_sx: &self.hash_sx,
            image_dirs: self.image_dirs,
            base_image_kind: BaseImageKind::Startup,
            grid_cell_size: self.grid_cell_size,
            force_resize_grid_images: self.cache.grid_cell_size != self.grid_cell_size,
            details_cell_size: self.details_cell_size,
            force_resize_details_images: self.cache.details_cell_size != self.details_cell_size,
            ferry_image_infos
        };
        ferry_images(ferry_images_info);

        wait_group.wait();

        // Match window background to egui
        let win_hnd = frame.window_handle().unwrap();
        if let RawWindowHandle::Win32(hnd) = win_hnd.as_raw() {
            fix_background_brush(hnd);
        }

        self.is_first_frame = false;
    }

    #[hotpath::measure]
    fn continue_update(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default()
            .frame(self.frame)
            .show(ctx, |ui: &mut egui::Ui| Self::central_panel(self, ui));

        if !self.open_error_win &&
            let Ok(msg) = self.error_rx.try_recv()
        {
            self.error_msg = msg;
            self.open_error_win = true;
        }

        egui::Window::new("Error")
            .open(&mut self.open_error_win)
            .auto_sized()
            .show(ctx, |ui| ui.label(self.error_msg.as_str()));

        // Close and save cache to file
        if ctx.input(|state| state.viewport().close_requested()) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));

            for image_file_name in self.cached_images_to_remove.drain(..) {
                let paths = [
                    self.image_dirs.grid.join(image_file_name.as_ref()).with_added_extension("webp"),
                    self.image_dirs.details.join(image_file_name.as_ref()).with_added_extension("webp")
                ];

                for path in paths {
                    fs::remove_file(path.as_path()).unwrap_or_else(|err| error!("{}: failed to remove cached image: {}: {}", module_path!(), path.display(), err));
                }
            }

            self.cache.grid_cell_size = self.grid_cell_size;
            self.cache.details_cell_size = self.details_cell_size;
            self.cache.tags = self.tags.keys().cloned().collect();
            self.cache.images = self.images.keys().map(|key| Rc::from(key.as_ref())).collect();
            self.cache.entries.clear();

            let mut grid_entry_tags = vec![Some(Vec::with_capacity(self.tags.len())); self.grid_entries.len()];
            for (tag_i, (_, set)) in self.tags.iter().enumerate() {
                for grid_entry_i in set {
                    grid_entry_tags[*grid_entry_i].as_mut().unwrap().push(tag_i);
                }
            }

            while let Ok(HashInfo { grid_entry_i, hash }) = self.hash_rx.try_recv() {
                self.grid_entries[grid_entry_i].hash = Some(hash);
            }

            for (i, info) in self.grid_entries.drain(..).enumerate() {
                self.cache.entries.insert(
                    info.path,
                    CacheEntryInfo {
                        image_file_name_i: info.image_file_name_i,
                        sort_name: info.sort_name,
                        hash: info.image_file_name_i.map(|_| info.hash).unwrap_or_default(),
                        tags: std::mem::take(&mut grid_entry_tags[i]).unwrap()
                    }
                );
            }

            let cache_file = File::options()
                .truncate(true)
                .write(true)
                .open(self.cache_path.as_path()).unwrap();
            serde_json::to_writer_pretty(cache_file, &self.cache).unwrap();
        }
    }

    fn reset_grid_view(&mut self, ctx: &egui::Context) {
        self.grid_view.clear();
        self.grid_view.extend(0..self.grid_entries.len());
        self.sort_grid_view();

        let visible_cell_count = self.reset_residence(ctx);
        let stream = Stream::default().with_flatten_load(self.residence.clone(), 0..visible_cell_count, &self.grid_view);
        self.stream(ctx, &stream, true);

        self.animate = false;
        self.active_tag = None;
    }

    fn reset_residence(&mut self, ctx: &egui::Context) -> usize {
        let available_rect = ctx.available_rect();
        let available_row_cell_count = (available_rect.width() - self.grid_cell_size.x).div(self.grid_cell_space.x).ceil() as usize;
            // content_rect.width() - (self.grid_cell_size.x * avail_row_cell_count - GRID_IMAGE_SPACING.x) <= self.grid_cell_size.x
        let available_col_cell_count = (available_rect.height()).div(self.grid_cell_space.y).ceil() as usize;
        let visible_cell_count = (available_row_cell_count * available_col_cell_count).clamp(1, self.grid_view.len());
        let resident_cell_count = (visible_cell_count + self.lookahead * available_row_cell_count).min(self.grid_view.len());

        self.residence = 0..resident_cell_count;

        visible_cell_count
    }

    fn sort_grid_view(&mut self) {
        sort_grid_view(&mut self.grid_view, &self.grid_entries);
    }

    fn stream(&mut self, ctx: &egui::Context, stream: &Stream, poll_ready: bool) {
        if !stream.drop.is_empty() {
            let (sx, rx) = mpsc::channel();

            for grid_entry_i in stream.drop.iter().copied() {
                let grid_entry_info = &self.grid_entries[grid_entry_i];

                if let Some(image_file_name_i) = grid_entry_info.image_file_name_i {
                    let (_, image_states) = self.images.get_index_mut(image_file_name_i).unwrap();

                    image_states.ref_count = image_states.ref_count.saturating_sub(1);
                    if image_states.ref_count == 0 {
                        sx.send(std::mem::take(image_states)).unwrap();
                    }
                }
            }

            self.thread_pool.spawn_fifo(move || {
                for image_states in rx.into_iter() {
                    if let ImageState::Pending(rx) = image_states.grid {
                        _ = rx.blocking_recv();
                    }
                    if let ImageState::Pending(rx) = image_states.details {
                        _ = rx.blocking_recv();
                    }
                }
            });
        }

        let ferry_image_infos = stream.load_first.iter().chain(stream.load_after.iter()).copied()
            .enumerate()
            .filter_map(|(enum_i, grid_entry_i)| {
                let grid_entry_info = &self.grid_entries[grid_entry_i];

                if let Some(image_file_name_i) = grid_entry_info.image_file_name_i {
                    let (grid_image_state_sx, grid_image_state_rx) = oneshot::channel();
                    let (details_image_state_sx, details_image_state_rx) = oneshot::channel();
                    let (image_file_name, image_states) = self.images.get_index_mut(image_file_name_i).unwrap();

                    if image_states.is_none() {
                        image_states.grid = ImageState::Pending(grid_image_state_rx);
                        image_states.details = ImageState::Pending(details_image_state_rx);

                        return Some(FerryImageInfo {
                            image_file_name: image_file_name.clone(),
                            expected_hash: grid_entry_info.hash.clone(),
                            grid_entry_i,
                            grid_image_state_sx,
                            details_image_state_sx,
                            wait_ready: None,
                            poll_ready: poll_ready.and_then(||
                                enum_i.lt(&stream.load_first.len()).then_some(self.grid_view_poll_ready.clone())
                            )
                        })
                    }
                }

                None
            })
            .collect::<Vec<_>>();

        let ferry_images_info = FerryImagesInfo {
            ctx,
            thread_pool: &self.thread_pool,
            hash_sx: &self.hash_sx,
            image_dirs: self.image_dirs,
            base_image_kind: BaseImageKind::Startup,
            grid_cell_size: self.grid_cell_size,
            force_resize_grid_images: self.cache.grid_cell_size != self.grid_cell_size,
            details_cell_size: self.details_cell_size,
            force_resize_details_images: self.cache.details_cell_size != self.details_cell_size,
            ferry_image_infos
        };
        ferry_images(ferry_images_info);
    }

    fn update_residence(&mut self, visible_row_range: &Range<usize>, row_cell_count: usize, max_cell_count: usize) -> Option<(Residence, Stream)> {
        let proximity = self.proximity * row_cell_count;
        let lookahead = self.lookahead * row_cell_count;
        let visible_cell_range = (visible_row_range.start * row_cell_count)..(visible_row_range.end * row_cell_count);
        let mut new_residence = self.residence.clone();

        let range_starts_diff = visible_cell_range.start.saturating_sub(self.residence.start);
        let range_ends_diff = self.residence.end.saturating_sub(visible_cell_range.end);

        // Proximal end
        if range_ends_diff <= proximity {
            new_residence.end = visible_cell_range.end.add(lookahead).min(max_cell_count);
        }
        // Distal start
        if range_starts_diff >= lookahead + proximity {
            new_residence.start = visible_cell_range.start.saturating_sub(lookahead);
        }
        // Proximal start
        if range_starts_diff <= proximity {
            new_residence.start = visible_cell_range.start.saturating_sub(lookahead);
        }
        // Distal end
        if range_ends_diff >= lookahead + proximity {
            new_residence.end = visible_cell_range.end.add(lookahead).min(max_cell_count);
        }

        let cmp = self.residence.compare(&new_residence);
        let stream_view = match cmp {
            RangeCmpResult::RangeEmpty |
            RangeCmpResult::CompletelyTheSame |
            RangeCmpResult::CompletelyIncluded { .. } |
            RangeCmpResult::MiddleIncluded { .. } =>
                return None,
            RangeCmpResult::NotIncludedBelow |
            RangeCmpResult::NotIncludedAbove =>
                StreamView::default().with_load(new_residence.clone()).with_drop(self.residence.clone()),
            RangeCmpResult::EndIncluded { other_after, original_part_which_is_not_included, .. } =>
                StreamView::default().with_load(other_after).with_drop(original_part_which_is_not_included),
            RangeCmpResult::StartIncluded { other_before, original_part_which_is_not_included, .. } =>
                StreamView::default().with_load(other_before).with_drop(original_part_which_is_not_included),
            RangeCmpResult::SameStartOriginalShorter { other_after_not_included, .. } =>
                StreamView::default().with_load(other_after_not_included),
            RangeCmpResult::SameStartOtherShorter { original_after_not_included, .. } =>
                StreamView::default().with_drop(original_after_not_included),
            RangeCmpResult::SameEndOriginalShorter { other_before_not_included, .. } =>
                StreamView::default().with_load(other_before_not_included),
            RangeCmpResult::SameEndOtherShorter { original_before_not_included, .. } =>
                StreamView::default().with_drop(original_before_not_included)
        };
        let stream = stream_view.flatten(&self.grid_view);

        Some((new_residence, stream))
    }

    fn central_panel(&mut self, ui: &mut egui::Ui) {
        match self.view_kind {
            ViewKind::Grid => {
                if ui.ctx().input(|state| state.pointer.button_released(egui::PointerButton::Extra1)) && self.active_tag.is_some() {
                    self.reset_grid_view(ui.ctx());
                }

                self.tag_win(ui);

                if Arc::strong_count(&self.grid_view_poll_ready) == 1 {
                    let opacity = match self.animate {
                        true => ui.ctx().animate_bool_with_time_and_easing("animate".into(), true, 0.3, egui::emath::easing::cubic_out),
                        false => {
                            self.animate = true;

                            ui.ctx().clear_animations();
                            ui.ctx().animate_bool_with_time_and_easing("animate".into(), false, 0.3, egui::emath::easing::cubic_out)
                        }
                    };
                    ui.set_opacity(opacity);

                    self.grid_view(ui);
                }
            },
            ViewKind::Details => {
                if ui.ctx().input(|state| state.pointer.button_released(egui::PointerButton::Extra1) || state.key_pressed(egui::Key::Escape)) {
                    self.pop_dir();
                };

                self.details_view(ui);
            }
        }
    }

    fn tag_win(&mut self, ui: &mut egui::Ui) {
        let max_rect = ui.max_rect();
        let tag_win_rect = egui::Rect::from_min_size(
            max_rect.min,
            [250.0, (max_rect.height() - FRAME_MARGIN).max(0.0)].into()
        );

        let tag_win = self.open_tag_win.and_then(|| {
            egui::Window::new("view_win")
                .fixed_rect(tag_win_rect)
                .title_bar(false)
                .fade_in(true)
                .fade_out(true)
                .show(ui.ctx(), |ui| {
                    ui.with_layout(egui::Layout::top_down_justified(egui::Align::Center), |ui| {
                        ui.heading("Tags");

                        ui.separator();

                        self.tag_win_buttons(ui);

                        if let Some((tag, op)) = self.tag_win_button_menu.tag_op.as_ref() {
                            let tag_is_active = self.active_tag.as_ref().map(|active_tag| active_tag == tag).unwrap_or(false);

                            match op {
                                TagOp::Rename => {
                                    let set = self.tags.remove(tag).unwrap();
                                    let tag: Rc<str> = Rc::from(self.tag_win_rename_edit.as_str());

                                    if tag_is_active {
                                        self.active_tag = Some(tag.clone());
                                    }
                                    self.tags.insert(tag, set);

                                    self.tag_win_rename_edit.clear();
                                },
                                TagOp::Remove => {
                                    self.tags.remove(tag);

                                    if tag_is_active {
                                        self.active_tag = None;
                                    }
                                }
                            }
                        }

                        ui.take_available_space();
                    })
                })
        });

        if !self.tag_win_button_menu.is_open {
            let hover_pos = ui.ctx().input(|state| state.pointer.hover_pos());

            match hover_pos {
                Some(hover_pos) => {
                    self.tag_win_cursor_checked = false;

                    if let Some(tag_win) = tag_win.as_ref() {
                        match tag_win.response.contains_pointer() {
                            true => self.tag_win_stamp = Some(now!()),
                            false => if hover_pos.x > tag_win.response.rect.right() {
                                self.tag_win_stamp = None;
                                self.open_tag_win = false;
                            }
                        }
                    }
                }
                None => {
                    if !self.tag_win_cursor_checked {
                        let mut cursor_pos = POINT::default();
                        unsafe { if GetCursorPos(&mut cursor_pos).is_err() {
                            return
                        } }
                        #[allow(clippy::cast_precision_loss)]
                        let cursor_pos = egui::pos2(cursor_pos.x as f32, cursor_pos.y as f32);

                        if let Some(inner_rect) = ui.ctx().input(|state| state.viewport().inner_rect) {
                            let cursor_catch_rect = egui::Rect::everything_left_of(inner_rect.left());

                            if cursor_catch_rect.contains(cursor_pos) {
                                self.tag_win_stamp = Some(now!());
                                self.open_tag_win = true;
                            }

                            self.tag_win_cursor_checked = true;
                        }
                    }
                }
            }

            if let Some(tag_win_stamp) = self.tag_win_stamp && tag_win_stamp.elapsed() > Duration::from_secs(3) {
                self.tag_win_stamp = None;
                self.open_tag_win = false;
            }
        }
    }

    fn tag_win_buttons(&mut self, ui: &mut egui::Ui) {
        let all_button_resp = ui.button("All");

        if all_button_resp.clicked() && self.active_tag.is_some() {
            self.reset_grid_view(ui.ctx());
        }
        if self.active_tag.is_none() {
            all_button_resp.highlight();
        }

        self.tag_win_button_menu = self.tags.iter().fold(TagWinButtonMenuDeferred::default(), |mut deferred, (tag, set)| {
            if !set.is_empty() {
                let tag_button_resp = ui.button(tag.as_ref());

                let tag_button_menu = egui::Popup::context_menu(&tag_button_resp)
                    .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                    .show(|ui| {
                        let tag_rename_edit_resp = egui::TextEdit::singleline(&mut self.tag_win_rename_edit)
                            .hint_text("Rename")
                            .show(ui)
                            .response;

                        tag_rename_edit_resp.request_focus();

                        if ui.input(|state| state.key_pressed(egui::Key::Enter)) {
                            if !self.tag_win_rename_edit.is_empty() {
                                deferred.tag_op = Some((tag.clone(), TagOp::Rename));

                                ui.close();
                            } else {
                                tag_rename_edit_resp.request_focus();
                            }
                        }

                        let tag_remove_button_resp = ui.button("Remove");
                        if tag_remove_button_resp.clicked() {
                            deferred.tag_op = Some((tag.clone(), TagOp::Remove));

                            ui.close();
                        }
                    });

                deferred.is_open = tag_button_menu.is_some();
                if let Some(tag_button_menu) = tag_button_menu && tag_button_menu.response.should_close() {
                    self.tag_win_stamp = Some(now!());
                }

                if tag_button_resp.clicked() && self.active_tag.as_ref().is_none_or(|active_tag| active_tag != tag) {
                    deferred.stream = Some(Stream::default().with_flatten_drop(self.residence.clone(), &self.grid_view));
                    populate_grid_view(&mut self.grid_view, &self.grid_entries, set);

                    self.animate = false;
                    self.active_tag = Some(tag.clone());
                    self.grid_view_selected_i = None;
                }
                if let Some(active_tag) = self.active_tag.as_ref() && active_tag == tag {
                    tag_button_resp.highlight();
                }
            }

            deferred
        });

        if let Some(stream) = self.tag_win_button_menu.stream.take() {
            let visible_cell_count = self.reset_residence(ui.ctx());
            let stream = stream.with_flatten_load(self.residence.clone(), 0..visible_cell_count, &self.grid_view);
            self.stream(ui.ctx(), &stream, true);
        }
    }

    fn grid_view(&mut self, ui: &mut egui::Ui) {
        ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
            ui.spacing_mut().item_spacing = GRID_IMAGE_SPACING;

            let max_cell_count = self.grid_view.len();
            let row_cell_count = (ui.available_width() - self.grid_cell_size.x).div(self.grid_cell_space.x).ceil() as usize;
            let row_cell_count = row_cell_count.clamp(1, max_cell_count);
            let max_row_count = max_cell_count.div_ceil(row_cell_count);

            self.grid_scroll_offset = egui::ScrollArea::new([false, true])
                .auto_shrink(false)
                .scroll_source(egui::scroll_area::ScrollSource::SCROLL_BAR | egui::scroll_area::ScrollSource::MOUSE_WHEEL)
                .wheel_scroll_multiplier([1.0, self.scroll_multiplier].into())
                .vertical_scroll_offset(self.grid_scroll_offset)
                .show_rows(ui, self.grid_cell_size.y, max_row_count, |ui, row_range| {
                    if let Some((residence, stream)) = self.update_residence(&row_range, row_cell_count, max_cell_count) {
                        self.stream(ui.ctx(), &stream, false);

                        self.residence = residence;
                    }

                    let available_rect = ui.available_rect_before_wrap();

                    #[allow(clippy::cast_precision_loss)]
                    let table_width = row_cell_count as f32 * self.grid_cell_space.x - GRID_IMAGE_SPACING.x;
                    let table_rect_min_x = (available_rect.center().x - table_width / 2.0).floor();
                    let table_rect = egui::Rect::from_min_size(
                        [table_rect_min_x, available_rect.top()].into(),
                        [table_width, available_rect.height()].into(),
                    );

                    ui.scope_builder(egui::UiBuilder::new().max_rect(table_rect), |ui| {
                        let table_row_count = row_range.end - row_range.start;
                        self.grid_table(ui, row_range.start, max_cell_count, table_row_count, row_cell_count);
                    });
                })
                .state.offset.y;
        });
    }

    fn grid_table(&mut self, ui: &mut egui::Ui, row_start: usize, mut max_cell_count: usize, row_count: usize, row_cell_count: usize) {
        egui_extras::TableBuilder::new(ui)
            .striped(false)
            .vscroll(false)
            .cell_layout(egui::Layout::top_down(egui::Align::Center))
            .columns(egui_extras::Column::initial(self.grid_cell_size.x).at_most(self.grid_cell_size.x), row_cell_count)
            .body(|body| {
                body.rows(self.grid_cell_size.y, row_count, |mut row| {
                    self.grid_view_i = (row_start + row.index()) * row_cell_count;

                    while row.col_index() < row_cell_count && self.grid_view_i < max_cell_count {
                        row.col(|ui| {
                            ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                                self.grid_cell(ui);
                            });
                        });

                        // Cell entry might have its tag removed while view is active and table is still being updated
                        match self.grid_view_entry_removed {
                            true => {
                                max_cell_count -= 1;
                                self.grid_view_entry_removed = false;
                            },
                            false => self.grid_view_i += 1
                        }
                    }
                });
            });
    }

    fn grid_cell(&mut self, ui: &mut egui::Ui) {
        self.grid_entry_i = self.grid_view[self.grid_view_i];
        let grid_entry_info = &self.grid_entries[self.grid_entry_i];
        let mut image_info = grid_entry_info.image_file_name_i.and_then(|image_file_name_i| self.images.get_index_mut(image_file_name_i));

        let cell_resp = match image_info.as_mut() {
            Some((image_file_name, ImageStates { grid: grid_state, .. })) => {
                try_add_image(ui, grid_state, image_file_name.as_ref(), grid_entry_info.stem.as_ref())
            },
            None => add_label(ui, grid_entry_info.stem.as_ref())
        };

        if cell_resp.clicked() {
            self.details_grid_entry_i = self.grid_entry_i;
            self.details_dir_entries.clear();

            match grid_entry_info.path.is_dir() {
                true => replace_dir_entries(&mut self.details_dir_entries, &grid_entry_info.path),
                false => self.details_dir_entries.push(
                    DirEntryInfo {
                        path: grid_entry_info.path.clone(),
                        stem: grid_entry_info.stem.to_string(),
                        file_kind: grid_entry_info.file_kind
                    }
                )
            }

            self.view_kind = ViewKind::Details;
        }

        let cell_context_menu_resp = self.grid_cell_context_menu(ui, &cell_resp);

        match self.grid_view_selected_i {
            Some(grid_selected_cell_i) => if grid_selected_cell_i == self.grid_view_i { // Cell was secondary clicked
                stroke_rect(ui, cell_resp.rect);

                if cell_context_menu_resp.is_none() {
                    self.grid_view_selected_i = None // Context menu was closed - deselect cell
                }
            },
            // No context menu
            None => if cell_resp.hovered() {
                stroke_rect(ui, cell_resp.rect);
            }
        }
    }

    fn grid_cell_context_menu(&mut self, cell_ui: &mut egui::Ui, cell_resp: &egui::Response) -> Option<egui::InnerResponse<()>> {
        let close_behaviour = match cell_resp.clicked_elsewhere() {
            true => egui::PopupCloseBehavior::CloseOnClickOutside,
            false => egui::PopupCloseBehavior::IgnoreClicks
        };

        egui::Popup::context_menu(cell_resp)
            .close_behavior(close_behaviour)
            .show(|ui| {
                if ui.button("Image").clicked() {
                    ui.close();

                    let pick_image_file = rfd::FileDialog::new()
                        .add_filter("images", IMAGE_EXTS)
                        .pick_file();

                    if let Some(path) = pick_image_file {
                        self.pick_image(ui.ctx(), path).unwrap_or_else(|err| {
                            error!("{}: failed to pick image: {}", module_path!(), err);
                        });
                    }
                }

                self.grid_cell_sort_menu(ui);
                self.grid_cell_tags_menu(ui);

                let painter = ui.painter().clone().with_layer_id(cell_ui.layer_id());
                stroke_rect_painter(painter, cell_resp.rect);

                self.grid_view_selected_i = Some(self.grid_view_i);
            })
    }

    fn pick_image(&mut self, ctx: &egui::Context, path: PathBuf) -> Res<()> {
        let (grid_image_state_sx, grid_image_state_rx) = oneshot::channel();
        let (details_image_state_sx, details_image_state_rx) = oneshot::channel();

        let image_file_name_i = self.grid_entries[self.grid_entry_i].image_file_name_i;
        let grid_entry_stem = self.grid_entries[self.grid_entry_i].stem.as_ref();
        let pick_image_ext = path.get_file_ext()?;

        let (new_image_file_name, prev_image_file_name) = match image_file_name_i {
            Some(image_file_name_i) => {
                let (prev_image_file_name, image_states) = self.images.get_index_mut(image_file_name_i).unwrap();

                match &mut image_states.ref_count { // Multiple entries reference the image
                    ref_count @ 2.. => {
                        const BYTES: usize = 2;
                        const GRAMMAR_CHAR_COUNT: usize = 4;
                        const DUP_DIGIT_COUNT: usize = 2;
                        let mut s = String::with_capacity(grid_entry_stem.len() + pick_image_ext.len() + BYTES * (DUP_DIGIT_COUNT + GRAMMAR_CHAR_COUNT));

                        for i in *ref_count.. {
                            write!(s, "{} ({}).{}", grid_entry_stem, i, pick_image_ext).unwrap();
                            let check_path = self.image_dirs.base.join(&s);

                            if !check_path.try_exists()? {
                                break
                            }
                        }
                        let new_image_file_name: Arc<str> = Arc::from(s.as_str());

                        *ref_count -= 1;

                        (new_image_file_name, None)
                    },
                    _ => { // Only this entry references the image
                        let new_image_file_name = concat_string!(grid_entry_stem, ".", pick_image_ext);
                        let remove_prev_image = new_image_file_name.as_str() != prev_image_file_name.as_ref();

                        match remove_prev_image {
                            true => {
                                let new_image_file_name: Arc<str> = Arc::from(new_image_file_name.as_str());

                                (new_image_file_name, Some(prev_image_file_name.clone()))
                            },
                            false => {
                                let same_image_file_name = prev_image_file_name.clone();

                                (same_image_file_name, None)
                            }
                        }
                    }
                }
            },
            None => { // Entry doesn't reference an image
                let new_image_file_name = concat_string!(grid_entry_stem, ".", pick_image_ext);
                let new_image_file_name: Arc<str> = Arc::from(new_image_file_name.as_str());

                (new_image_file_name, None)
            }
        };
        let (image_file_name_i, _) = self.images.insert_full(new_image_file_name.clone(), ImageStates { grid: ImageState::Pending(grid_image_state_rx), details: ImageState::Pending(details_image_state_rx), ref_count: 1 });
        self.grid_entries[self.grid_entry_i].image_file_name_i = Some(image_file_name_i);

        let base_image_path = self.image_dirs.base.join(new_image_file_name.as_ref());

        let ferry_images_info = FerryImagesInfo {
            ctx,
            thread_pool:  &self.thread_pool,
            hash_sx: &self.hash_sx,
            image_dirs: self.image_dirs,
            base_image_kind: BaseImageKind::Pick { path: path.clone() },
            grid_cell_size: self.grid_cell_size,
            force_resize_grid_images: false,
            details_cell_size: self.details_cell_size,
            force_resize_details_images: false,
            ferry_image_infos: vec![
                FerryImageInfo {
                    image_file_name: new_image_file_name,
                    expected_hash: None,
                    grid_entry_i: self.grid_entry_i,
                    grid_image_state_sx,
                    details_image_state_sx,
                    wait_ready: None,
                    poll_ready: None
                }
            ]
        };
        ferry_images(ferry_images_info);

        let image_dirs = self.image_dirs;
        self.thread_pool.spawn(move || {
            _ = fs::copy(&path, &base_image_path).inspect_err(|err| {
                error!("{}: failed to copy image to cache: {}: {}", module_path!(), path.display(), err);
            });

            if let Some(prev_image_file_name) = prev_image_file_name {
                let paths = [
                    image_dirs.base.join(prev_image_file_name.as_ref()),
                    image_dirs.grid.join(prev_image_file_name.as_ref()).with_added_extension("webp"),
                    image_dirs.details.join(prev_image_file_name.as_ref()).with_added_extension("webp")
                ];
                for path in paths {
                    _ = fs::remove_file(&path).inspect_err(|err| {
                        error!("{}: failed to remove image: {}: {}", module_path!(), path.display(), err);
                    });
                }
            }
        });

        Ok(())
    }

    fn grid_cell_sort_menu(&mut self, ui: &mut egui::Ui) {
        self.grid_entry_i = self.grid_view[self.grid_view_i];
        let grid_entry_info = &mut self.grid_entries[self.grid_entry_i];

        let mut sort_grid_view = false;
        ui.menu_button("Sort name", |ui| {
            let sort_name_edit_resp = egui::TextEdit::singleline(&mut self.sort_name_edit)
                .hint_text(grid_entry_info.sort_name.as_deref().unwrap_or("Add"))
                .show(ui)
                .response;

            if ui.input(|state| state.key_pressed(egui::Key::Enter)) {
                if !self.sort_name_edit.is_empty() {
                    grid_entry_info.sort_name = Some(Rc::from(self.sort_name_edit.as_str()));

                    self.sort_name_edit.clear();
                    sort_grid_view = true;
                }

                sort_name_edit_resp.request_focus();
            }

            if grid_entry_info.sort_name.is_some() && ui.button("Remove").clicked() {
                grid_entry_info.sort_name = None;

                self.sort_name_edit.clear();
                sort_grid_view = true;
            }
        });

        if sort_grid_view {
            self.sort_grid_view();

            ui.close();
        }
    }

    fn grid_cell_tags_menu(&mut self, ui: &mut egui::Ui) -> egui::InnerResponse<Option<()>> {
        ui.menu_button("Tags", |ui| {
            // Add tag
            let tag_add_edit_resp = egui::TextEdit::singleline(&mut self.tag_add_edit)
                .hint_text("Add")
                .show(ui)
                .response;

            if ui.input(|state| state.key_pressed(egui::Key::Enter)) {
                if !self.tag_add_edit.is_empty() {
                    self.tags.entry(Rc::from(self.tag_add_edit.as_str()))
                        .and_modify(|set| _ = set.insert(self.grid_entry_i))
                        .or_insert([self.grid_entry_i].into_iter().collect());

                    self.tag_add_edit.clear();
                }

                tag_add_edit_resp.request_focus();
            }

            ui.separator();

            // Select from existing tags
            for (tag, set) in self.tags.iter_mut() { if !set.is_empty() {
                let tag_button_resp = ui.button(tag.as_ref());

                match (tag_button_resp.clicked(), set.contains(&self.grid_entry_i)) {
                    (_tag_button_clicked @ true, _entry_is_tagged @ true) => {
                        set.remove(&self.grid_entry_i);

                        let tag_is_active = self.active_tag.as_ref().map(|active_tag| active_tag == tag).unwrap_or(false);
                        if tag_is_active {
                            match set.is_empty() {
                                true => self.active_tag = None,
                                false => {
                                    populate_grid_view(&mut self.grid_view, &self.grid_entries, set);

                                    self.grid_view_entry_removed = true;
                                }
                            }

                            ui.close();
                        }
                    },
                    (_tag_button_clicked @ true, _entry_is_tagged @ false) => _ = set.insert(self.grid_entry_i),
                    (_tag_button_clicked @ false, _entry_is_tagged @ true) => _ = tag_button_resp.highlight(),
                    _ => ()
                }
            } }
        })
    }

    fn details_view(&mut self, ui: &mut egui::Ui) {
        let (image_file_name, image_states) = self.grid_entries[self.details_grid_entry_i].image_file_name_i
            .and_then(|image_file_name_i| self.images.get_index_mut(image_file_name_i))
            .unzip();

        // Subdivisions
        let middle_subd_width = 2.0 * FRAME_MARGIN + SEPARATOR_WIDTH;
        let side_subd_width = ((ui.available_width() - middle_subd_width) / 2.0).floor();
        let subd_height = ui.available_height();

        let (total_alloc_rect,
            total_alloc_resp
        ) = ui.allocate_exact_size(ui.available_size(), egui::Sense::click_and_drag());

        // Image
        let left_subd_rect = egui::Rect::from_min_max(
            total_alloc_rect.left_top(),
            total_alloc_rect.left_top() + [side_subd_width, subd_height].into()
        );
        let image_rect = egui::Rect::from_center_size(
            left_subd_rect.center(),
            self.details_cell_size
        );

        ui.scope_builder(egui::UiBuilder::new().max_rect(image_rect), |ui| {
            let details_state = image_states.map(|states| &mut states.details);
            let dir_name = self.grid_entries[self.details_grid_entry_i].stem.as_ref();

            match details_state {
                Some(details_state) => try_add_image(ui, details_state, image_file_name.unwrap().as_ref(), dir_name),
                None => add_label(ui, dir_name)
            }
        });

        // Separator
        let middle_subd_rect = egui::Rect::from_min_max(
            left_subd_rect.right_top(),
            left_subd_rect.right_top() + [middle_subd_width, subd_height].into()
        );

        ui.scope_builder(egui::UiBuilder::new().max_rect(middle_subd_rect), |ui| {
            ui.with_layout(egui::Layout::centered_and_justified(egui::Direction::LeftToRight), |ui| {
                ui.separator();
            });
        });

        // Dir entries
        let right_subd_rect = egui::Rect::from_min_max(
            middle_subd_rect.right_top(),
            middle_subd_rect.right_top() + [side_subd_width, subd_height].into()
        );

        ui.scope_builder(egui::UiBuilder::new().max_rect(right_subd_rect), |ui| {
            egui::ScrollArea::vertical()
                .wheel_scroll_multiplier([1.0, self.scroll_multiplier].into())
                .show(ui, |ui| {
                    let button_height = ui.spacing().interact_size.y;
                    let button_spacing = ui.spacing().item_spacing[1];
                    #[allow(clippy::cast_precision_loss)]
                    let button_count = self.details_dir_entries.len() as f32;
                    let buttons_height = button_count * (button_height + button_spacing);

                    let remaining_space = ui.available_height() - buttons_height;
                    let top_padding = (remaining_space / 2.0).floor().max(0.0);

                    ui.with_layout(egui::Layout::top_down_justified(egui::Align::Center), |ui| {
                        ui.add_space(top_padding);

                        if self.details_dir_entries.is_empty() {
                            ui.take_available_space();
                        } else {
                            self.dir_entries(ui);
                        }
                    });
                });
        });

        self.details_context_menu(&total_alloc_resp);
    }

    fn dir_entries(&mut self, ui: &mut egui::Ui) {
        egui::Frame::group(ui.style())
            .stroke(egui::Stroke::NONE)
            .inner_margin(egui::Margin::ZERO)
            .show(ui, |ui| {
                self.details_button_resps.clear();
                self.details_button_resps.extend(
                    self.details_dir_entries.iter()
                        .map(|info| ui.button(info.stem.as_str()))
                );

                #[derive(Default)]
                struct Deferred {
                    hovered_i: Option<usize>,
                    push_dir: Option<PathBuf>
                }

                let Deferred { hovered_i, push_dir } = self.details_button_resps.iter()
                    .zip(&self.details_dir_entries)
                    .enumerate()
                    .fold(Deferred::default(), |mut deferred, (i, (resp, dir_entry_info))| {
                        if resp.hovered() {
                            deferred.hovered_i = Some(i);
                        }

                        if resp.clicked() {
                            match dir_entry_info.file_kind {
                                FileKind::Dir => deferred.push_dir = Some(dir_entry_info.path.clone()),
                                _ => {
                                    let discord_info = self.discord_enabled.then_some(self.make_discord_info(dir_entry_info));

                                    open_media(
                                        dir_entry_info.path.clone(),
                                        dir_entry_info.file_kind,
                                        self.maintain_sample_rate,
                                        self.override_glsl_shaders,
                                        discord_info,
                                        self.discord_display_kind,
                                        self.error_sx.clone()
                                    );
                                }
                            }
                        }

                        deferred
                    });

                if let Some(i) = hovered_i { self.details_hovered_dir_entry_i = i }
                if let Some(dir) = push_dir { self.push_dir(dir) }
            });
    }

    fn push_dir(&mut self, dir: PathBuf) {
        replace_dir_entries(&mut self.details_dir_entries, &dir);

        self.details_hovered_dir_entry_i = 0;
        self.details_levels.push(dir);
    }

    fn pop_dir(&mut self) {
        self.details_hovered_dir_entry_i = 0;

        if self.details_levels.is_empty() {
            self.view_kind = ViewKind::Grid;

            return
        }

        self.details_levels.pop();
        if self.details_levels.is_empty() {
            replace_dir_entries(&mut self.details_dir_entries, &self.grid_entries[self.details_grid_entry_i].path);
        } else {
            let dir = self.details_levels.last().unwrap();
            replace_dir_entries(&mut self.details_dir_entries, dir);
        }
    }

    fn make_discord_info(&self, dir_entry_info: &DirEntryInfo) -> config::DiscordActivityInfo {
        let dir_name = &self.grid_entries[self.details_grid_entry_i].stem;

        config::DiscordActivityInfo {
            app_id: match self.discord_watching {
                Watching::Movie => self.discord_app_ids.movies.unwrap().to_string(), // App ID is Some when Discord is enabled
                Watching::TV => self.discord_app_ids.tv.unwrap().to_string(),
                Watching::Words => self.discord_app_ids.words.unwrap().to_string()
            },
            activity: config::DiscordActivity::Watching,
            details: match self.discord_details.is_empty() {
                true => match self.discord_watching {
                    Watching::TV => dir_name.to_string(),
                    _ => dir_entry_info.stem.clone()
                },
                false => self.discord_details.clone()
            },
            state: self.discord_watching.eq(&Watching::TV).then(|| {
                match self.discord_state.is_empty() {
                    true => dir_entry_info.stem.clone(),
                    false => self.discord_state.clone()
                }
            }),
            large_image: match self.discord_watching {
                Watching::TV => Some(to_discord_asset_name(dir_name)),
                _ => Some(to_discord_asset_name(dir_entry_info.stem.as_str()))
            }
        }
    }

    fn details_context_menu(&mut self, total_alloc_resp: &egui::Response) {
        let close_behaviour = match total_alloc_resp.clicked() {
            true => egui::PopupCloseBehavior::CloseOnClickOutside,
            false => egui::PopupCloseBehavior::IgnoreClicks
        };

        egui::Popup::context_menu(total_alloc_resp)
            .close_behavior(close_behaviour)
            .show(|ui| {
                ui.add_enabled_ui(!self.details_dir_entries.is_empty(), |ui| {
                    ui.checkbox(&mut self.maintain_sample_rate, "Maintain sample rate");
                    ui.add_enabled(self.enable_override_glsl_shaders_checkbox, egui::Checkbox::new(&mut self.override_glsl_shaders, "Override GLSL shaders"));
                    ui.menu_button("Discord Rich Presence", |ui| self.discord_menu(ui));
                });
            });
    }

    fn discord_menu(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            let can_enable_discord = match self.discord_watching {
                Watching::Movie => self.discord_app_ids.movies.is_some(),
                Watching::TV => self.discord_app_ids.tv.is_some(),
                Watching::Words => self.discord_app_ids.words.is_some()
            };
            self.discord_enabled &= can_enable_discord;

            ui.add_enabled(can_enable_discord, egui::Checkbox::new(&mut self.discord_enabled, "Enable"));

            ui.separator();

            if ui.add_enabled(self.discord_app_ids.tv.is_some(), egui::RadioButton::new(self.discord_watching == Watching::TV, "TV")).clicked() {
                self.discord_watching = Watching::TV;
            };
            if ui.add_enabled(self.discord_app_ids.movies.is_some(), egui::RadioButton::new(self.discord_watching == Watching::Movie, "Movie")).clicked() {
                self.discord_watching = Watching::Movie;
            };
            if ui.add_enabled(self.discord_app_ids.words.is_some(), egui::RadioButton::new(self.discord_watching == Watching::Words, "Words")).clicked() {
                self.discord_watching = Watching::Words;
            };
        });

        ui.shrink_width_to_current();

        let grid = egui::Grid::new("grid").num_columns(2);
        grid.show(ui, |ui| {
            ui.label("Details");

            let dir_entry_stem = self.details_dir_entries[self.details_hovered_dir_entry_i].stem.as_str();

            let details_hint_text = match self.discord_watching {
                Watching::TV => self.grid_entries[self.details_grid_entry_i].stem.as_ref(),
                _ => dir_entry_stem
            };
            let details_text_edit = egui::TextEdit::singleline(&mut self.discord_details).hint_text(details_hint_text);

            ui.add(details_text_edit);

            ui.end_row();

            if self.discord_watching == Watching::TV {
                ui.label("State");

                let state_text_edit = egui::TextEdit::singleline(&mut self.discord_state).hint_text(dir_entry_stem);

                ui.add(state_text_edit);
            }
        });
    }
}

pub fn begin(kind: Kind) -> Res<(), { loc_var!(Gui) }> {
    let icon_data = eframe::icon_data::from_png_bytes(include_bytes!("../../../assets/icon.png"))?;
    let mut viewport = egui::ViewportBuilder::default()
        .with_icon(icon_data);

    if let Kind::MediaBrowser = kind {
        let config = config::get().read()?;

        if let Some(size) = config.media_browser.as_ref().and_then(|mb| mb.window_inner_size) {
            #[allow(clippy::cast_precision_loss)]
            let (width, height) = (size.width as f32, size.height as f32);

            viewport = viewport.with_inner_size([width, height]);
        }
    }

    let dx12_backend_options = wgpu::Dx12BackendOptions {
        presentation_system: wgpu::wgt::Dx12SwapchainKind::DxgiFromVisual,
        ..default!()
    };
    let wgpu_setup = egui_wgpu::WgpuSetup::CreateNew(egui_wgpu::WgpuSetupCreateNew {
        instance_descriptor: wgpu::InstanceDescriptor {
            backends: wgpu::Backends::DX12,
            backend_options: wgpu::BackendOptions {
                dx12: dx12_backend_options,
                ..default!()
            },
            ..default!()
        },
        power_preference: wgpu::PowerPreference::HighPerformance,
        ..default!()
    });
    let native_options = eframe::NativeOptions {
        viewport,
        renderer: eframe::Renderer::Wgpu,
        wgpu_options: egui_wgpu::WgpuConfiguration {
            present_mode: wgpu::PresentMode::Fifo,
            desired_maximum_frame_latency: Some(1),
            wgpu_setup,
            ..default!()
        },
        centered: true,
        ..default!()
    };

    eframe::run_native(
        "Ogos",
        native_options,
        Box::new(|cctx| {
            cctx.egui_ctx.options_mut(|options| options.reduce_texture_memory = true);
            cctx.egui_ctx.set_pixels_per_point(1.0);
            cctx.egui_ctx.style_mut(|style| {
                let factor = 1.5;

                style.spacing.interact_size = (style.spacing.interact_size * factor).round();
                style.spacing.button_padding = (style.spacing.button_padding * factor).round();
                style.spacing.item_spacing = (style.spacing.item_spacing * factor).round();
                style.spacing.icon_spacing = (style.spacing.icon_spacing * factor).round();
                style.spacing.icon_width = (style.spacing.icon_width * factor).round();
                style.spacing.icon_width_inner = (style.spacing.icon_width_inner * factor).round();

                for (_, font_id) in style.text_styles.iter_mut() {
                    font_id.size = (font_id.size * factor).round();
                }
            });

            let app: Box<dyn eframe::App> = match kind {
                Kind::Info { msg } => {
                    cctx.egui_ctx.style_mut(|style| style.wrap_mode = Some(egui::TextWrapMode::Wrap));

                    Box::new(Info::new(msg))
                },
                Kind::MediaBrowser => Box::new(MediaBrowser::new(&cctx.egui_ctx)?)
            };

            Ok(app)
        })
    )?;

    Ok(())
}
