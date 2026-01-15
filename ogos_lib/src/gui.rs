use crate::{
    common::*,
    config::{self, *},
    discord,
    video
};
use ogos_err::*;

use concat_string::*;
use discord_rich_presence::*;
use eframe::{
    egui,
    egui_wgpu,
    wgpu
};
use indexmap::*;
use log::*;
use rayon::*;
use serde::*;
use std::{
    collections::*,
    f64::consts::PI,
    fs::{self, *},
    io::Read,
    path::*,
    rc::*,
    sync::*,
    thread,
    time::*
};
use tokio::sync::oneshot::{self, error::*};
use windows::Win32::{
    Foundation::POINT,
    UI::WindowsAndMessaging::*
};

const CELL_STROKE: egui::Stroke = egui::Stroke { width: 3.0, color: egui::Color32::from_rgb(250, 246, 235) };
const CHUNK_BYTE_COUNT: u64 = 500 * 1024;
const GRID_IMAGE_SPACING: egui::Vec2 = egui::vec2(30.0, 30.0);
const FRAME_MARGIN: f32 = 15.0;
const SEPARATOR_WIDTH: f32 = 2.0;

#[derive(Clone)]
struct DirEntryInfo {
    path: PathBuf,
    stem: String,
    file_kind: FileKind
}

pub(crate) enum Kind {
    Discord { name: String, discord_info: DiscordInfoView<'static> },
    MediaBrowser
}

#[derive(Default, Deserialize, PartialEq)]
enum Watching {
    Movie,
    #[default]
    TV,
    Words
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

pub(crate) struct Discord {
    name: String,
    discord_info: DiscordInfoView<'static>
}
impl Discord {
    pub(crate) fn new(_cctx: &eframe::CreationContext<'_>, name: String, discord_info: DiscordInfoView<'static>) -> Self {
        Self {
            name,
            discord_info
        }
    }
}
impl eframe::App for Discord {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        ctx.set_pixels_per_point(1.0);

        egui::CentralPanel::default().show(ctx, |ui| {
            egui::ScrollArea::new([false, true]).show(ui, |ui| {
                ui.heading(&self.name);

                ui.separator();

                let text_edit = egui::TextEdit::singleline(&mut self.discord_info.details).desired_width(f32::INFINITY);
                let details = ui.label("Details");

                ui.add(text_edit).labelled_by(details.id);
            });
        });
    }
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

fn ferry_image(info: ImageFerryInfo) -> Res1<()> {
    let ImageFerryInfo {
        src_path,
        image_state_sx,
        resize
    } = info;

    match resize {
        Some(resize) => {
            let ImageFerryResize { dst_path, dst_size } = resize;

            let (color_image, pixels) = load_and_resize_color_image(src_path.as_path(), dst_size).unwrap();
            let color_image_size = color_image.size;

            image_state_sx.send(Ok(color_image)).unwrap();

            let dst_file = fs::File::create(dst_path)?;
            let dst_encoder = image::codecs::webp::WebPEncoder::new_lossless(&dst_file);

            dst_encoder.encode(pixels.as_slice(), color_image_size[0] as u32, color_image_size[1] as u32, image::ExtendedColorType::Rgba8)?;
        },
        None => {
            let color_image = load_color_image(src_path.as_path())?;
            image_state_sx.send(Ok(color_image)).unwrap();
        }
    }

    Ok(())
}

fn load_image(path: &Path) -> ResVar<image::ImageBuffer<image::Rgba<u8>, Vec<u8>>> {
    let image = image::open(path)?;

    Ok(match image {
        image::DynamicImage::ImageRgba8(image) => image,
        _ => image.to_rgba8()
    })
}

fn load_color_image(path: &Path) -> ResVar<egui::ColorImage> {
    let image = load_image(path)?;

    let (src_width, src_height) = image.dimensions();
    let src_pixels = image.as_raw();

    let color_image = egui::ColorImage::from_rgba_unmultiplied([src_width as usize, src_height as usize], src_pixels);

    Ok(color_image)
}

fn load_and_resize_color_image(path: &Path, size: egui::Vec2) -> Res1<(egui::ColorImage, Vec<u8>)> {
    use rgb::FromSlice;

    let image = load_image(path)?;
    let (src_width, src_height) = image.dimensions();
    #[allow(clippy::cast_precision_loss)]
    let aspect_ratio_h = src_height as f32 / src_width as f32;

    let src_width = src_width as usize;
    let src_height = src_height as usize;
    let dst_width = size[0] as usize;
    let dst_height = (size[0] * aspect_ratio_h).round() as usize;

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
    let src_pixels = image.as_raw();
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

    Ok((color_image, dst_pixels))
}

unsafe fn open_media(path: PathBuf, file_kind: FileKind, maintain_sample_rate: bool, use_glsl_shaders: bool, discord_info: Option<DiscordInfo>) {
    thread::spawn(move || {
        (|| -> Res<()> {
            let ipc_client = discord_info.as_ref().map(|discord_info| -> Res<_> {
                let mut ipc_client = DiscordIpcClient::new(discord_info.client_id.as_str());

                discord::begin(&mut ipc_client, &discord_info.as_view())?;

                Ok(ipc_client)
            })
            .transpose()?;

            match file_kind {
                FileKind::Vid => video::launch_mpv(path.as_path(), maintain_sample_rate.into(), use_glsl_shaders)?,
                _ => opener::open(path.as_path())?
            }

            if let Some(mut ipc_client) = ipc_client {
                ipc_client.clear_activity()?;
                ipc_client.close()?;
            }

            Ok(())
        })()
        .unwrap_or_else(|err| {
            error!("{}: failed to launch media: {}", module_path!(), err);
        });
    });
}

fn save_image(path: PathBuf, pixels: &[u8], width: u32, height: u32) -> Res<()> {
    let image_file = fs::File::create(path)?;
    let encoder = image::codecs::webp::WebPEncoder::new_lossless(image_file);
    encoder.encode(pixels, width, height, image::ExtendedColorType::Rgba8)?;

    Ok(())
}

fn add_image(ui: &mut egui::Ui, tex: &egui::TextureHandle) -> egui::Response {
    let image = egui::Image::new(tex).sense(egui::Sense::click_and_drag()).fit_to_exact_size(tex.size_vec2());

    ui.add(image)
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
    ui.allocate_response(ui.available_size(), egui::Sense::hover())
}

fn try_add_image(ui: &mut egui::Ui, image_state: &mut ImageState, name_tex: &str, label: &str) -> egui::Response {
    match image_state {
        ImageState::Ready(tex) => add_image(ui, tex),
        ImageState::Pending(rx) => {
            let recvd = match rx.try_recv() {
                Ok(res) => res,
                Err(TryRecvError::Empty) => return alloc_hover_response(ui),
                Err(TryRecvError::Closed) => Err(())
            };

            match recvd {
                Ok(color_image) => {
                    let tex = ui.ctx().load_texture(name_tex, color_image, default!());
                    let resp = add_image(ui, &tex);
                    *image_state = ImageState::Ready(tex);

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
    }
}

fn stroke_rect(ui: &mut egui::Ui, rect: egui::Rect) {
    ui.painter().rect_stroke(rect, 0.0, CELL_STROKE, egui::StrokeKind::Outside);
}

fn stroke_rect_painter(painter: egui::Painter, rect: egui::Rect) {
    painter.rect_stroke(rect, 0.0, CELL_STROKE, egui::StrokeKind::Outside);
}

fn update_active_view(active_view: &mut Vec<usize>, set: &BTreeSet<usize>) {
    active_view.clear();
    active_view.extend(set.iter().cloned());
}

#[derive(Default)]
enum ImageState {
    #[default]
    None,
    Pending(oneshot::Receiver<Result<egui::ColorImage, ()>>),
    Ready(egui::TextureHandle),
    Failed
}

#[derive(Default)]
enum ViewKind {
    #[default]
    Grid,
    Details
}

#[derive(Serialize, Deserialize)]
struct CacheEntryInfo {
    #[serde(rename = "image")]
    image_file_name_i: Option<usize>,
    hash: Option<Arc<str>>,
    #[serde(default)]
    tags: Vec<usize>
}

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

#[derive(Default)]
struct DetailsInfo {
    image_file_name: Option<Arc<str>>,
    dir_name: Rc<str>,
    dir_entries: Vec<DirEntryInfo>
}

#[derive(Eq, Ord, PartialEq, PartialOrd)]
struct GridEntryInfo {
    path: PathBuf,
    stem: Rc<str>,
    image_file_name_i: Option<usize>,
    hash: Option<Arc<str>>
}

struct ImageDirs {
    base: PathBuf,
    grid: PathBuf,
    details: PathBuf
}

struct ImageFerryInfo {
    src_path: PathBuf,
    image_state_sx: oneshot::Sender<Result<egui::ColorImage, ()>>,
    resize: Option<ImageFerryResize>
}

struct ImageFerryResize {
    dst_path: PathBuf,
    dst_size: egui::Vec2
}

#[derive(Default)]
struct ImageStates {
    grid_state: ImageState,
    details_state: ImageState
}

struct MenuPointerState {
    pointer_contained: bool,
    entry_clicked: bool
}

#[derive(Default)]
struct PointerContained(bool);

struct MediaBrowser<'a> {
    _thread_pool: Arc<rayon::ThreadPool>,
    image_dirs: Arc<ImageDirs>,
    images: IndexMap<Arc<str>, ImageStates>,
    hash_rx: mpsc::Receiver<(usize, Arc<str>)>,
    cache_path: PathBuf,
    cache: Cache,
    cached_images_to_remove: Vec<Rc<str>>,
    frame: egui::Frame,
    view_kind: ViewKind,
    grid_entries: Vec<GridEntryInfo>,
    grid_cell_size: egui::Vec2,
    tags: BTreeMap<Rc<str>, BTreeSet<usize>>,
    active_tag: Option<Rc<str>>,
    active_view: Vec<usize>, // Indices into grid_entries
    removed_entry_from_active_view: bool,
    selected_cell: Option<usize>,
    tag_edit: String,
    show_filter_win: bool,
    filter_win_stamp: Option<Instant>,
    filter_win_cursor_checked: bool,
    details_info: DetailsInfo,
    details_cell_size: egui::Vec2,
    details_button_resps: Vec<egui::Response>,
    vscroll_multiplier: f32,
    hovered_details_entry_i: usize,
    maintain_sample_rate: bool,
    use_glsl_shaders: bool,
    discord_app_ids: DiscordAppIds<'a>,
    discord_enabled: bool,
    discord_watching: Watching,
    discord_details: String,
    discord_state: String,
    discord_display_kind: DiscordDisplayKind
}
impl<'a> eframe::App for MediaBrowser<'a> {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default()
            .frame(self.frame)
            .show(ctx, |ui: &mut egui::Ui| Self::central_panel(self, ui));

        // Close and save cache to file
        if ctx.input(|state| state.viewport().close_requested()) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));

            for image_file_name in self.cached_images_to_remove.drain(..) {
                let paths = [
                    self.image_dirs.grid.join(image_file_name.as_ref()).with_extension("webp"),
                    self.image_dirs.details.join(image_file_name.as_ref()).with_extension("webp")
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

            for (grid_entry_i, hash) in self.hash_rx.iter() {
                self.grid_entries[grid_entry_i].hash = Some(hash);
            }

            for (i, info) in self.grid_entries.drain(..).enumerate() {
                self.cache.entries.insert(
                    info.path,
                    CacheEntryInfo {
                        image_file_name_i: info.image_file_name_i,
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
}
impl<'a> MediaBrowser<'a> {
    fn new(ctx: &egui::Context) -> Res<Self> {
        // let now = now!();

        // let elapsed = now.elapsed();
        // info!("elapsed = {}", elapsed.as_micros());

        let thread_pool = Arc::new(rayon::ThreadPoolBuilder::new()
            .num_threads(thread::available_parallelism()?.get())
            .build()?);
        let (hash_sx, hash_rx) = mpsc::channel();
        let (ferry_sx, ferry_rx) = mpsc::channel();

        let current_exe_dir = CURRENT_EXE_DIR.get().unwrap();
        let image_dir = current_exe_dir.join("images");
        let grid_image_dir = image_dir.join("grid");
        let details_image_dir = image_dir.join("details");
        let image_dirs = Arc::new(ImageDirs {
            base: image_dir,
            grid: grid_image_dir,
            details: details_image_dir
        });

        let cache_path = image_dirs.base.join("cache").with_extension("json");
        let cache_slc = fs::read(&cache_path)?;
        let mut cache: Cache = serde_json::from_slice(&cache_slc)?;
        let mut cached_images_to_remove = Vec::new();

        let mut tags = cache.tags.iter()
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

        let config = config::get().read()?;
        let (media_dirs,
            grid_cell_width,
            details_cell_width,
            vscroll_multiplier
        ) = config.media_browser.as_ref()
            .map(|media_browser_config| {
                #[allow(clippy::cast_precision_loss)]
                (
                    &media_browser_config.dirs,
                    media_browser_config.grid_cell_width.next_multiple_of(2) as f32,
                    media_browser_config.details_cell_width.next_multiple_of(2) as f32,
                    media_browser_config.vscroll_multiplier
                )
            })
            .ok_or(ErrVar::MissingConfigKey { name: config::MediaBrowser::NAME })?;
        let grid_cell_size = egui::vec2(grid_cell_width, grid_cell_width * 1.5);
        let details_cell_size = egui::vec2(details_cell_width, details_cell_width * 1.5);
        let grid_cell_size_changed = cache.grid_cell_size != grid_cell_size;
        let details_cell_size_changed = cache.details_cell_size != details_cell_size;

        let mut grid_entries = media_dirs.iter()
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
                    let cache_entry_info = cache.entries.get_mut(&path);

                    let try_get_image_file_name_i = || {
                        let exts = [ ".jpg", ".jpeg", ".png", ".webp"];

                        for ext in exts {
                            let attempt = concat_string!(stem, ext);

                            if let Some(image_file_name_i) = images.get_index_of(attempt.as_str()) {
                                return Some(image_file_name_i)
                            }
                        }

                        None
                    };
                    let image_file_name_i = match cache_entry_info.as_ref() {
                        Some(info) => {
                            let image_file_name = info.image_file_name_i
                                .and_then(|image_file_name_i| cache.images.get_index(image_file_name_i));
                            let image_file_name_i = image_file_name
                                .and_then(|image_file_name| images.get_index_of(image_file_name.as_ref()));

                            image_file_name_i.or_else(|| {
                                if let Some(image_file_name) = image_file_name {
                                    // Entry was cached but its image was moved or deleted - remove cached images
                                    cached_images_to_remove.push(image_file_name.clone());
                                }

                                try_get_image_file_name_i()
                            })
                        },
                        None => try_get_image_file_name_i()
                    };

                    let hash = cache_entry_info.and_then(|info| info.hash.clone());

                    Ok(Some(GridEntryInfo { path, stem, image_file_name_i, hash }))
                })
                .unwrap_or_else(|err| {
                    error!("{}: failed to read dir entry: {}", module_path!(), err);

                    None
                })
            })
            .collect::<Vec<_>>();
        grid_entries.sort_by(|a, b| a.stem.cmp(&b.stem));

        let discord_app_ids = config.discord.app_ids.clone();
        let discord_display_kind = config.discord.display_kind;
        drop(config);

        for (entry_i, info) in grid_entries.iter().enumerate() {
            // Fill tags
            if let Some(CacheEntryInfo { tags: tag_is, .. }) = cache.entries.get_mut(&info.path) {
                for tag_i in tag_is {
                    let tag = &cache.tags[*tag_i];
                    let set = tags.get_mut(tag);

                    if let Some(set) = set {
                        set.insert(entry_i); // Grid entries are sorted / indices are stable
                    }
                }
            }

            // Load/resize images
            if let Some(image_file_name_i) = info.image_file_name_i {
                let ctx = ctx.clone();
                let hash_sx = hash_sx.clone();
                let ferry_sx = ferry_sx.clone();
                let image_dirs = image_dirs.clone();
                let (image_file_name, ImageStates { grid_state, details_state }) = images.get_index_mut(image_file_name_i).unwrap();
                let image_file_name = image_file_name.clone();
                let expected_hash = info.hash.clone();
                let (grid_image_state_sx, grid_image_state_rx) = oneshot::channel();
                let (details_image_state_sx, details_image_state_rx) = oneshot::channel();
                *grid_state = ImageState::Pending(grid_image_state_rx);
                *details_state = ImageState::Pending(details_image_state_rx);

                thread_pool.spawn_fifo(move || {
                    (|| -> Res<()> {
                        let image_path = image_dirs.base.join(image_file_name.as_ref());
                        let grid_image_path = image_dirs.grid.join(image_file_name.as_ref()).with_extension("webp");
                        let details_image_path = image_dirs.details.join(image_file_name.as_ref()).with_extension("webp");

                        // Compute hash on chunk
                        let mut image_file = File::open(image_path.as_path())?;
                        let mut hasher = blake3::Hasher::new();
                        let mut chunk = [0_u8; CHUNK_BYTE_COUNT as usize];
                        image_file.read(&mut chunk)?;
                        let computed_hash = Arc::from(hasher.update(&chunk).finalize().to_hex().as_str());
                        let hash_mismatches = expected_hash.is_none_or(|expected_hash| expected_hash != computed_hash);

                        let load_grid_image = |grid_image_state_sx: oneshot::Sender<Result<egui::ColorImage, ()>>| {
                            match load_color_image(grid_image_path.as_path()) {
                                Ok(grid_image) => grid_image_state_sx.send(Ok(grid_image)).unwrap(),
                                Err(err) => {
                                    error!("{}: failed to load image: {}", module_path!(), err);

                                    grid_image_state_sx.send(Err(())).unwrap();
                                }
                            }
                        };
                        let resize_grid_image = |grid_image_path, grid_image_state_sx: oneshot::Sender<Result<egui::ColorImage, ()>>| {
                            match load_and_resize_color_image(image_path.as_path(), grid_cell_size) {
                                Ok((grid_image, grid_pixels)) => {
                                    let grid_image_width = grid_image.width() as u32;
                                    let grid_image_height = grid_image.height() as u32;

                                    grid_image_state_sx.send(Ok(grid_image)).unwrap();
                                    ctx.request_repaint();

                                    hash_sx.send((entry_i, computed_hash)).unwrap();

                                    save_image(grid_image_path, grid_pixels.as_slice(), grid_image_width, grid_image_height)
                                        .unwrap_or_else(|err| error!("{}: failed to save image: {}: {}", module_path!(), image_file_name, err));
                                },
                                Err(err) => {
                                    error!("{}: failed to load and resize image: {}: {}", module_path!(), image_file_name, err);

                                    grid_image_state_sx.send(Err(())).unwrap();
                                }
                            }
                        };
                        let ferry_load_details_image = |details_image_path, details_image_state_sx| {
                            ferry_sx.send((
                                image_file_name.clone(),
                                ImageFerryInfo {
                                    src_path: details_image_path,
                                    image_state_sx: details_image_state_sx,
                                    resize: None
                                }
                            ))
                            .unwrap();
                        };
                        let ferry_resize_details_image = |image_path, details_image_path, details_image_state_sx| {
                            ferry_sx.send((
                                image_file_name.clone(),
                                ImageFerryInfo {
                                    src_path: image_path,
                                    image_state_sx: details_image_state_sx,
                                    resize: Some(ImageFerryResize {
                                        dst_path: details_image_path,
                                        dst_size: details_cell_size
                                    })
                                }
                            ))
                            .unwrap();
                        };

                        if hash_mismatches {
                            resize_grid_image(grid_image_path, grid_image_state_sx);
                            ferry_resize_details_image(image_path, details_image_path, details_image_state_sx);

                            return Ok(())
                        }
                        match grid_cell_size_changed {
                            true => resize_grid_image(grid_image_path, grid_image_state_sx),
                            false => load_grid_image(grid_image_state_sx)
                        }
                        match details_cell_size_changed {
                            true => ferry_resize_details_image(image_path, details_image_path, details_image_state_sx),
                            false => ferry_load_details_image(details_image_path, details_image_state_sx)
                        }

                        Ok(())
                    })()
                    .unwrap_or_else(|err| {
                        error!("{}: failed to ferry image: {}: {}", module_path!(), image_file_name, err);
                    });
                });
            }
        }

        drop(ferry_sx);
        let thread_pool_ = thread_pool.clone();
        thread::spawn(move || {
            for (image_file_name, info) in ferry_rx {
                thread_pool_.spawn_fifo(move || {
                    ferry_image(info).unwrap_or_else(|err| {
                        error!("{}: failed to ferry image: {}: {}", module_path!(), image_file_name, err);
                    });
                });
            }
        });

        let frame = egui::Frame::central_panel(&ctx.style()).inner_margin(
            egui::Margin::symmetric(FRAME_MARGIN as i8, FRAME_MARGIN as i8)
        );

        Ok(Self {
            _thread_pool: thread_pool,
            image_dirs,
            images,
            hash_rx,
            cache_path,
            cache,
            cached_images_to_remove,
            frame,
            view_kind: ViewKind::Grid,
            grid_entries,
            grid_cell_size,
            tags,
            active_tag: default!(),
            active_view: default!(),
            removed_entry_from_active_view: default!(),
            selected_cell: default!(),
            tag_edit: default!(),
            show_filter_win: default!(),
            filter_win_stamp: default!(),
            filter_win_cursor_checked: default!(),
            details_info: default!(),
            details_cell_size,
            details_button_resps: Vec::with_capacity(24),
            hovered_details_entry_i: default!(),
            vscroll_multiplier,
            maintain_sample_rate: default!(),
            use_glsl_shaders: default!(),
            discord_app_ids,
            discord_enabled: default!(),
            discord_watching: default!(),
            discord_details: default!(),
            discord_state: default!(),
            discord_display_kind
        })
    }

    fn central_panel(&mut self, ui: &mut egui::Ui) {
        match self.view_kind {
            ViewKind::Grid => {
                self.filter_win(ui);
                self.grid_view(ui);
            },
            ViewKind::Details => self.details_view(ui)
        }
    }

    fn filter_win(&mut self, ui: &mut egui::Ui) {
        let max_rect = ui.max_rect();
        let filter_win_rect = egui::Rect::from_min_size(
            max_rect.min,
            [250.0, (max_rect.height() - FRAME_MARGIN).max(0.0)].into()
        );

        let filter_win_resp = egui::Window::new("filter_win")
            .fixed_rect(filter_win_rect)
            .title_bar(false)
            .fade_in(true)
            .fade_out(true)
            .open(&mut self.show_filter_win)
            .show(ui.ctx(), |ui| {
                ui.with_layout(egui::Layout::top_down_justified(egui::Align::Center), |ui| {
                    ui.heading("Tags");

                    ui.separator();

                    let all_button_resp = ui.button("All");

                    if all_button_resp.clicked() {
                        self.active_tag = None;
                    }
                    if self.active_tag.is_none() {
                        all_button_resp.highlight();
                    }

                    for (tag, set) in self.tags.iter() {
                        if !set.is_empty() {
                            let tag_button_resp = ui.button(tag.as_ref());

                            if tag_button_resp.clicked() {
                                update_active_view(&mut self.active_view, set);

                                self.active_tag = Some(tag.clone());
                                self.selected_cell = None;
                            }
                            if let Some(active_tag) = self.active_tag.as_ref() && active_tag == tag {
                                tag_button_resp.highlight();
                            }
                        }
                    }

                    ui.take_available_space();
                });
            });

        let hover_pos = ui.ctx().input(|state| state.pointer.hover_pos());
        match hover_pos {
            Some(hover_pos) => {
                self.filter_win_cursor_checked = false;

                if let Some(resp) = filter_win_resp.as_ref() {
                    match resp.response.contains_pointer() {
                        true => self.filter_win_stamp = Some(now!()),
                        false => {
                            if hover_pos.x > resp.response.rect.right() {
                                self.filter_win_stamp = None;
                                self.show_filter_win = false;
                            }
                        }
                    }
                }
            }
            None => {
                if !self.filter_win_cursor_checked {
                    let mut cursor_pos = POINT::default();
                    unsafe { if GetCursorPos(&mut cursor_pos).is_err() {
                        return
                    } }
                    #[allow(clippy::cast_precision_loss)]
                    let cursor_pos = egui::pos2(cursor_pos.x as f32, cursor_pos.y as f32);

                    if let Some(inner_rect) = ui.ctx().input(|state| state.viewport().inner_rect) {
                        let cursor_catch_rect = egui::Rect::everything_left_of(inner_rect.left());

                        if cursor_catch_rect.contains(cursor_pos) {
                            self.filter_win_stamp = Some(now!());
                            self.show_filter_win = true;
                        }

                        self.filter_win_cursor_checked = true;
                    }
                }
            }
        }

        if let Some(filter_win_stamp) = self.filter_win_stamp && filter_win_stamp.elapsed() > Duration::from_secs(3) {
            self.filter_win_stamp = None;
            self.show_filter_win = false;
        }
    }

    fn grid_view(&mut self, ui: &mut egui::Ui) {
        ui.with_layout(egui::Layout::top_down(egui::Align::Center), |ui| {
            egui::ScrollArea::new([false, true])
                .auto_shrink(false)
                .scroll_source(egui::scroll_area::ScrollSource::SCROLL_BAR | egui::scroll_area::ScrollSource::MOUSE_WHEEL)
                .wheel_scroll_multiplier([1.0, self.vscroll_multiplier].into())
                .show(ui, |ui| {
                    ui.add_space(FRAME_MARGIN);

                    ui.spacing_mut().item_spacing = GRID_IMAGE_SPACING;

                    let cell_space_x = self.grid_cell_size.x + GRID_IMAGE_SPACING.x;
                    let cell_space_y = self.grid_cell_size.y + GRID_IMAGE_SPACING.y;

                    let table_cell_count = match self.active_tag.as_ref() {
                        Some(_) => self.active_view.len(),
                        None => self.grid_entries.len()
                    };
                    let row_cell_count = (ui.available_width().div_euclid(cell_space_x) as usize)
                        .clamp(1, table_cell_count);
                    let table_row_count = table_cell_count.div_ceil(row_cell_count);

                    #[allow(clippy::cast_precision_loss)]
                    let (table_width,table_height) = (
                        row_cell_count as f32 * cell_space_x - GRID_IMAGE_SPACING.x, // First cell isn't initially offset by item spacing
                        table_row_count as f32 * cell_space_y - GRID_IMAGE_SPACING.y
                    );
                    let remaining_space = (ui.available_height() - table_height).max(0.0);
                    let top_padding = remaining_space / 2.0;

                    let available_rect = ui.available_rect_before_wrap();
                    let table_rect_min_x = (available_rect.center().x - table_width / 2.0).floor();
                    let table_rect = egui::Rect::from_min_size(
                        [table_rect_min_x, available_rect.top() + top_padding].into(),
                        [table_width, table_height].into(),
                    );

                    ui.scope_builder(egui::UiBuilder::new().max_rect(table_rect), |ui| {
                        self.table(ui, table_cell_count, table_row_count, row_cell_count);
                    });

                    ui.ctx().input(|state| {
                        if state.pointer.button_released(egui::PointerButton::Extra1) {
                            self.active_tag = None;
                        }
                    });
                });
        });
    }

    fn table(&mut self, ui: &mut egui::Ui, mut table_cell_count: usize, table_row_count: usize, row_cell_count: usize) {
        egui_extras::TableBuilder::new(ui)
            .striped(false)
            .vscroll(false)
            .cell_layout(egui::Layout::top_down(egui::Align::Center))
            .columns(egui_extras::Column::initial(self.grid_cell_size.x).at_most(self.grid_cell_size.x), row_cell_count)
            .body(|body| {
                body.rows(self.grid_cell_size.y, table_row_count, |mut row| {
                    let mut cell_i = row.index() * row_cell_count + row.col_index();

                    while row.col_index() < row_cell_count && cell_i < table_cell_count {
                        row.col(|ui| {
                            ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                                self.cell(ui, cell_i);
                            });
                        });

                        // Cell entry might have its tag removed while view is active and table is still being updated
                        match self.removed_entry_from_active_view {
                            true => {
                                table_cell_count -= 1;
                                self.removed_entry_from_active_view = false;
                            },
                            false => cell_i += 1
                        }
                    }
                });
            });
    }

    fn cell(&mut self, ui: &mut egui::Ui, cell_i: usize) {
        let grid_entry_i = self.active_tag.as_ref().map(|_| self.active_view[cell_i]).unwrap_or(cell_i);
        let grid_entry_info = &self.grid_entries[grid_entry_i];
        let mut image_info = grid_entry_info.image_file_name_i.and_then(|image_file_name_i| self.images.get_index_mut(image_file_name_i));

        let cell_resp = match image_info.as_mut() {
            Some((image_file_name, ImageStates { grid_state, .. })) => {
                try_add_image(ui, grid_state, image_file_name.as_ref(), grid_entry_info.stem.as_ref())
            },
            None => add_label(ui, grid_entry_info.stem.as_ref())
        };

        if cell_resp.clicked() && grid_entry_info.path.is_dir() {
            let dir_entries = || -> Res<_> {
                let read_dir = grid_entry_info.path.read_dir()
                    .inspect_err(|err| error!("{}: failed to read dir: {}", module_path!(), err))?;

                Ok(read_dir.filter_map(|dir_entry| {
                    dir_entry.map_err(into!()).and_then(|dir_entry| -> Res<_> {
                        let path = dir_entry.path();
                        let stem = path.get_file_stem()?.to_string();
                        let file_kind = path.get_file_kind()?;

                        Ok(DirEntryInfo { path, stem, file_kind })
                    })
                    .inspect_err(|err|  error!("{}: failed to read dir entry: {}", module_path!(), err))
                    .ok()
                })
                .collect::<Vec<_>>())
            };

            self.details_info = DetailsInfo {
                image_file_name: image_info.map(|info| info.0.clone()),
                dir_name: self.grid_entries[grid_entry_i].stem.clone(),
                dir_entries: dir_entries().unwrap_or_default()
            };
            self.view_kind = ViewKind::Details;
        }

        let cell_context_menu_resp = self.cell_context_menu(ui, grid_entry_i, cell_i, &cell_resp);

        match self.selected_cell {
            Some(cell_i_) => if cell_i_ == cell_i { // Cell was secondary clicked
                stroke_rect(ui, cell_resp.rect);

                match cell_context_menu_resp {
                    Some(resp) => { // Context menu is open
                        if resp.inner.entry_clicked ||
                            !resp.inner.pointer_contained && ui.input(|state| state.pointer.primary_clicked()) // Clicked outside the menu
                        {
                            egui::Popup::close_all(ui.ctx());
                        }
                    },
                    None => self.selected_cell = None // Context menu was closed - deselect cell
                }
            },
            // No context menu
            None => if cell_resp.hovered() {
                stroke_rect(ui, cell_resp.rect);
            }
        }
    }

    fn cell_context_menu(&mut self, cell_ui: &mut egui::Ui, grid_entry_i: usize, cell_i: usize, cell_resp: &egui::Response) -> Option<egui::InnerResponse<MenuPointerState>> {
        egui::Popup::context_menu(cell_resp)
            .close_behavior(egui::PopupCloseBehavior::IgnoreClicks) // Close manually to avoid close/show flash on right click cell while menu is open
            .show(|ui| {
                let tags_menu_resp = self.tags_menu(ui, grid_entry_i, cell_i);

                let painter = ui.painter().clone().with_layer_id(cell_ui.layer_id());
                stroke_rect_painter(painter, cell_resp.rect);

                self.selected_cell = Some(cell_i);

                MenuPointerState {
                    pointer_contained: ui.ui_contains_pointer() || tags_menu_resp.inner.unwrap_or_default().0,
                    entry_clicked: tags_menu_resp.response.clicked() || tags_menu_resp.response.secondary_clicked()
                }
            })
    }

    fn tags_menu(&mut self, ui: &mut egui::Ui, grid_entry_i: usize, cell_i: usize) -> egui::InnerResponse<Option<PointerContained>> {
        ui.menu_button("Tags", |ui| {
            // Add tag
            let tag_edit_resp = egui::TextEdit::singleline(&mut self.tag_edit)
                .hint_text("Add")
                .show(ui)
                .response;

            if tag_edit_resp.lost_focus() && ui.input(|state| state.key_pressed(egui::Key::Enter)) {
                self.tags.entry(Rc::from(self.tag_edit.as_str()))
                    .and_modify(|set| _ = set.insert(cell_i))
                    .or_insert([cell_i].into_iter().collect());

                self.tag_edit.clear();
                tag_edit_resp.request_focus();
            }

            ui.separator();

            // Select from existing tags
            for (tag, set) in self.tags.iter_mut() { if !set.is_empty() {
                let tag_button_resp = ui.button(tag.as_ref());

                match (tag_button_resp.clicked(), set.contains(&grid_entry_i)) {
                    (_tag_button_clicked @ true, _entry_is_tagged @ true) => {
                        set.remove(&grid_entry_i);

                        let tag_is_active = self.active_tag.as_ref().map(|active_tag| active_tag == tag).unwrap_or_default();
                        if tag_is_active {
                            match set.is_empty() {
                                true => self.active_tag = None,
                                false => {
                                    update_active_view(&mut self.active_view, set);

                                    self.removed_entry_from_active_view = true;
                                }
                            }

                            ui.close();
                        }
                    },
                    (_tag_button_clicked @ true, _entry_is_tagged @ false) => _ = set.insert(grid_entry_i),
                    (_tag_button_clicked @ false, _entry_is_tagged @ true) => _ = tag_button_resp.highlight(),
                    _ => ()
                }
            } }

            PointerContained(ui.ui_contains_pointer())
        })
    }

    fn details_view(&mut self, ui: &mut egui::Ui) {
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
            let dir_name = &self.details_info.dir_name;
            let image_file_name = &self.details_info.image_file_name;

            let details_state = image_file_name.as_ref()
                .and_then(|image_file_name| self.images.get_mut(image_file_name.as_ref()))
                .map(|info|&mut info.details_state );

            match details_state {
                Some(details_state) => try_add_image(ui, details_state, image_file_name.as_ref().unwrap().as_ref(), dir_name.as_ref()),
                None => add_label(ui, dir_name.as_ref())
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
                .wheel_scroll_multiplier([1.0, self.vscroll_multiplier].into())
                .show(ui, |ui| {
                    let button_height = ui.spacing().interact_size.y;
                    let button_spacing = ui.spacing().item_spacing[1];
                    #[allow(clippy::cast_precision_loss)]
                    let button_count = self.details_info.dir_entries.len() as f32;
                    let buttons_height = button_count * (button_height + button_spacing);

                    let remaining_space = ui.available_height() - buttons_height;
                    let top_padding = (remaining_space / 2.0).floor().max(0.0);

                    ui.with_layout(egui::Layout::top_down_justified(egui::Align::Center), |ui| {
                        ui.add_space(top_padding);

                        if self.details_info.dir_entries.is_empty() {
                            ui.take_available_space();
                        } else {
                            self.dir_entries(ui);
                        }
                    });
                });
        });

        self.details_context_menu(&total_alloc_resp);

        ui.ctx().input(|state| {
            if state.pointer.button_released(egui::PointerButton::Extra1) {
                self.view_kind = ViewKind::Grid;
            }
        });
    }

    fn dir_entries(&mut self, ui: &mut egui::Ui) {
        egui::Frame::group(ui.style())
            .stroke(egui::Stroke::NONE)
            .inner_margin(egui::Margin::ZERO)
            .show(ui, |ui| {
                self.details_button_resps.clear();
                self.details_button_resps.extend(
                    self.details_info.dir_entries.iter()
                        .map(|info| ui.button(info.stem.as_str()).interact(egui::Sense::click()))
                );

                for (i, resp) in self.details_button_resps.iter().enumerate() {
                    if resp.hovered() {
                        self.hovered_details_entry_i = i;
                    }

                    if resp.clicked() {
                        let discord_info = self.discord_enabled.then_some(self.make_discord_info(i));

                        unsafe {
                            open_media(
                                self.details_info.dir_entries[i].path.clone(),
                                self.details_info.dir_entries[i].file_kind,
                                self.maintain_sample_rate,
                                self.use_glsl_shaders,
                                discord_info
                            );
                        }
                    }
                }
            });
    }

    fn make_discord_info(&self, i: usize) -> config::DiscordInfo {
        config::DiscordInfo {
            client_id: match self.discord_watching {
                Watching::Movie => self.discord_app_ids.movies.unwrap().to_string(), // App ID is Some when Discord is enabled
                Watching::TV => self.discord_app_ids.tv.unwrap().to_string(),
                Watching::Words => self.discord_app_ids.words.unwrap().to_string()
            },
            activity: config::DiscordActivity::Watching,
            details: match self.discord_details.is_empty() {
                true => match self.discord_watching {
                    Watching::TV => self.details_info.dir_name.to_string(),
                    _ => self.details_info.dir_entries[i].stem.clone()
                },
                false => self.discord_details.clone()
            },
            state: self.discord_watching.eq(&Watching::TV).then(|| {
                match self.discord_state.is_empty() {
                    true => self.details_info.dir_entries[i].stem.clone(),
                    false => self.discord_state.clone()
                }
            }),
            display_kind: self.discord_display_kind,
            large_image: match self.discord_watching {
                Watching::TV => Some(to_discord_asset_name(self.details_info.dir_name.as_ref())),
                _ => Some(to_discord_asset_name(self.details_info.dir_entries[i].stem.as_str()))
            },
            chess_username: None
        }
    }

    fn details_context_menu(&mut self, total_alloc_resp: &egui::Response) {
        egui::Popup::context_menu(total_alloc_resp)
            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
            .show(|ui| {
                ui.add_enabled_ui(!self.details_info.dir_entries.is_empty(), |ui| {
                    ui.checkbox(&mut self.maintain_sample_rate, "Maintain sample rate");
                    ui.checkbox(&mut self.use_glsl_shaders, "Override GLSL shaders");
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

        let details_label_galley = egui::WidgetText::from("Details")
            .into_galley(ui, None, f32::INFINITY, egui::FontSelection::Default);
        let details_text_edit_width = ui.available_width() - details_label_galley.rect.width();

        let margin = egui::Margin::symmetric(4, 2);
        let grid = egui::Grid::new("grid").num_columns(2);
        grid.show(ui, |ui| {
            ui.label(details_label_galley);

            let details_hint_text = match self.discord_watching {
                Watching::TV => self.details_info.dir_name.as_ref(),
                _ => self.details_info.dir_entries[self.hovered_details_entry_i].stem.as_str()
            };
            let details_hint_galley = egui::WidgetText::from(details_hint_text)
                .into_galley(ui, Some(egui::TextWrapMode::Truncate), ui.available_width() - margin.sum().x, egui::FontSelection::Default);
            let details_text_edit = egui::TextEdit::singleline(&mut self.discord_details)
                .desired_width(details_text_edit_width)
                .hint_text(details_hint_galley);

            ui.add(details_text_edit);

            ui.end_row();

            if self.discord_watching == Watching::TV {
                ui.label("State");

                let state_hint_text = self.details_info.dir_entries[self.hovered_details_entry_i].stem.as_str();
                let state_hint_galley = egui::WidgetText::from(state_hint_text)
                    .into_galley(ui, Some(egui::TextWrapMode::Truncate), ui.available_width() - margin.sum().x, egui::FontSelection::Default);
                let state_text_edit = egui::TextEdit::singleline(&mut self.discord_state)
                    .hint_text(state_hint_galley)
                    .desired_width(details_text_edit_width);

                ui.add(state_text_edit);
            }
        });
    }
}

pub(crate) fn begin(kind: Kind) -> Res<(), { loc_var!(Gui) }> {
    let mut viewport = egui::ViewportBuilder::default()
        .with_maximize_button(false);
    if let Kind::MediaBrowser = kind {
        let config = config::get().read()?;

        if let Some(size) = config.media_browser.as_ref().and_then(|mb| mb.window_inner_size) {
            #[allow(clippy::cast_precision_loss)]
            let (width, height) = (size.width as f32, size.height as f32);

            viewport = viewport.with_inner_size([width, height]);
        }
    }

    let native_options = eframe::NativeOptions {
        viewport,
        renderer: eframe::Renderer::Wgpu,
        wgpu_options: egui_wgpu::WgpuConfiguration {
            present_mode: wgpu::PresentMode::Fifo,
            desired_maximum_frame_latency: Some(1),
            wgpu_setup: egui_wgpu::WgpuSetup::CreateNew(
                egui_wgpu::WgpuSetupCreateNew {
                    instance_descriptor: wgpu::InstanceDescriptor {
                        backends: wgpu::Backends::DX12,
                        backend_options: wgpu::BackendOptions {
                            dx12: wgpu::Dx12BackendOptions {
                                presentation_system: wgpu::wgt::Dx12SwapchainKind::DxgiFromVisual,
                                ..default!()
                            },
                            ..default!()
                        },
                        ..default!()
                    },
                    power_preference: wgpu::PowerPreference::HighPerformance,
                    ..default!()
                }
            ),
            ..default!()
        },
        centered: true,
        ..default!()
    };

    eframe::run_native(
        "Ogos",
        native_options,
        Box::new(|cctx| {
            cctx.egui_ctx.set_pixels_per_point(1.0);

            let mut style = (*cctx.egui_ctx.style()).clone();
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

            cctx.egui_ctx.set_style(style);

            Ok(match kind {
                Kind::Discord { name, discord_info } => Box::new(Discord::new(cctx, name, discord_info)),
                Kind::MediaBrowser => Box::new(MediaBrowser::new(&cctx.egui_ctx)?)
            })
        })
    )?;

    Ok(())
}
