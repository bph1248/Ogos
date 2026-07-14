use ogos_common::*;
use ogos_config as config;
use config::*;
use ogos_core::*;
use ogos_discord as discord;
use ogos_err::*;
use ogos_video as video;

#[cfg(feature = "hotpath")]
use const_format::*;
use concat_string::*;
use crossbeam::sync::*;
use crossbeam::channel as mpmc;
use discord_rich_presence::*;
use eframe::{
    egui::{
        self,
        containers::scroll_area::{
            ScrollBarVisibility,
            ScrollSource
        }
    },
    egui_wgpu,
    wgpu
};
use fast_image_resize as fir;
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
    io::{self, Read},
    mem,
    ops::*,
    path::*,
    process::*,
    rc::*,
    sync::{atomic::*, *},
    thread,
    time::*
};
use tap::TapOptional;
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
const BLACKMAN_SUPPORT: f64 = 3.;
const CELL_STROKE: egui::Stroke = egui::Stroke { width: 3.0, color: egui::Color32::from_rgb(250, 246, 235) };
const DEFAULT_FRAME_INNER_MARGIN: f32 = 8.0;
const DETAILS_ENTRY_COUNT: usize = 64;
const FRAME_INNER_MARGIN: f32 = 15.0;
const GRID_IMAGE_SPACING: egui::Vec2 = egui::vec2(30.0, 30.0);
const IMAGE_EXTS: &[&str] = &["jpg", "jpeg", "png", "webp"];
const MANGA_CONTEXT_MENU_MIN_WIDTH: f32 = 201.;
const SEPARATOR_WIDTH: f32 = 2.0;

type CacheReady = Option<WaitGroup>;
type ImageResult = Result<ImageInfo, (Stage, usize)>;
type Residence = Range<usize>;
type ShouldStream = bool;
type VisiblePageCount = usize;
type WrittenSize = usize;

const fn spring_damper() -> SpringDamperCache {
    SpringDamperCache {
        multiplier: 5.,
        angular_frequency: 50.,
        damping_ratio: 1. / 0.6,
        should_smooth: false
    }
}
const fn spring_damper_manga() -> SpringDamperCache {
     SpringDamperCache {
        multiplier: 9.,
        angular_frequency: 40.,
        damping_ratio: 1. / 0.4,
        should_smooth: true
    }
}

#[derive(Serialize, Deserialize)]
struct Cache {
    grid_cell_size: egui::Vec2,
    details_cell_size: egui::Vec2,
    #[serde(default = "spring_damper")]
    spring_damper: SpringDamperCache,
    #[serde(default = "spring_damper_manga")]
    spring_damper_manga: SpringDamperCache,
    images: IndexSet<Rc<str>>,
    tags: Vec<Rc<str>>,
    entries: HashMap<PathBuf, CacheEntryInfo>
}

#[derive(Serialize, Deserialize)]
struct CacheEntryInfo {
    #[serde(rename = "image")]
    image_i: Option<usize>,
    #[serde(default, skip_serializing_if = "ShouldScale::is_false")]
    should_scale: ShouldScale,
    sort_name: Option<Rc<str>>,
    metadata: Option<Arc<Metadata>>,
    #[serde(default)]
    tags: Vec<usize>
}

#[derive(Clone)]
struct CacheReadies {
    grid: CacheReady,
    details: CacheReady
}
impl CacheReadies {
    const NONE: Self = Self { grid: None, details: None };

    fn new() -> Self {
        Self { grid: Some(WaitGroup::new()), details: Some(WaitGroup::new()) }
    }
}

#[derive(Clone)]
struct DirEntryInfo {
    path: PathBuf,
    stem: String,
    file_kind: FileKind
}

#[derive(Clone, Copy, Default)]
pub struct Extent2dF {
    pub width: f32,
    pub height: f32
}
impl Extent2dF {
    fn aspect_ratio_v(&self) -> f32 {
        self.height / self.width
    }

    fn orientation(&self) -> Orientation {
        let aspect_ratio_v = self.height / self.width;

        aspect_ratio_v.into()
    }
}
impl From<[f32; 2]> for Extent2dF {
    fn from(value: [f32; 2]) -> Self {
        Self {
            width: value[0],
            height: value[1]
        }
    }
}
impl From<(u32, u32)> for Extent2dF {
    fn from(value: (u32, u32)) -> Self {
        Self {
            width: value.0 as f32,
            height: value.1 as f32
        }
    }
}
impl From<Extent2dU> for Extent2dF {
    fn from(value: Extent2dU) -> Self {
        Self {
            width: value.width as f32,
            height: value.height as f32
        }
    }
}
impl From<egui::Vec2> for Extent2dF {
    fn from(value: egui::Vec2) -> Self {
        Self {
            width: value.x,
            height: value.y
        }
    }
}
impl From<&wgpu::Texture> for Extent2dF {
    fn from(value: &wgpu::Texture) -> Self {
        Self {
            width: value.width() as f32,
            height: value.height() as f32
        }
    }
}
impl From<Extent2dF> for Extent2dU {
    fn from(value: Extent2dF) -> Self {
        Extent2dU {
            width: value.width as u32,
            height: value.height as u32
        }
    }
}
impl From<Extent2dF> for egui::Vec2 {
    fn from(value: Extent2dF) -> Self {
        egui::vec2(value.width, value.height)
    }
}

struct FerryImageInfo {
    image_file_name: Arc<str>,
    expected_metadata: Option<Arc<Metadata>>,
    grid_entry_i: usize,
    signal_cache_readies: CacheReadies,
    wait_cache_readies: CacheReadies,
    signal_tex_ready: Option<PollReady>
}

struct FerryImagesInfo<'a> {
    ctx: &'a egui::Context,
    thread_pool: &'a Arc<ThreadPool>,
    metadata_sx: Option<&'a mpmc::Sender<MetadataInfo>>,
    image_dirs: &'static ImageDirs,
    base_image_kind: BaseImageKind,
    grid_cell_extent: Extent2dF,
    details_cell_extent: Extent2dF,
    grid_relay: mpmc::Sender<ImageResult>,
    details_relay: mpmc::Sender<ImageResult>,
    ferry_image_infos: Vec<FerryImageInfo>,
    error_sx: mpmc::Sender<String>
}

struct FerryImageInfoManga {
    archive_i: usize,
    image_kind: ImageKind,
    view_i: usize,
    scale: Option<ScaleImageManga>,
    signal_tex_ready: Option<PollReady>
}

struct FerryImagesInfoManga<'a> {
    ctx: &'a egui::Context,
    thread_pool: &'a Arc<ThreadPool>,
    archive_path: Arc<PathBuf>,
    relay: mpmc::Sender<ImageResult>,
    ferry_image_infos: Vec<FerryImageInfoManga>,
    error_sx: mpmc::Sender<String>
}

struct FerryBaseImageInfo<'a> {
    ctx: egui::Context,
    src_path: &'a Path,
    dst_path: &'a Path,
    cell_extent: Extent2dF,
    stage: Stage,
    grid_entry_i: usize,
    relay: mpmc::Sender<ImageResult>,
    signal_cache_ready: CacheReady,
    signal_tex_ready: Option<PollReady>
}

struct FerryImageMangaInfo {
    ctx: egui::Context,
    archive_path: Arc<PathBuf>,
    archive_i: usize,
    image_kind: ImageKind,
    view_i: usize,
    scale: Option<ScaleImageManga>,
    relay: mpmc::Sender<ImageResult>,
    signal_tex_ready: Option<PollReady>
}

struct FerryCachedImageInfo<'a> {
    ctx: egui::Context,
    path: &'a Path,
    stage: Stage,
    grid_entry_i: usize,
    relay: mpmc::Sender<ImageResult>,
    wait_cache_ready: CacheReady,
    signal_tex_ready: Option<PollReady>
}

struct QueueFerryBaseImageInfo<'a> {
    queue: mpmc::Sender<QueueImageInfo>,
    src_path: &'a Path,
    dst_path: &'a Path,
    cell_extent: Extent2dF,
    grid_entry_i: usize,
    relay: mpmc::Sender<ImageResult>,
    signal_cache_ready: CacheReady
}

struct QueueFerryCachedImageInfo<'a> {
    queue: mpmc::Sender<QueueImageInfo>,
    path: &'a Path,
    grid_entry_i: usize,
    relay: mpmc::Sender<ImageResult>,
    wait_cache_ready: CacheReady
}

#[derive(Default)]
struct GridCellHighlights {
    selected: Vec<egui::Rect>,
    hovered: Option<egui::Rect>
}

struct GridEntryInfo {
    path: PathBuf,
    stem: Rc<str>,
    sort_name: Option<Rc<str>>,
    file_kind: FileKind,
    image_i: Option<usize>,
    metadata: Option<Arc<Metadata>>
}

struct GridViewCellCounts {
    row: usize,
    max: usize
}

struct ImageDirs {
    base: PathBuf,
    grid: PathBuf,
    details: PathBuf
}

struct ImageInfo {
    image: image::RgbaImage,
    stage: Stage,
    index: usize,
    signal_tex_ready: Option<PollReady>
}

#[derive(Default)]
struct ImageStates {
    grid: ImageState,
    details: ImageState,
    ref_count: usize
}
impl ImageStates {
    fn clone_cache_readies_on_scale(&mut self) -> CacheReadies {
        CacheReadies {
            grid: self.grid.clone_cache_ready_on_scale(),
            details: self.details.clone_cache_ready_on_scale()
        }
    }

    fn iter(&self) -> ImageStatesIter<'_> {
        ImageStatesIter {
            index: 0,
            grid: &self.grid,
            details: &self.details
        }
    }

    fn new_none_check_cache(cache_readies: CacheReadies) -> Self {
        Self {
            grid: ImageState::NoneCheckCache { cache_ready: cache_readies.grid },
            details: ImageState::NoneCheckCache { cache_ready: cache_readies.details },
            ref_count: 0
        }
    }

    fn take_cache_readies(&mut self) -> CacheReadies {
        CacheReadies {
            grid: self.grid.take_cache_ready(),
            details: self.details.take_cache_ready()
        }
    }

    fn take_cache_readies_on_not_scale(&mut self) -> CacheReadies {
        CacheReadies {
            grid: self.grid.take_cache_ready_on_not_scale(),
            details: self.details.take_cache_ready_on_not_scale()
        }
    }

    fn should_scale(&self) -> ShouldScale {
        ShouldScale {
            grid: matches!(self.grid, ImageState::Scale { .. }),
            details: matches!(self.details, ImageState::Scale { .. })
        }
    }
}

struct ImageStatesIter<'a> {
    index: usize,
    grid: &'a ImageState,
    details: &'a ImageState
}
impl<'a> Iterator for ImageStatesIter<'a> {
    type Item = &'a ImageState;

    fn next(&mut self) -> Option<Self::Item> {
        let item = match self.index {
            0 => Some(self.grid),
            1 => Some(self.details),
            _ => None
        };

        self.index += 1;

        item
    }
}

struct LateInit<T> {
    inner: Option<T>
}
impl<T> LateInit<T> {
    fn set(&mut self, value: T) {
        self.inner = Some(value);
    }
}
impl<T> Default for LateInit<T> {
    fn default() -> Self {
        Self { inner: default!() }
    }
}
impl<T> Deref for LateInit<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        self.inner.as_ref().unwrap()
    }
}

#[derive(Default)]
struct Manga {
    archive: LateInit<zip::ZipArchive<io::BufReader<fs::File>>>,
    archive_path: LateInit<Arc<PathBuf>>,
    archive_pages: Vec<ArchivePageInfo>,
    archive_pages_width: f32, // Hang on to this to get view width later on when resizing
    view: Vec<ViewPageInfo>,
    view_extent: Extent2dF,
    scale: f32,
    scale_drag_anchor: f32,
    flagged_scale: Option<egui::Rect>,
    filter: fir::FilterType,
    scroll_kind: ScrollKind,
    scroll_offset: egui::Vec2,
    scroll_offset_y_anchor: Option<f32>,
    spring_damper: SpringDamper,
    prev_viewport_top: f32,
    some_prev_delta: bool,
    secondary_was_down: bool,
    residence: Range<usize>,
    visible_view: Range<usize>,
    stream: Stream,
    to_thanatos: LateInit<mpmc::Sender<Soul>>
}
impl Manga {
    fn new(spring_damper: SpringDamper) -> Self {
        Self {
            scale: 100.,
            filter: fir::FilterType::Custom(blackman_filter_fir()),
            spring_damper,
            ..default!()
        }
    }

    fn reset(&mut self) {
        let view = mem::take(&mut self.view);
        for page_info in view {
            if let ImageStateManga::Ready { .. } = page_info.image_state {
                self.to_thanatos.send(Soul::ImageState(page_info.image_state.into())).unwrap();
            }
        }

        *self = Manga {
            spring_damper: mem::take(&mut self.spring_damper),
            ..default!()
        }
    }

    fn flag_scale(&mut self, ui: &mut egui::Ui, scale: f32, viewport: egui::Rect) {
        if scale == 100. && self.scale == 100. {
            return
        }

        self.scale = scale;
        self.flagged_scale = Some(viewport);

        ui.close();
    }
}

#[derive(Deserialize, Eq, PartialEq, Serialize)]
struct Metadata {
    created: SystemTime,
    modified: SystemTime,
    len: u64
}

struct MetadataInfo {
    grid_entry_i: usize,
    metadata: Arc<Metadata>
}

struct ArchivePageInfo {
    name: String,
    index: usize,
    image_kind: ImageKind,
    extent: Extent2dF
}

struct ViewPageInfo {
    archive_i: usize,
    image_kind: ImageKind,
    offset: f32,
    extent: Extent2dF,
    image_state: ImageStateManga
}

struct PartialTex {
    tex: wgpu::Texture,
    tex_id: egui::TextureId,
    captive: Option<(image::RgbaImage, Option<PollReady>)>,
    stage: Stage,
    index: usize,
    offset: usize,
    row_size: usize,
    chunk_row_count: usize
}

struct PendingTagOp {
    tag: Rc<str>,
    op: TagOp
}

pub struct PollReady {
    count: Arc<AtomicUsize>,
}
impl PollReady {
    fn new() -> Self {
        Self {
            count: Arc::new(AtomicUsize::new(0)),
        }
    }

    fn mark_done(&self) {
        self.count.fetch_sub(1, Ordering::Relaxed);
    }

    fn is_ready(&self) -> bool {
        self.count.load(Ordering::Relaxed) == 0
    }
}
impl Clone for PollReady {
    fn clone(&self) -> Self {
        self.count.fetch_add(1, Ordering::Relaxed);

        Self {
            count: self.count.clone()
        }
    }
}

struct QueueImageInfo {
    path: PathBuf,
    grid_entry_i: usize,
    relay: mpmc::Sender<ImageResult>,
    scale: Option<ScaleImage>,
    cache_ready: CacheReady
}

struct ResetResidence {
    row_cell_count: usize,
    visible_cell_count: usize
}

struct ScaleImage {
    dst_path: PathBuf,
    cell_extent: Extent2dF
}

struct ScaleImageManga {
    extent: Extent2dF,
    filter: fir::FilterType
}

struct ScrollAreaInfo {
    scroll_source: ScrollSource,
    drag_by: egui::PointerButton,
    stop_kinesis: bool,
    scroll_offset: egui::Vec2,
    scroll_multiplier: egui::Vec2
}

#[derive(Default, Deserialize, Serialize)]
struct ShouldScale {
    grid: bool,
    details: bool
}
impl ShouldScale {
    fn is_false(&self) -> bool {
        !self.grid  && !self.details
    }
}

//$ Attribute: https://www.ryanjuckett.com/damped-springs/
#[derive(Default)]
struct SpringDamper {
    multiplier: f32,
    pos: f32,
    vel: f32,
    equilibrium_pos: f32, // Target
    delta: f32,

    angular_frequency: f32, // Omega, ω, stiffness
    damping_ratio: f32, // Zeta, ζ, bounce reciprocal

    pos_pos_coef: f32,
    pos_vel_coef: f32,
    vel_pos_coef: f32,
    vel_vel_coef: f32,

    should_smooth: bool,
    multiplier_edit: String,
    stiffness_edit: String,
    bounce_edit: String,
    multiplier_display: String,
    stiffness_display: String,
    bounce_display: String
}
impl SpringDamper {
    fn step(&mut self, ui: &mut egui::Ui) {
        const EPSILON: f32 = 0.0001;
        const TOLERANCE: f32 = 0.5;

        let (dt, delta) = ui.input(|i| {
            let dt = i.unstable_dt.min(1. / 240.); //$ Hardcoded min

            let delta = match self.should_smooth {
                true => i.smooth_scroll_delta,
                false => {
                    let mut delta_ = egui::Vec2::default();
                    for event in i.events.iter() {
                        if let egui::Event::MouseWheel { delta, .. } = event {
                            delta_ += *delta;
                        }
                    }

                    delta_ * 40.
                }
            };

            (dt, delta)
        });
        self.equilibrium_pos -= delta.y * self.multiplier;

        // Force values into legal range
        let angular_frequency = self.angular_frequency.max(0.);
        let damping_ratio = self.damping_ratio.max(0.);

        // If there is no angular frequency, the spring will not move and we can return identity
        if angular_frequency < EPSILON {
            self.pos_pos_coef = 1.;
            self.pos_vel_coef = 0.;
            self.vel_pos_coef = 0.;
            self.vel_vel_coef = 1.;

            return
        }

        if damping_ratio > 1. + EPSILON { // Over-damped
            let za = -angular_frequency * damping_ratio;
            let zb = angular_frequency * (damping_ratio * damping_ratio - 1.).sqrt();
            let z1 = za - zb;
            let z2 = za + zb;

            let e1 = (z1 * dt).exp();
            let e2 = (z2 * dt).exp();

            let inv_2zb = 1. / (2. * zb); // = 1 / (z2 - z1)

            let e1_over_2zb = e1 * inv_2zb;
            let e2_over_2zb = e2 * inv_2zb;

            let z1e1_over_2zb = z1 * e1_over_2zb;
            let z2e2_over_2zb = z2 * e2_over_2zb;

            self.pos_pos_coef =  e1_over_2zb * z2 - z2e2_over_2zb + e2;
            self.pos_vel_coef = -e1_over_2zb      + e2_over_2zb;
            self.vel_pos_coef = (z1e1_over_2zb - z2e2_over_2zb + e2) * z2;
            self.vel_vel_coef = -z1e1_over_2zb + z2e2_over_2zb;
        }
        else if damping_ratio < 1. - EPSILON { // Under-damped
            let omega_zeta = angular_frequency * damping_ratio;
            let alpha      = angular_frequency * (1. - damping_ratio * damping_ratio).sqrt();

            let exp_term = (-omega_zeta * dt).exp();
            let cos_term = (alpha * dt).cos();
            let sin_term = (alpha * dt).sin();

            let inv_alpha = 1. / alpha;

            let exp_sin = exp_term * sin_term;
            let exp_cos = exp_term * cos_term;
            let exp_omega_zeta_sin_over_alpha = exp_term * omega_zeta * sin_term * inv_alpha;

            self.pos_pos_coef = exp_cos + exp_omega_zeta_sin_over_alpha;
            self.pos_vel_coef = exp_sin * inv_alpha;
            self.vel_pos_coef = -exp_sin * alpha - omega_zeta * exp_omega_zeta_sin_over_alpha;
            self.vel_vel_coef =  exp_cos - exp_omega_zeta_sin_over_alpha;
        }
        else { // Critically damped
            let exp_term      = (-angular_frequency * dt).exp();
            let time_exp      = dt * exp_term;
            let time_exp_freq = time_exp * angular_frequency;

            self.pos_pos_coef = time_exp_freq + exp_term;
            self.pos_vel_coef = time_exp;
            self.vel_pos_coef = -angular_frequency * time_exp_freq;
            self.vel_vel_coef = -time_exp_freq + exp_term;
        }

        let old_pos = self.pos;
        let old_vel = self.vel;
        let displacement = self.pos - self.equilibrium_pos; // Update in equilibrium relative space

        self.pos = self.pos_pos_coef * displacement + self.pos_vel_coef * old_vel + self.equilibrium_pos;
        self.vel = self.vel_pos_coef * displacement + self.vel_vel_coef * old_vel;

        self.delta = self.pos - old_pos;

        let settled = displacement.abs() < TOLERANCE && self.vel.abs() < TOLERANCE;
        if settled {
            self.pos = self.equilibrium_pos;
            self.vel = 0.;
            self.delta = 0.;
        } else {
            ui.request_repaint();
        }
    }

    fn stop(&mut self) {
        self.pos = 0.;
        self.vel = 0.;
        self.equilibrium_pos = 0.;
        self.delta = 0.
    }

    fn update_bounce(&mut self, bounce: f32) {
        let zeta = bounce.recip();
        self.damping_ratio = zeta;
    }

    fn update_stiffness(&mut self, stiffness: f32) {
        self.angular_frequency = stiffness;
    }

    fn update_display(&mut self) {
        self.multiplier_display.clear();
        self.stiffness_display.clear();
        self.bounce_display.clear();

        write!(self.multiplier_display, "{}", self.multiplier).unwrap();
        write!(self.stiffness_display, "{}", self.angular_frequency).unwrap();
        write!(self.bounce_display, "{}", self.damping_ratio.recip()).unwrap();
    }
}
impl AsRef<Self> for SpringDamper {
    fn as_ref(&self) -> &Self {
        self
    }
}
impl From<SpringDamperCache> for SpringDamper {
    fn from(value: SpringDamperCache) -> Self {
        Self {
            multiplier: value.multiplier,
            angular_frequency: value.angular_frequency,
            damping_ratio: value.damping_ratio,
            should_smooth: value.should_smooth,
            multiplier_display: format!("{}", value.multiplier),
            stiffness_display: format!("{}", value.angular_frequency),
            bounce_display: format!("{}", value.damping_ratio.recip()),
            ..default!()
        }
    }
}

#[derive(Clone, Copy, Deserialize, Serialize)]
struct SpringDamperCache {
    multiplier: f32,
    angular_frequency: f32,
    damping_ratio: f32,
    should_smooth: bool
}
impl From<&SpringDamper> for SpringDamperCache {
    fn from(value: &SpringDamper) -> Self {
        Self {
            multiplier: value.multiplier,
            angular_frequency: value.angular_frequency,
            damping_ratio: value.damping_ratio,
            should_smooth: value.should_smooth
        }
    }
}

#[derive(Default)]
struct Stream {
    drop: HashSet<usize>,
    load_first: HashSet<usize>,
    load_after: HashSet<usize>
}
impl Stream {
    fn clear(&mut self) {
        self.drop.clear();
        self.load_first.clear();
        self.load_after.clear();
    }

    fn flatten_drop(&mut self, drop: Range<usize>, grid_view: &[usize]) {
        self.drop.clear();

        for grid_view_i in drop {
            let grid_entry_i = grid_view[grid_view_i];

            self.drop.insert(grid_entry_i);
        }
    }

    fn flatten_load(&mut self, load: Range<usize>, visible: Range<usize>, grid_view: &[usize]) {
        self.load_first.clear();
        self.load_after.clear();

        for grid_view_i in load {
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
    }

    fn refresh_manga(&mut self, stream_builder: StreamBuilder, visible: Range<usize>) {
        self.clear();

        for view_i in stream_builder.drop {
            self.drop.insert(view_i);
        }

        for view_i in stream_builder.load {
            let overlap = self.drop.remove(&view_i);

            if !overlap {
                if visible.contains(&view_i) {
                    self.load_first.insert(view_i);
                } else {
                    self.load_after.insert(view_i);
                }
            }
        }
    }

    fn refresh_flatten(&mut self, stream_builder: StreamBuilder, visible: Range<usize>, grid_view: &[usize]) {
        self.flatten_drop(stream_builder.drop, grid_view);
        self.flatten_load(stream_builder.load, visible, grid_view);
    }
}

#[derive(Default)]
struct StreamBuilder {
    drop: Range<usize>,
    load: Range<usize>
}
impl StreamBuilder {
    fn with_drop(mut self, drop: Range<usize>) -> Self {
        self.drop = drop;

        self
    }

    fn with_load(mut self, load: Range<usize>) -> Self {
        self.load = load;

        self
    }
}

struct WriteTex {
    tex: wgpu::Texture,
    captive: Option<(image::RgbaImage, Option<PollReady>)>,
    offset: usize,
    row_count: usize,
    last_write: bool
}

#[derive(Clone)]
enum BaseImageKind {
    Pick { path: PathBuf },
    Startup
}

enum ButtonState {
    Up,
    Down
}
impl ButtonState {
    fn new(is_down: bool) -> Self {
        if is_down { Self::Down } else { Self::Up }
    }
}

enum GridViewOp {
    Refresh,
    Reset
}

pub enum GuiKind {
    Info { msg: String },
    MediaBrowser
}

#[derive(Clone, Copy)]
enum ImageKind {
    Jpeg,
    Png
}

#[derive(Default)]
enum ImageState {
    #[default]
    None,
    NoneCheckCache { cache_ready: CacheReady },
    Scale { cache_ready: CacheReady },
    Ready { tex_id: egui::TextureId, extent: Extent2dF, cache_ready: CacheReady },
    Failed
}
impl ImageState {
    fn clone_cache_ready_on_scale(&mut self) -> CacheReady {
        match self {
            Self::Scale { cache_ready } => cache_ready.clone(),
            _ => None
        }
    }

    fn take_cache_ready(&mut self) -> CacheReady {
        match self {
            Self::NoneCheckCache { cache_ready } |
            Self::Scale { cache_ready } |
            Self::Ready { cache_ready, .. } =>
                mem::take(cache_ready),
            _ => None
        }
    }

    fn take_cache_ready_on_not_scale(&mut self) -> CacheReady {
        match self {
            Self::Scale { .. } => None,
            _ => self.take_cache_ready()
        }
    }
}
impl From<ImageStateManga> for ImageState {
    fn from(value: ImageStateManga) -> Self {
        match value {
            ImageStateManga::Ready { tex_id, extent, .. } =>
                Self::Ready { tex_id, extent, cache_ready: None },
            _ => Self::None
        }
    }
}

#[derive(Default)]
enum ImageStateManga {
    #[default]
    None,
    Ready { tex_id: egui::TextureId, extent: Extent2dF },
    Failed
}

#[derive(Clone, Copy, Debug)]
enum Orientation {
    Tall,
    Wide
}
impl From<f32> for Orientation {
    fn from(value: f32) -> Self {
        match value < ASPECT_RATIO_3_2 {
            true => Orientation::Wide,
            false => Orientation::Tall
        }
    }
}

enum SelectionKind {
    Single,
    Multi
}

#[derive(Default, PartialEq)]
enum ScrollKind {
    EaseInOut,
    #[default]
    SpringDamper
}

enum Soul {
    ImageState(ImageState),
    ImageStates(ImageStates),
    RgbaImage(image::RgbaImage)
}

#[derive(Clone, Copy)]
enum Stage {
    Grid,
    Details,
    Manga
}

enum TagOp {
    Rename,
    Remove
}

#[derive(Default)]
enum ViewKind {
    #[default]
    Grid,
    Details,
    InitManga { selected_details_dir_entry_i: usize },
    WaitManga,
    Manga
}

#[derive(Default, Deserialize, PartialEq)]
enum Watching {
    Movie,
    #[default]
    TV,
    Words
}

fn try_add_image(ui: &mut egui::Ui, image_state: &mut ImageState, label: &str) -> egui::Response {
    match image_state {
        ImageState::Ready { tex_id, extent, .. } => {
            let tex = egui::load::SizedTexture::new(*tex_id, *extent);
            let image = egui::Image::new(tex).sense(egui::Sense::click());

            match extent.orientation() {
                Orientation::Tall => ui.add(image),
                Orientation::Wide => ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| ui.add(image)).inner
            }
        },
        ImageState::Failed => add_label(ui, label),
        _ => alloc_hover_response(ui)
        // _ => add_label(ui, label)
    }
}

fn try_add_image_manga(ui: &mut egui::Ui, image_state: &mut ImageStateManga, rect: egui::Rect) -> Option<egui::Response> {
    if let ImageStateManga::Ready { tex_id, extent, .. } = image_state {
        let tex = egui::load::SizedTexture::new(*tex_id, *extent);
        let image = egui::Image::new(tex).sense(egui::Sense::click());

        let mut ui = ui.new_child(egui::UiBuilder::new().max_rect(rect).layout(egui::Layout::top_down(egui::Align::Center)));

        return Some(ui.add(image))
    }

    None
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

#[hotpath::measure]
fn alloc_texture(wgpu: &egui_wgpu::RenderState, width: u32, height: u32, render_attachment: bool) -> (wgpu::Texture, wgpu::TextureView) {
    let egui_wgpu::RenderState { device, .. } = wgpu;

    let usage = wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::TEXTURE_BINDING;
    let usage = if render_attachment { usage | wgpu::TextureUsages::RENDER_ATTACHMENT } else { usage };
    let tex_desc = wgpu::TextureDescriptor {
        label: Some("hi"),
        size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage,
        view_formats: default!()
    };
    let tex = device.create_texture(&tex_desc);
    let tex_view = tex.create_view(&default!());

    (tex, tex_view)
}

fn alloc_clear_texture(wgpu: &egui_wgpu::RenderState, image: &image::RgbaImage) -> (wgpu::Texture, egui::TextureId) {
    let (width, height) = image.dimensions();

    let (tex, tex_view) = alloc_texture(wgpu, width, height, true);
    let tex_id = register_native_texture(wgpu, &tex_view);
    clear_texture(wgpu, tex_view);

    (tex, tex_id)
}

fn alloc_write_texture(wgpu: &egui_wgpu::RenderState, image: &image::RgbaImage) -> (wgpu::Texture, egui::TextureId) {
    let (width, height) = image.dimensions();

    let (tex, tex_view) = alloc_texture(wgpu, width, height, false);
    let tex_id = register_native_texture(wgpu, &tex_view);
    write_texture(wgpu, &tex, image, 0, height as usize);

    (tex, tex_id)
}

#[hotpath::measure]
fn clear_texture(wgpu: &egui_wgpu::RenderState, tex_view: wgpu::TextureView) {
    let render_pass_descriptor = wgpu::RenderPassDescriptor {
        label: None,
        color_attachments: &[
            Some(wgpu::RenderPassColorAttachment {
                view: &tex_view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store
                }
            })
        ],
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None
    };

    let mut encoder = wgpu.device.create_command_encoder(&default!());
    encoder.begin_render_pass(&render_pass_descriptor);
    let command_buffer = encoder.finish();

    wgpu.queue.submit([command_buffer]);
}

#[hotpath::measure]
fn register_native_texture(wgpu: &egui_wgpu::RenderState, tex_view: &wgpu::TextureView) -> egui::TextureId {
    let egui_wgpu::RenderState { device, renderer, .. } = wgpu;

    renderer.write().register_native_texture(device, tex_view, wgpu::FilterMode::Nearest)
}

#[hotpath::measure]
fn write_texture(wgpu: &egui_wgpu::RenderState, tex: &wgpu::Texture, image: &image::RgbaImage, offset: usize, row_count: usize) {
    let egui_wgpu::RenderState { queue, .. } = wgpu;

    let row_size = 4 * image.width() as usize;

    let region_coord = wgpu::TexelCopyTextureInfo {
        texture: tex,
        mip_level: 0,
        origin: wgpu::Origin3d { x: 0, y: offset as u32, z: 0 },
        aspect: wgpu::TextureAspect::All
    };
    let region_extent = wgpu::Extent3d {
        width: image.width(),
        height: row_count as u32,
        depth_or_array_layers: 1
    };
    let data_layout = wgpu::TexelCopyBufferLayout {
        offset: offset.mul(row_size) as u64,
        bytes_per_row: Some(row_size as u32),
        rows_per_image: None
    };
    queue.write_texture(region_coord, image.as_raw(), data_layout, region_extent);
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

    if t >= BLACKMAN_SUPPORT {
        0.0
    } else {
        let window = 0.42 +
            0.5 * (PI * t / BLACKMAN_SUPPORT).cos() +
            0.08 * (2.0 * PI * t / BLACKMAN_SUPPORT).cos();

        sinc(t) * window
    }
}

#[cfg(feature = "resize")]
fn blackman_filter() -> resize::Filter {
    resize::Filter::new(
        Box::new(|x: f32| -> f32 {
            blackman(f64::from(x)) as f32
        }),
        BLACKMAN_SUPPORT as f32
    )
}

fn blackman_filter_fir() -> fir::Filter {
    fir::Filter::new("Blackman", blackman, BLACKMAN_SUPPORT).unwrap()
}

fn demeter(ctx: egui::Context, wgpu: egui_wgpu::RenderState, image_rx: mpmc::Receiver<ImageInfo>, partial_tex_sx: mpmc::Sender<PartialTex>, chunk_size: usize) {
    for ImageInfo { image, stage, index: view_i, signal_tex_ready } in image_rx.iter() {
        hotpath::measure_block!(formatcp!("{}::demeter", module_path!()), {
            let image_size = image.as_raw().len();
            let row_size = 4 * image.width() as usize;
            let chunk_row_count = chunk_size.div(row_size).max(1);

            if image_size < chunk_size {
                let offset = image.height() as usize;
                let (tex, tex_id) = alloc_write_texture(&wgpu, &image);

                _ = partial_tex_sx.send(PartialTex { tex, tex_id, captive: Some((image, signal_tex_ready)), stage, index: view_i, offset, row_size, chunk_row_count });
            } else {
                let (tex, tex_id) = alloc_clear_texture(&wgpu, &image);

                _ = partial_tex_sx.send(PartialTex { tex, tex_id, captive: Some((image, signal_tex_ready)), stage, index: view_i, offset: 0, row_size, chunk_row_count });
            }
        });

        ctx.request_repaint();
    }
}

fn hephaestus(ctx: egui::Context, wgpu: egui_wgpu::RenderState, write_tex_rx: mpmc::Receiver<WriteTex>) {
    let mut captive_ = None;

    for WriteTex { tex, captive, offset, row_count, last_write } in write_tex_rx.iter() {
        hotpath::measure_block!(formatcp!("{}::hephaestus", module_path!()), {
            if captive.is_some() {
                captive_ = captive;
            }

            if let Some((image, signal_tex_ready)) = captive_.as_ref() {
                write_texture(&wgpu, &tex, image, offset, row_count);

                if last_write {
                    if let Some(signal_tex_ready) = signal_tex_ready.as_ref() {
                        signal_tex_ready.mark_done()
                    }
                    captive_ = None;
                }

                ctx.request_repaint();
            }
        });
    }
}

fn thanatos(wgpu: egui_wgpu::RenderState, image_state_rx: mpmc::Receiver<Soul>) {
    let egui_wgpu::RenderState { renderer, ..} = wgpu;

    for soul in image_state_rx.iter() {
        hotpath::measure_block!(formatcp!("{}::thanatos", module_path!()), {
            match soul {
                Soul::ImageState(image_state) => if let ImageState::Ready { tex_id, .. } = image_state {
                    renderer.write().free_texture(&tex_id);
                },
                Soul::ImageStates(image_states) => {
                    for image_state in image_states.iter() {
                        if let ImageState::Ready { tex_id, .. } = image_state {
                            renderer.write().free_texture(tex_id);
                        }
                    }
                },
                Soul::RgbaImage(image) => drop(image)
            }
        });
    }
}

#[hotpath::measure]
fn load_rgba_image(path: &Path) -> ResVar<image::ImageBuffer<image::Rgba<u8>, Vec<u8>>> {
    let image = image::open(path)?;

    Ok(match image {
        image::DynamicImage::ImageRgba8(image) => image,
        _ => image.to_rgba8()
    })
}

#[hotpath::measure]
fn load_rgba_image_cached(path: &Path) -> ResVar<image::ImageBuffer<image::Rgba<u8>, Vec<u8>>> {
    let mut file = fs::File::open(path)?;
    let mut buf = Vec::new();
    file.read_to_end(&mut buf)?;

    let (buf, width, height) = zenwebp::oneshot::decode_rgba(&buf).map_err(|err| err.decompose().0)?;

    Ok(image::RgbaImage::from_vec(width, height, buf).unwrap())
}

#[hotpath::measure]
fn load_rgba_image_manga(archive_path: Arc<PathBuf>, archive_i: usize, image_kind: ImageKind) -> ResVar<image::RgbaImage> {
    let archive = fs::File::open(archive_path.as_path())?;
    let archive = io::BufReader::new(archive);
    let mut archive = zip::ZipArchive::new(archive)?;

    let mut zip_file = archive.by_index(archive_i)?;
    let mut bytes = Vec::new();
    zip_file.read_to_end(&mut bytes)?;
    let reader = io::BufReader::new(io::Cursor::new(bytes));

    let image = match image_kind {
        ImageKind::Jpeg => {
            let opts = zune_jpeg::zune_core::options::DecoderOptions::new_fast()
                .jpeg_set_out_colorspace(zune_jpeg::zune_core::colorspace::ColorSpace::RGBA);
            let mut decoder = zune_jpeg::JpegDecoder::new_with_options(reader, opts);

            let decoded = decoder.decode().unwrap();
            let dimensions = decoder.dimensions().unwrap();

            image::RgbaImage::from_vec(dimensions.0 as u32, dimensions.1 as u32, decoded).unwrap()
        },
        ImageKind::Png => {
            let opts = zune_png::zune_core::options::DecoderOptions::new_fast()
                .png_set_add_alpha_channel(true)
                .png_set_decode_animated(false)
                .png_set_strip_to_8bit(true);
            let mut decoder = zune_png::PngDecoder::new_with_options(reader, opts);

            let decoded = decoder.decode().unwrap();
            let decoded = decoded.u8().unwrap();
            let dimensions = decoder.dimensions().unwrap();
            let width = dimensions.0 as u32;
            let height = dimensions.1 as u32;

            let color_space = decoder.colorspace().unwrap();
            match color_space {
                zune_png::zune_core::colorspace::ColorSpace::RGB => {
                    let rgb_image = image::RgbImage::from_vec(width, height, decoded).unwrap();

                    image::RgbaImage::from_fn(width, height, |x, y| {
                        let pixel = rgb_image.get_pixel(x, y);

                        image::Rgba([pixel[0], pixel[1], pixel[3], 255])
                    })
                },
                zune_png::zune_core::colorspace::ColorSpace::RGBA => image::RgbaImage::from_vec(width, height, decoded).unwrap(),
                zune_png::zune_core::colorspace::ColorSpace::Luma => {
                    let gray_image = image::GrayImage::from_vec(width, height, decoded).unwrap();

                    image::RgbaImage::from_fn(width, height, |x, y| {
                        let pixel = gray_image.get_pixel(x, y);
                        let luma = pixel[0];

                        image::Rgba([luma, luma, luma, 255])
                    })
                }
                zune_png::zune_core::colorspace::ColorSpace::LumaA => {
                    let gray_alpha_image = image::GrayAlphaImage::from_vec(width, height, decoded).unwrap();

                    image::RgbaImage::from_fn(width, height, |x, y| {
                        let pixel = gray_alpha_image.get_pixel(x, y);
                        let luma = pixel[0];
                        let alpha = pixel[1];

                        image::Rgba([luma, luma, luma, alpha])
                    })
                },
                _ => Err(ErrVar::InvalidPngColorSpace { color_space })?
            }
        }
    };

    Ok(image)
}

#[cfg(feature = "resize")]
#[hotpath::measure]
fn resize_image_manga(mut image: image::RgbaImage, dst_extent: Extent2dF, _filter: fir::FilterType) -> ResVar<image::RgbaImage> {
    use rgb::FromSlice;

    let (src_width, src_height) = image.dimensions();
    let Extent2dU { width: dst_width, height: dst_height } = dst_extent.into();
    let mut tmp_image = image::RgbaImage::new(dst_width, src_height);
    let mut dst_image = image::RgbaImage::new(dst_width, dst_height);

    let srgb_mapper = fir::create_srgb_mapper();
    srgb_mapper.forward_map_inplace(&mut image)?;

    let mut resizer = resize::new(
        src_width as usize,
        src_height as usize,
        dst_width as usize,
        src_height as usize,
        resize::Pixel::RGBA8,
        resize::Type::Custom(blackman_filter())
    )?;
    resizer.resize(image.as_rgba(), tmp_image.as_rgba_mut())?;
    let mut resizer = resize::new(
        dst_width as usize,
        src_height as usize,
        dst_width as usize,
        dst_height as usize,
        resize::Pixel::RGBA8,
        resize::Type::Custom(blackman_filter())
    )?;
    resizer.resize(tmp_image.as_rgba(), dst_image.as_rgba_mut())?;

    srgb_mapper.backward_map_inplace(&mut dst_image)?;

    Ok(dst_image)
}

#[cfg(not(feature = "resize"))]
#[hotpath::measure]
fn resize_image_manga(mut image: image::RgbaImage, extent: Extent2dF, filter: fir::FilterType) -> ResVar<image::RgbaImage> {
    let srgb_mapper = fir::create_srgb_mapper();
    srgb_mapper.forward_map_inplace(&mut image)?;

    let mut resizer = fir::Resizer::new();
    unsafe {
        if is_x86_feature_detected!("avx2") {
            resizer.set_cpu_extensions(fir::CpuExtensions::Avx2);
        } else if is_x86_feature_detected!("sse4.1") {
            resizer.set_cpu_extensions(fir::CpuExtensions::Sse4_1);
        }
    };
    let opts = fir::ResizeOptions { algorithm: fir::ResizeAlg::Convolution(filter), mul_div_alpha: false, ..default!() };

    let mut dst_image = image::RgbaImage::new(
        extent.width as u32,
        extent.height as u32
    );
    resizer.resize(&image, &mut dst_image, &opts).unwrap();

    srgb_mapper.backward_map_inplace(&mut dst_image)?;

    Ok(dst_image)
}

fn ferry_base_image(info: FerryBaseImageInfo) -> Res1<()> {
    let FerryBaseImageInfo { ctx, src_path, dst_path, cell_extent, stage, grid_entry_i, relay, signal_cache_ready, signal_tex_ready } = info;

    let inner = || -> Res1<image::RgbaImage> {
        let src_image = load_rgba_image(src_path)?;

        let aspect_ratio_v = Extent2dF::from(src_image.dimensions()).aspect_ratio_v();
        let (dst_width, dst_height) = match Orientation::from(aspect_ratio_v) {
            Orientation::Tall => (cell_extent.height.div(aspect_ratio_v).round(), cell_extent.height),
            Orientation::Wide => (cell_extent.width, cell_extent.width.mul(aspect_ratio_v).round())
        };
        let dst_image = resize_image_manga(src_image, [dst_width, dst_height].into(), fir::FilterType::Custom(blackman_filter_fir()))?;

        Ok(dst_image)
    };

    match inner() {
        Ok(image) => {
            let image_ = image.clone();

            if relay.send(Ok(ImageInfo { image: image_, stage, index: grid_entry_i, signal_tex_ready })).is_ok() {
                ctx.request_repaint();
            }

            let image_file = fs::File::create(dst_path)?;
            let encoder = image::codecs::webp::WebPEncoder::new_lossless(image_file);
            let (width, height) = image.dimensions();
            encoder.encode(image.as_raw(), width, height, image::ExtendedColorType::Rgba8)?;

            drop(signal_cache_ready)
        },
        Err(err) => {
            _ = relay.send(Err((stage, grid_entry_i)));

            Err(err)?;
        }
    }

    Ok(())
}

fn ferry_image_manga(info: FerryImageMangaInfo) -> Res1<()> {
    let FerryImageMangaInfo { ctx, archive_path, archive_i, image_kind, view_i, scale, relay, signal_tex_ready } = info;

    let inner = || -> Res1<image::RgbaImage> {
        let src_image = load_rgba_image_manga(archive_path, archive_i, image_kind)?;

        let dst_image = if let Some(ScaleImageManga { extent, filter }) = scale {
            resize_image_manga(src_image, extent, filter)?
        } else {
            src_image
        };

        Ok(dst_image)
    };

    match inner() {
        Ok(image) => if relay.send(Ok(ImageInfo { image, stage: Stage::Manga, index: view_i, signal_tex_ready })).is_ok() {
            ctx.request_repaint();
        },
        Err(err) => {
            _ = relay.send(Err((Stage::Manga, view_i)));

            Err(err)?;
        }
    }

    Ok(())
}

fn ferry_cached_image(info: FerryCachedImageInfo) -> Res1<()> {
    let FerryCachedImageInfo { ctx, path, stage, grid_entry_i, relay, wait_cache_ready, signal_tex_ready } = info;

    let inner = || -> ResVar<image::RgbaImage> {
        let image = load_rgba_image_cached(path)?;

        Ok(image)
    };

    if let Some(wait_cache_ready) = wait_cache_ready {
        wait_cache_ready.wait();
    }

    match inner() {
        Ok(image) => if relay.send(Ok(ImageInfo { image, stage, index: grid_entry_i, signal_tex_ready })).is_ok() {
            ctx.request_repaint();
        },
        Err(err) => {
            _ = relay.send(Err((stage, grid_entry_i)));

            Err(err)?;
        }
    }

    Ok(())
}

fn queue_ferry_base_image(info: QueueFerryBaseImageInfo) {
    let QueueFerryBaseImageInfo { queue, src_path, dst_path, cell_extent, grid_entry_i, relay, signal_cache_ready } = info;

    queue.send(QueueImageInfo {
        path: src_path.to_path_buf(),
        grid_entry_i,
        relay,
        scale: Some(ScaleImage {
            dst_path: dst_path.to_path_buf(),
            cell_extent
        }),
        cache_ready: signal_cache_ready
    })
    .unwrap();
}

fn queue_ferry_cached_image(info: QueueFerryCachedImageInfo) {
    let QueueFerryCachedImageInfo { queue, path, grid_entry_i, relay, wait_cache_ready } = info;

    queue.send(QueueImageInfo {
        path: path.to_path_buf(),
        grid_entry_i,
        relay,
        scale: None,
        cache_ready: wait_cache_ready
    })
    .unwrap();
}

fn ferry_images(info: FerryImagesInfo) {
    let FerryImagesInfo {
        ctx,
        thread_pool,
        metadata_sx,
        image_dirs,
        base_image_kind,
        grid_cell_extent,
        details_cell_extent,
        grid_relay,
        details_relay,
        ferry_image_infos,
        error_sx
    } = info;

    fn handle_err(error_sx: mpmc::Sender<String>, err: ErrLoc) {
        let msg = format!("{}: failed to ferry image: {}", module_path!(), err);
        send_log_err_msg(&error_sx, msg);
    }

    let (to_queue, from_queue) = mpmc::unbounded();
    for info in ferry_image_infos {
        let FerryImageInfo {
            image_file_name,
            expected_metadata,
            grid_entry_i,
            signal_cache_readies,
            wait_cache_readies,
            signal_tex_ready
        } = info;

        let ctx = ctx.clone();
        let metadata_sx = metadata_sx.cloned();
        let to_queue = to_queue.clone();
        let base_image_kind = base_image_kind.clone();
        let grid_relay = grid_relay.clone();
        let details_relay = details_relay.clone();
        let error_sx = error_sx.clone();

        thread_pool.spawn_fifo(move || {
            (|| -> Res<()> {
                let base_image_path = match base_image_kind {
                    BaseImageKind::Pick { path } => path,
                    BaseImageKind::Startup => image_dirs.base.join(image_file_name.as_ref())
                };
                let grid_image_path = image_dirs.grid.join(image_file_name.as_ref()).with_added_extension("webp");
                let details_image_path = image_dirs.details.join(image_file_name.as_ref()).with_added_extension("webp");

                // Check metadata for file changes
                let base_image_file = File::open(base_image_path.as_path())?;
                let metadata = base_image_file.metadata()?;
                let metadata = Arc::new(Metadata {
                    created: metadata.created()?,
                    modified: metadata.modified()?,
                    len: metadata.len()
                });
                let metadata_differs = expected_metadata.is_none_or(|expected_metadata| expected_metadata != metadata);

                match metadata_differs {
                    true => {
                        let ferry_base_image_info = FerryBaseImageInfo {
                            ctx,
                            src_path: &base_image_path,
                            dst_path: &grid_image_path,
                            cell_extent: grid_cell_extent,
                            stage: Stage::Grid,
                            grid_entry_i,
                            relay: grid_relay,
                            signal_cache_ready: signal_cache_readies.grid,
                            signal_tex_ready
                        };
                        ferry_base_image(ferry_base_image_info)?;

                        let queue_ferry_base_image_info = QueueFerryBaseImageInfo {
                            queue: to_queue,
                            src_path: &base_image_path,
                            dst_path: &details_image_path,
                            cell_extent: details_cell_extent,
                            grid_entry_i,
                            relay: details_relay,
                            signal_cache_ready: signal_cache_readies.details
                        };
                        queue_ferry_base_image(queue_ferry_base_image_info);

                        if let Some(metadata_sx) = metadata_sx {
                            metadata_sx.send(MetadataInfo { grid_entry_i, metadata }).unwrap();
                        }
                    },
                    false => {
                        match signal_cache_readies.grid.is_some() {
                            true => {
                                let ferry_base_image_info = FerryBaseImageInfo {
                                    ctx,
                                    src_path: &base_image_path,
                                    dst_path: &grid_image_path,
                                    cell_extent: grid_cell_extent,
                                    stage: Stage::Grid,
                                    grid_entry_i,
                                    relay: grid_relay,
                                    signal_cache_ready: signal_cache_readies.grid,
                                    signal_tex_ready
                                };
                                ferry_base_image(ferry_base_image_info)?
                            },
                            false => {
                                drop(signal_cache_readies.grid);

                                let ferry_cached_image_info = FerryCachedImageInfo {
                                    ctx,
                                    path: &grid_image_path,
                                    stage: Stage::Grid,
                                    grid_entry_i,
                                    relay: grid_relay,
                                    wait_cache_ready: wait_cache_readies.grid,
                                    signal_tex_ready
                                };
                                ferry_cached_image(ferry_cached_image_info)?;
                            }
                        }
                        match signal_cache_readies.details.is_some() {
                            true => {
                                let queue_ferry_base_image_info = QueueFerryBaseImageInfo {
                                    queue: to_queue,
                                    src_path: &base_image_path,
                                    dst_path: &details_image_path,
                                    cell_extent: details_cell_extent,
                                    grid_entry_i,
                                    relay: details_relay,
                                    signal_cache_ready: signal_cache_readies.details
                                };
                                queue_ferry_base_image(queue_ferry_base_image_info)
                            },
                            false => {
                                drop(signal_cache_readies.details);

                                let queue_ferry_cached_image_info = QueueFerryCachedImageInfo {
                                    queue: to_queue,
                                    path: &details_image_path,
                                    grid_entry_i,
                                    relay: details_relay,
                                    wait_cache_ready: wait_cache_readies.details
                                };
                                queue_ferry_cached_image(queue_ferry_cached_image_info)
                            }
                        }
                    }
                }

                Ok(())
            })()
            .unwrap_or_else(|err| handle_err(error_sx, err));
        });
    }

    drop(to_queue);
    let ctx = ctx.clone();
    let thread_pool = thread_pool.clone();
    thread::spawn(move || {
        for image_info in from_queue {
            let QueueImageInfo {
                path,
                grid_entry_i,
                relay,
                scale,
                cache_ready
            } = image_info;

            let ctx = ctx.clone();
            let error_sx = error_sx.clone();

            thread_pool.spawn_fifo(move || {
                (|| -> Res<()> {
                    match scale {
                        Some(scale) => {
                            let ScaleImage { dst_path, cell_extent } = scale;

                            let ferry_base_image_info = FerryBaseImageInfo {
                                ctx,
                                src_path: &path,
                                dst_path: &dst_path,
                                cell_extent,
                                stage: Stage::Details,
                                grid_entry_i,
                                relay,
                                signal_cache_ready: cache_ready,
                                signal_tex_ready: None
                            };
                            ferry_base_image(ferry_base_image_info)?;
                        },
                        None => {
                            let ferry_cached_image_info = FerryCachedImageInfo {
                                ctx,
                                path: &path,
                                stage: Stage::Details,
                                grid_entry_i,
                                relay,
                                wait_cache_ready: cache_ready,
                                signal_tex_ready: None
                            };
                            ferry_cached_image(ferry_cached_image_info)?
                        }
                    }

                    Ok(())
                })()
                .unwrap_or_else(|err| handle_err(error_sx, err));
            });
        }
    });
}

fn ferry_images_manga(info: FerryImagesInfoManga) {
    let FerryImagesInfoManga {
        ctx,
        thread_pool,
        archive_path,
        relay,
        ferry_image_infos,
        error_sx
    } = info;

    for info in ferry_image_infos {
        let ctx = ctx.clone();
        let archive_path = archive_path.clone();
        let relay = relay.clone();
        let error_sx = error_sx.clone();

        thread_pool.spawn_fifo(move || {
            (|| -> Res<()> {
                let ferry_image_manga_info = FerryImageMangaInfo {
                    ctx,
                    archive_path,
                    archive_i: info.archive_i,
                    image_kind: info.image_kind,
                    view_i: info.view_i,
                    scale: info.scale,
                    relay,
                    signal_tex_ready: info.signal_tex_ready
                };
                ferry_image_manga(ferry_image_manga_info)?;

                Ok(())
            })()
            .unwrap_or_else(|err| {
                let msg = format!("{}: failed to ferry image: {}", module_path!(), err);
                send_log_err_msg(&error_sx, msg);
            });
        });
    }
}

fn fix_background_brush(hnd: Win32WindowHandle) {
    fn make_colorref(r: u8, g: u8, b: u8) -> COLORREF {
        COLORREF(u32::from(r) | u32::from(g) << 8 | u32::from(b) << 16)
    }

    let hwnd = HWND(hnd.hwnd.get() as *mut c_void);
    (|| -> Res<()> {
        unsafe {
            let new_brush = CreateSolidBrush(make_colorref(27, 27, 27));
            let res = SetClassLongPtrW(hwnd, GCLP_HBRBACKGROUND, new_brush.0 as isize);

            if res == 0 { // Either the brush wasn't set previously or the function failed
                let maybe_err = GetLastError();

                let check = GetClassLongPtrW(hwnd, GCLP_HBRBACKGROUND).win32_core_ok()?;
                if check != new_brush.0 as usize {
                    maybe_err.ok()?;
                }
            }

            InvalidateRect(Some(hwnd), None, true).ok()?;
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

    AssocQueryStringW(ASSOCF_INIT_DEFAULTTOSTAR, ASSOCSTR_EXECUTABLE, *ext, None, Some(path_str), &mut path_len).ok()?;

    let path_str = String::from_utf16(&buffer[..path_len as usize - 1])?;

    Ok(PathBuf::from(path_str))
} }

fn get_image_states_mut(images: &mut IndexMap<Arc<str>, ImageStates>, image_i: Option<usize>) -> Option<&mut ImageStates> {
    let (_, image_states) = image_i.and_then(|image_i| images.get_index_mut(image_i)).unzip();

    image_states
}

fn get_image_states_from_grid_entry_mut<'a>(images: &'a mut IndexMap<Arc<str>, ImageStates>, grid_entries: &[GridEntryInfo], grid_entry_i: usize) -> Option<&'a mut ImageStates> {
    let (_, image_states) = grid_entries.get(grid_entry_i)
        .and_then(|grid_entry_info| grid_entry_info.image_i)
        .and_then(|image_i| images.get_index_mut(image_i))
        .unzip();

    image_states
}

fn highlight_rect(ui: &mut egui::Ui, rect: egui::Rect) {
    ui.painter().rect_stroke(rect, 0.0, CELL_STROKE, egui::StrokeKind::Outside);
}

fn init_residence(max_cell_count: usize, central_size: egui::Vec2, grid_cell_size: egui::Vec2, grid_cell_space: egui::Vec2, lookahead: usize) -> Residence {
    let available_row_cell_count = (central_size.x - grid_cell_size.x).div(grid_cell_space.x).ceil() as usize;
    let available_col_cell_count = central_size.y.div(grid_cell_space.y).ceil() as usize;
    let visible_cell_count = (available_row_cell_count * available_col_cell_count).clamp(1, max_cell_count);

    let resident_cell_count = (visible_cell_count + lookahead * available_row_cell_count).min(max_cell_count);
    let residence = 0..resident_cell_count;

    residence
}

fn open_media(path: PathBuf, file_kind: FileKind, maintain_sample_rate: bool, override_glsl_shaders: bool, discord_activity_info: Option<DiscordActivityInfo>, discord_display_kind: DiscordDisplayKind, error_sx: mpmc::Sender<String>) {
    thread::spawn(move || {
        (|| -> Res<()> {
            let ipc_client = discord_activity_info.as_ref().map(|activity_info| -> Res<_> {
                let mut ipc_client = DiscordIpcClient::new(activity_info.app_id.as_str());

                discord::begin(&mut ipc_client, &activity_info.as_view(), discord_display_kind)?;

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
            send_log_err_msg(&error_sx, msg);
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

fn requested_clear_selection(ui: &mut egui::Ui) -> bool {
    ui.ctx().input(|state| state.modifiers.ctrl && state.key_released(egui::Key::D))
}

fn requested_go_back(ui: & mut egui::Ui) -> bool {
    ui.ctx().input(|state|
        state.pointer.button_released(egui::PointerButton::Extra1) || state.key_released(egui::Key::Escape))
}

fn send_log_err_msg(error_sx: &mpmc::Sender<String>, msg: String) {
    error!("{}", msg);
    error_sx.send(msg).unwrap();
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
    resized_viewport: bool
}
impl eframe::App for Info {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        if !self.resized_viewport {
            let screen_size = ui.input(|i| i.viewport().monitor_size).unwrap();
            let win_size = screen_size.div(2.0).yx();

            let content_size = egui::CentralPanel::default()
                .show(ui, |ui: &mut egui::Ui| {
                    ui.set_max_width(win_size.x);

                    Self::central_panel(self, ui)
                })
                .inner;

            let win_size = win_size.min(content_size) + egui::Vec2::splat(2.0 * DEFAULT_FRAME_INNER_MARGIN);
            let win_pos = egui::pos2(
                (screen_size.x - win_size.x) / 2.0,
                (screen_size.y - win_size.y).div(2.0).max(0.0)
            );

            ui.send_viewport_cmd(egui::ViewportCommand::InnerSize(win_size));
            ui.send_viewport_cmd(egui::ViewportCommand::OuterPosition(win_pos));
            ui.send_viewport_cmd(egui::ViewportCommand::Focus);

            self.resized_viewport = true;
        }

        egui::CentralPanel::default()
            .show(ui, |ui: &mut egui::Ui| Self::central_panel(self, ui));
    }
}
impl Info {
    fn new(msg: String) -> Self {
        Self {
            msg,
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
    wgpu: egui_wgpu::RenderState,
    thread_pool: Arc<rayon::ThreadPool>,
    image_dirs: &'static ImageDirs,
    images: IndexMap<Arc<str>, ImageStates>,
    deferred_metadata_sx: mpmc::Sender<MetadataInfo>,
    deferred_metadata_rx: mpmc::Receiver<MetadataInfo>,
    pick_image_metadata_sx: mpmc::Sender<MetadataInfo>,
    pick_image_metadata_rx: mpmc::Receiver<MetadataInfo>,
    cache_path: PathBuf,
    cache: Cache,
    missing_base_images: Vec<Rc<str>>,
    frame: egui::Frame,
    central_rect: egui::Rect,
    view_kind: ViewKind,
    grid_entries: Vec<GridEntryInfo>,
    grid_entry_i: usize,
    grid_entries_selection: HashSet<usize>,
    grid_entries_selection_kind: Option<SelectionKind>,
    grid_cell_size: egui::Vec2,
    grid_cell_space: egui::Vec2,
    grid_cell_highlights: GridCellHighlights,
    grid_cell_tags_menu_selection: HashSet<Rc<str>>,
    grid_scroll_offset: f32,
    /// Indices into [`grid_entries`]
    grid_view: Vec<usize>,
    grid_view_i: usize,
    grid_view_pending_op: Option<GridViewOp>,
    lookahead: usize,
    proximity: usize,
    animation: Option<AnimationInfo>,
    residence: Range<usize>,
    stream: Stream,
    animate_bool: bool,
    sort_name_edit: String,
    new_tag_edit: String,
    /// Sets of indices into [`grid_entries`]
    tags: BTreeMap<Rc<str>, BTreeSet<usize>>,
    active_tag: Option<Rc<str>>,
    tag_win_should_open: bool,
    tag_win_button_menu_is_open: bool,
    tag_win_button_pending_tag_op: Option<PendingTagOp>,
    tag_win_rename_edit: String,
    tag_win_time_stamp: Option<Instant>,
    tag_win_cursor_checked: bool,
    details_grid_entry_i: usize,
    details_dir_entries: Vec<DirEntryInfo>,
    details_cell_size: egui::Vec2,
    details_hovered_dir_entry_i: usize,
    details_levels: Vec<PathBuf>,
    scroll_kind: ScrollKind,
    scroll_multiplier: f32,
    scroll_multiplier_display: String,
    scroll_multiplier_edit: String,
    spring_damper: SpringDamper,
    maintain_sample_rate: bool,
    override_glsl_shaders: bool,
    enable_override_glsl_shaders_checkbox: bool,
    discord_app_ids: DiscordAppIds<'a>,
    discord_enabled: bool,
    discord_watching: Watching,
    discord_details_edit: String,
    discord_state_edit: String,
    discord_display_kind: DiscordDisplayKind,
    open_error_win: bool,
    image_sx: mpmc::Sender<ImageResult>,
    from_charon: mpmc::Receiver<ImageResult>,
    to_hephaestus: mpmc::Sender<WriteTex>,
    to_demeter: mpmc::Sender<ImageInfo>,
    from_demeter: mpmc::Receiver<PartialTex>,
    to_thanatos: mpmc::Sender<Soul>,
    partial_tex_stash: VecDeque<PartialTex>,
    to_demeter_stash: VecDeque<ImageInfo>,
    chunk_size: usize,
    poll_ready: PollReady,
    manga: Manga,
    error_sx: mpmc::Sender<String>,
    error_rx: mpmc::Receiver<String>,
    error_msg: String,
    frame_count: usize
}
impl<'a> eframe::App for MediaBrowser<'a> {
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        egui::Color32::from_rgba_unmultiplied(27, 27, 27, 255).to_normalized_gamma_f32()
    }

    #[hotpath::measure]
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        for MetadataInfo { grid_entry_i, metadata } in self.pick_image_metadata_rx.try_iter() {
            let grid_entry_info = &mut self.grid_entries[grid_entry_i];
            grid_entry_info.metadata = Some(metadata);
        }

        egui::CentralPanel::default()
            .frame(self.frame)
            .show(ui, |ui: &mut egui::Ui| {
                self.central_rect = ui.available_rect_before_wrap();

                match self.view_kind {
                    ViewKind::Grid => self.central_panel_grid(ui),
                    ViewKind::Details => self.central_panel_details(ui),
                    ViewKind::InitManga { selected_details_dir_entry_i } => self.init_manga(ui, selected_details_dir_entry_i),
                    ViewKind::WaitManga => self.wait_manga(),
                    ViewKind::Manga => self.central_panel_manga(ui)
                }
            });

        if !self.open_error_win &&
            let Ok(msg) = self.error_rx.try_recv()
        {
            self.error_msg = msg;
            self.open_error_win = true;
        }

        egui::Window::new("Error")
            .open(&mut self.open_error_win)
            .auto_sized()
            .show(ui, |ui| ui.label(self.error_msg.as_str()));

        self.frame_count += 1;
    }

    fn on_exit(&mut self) {
        for image_file_name in self.missing_base_images.drain(..) { // Base images ref'd by cache entries that have been moved or deleted - remove grid/details images from disk
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
        self.cache.spring_damper = self.spring_damper.as_ref().into();
        self.cache.spring_damper_manga = self.manga.spring_damper.as_ref().into();
        self.cache.tags = self.tags.keys().cloned().collect();
        self.cache.images = self.images.keys().map(|key| Rc::from(key.as_ref())).collect();
        self.cache.entries.clear();

        let mut grid_entry_tags = vec![Vec::with_capacity(self.tags.len()); self.grid_entries.len()];
        for (tag_i, (_, set)) in self.tags.iter().enumerate() {
            for grid_entry_i in set {
                grid_entry_tags[*grid_entry_i].push(tag_i);
            }
        }

        while let Ok(MetadataInfo { grid_entry_i, metadata }) = self.deferred_metadata_rx.try_recv() {
            self.grid_entries[grid_entry_i].metadata = Some(metadata);
        }

        for (i, info) in mem::take(&mut self.grid_entries).into_iter().enumerate() {
            let image_states = self.get_image_states_mut(info.image_i);
            let should_scale = image_states.as_ref()
                .map(|image_states| image_states.should_scale())
                .unwrap_or_default();

            self.cache.entries.insert(
                info.path,
                CacheEntryInfo {
                    image_i: info.image_i,
                    should_scale,
                    sort_name: info.sort_name,
                    metadata: info.image_i.map(|_| info.metadata).unwrap_or_default(),
                    tags: mem::take(&mut grid_entry_tags[i])
                }
            );
        }

        let cache_file = File::options()
            .truncate(true)
            .write(true)
            .open(&self.cache_path).unwrap();
        serde_json::to_writer_pretty(cache_file, &self.cache).unwrap();
    }
}
impl<'a> MediaBrowser<'a> {
    fn new(ctx: &egui::Context, wgpu: &egui_wgpu::RenderState, win_inner_extent: Extent2dU) -> Res<Self> {
        let config = config::get().read()?;
        let (media_dirs,
            grid_cell_width,
            details_cell_width,
            scroll_multiplier,
            lookahead,
            proximity,
            animation
        ) = config.media_browser.as_ref()
            .map(|media_browser_config| {
                (
                    &media_browser_config.dirs,
                    media_browser_config.grid_cell_width.next_multiple_of(2) as f32,
                    media_browser_config.details_cell_width.next_multiple_of(2) as f32,
                    media_browser_config.scroll_multiplier,
                    media_browser_config.lookahead,
                    media_browser_config.proximity,
                    media_browser_config.animation
                )
            })
            .ok_or(ErrVar::MissingConfigOption { name: config::MediaBrowser::NAME })?;

        if media_dirs.is_empty() { Err(ErrVar::MissingDirs)?; }
        if lookahead < 2 { Err(ErrVar::InvalidLookahead(lookahead))?; }
        let proximity_range = 1..lookahead;
        if !(proximity_range.contains(&proximity)) { Err(ErrVar::InvalidProximity(proximity))?; }

        let grid_cell_size = egui::vec2(grid_cell_width, grid_cell_width * ASPECT_RATIO_3_2);
        let grid_cell_space = grid_cell_size + GRID_IMAGE_SPACING;
        let details_cell_size = egui::vec2(details_cell_width, details_cell_width * ASPECT_RATIO_3_2);
        let discord_app_ids = config.discord.app_ids.clone();
        let discord_display_kind = config.discord.display_kind;
        let enable_override_glsl_shaders_checkbox = config.mpv.as_ref().map(|mpv_config| mpv_config.override_glsl_shaders.is_some()).unwrap_or(false);

        let thread_pool = Arc::new(rayon::ThreadPoolBuilder::new()
            .num_threads(thread::available_parallelism()?.get().saturating_sub(4))
            .build()?);
        let (deferred_metadata_sx, deferred_metadata_rx) = mpmc::unbounded();
        let (pick_image_metadata_sx, pick_image_metadata_rx) = mpmc::bounded(1);

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
        let mut missing_base_images = Vec::new();

        let spring_damper = cache.spring_damper.into();
        let spring_damper_manga = cache.spring_damper_manga.into();

        let frame = egui::Frame::central_panel(&ctx.global_style()).inner_margin(
            egui::Margin::symmetric(FRAME_INNER_MARGIN as i8, FRAME_INNER_MARGIN as i8)
        );

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

                    let try_get_image_i = |images: &mut IndexMap<Arc<str>, ImageStates>| {
                        for ext in IMAGE_EXTS {
                            let attempt = concat_string!(stem, ".", ext);

                            if let Some((image_i, _, states)) = images.get_full_mut(attempt.as_str()) {
                                states.ref_count += 1;

                                return Some(image_i)
                            }
                        }

                        None
                    };

                    let cache_entry_info = cache.entries.get_mut(&path);
                    let grid_entry_info = match cache_entry_info {
                        Some(cache_entry_info) => {
                            let sort_name = cache_entry_info.sort_name.clone();
                            let image_i = cache_entry_info.image_i
                                .and_then(|cache_image_i| cache.images.get_index(cache_image_i))
                                .and_then(|image_file_name| images.get_full_mut(image_file_name.as_ref())
                                    .tap_none(|| missing_base_images.push(image_file_name.clone()))
                                )
                                .map(|(image_i, _, image_states)| {
                                    if cache_entry_info.should_scale.grid || cache.grid_cell_size != grid_cell_size {
                                        image_states.grid = ImageState::Scale { cache_ready: Some(WaitGroup::new()) };
                                    }
                                    if cache_entry_info.should_scale.details || cache.details_cell_size != details_cell_size {
                                        image_states.details = ImageState::Scale { cache_ready: Some(WaitGroup::new()) };
                                    }
                                    image_states.ref_count += 1;

                                    image_i
                                });
                            let metadata = cache_entry_info.metadata.clone();

                            GridEntryInfo { path, stem, sort_name, file_kind, image_i, metadata }
                        },
                        None => {
                            let sort_name = None;
                            let image_i = try_get_image_i(&mut images);
                            let metadata = None;

                            GridEntryInfo { path, stem, sort_name, file_kind, image_i, metadata }
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

        let mut grid_view = Vec::with_capacity(grid_entries.len());
        grid_view.extend(0..grid_entries.len());
        sort_grid_view(&mut grid_view, &grid_entries);

        drop(config);

        let ctx_ = ctx.clone();
        let wgpu_ = wgpu.clone();
        let (to_hephaestus, write_tex_rx) = mpmc::unbounded();
        thread::spawn(move || hephaestus(ctx_, wgpu_, write_tex_rx));

        let ctx_ = ctx.clone();
        let wgpu_ = wgpu.clone();
        let chunk_size = 16 * 1024_usize.pow(2);
        let (image_sx, from_charon) = mpmc::unbounded();
        let (to_demeter, image_rx) = mpmc::unbounded();
        let (partial_tex_sx, from_demeter) = mpmc::unbounded();
        thread::spawn(move || demeter(ctx_, wgpu_, image_rx, partial_tex_sx, chunk_size));

        let (to_thanatos, image_state_rx) = mpmc::unbounded();
        let wgpu_ = wgpu.clone();
        thread::spawn(move || thanatos(wgpu_, image_state_rx));

        let (error_sx, error_rx) = mpmc::unbounded();
        let error_msg = "".to_string();

        let win_inner_extent = Extent2dF::from(win_inner_extent);
        let central_size = egui::vec2(
            win_inner_extent.width.sub(2. * FRAME_INNER_MARGIN).max(0.),
            win_inner_extent.height.sub(2. * FRAME_INNER_MARGIN).max(0.),
        );
        let residence = init_residence(grid_view.len(), central_size, grid_cell_size, grid_cell_space, lookahead);
        let (resident_grid_sx, resident_grid_rx) = mpmc::unbounded();
        let ferry_image_infos = grid_view.iter().enumerate()
            .filter_map(|(grid_view_i, &grid_entry_i)| {
                let grid_entry_info = &grid_entries[grid_entry_i];

                // Fill tags
                if let Some(CacheEntryInfo { tags: tag_is, .. }) = cache.entries.get_mut(&grid_entry_info.path) {
                    for tag_i in tag_is {
                        let tag = &cache.tags[*tag_i];
                        let set = tags.get_mut(tag);

                        if let Some(set) = set {
                            set.insert(grid_entry_i);
                        }
                    }
                }

                if let Some(image_i) = grid_entry_info.image_i && grid_view_i < residence.end {
                    let (image_file_name, image_states) = images.get_index_mut(image_i).unwrap();

                    return Some(FerryImageInfo {
                        image_file_name: image_file_name.clone(),
                        expected_metadata: grid_entry_info.metadata.clone(),
                        grid_entry_i,
                        signal_cache_readies: image_states.clone_cache_readies_on_scale(),
                        wait_cache_readies: CacheReadies::NONE,
                        signal_tex_ready: None
                    })
                }

                None
            })
            .collect::<Vec<_>>();
        let ferry_images_info = FerryImagesInfo {
            ctx,
            thread_pool: &thread_pool,
            metadata_sx: Some(&deferred_metadata_sx),
            image_dirs,
            base_image_kind: BaseImageKind::Startup,
            grid_cell_extent: grid_cell_size.into(),
            details_cell_extent: details_cell_size.into(),
            grid_relay: resident_grid_sx.clone(),
            details_relay: image_sx.clone(),
            ferry_image_infos,
            error_sx: error_sx.clone()
        };
        ferry_images(ferry_images_info);

        drop(resident_grid_sx);
        for res in resident_grid_rx.iter() {
            match res {
                Ok(info) => {
                    let ImageInfo { image, index, .. } = info;

                    let (tex, tex_id) = alloc_write_texture(wgpu, &image);
                    _ = to_thanatos.send(Soul::RgbaImage(image));

                    let image_states = get_image_states_from_grid_entry_mut(&mut images, &grid_entries, index).unwrap();
                    image_states.grid = ImageState::Ready { tex_id, extent: tex.as_(), cache_ready: None };
                },
                Err((_, index)) => {
                    let image_states = get_image_states_from_grid_entry_mut(&mut images, &grid_entries, index).unwrap();
                    image_states.grid = ImageState::Failed;
                }
            }
        }

        let resident_ready = WaitGroup::new();
        let resident_ready_ = resident_ready.clone();
        wgpu.queue.on_submitted_work_done(move || drop(resident_ready_));
        wgpu.queue.submit(None);
        resident_ready.wait();

        Ok(Self {
            wgpu: wgpu.clone(),
            thread_pool,
            image_dirs,
            images,
            deferred_metadata_sx,
            deferred_metadata_rx,
            pick_image_metadata_sx,
            pick_image_metadata_rx,
            cache_path,
            cache,
            missing_base_images,
            frame,
            central_rect: egui::Rect::ZERO,
            view_kind: ViewKind::Grid,
            grid_entries,
            grid_entry_i: default!(),
            grid_entries_selection: default!(),
            grid_entries_selection_kind: default!(),
            grid_cell_size,
            grid_cell_space,
            grid_cell_highlights: default!(),
            grid_cell_tags_menu_selection: default!(),
            grid_scroll_offset: default!(),
            grid_view,
            grid_view_i: default!(),
            grid_view_pending_op: default!(),
            lookahead,
            proximity,
            animation,
            residence,
            stream: default!(),
            animate_bool: true,
            sort_name_edit: default!(),
            new_tag_edit: default!(),
            tags,
            active_tag: default!(),
            tag_win_should_open: default!(),
            tag_win_button_menu_is_open: default!(),
            tag_win_button_pending_tag_op: default!(),
            tag_win_rename_edit: default!(),
            tag_win_time_stamp: default!(),
            tag_win_cursor_checked: default!(),
            details_grid_entry_i: default!(),
            details_dir_entries: Vec::with_capacity(DETAILS_ENTRY_COUNT),
            details_cell_size,
            details_hovered_dir_entry_i: default!(),
            details_levels: Vec::with_capacity(16),
            scroll_kind: default!(),
            scroll_multiplier,
            scroll_multiplier_display: default!(),
            scroll_multiplier_edit: default!(),
            spring_damper,
            maintain_sample_rate: default!(),
            override_glsl_shaders: default!(),
            enable_override_glsl_shaders_checkbox,
            discord_app_ids,
            discord_enabled: default!(),
            discord_watching: default!(),
            discord_details_edit: default!(),
            discord_state_edit: default!(),
            discord_display_kind,
            image_sx,
            from_charon,
            to_hephaestus,
            to_demeter,
            from_demeter,
            to_thanatos,
            partial_tex_stash: default!(),
            to_demeter_stash: default!(),
            chunk_size,
            poll_ready: PollReady::new(),
            manga: Manga::new(spring_damper_manga),
            open_error_win: default!(),
            error_sx,
            error_rx,
            error_msg,
            frame_count: default!()
        })
    }

    fn assign_image_state(&mut self, tex_id: egui::TextureId, stage: Stage, extent: Extent2dF, index: usize) {
        match stage {
            Stage::Grid => {
                let image_states = self.get_image_states_from_grid_entry_mut(index).unwrap();
                let cache_ready = image_states.grid.take_cache_ready();
                image_states.grid = ImageState::Ready { tex_id, extent, cache_ready };
            },
            Stage::Details => {
                let image_states = self.get_image_states_from_grid_entry_mut(index).unwrap();
                let cache_ready = image_states.details.take_cache_ready();
                image_states.details = ImageState::Ready { tex_id, extent, cache_ready };
            },
            Stage::Manga => self.manga.view[index].image_state = ImageStateManga::Ready { tex_id, extent }
        }
    }

    fn get_animation_opacity(&mut self, ui: &mut egui::Ui) -> Option<f32> {
        self.animation.as_ref().map(|animation| {
            match self.animate_bool {
                true => ui.ctx().animate_bool_with_time_and_easing("animate".into(), true, animation.dur, animation.kind.as_easing()),
                false => {
                    ui.ctx().clear_animations();
                    self.animate_bool = true; // For future calls

                    ui.ctx().animate_bool_with_time_and_easing("animate".into(), false, animation.dur, animation.kind.as_easing())
                }
            }
        })
    }

    fn get_image_states_mut(&mut self, image_i: Option<usize>) -> Option<&mut ImageStates> {
        get_image_states_mut(&mut self.images, image_i)
    }

    fn get_image_states_from_grid_entry_mut(&mut self, grid_entry_i: usize) -> Option<&mut ImageStates> {
        get_image_states_from_grid_entry_mut(&mut self.images, &self.grid_entries, grid_entry_i)
    }

    #[hotpath::measure]
    fn init_manga(&mut self, ui: &mut egui::Ui, selected_details_dir_entry_i: usize) { //$ Slow
        let dir_entry_info = &self.details_dir_entries[selected_details_dir_entry_i];
        let archive_path = Arc::new(dir_entry_info.path.clone());
        let archive = fs::File::open(archive_path.as_path()).unwrap();
        let archive = io::BufReader::new(archive);
        let mut archive = zip::ZipArchive::new(archive).unwrap();

        self.manga.archive_pages.reserve_exact(archive.len());
        self.manga.view.reserve_exact(archive.len());

        for archive_i in 0..archive.len() {
            let page = archive.by_index(archive_i).unwrap();
            let name = page.name().to_string();
            let mut page = io::BufReader::new(page);

            let ext = Path::new(name.as_str()).get_file_ext().unwrap();
            let (image_kind, width, height) = match ext.to_ascii_lowercase().as_str() {
                "jpg" | "jpeg" => {
                    let mut decoder = jpeg_decoder::Decoder::new(page);
                    decoder.read_info().unwrap();
                    let decoder_info = decoder.info().unwrap();

                    (ImageKind::Jpeg, decoder_info.width as f32, decoder_info.height as f32)
                },
                "png" => {
                    let mut buf = [0_u8; 24];
                    page.read_exact(&mut buf).unwrap();

                    let width_slice = buf.get(16..20).unwrap();
                    let height_slice = buf.get(20..24).unwrap();
                    let width = u32::from_be_bytes(width_slice.try_into().unwrap());
                    let height = u32::from_be_bytes(height_slice.try_into().unwrap());

                    (ImageKind::Png, width as f32, height as f32)
                },
                _ => {
                    let msg = format!("{}: unsupported image format: archive index: {}, name: {}", module_path!(), archive_i, name);
                    send_log_err_msg(&self.error_sx, msg);

                    self.view_kind = ViewKind::Details;

                    return
                }
            };

            self.manga.archive_pages.push(ArchivePageInfo { name, index: archive_i, image_kind, extent: [width, height].into() });
            self.manga.archive_pages_width = self.manga.archive_pages_width.max(width);
        }
        self.manga.archive_pages.sort_unstable_by(|a, b| a.name.cmp(&b.name));

        let mut view_page_offset = 0_f32;
        for archive_page_info in self.manga.archive_pages.iter() {
            let view_page_info = ViewPageInfo {
                archive_i: archive_page_info.index,
                image_kind: archive_page_info.image_kind,
                offset: view_page_offset,
                extent: archive_page_info.extent,
                image_state: default!()
            };
            self.manga.view.push(view_page_info);

            view_page_offset += archive_page_info.extent.height;
        }
        self.manga.view_extent = [self.manga.archive_pages_width, view_page_offset].into();

        let visible_page_count = self.init_residence_manga();

        let ferry_image_infos = (0..visible_page_count)
            .map(|view_i| {
                let ViewPageInfo { archive_i, image_kind, .. } = self.manga.view[view_i];

                FerryImageInfoManga {
                    archive_i,
                    image_kind,
                    view_i,
                    scale: None,
                    signal_tex_ready: Some(self.poll_ready.clone())
                }
            })
            .collect::<Vec<_>>();
        let ferry_images_info = FerryImagesInfoManga {
            ctx: ui.ctx(),
            thread_pool: &self.thread_pool,
            archive_path: archive_path.clone(),
            relay: self.image_sx.clone(),
            ferry_image_infos,
            error_sx: self.error_sx.clone()
        };
        ferry_images_manga(ferry_images_info);

        let ferry_image_infos = (visible_page_count..self.manga.residence.end)
            .map(|view_i| {
                let ViewPageInfo { archive_i, image_kind, .. } = self.manga.view[view_i];

                FerryImageInfoManga {
                    archive_i,
                    image_kind,
                    view_i,
                    scale: None,
                    signal_tex_ready: None
                }
            })
            .collect::<Vec<_>>();
        let ferry_images_info = FerryImagesInfoManga {
            ctx: ui.ctx(),
            thread_pool: &self.thread_pool,
            archive_path: archive_path.clone(),
            relay: self.image_sx.clone(),
            ferry_image_infos,
            error_sx: self.error_sx.clone()
        };
        ferry_images_manga(ferry_images_info);

        self.manga.archive.set(archive);
        self.manga.archive_path.set(archive_path);
        self.manga.to_thanatos.set(self.to_thanatos.clone());

        self.view_kind = ViewKind::WaitManga;
    }

    fn wait_manga(&mut self) {
        self.stream_textures_stepped();

        if self.poll_ready.is_ready() {
            self.animate_bool = false;
            self.view_kind = ViewKind::Manga;
        }
    }

    fn init_residence_manga(&mut self) -> VisiblePageCount {
        let max_page_count = self.manga.view.len();

        let visible_page_count = self.manga.view.partition_point(|page_info| page_info.offset <= self.central_rect.height());
        let resident_page_count = (visible_page_count + self.lookahead).min(max_page_count);

        self.manga.residence = 0..resident_page_count;

        visible_page_count
    }

    fn reset_grid_entries_selection(&mut self) {
        self.grid_cell_tags_menu_selection.clear();
        self.grid_entries_selection.clear();
        self.grid_entries_selection_kind = None;
    }

    fn refresh_grid_view(&mut self, ui: &mut egui::Ui) -> GridViewCellCounts {
        self.stream.flatten_drop(self.residence.clone(), &self.grid_view);
        let active_tag = self.active_tag.as_deref().unwrap();
        let set = self.tags.get(active_tag).unwrap();
        populate_grid_view(&mut self.grid_view, &self.grid_entries, set);

        let ResetResidence { row_cell_count, visible_cell_count } = self.reset_residence();
        self.stream.flatten_load(self.residence.clone(), 0..visible_cell_count, &self.grid_view);
        self.stream(ui);

        self.reset_grid_entries_selection();

        GridViewCellCounts { row: row_cell_count, max: self.grid_view.len() }
    }

    fn reset_grid_view(&mut self, ui: &mut egui::Ui) -> GridViewCellCounts {
        self.grid_view.clear();
        self.grid_view.extend(0..self.grid_entries.len());
        self.sort_grid_view();

        let ResetResidence { row_cell_count, visible_cell_count } = self.reset_residence();
        let stream_builder = StreamBuilder::default().with_load(self.residence.clone());
        self.stream.refresh_flatten(stream_builder, 0..visible_cell_count, &self.grid_view);
        self.stream(ui);

        self.reset_grid_entries_selection();
        self.animate_bool = false;
        self.active_tag = None;

        GridViewCellCounts { row: row_cell_count, max: self.grid_view.len() }
    }

    fn reset_residence(&mut self) -> ResetResidence {
        let max_cell_count = self.grid_view.len();

        let available_row_cell_count = (self.central_rect.width() - self.grid_cell_size.x).div(self.grid_cell_space.x).ceil() as usize;
            // ui.available_width() - (self.grid_cell_size.x * avail_row_cell_count - GRID_IMAGE_SPACING.x) <= self.grid_cell_size.x
        let available_col_cell_count = self.central_rect.height().div(self.grid_cell_space.y).ceil() as usize;
        let visible_cell_count = (available_row_cell_count * available_col_cell_count).clamp(1, max_cell_count);
        let row_cell_count = available_row_cell_count.clamp(1, max_cell_count);

        let resident_cell_count = (visible_cell_count + self.lookahead * available_row_cell_count).min(max_cell_count);
        self.residence = 0..resident_cell_count;

        ResetResidence {
            visible_cell_count,
            row_cell_count
        }
    }

    fn sort_grid_view(&mut self) {
        sort_grid_view(&mut self.grid_view, &self.grid_entries);
    }

    fn stream(&mut self, ctx: &egui::Context) {
        if !self.stream.drop.is_empty() {
            for grid_entry_i in self.stream.drop.iter() {
                let grid_entry_info = &self.grid_entries[*grid_entry_i];

                if let Some(image_i) = grid_entry_info.image_i {
                    let (_, image_states) = self.images.get_index_mut(image_i).unwrap();

                    image_states.ref_count = image_states.ref_count.saturating_sub(1);
                    if image_states.ref_count == 0 {
                        let new_image_states = ImageStates::new_none_check_cache(image_states.take_cache_readies());

                        let old_image_states = mem::replace(image_states, new_image_states);
                        self.to_thanatos.send(Soul::ImageStates(old_image_states)).unwrap();
                    }
                }
            }
        }

        let mut make_ferry_images_info = |load: &HashSet<usize>, signal_tex_ready: bool| -> FerryImagesInfo {
            let ferry_image_infos = load.iter().copied()
                .filter_map(|grid_entry_i| {
                    let grid_entry_info = &mut self.grid_entries[grid_entry_i];

                    if let Some(image_i) = grid_entry_info.image_i {
                        let (image_file_name, image_states) = self.images.get_index_mut(image_i).unwrap();

                        return Some(FerryImageInfo {
                            image_file_name: image_file_name.clone(),
                            expected_metadata: grid_entry_info.metadata.clone(),
                            grid_entry_i,
                            signal_cache_readies: image_states.clone_cache_readies_on_scale(),
                            wait_cache_readies: image_states.take_cache_readies_on_not_scale(),
                            signal_tex_ready: signal_tex_ready.then(|| self.poll_ready.clone())
                        })
                    }

                    None
                })
                .collect::<Vec<_>>();

            FerryImagesInfo {
                ctx,
                thread_pool: &self.thread_pool,
                metadata_sx: Some(&self.deferred_metadata_sx),
                image_dirs: self.image_dirs,
                base_image_kind: BaseImageKind::Startup,
                grid_cell_extent: self.grid_cell_size.into(),
                details_cell_extent: self.details_cell_size.into(),
                grid_relay: self.image_sx.clone(),
                details_relay: self.image_sx.clone(),
                ferry_image_infos,
                error_sx: self.error_sx.clone()
            }
        };

        ferry_images(make_ferry_images_info(&self.stream.load_first, true));
        ferry_images(make_ferry_images_info(&self.stream.load_after, false));
    }

    fn stream_manga(&mut self, ctx: &egui::Context, scale: bool) {
        if !self.manga.stream.drop.is_empty() {
            for view_i in self.manga.stream.drop.iter() {
                let page_info = &mut self.manga.view[*view_i];
                let old_image_state = mem::take(&mut page_info.image_state);
                self.to_thanatos.send(Soul::ImageState(old_image_state.into())).unwrap();
            }
        }

        let mut make_ferry_images_info = |load: &HashSet<usize>, signal_tex_ready: bool| -> FerryImagesInfoManga {
            let ferry_image_infos = load.iter().copied()
                .map(|view_i| {
                    let page_info = &mut self.manga.view[view_i];

                    FerryImageInfoManga {
                        archive_i: page_info.archive_i,
                        image_kind: page_info.image_kind,
                        view_i,
                        scale: scale.then_some(ScaleImageManga { extent: self.manga.view[view_i].extent, filter: self.manga.filter }),
                        signal_tex_ready: signal_tex_ready.then(|| self.poll_ready.clone())
                    }
                })
                .collect::<Vec<_>>();

            FerryImagesInfoManga {
                ctx,
                thread_pool: &self.thread_pool,
                archive_path: self.manga.archive_path.clone(),
                relay: self.image_sx.clone(),
                ferry_image_infos,
                error_sx: self.error_sx.clone()
            }
        };

        ferry_images_manga(make_ferry_images_info(&self.manga.stream.load_first, true));
        ferry_images_manga(make_ferry_images_info(&self.manga.stream.load_after, false));
    }

    #[hotpath::measure]
    fn stream_textures_stepped(&mut self) {
        let mut sentinel = self.chunk_size;

        sentinel -= self.try_write_partial_tex(sentinel);

        while let Ok(mut partial_tex) = self.from_demeter.try_recv() {
            if partial_tex.offset == partial_tex.tex.height() as usize {
                if let Some((image, signal_tex_ready)) = partial_tex.captive.take() {
                    if let Some(signal_tex_ready) = signal_tex_ready {
                        signal_tex_ready.mark_done();
                    }
                    _ = self.to_thanatos.send(Soul::RgbaImage(image));
                }

                let PartialTex { tex, tex_id, stage, index, .. } = partial_tex;
                self.assign_image_state(tex_id, stage, tex.as_(), index);
            } else {
                self.partial_tex_stash.push_back(partial_tex);
                sentinel -= self.try_write_partial_tex(sentinel);
            }
        }

        while let Some(image_info) = self.to_demeter_stash.front() {
            let image_size = image_info.image.as_raw().len();

            if image_size <= sentinel {
                let image_info = self.to_demeter_stash.pop_front().unwrap();
                self.to_demeter.send(image_info).unwrap();
                sentinel -= image_size;

                continue
            }

            break
        }

        while let Ok(res) = self.from_charon.try_recv() {
            match res {
                Ok(image_info) => {
                    let image_size = image_info.image.as_raw().len();

                    if image_size <= sentinel {
                        // Writable now
                        _ = self.to_demeter.send(image_info);
                        sentinel -= image_size;

                        continue
                    }

                    if image_size > self.chunk_size {
                        // Clearable now
                        _ = self.to_demeter.send(image_info);

                        continue
                    }

                    // Writable next time
                    self.to_demeter_stash.push_back(image_info);

                    break
                },
                Err((stage, index)) => match stage {
                    Stage::Grid => self.get_image_states_from_grid_entry_mut(index).unwrap().grid = ImageState::Failed,
                    Stage::Details => self.get_image_states_from_grid_entry_mut(index).unwrap().details = ImageState::Failed,
                    Stage::Manga => self.manga.view[index].image_state = ImageStateManga::Failed
                }
            }
        }
    }

    fn try_write_partial_tex(&mut self, sentinel: usize) -> WrittenSize {
        if sentinel != 0 && let Some(partial_tex) = self.partial_tex_stash.front_mut() {
            let PartialTex { tex, captive, offset, .. } = partial_tex;
            let PartialTex { tex_id, stage, index, row_size, chunk_row_count, .. } = *partial_tex;

            let tex_height = tex.height() as usize;
            let remaining_row_count = tex_height.saturating_sub(*offset);
            let sentinel_row_count = sentinel / row_size;
            let write_row_count = chunk_row_count.min(sentinel_row_count).min(remaining_row_count);
            let last_write = write_row_count == remaining_row_count;
            let write_size = write_row_count * row_size;
            let write_tex = WriteTex { tex: tex.clone(), captive: captive.take(), offset: *offset, row_count: write_row_count, last_write };

            self.to_hephaestus.send(write_tex).unwrap();

            if last_write {
                let extent = Extent2dF { width: tex.width() as f32, height: tex.height() as f32 };
                self.assign_image_state(tex_id, stage, extent, index);

                self.partial_tex_stash.pop_front();
            } else {
                *offset += write_row_count;
            }

            return write_size
        }

        0
    }

    fn update_residence(&mut self, visible_row_range: &Range<usize>, row_cell_count: usize, max_cell_count: usize) -> ShouldStream {
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
        let stream_builder = match cmp {
            RangeCmpResult::RangeEmpty |
            RangeCmpResult::CompletelyTheSame |
            RangeCmpResult::CompletelyIncluded { .. } |
            RangeCmpResult::MiddleIncluded { .. } =>
                return false,
            RangeCmpResult::NotIncludedBelow |
            RangeCmpResult::NotIncludedAbove =>
                StreamBuilder::default().with_drop(self.residence.clone()).with_load(new_residence.clone()),
            RangeCmpResult::EndIncluded { other_after, original_part_which_is_not_included, .. } =>
                StreamBuilder::default().with_drop(original_part_which_is_not_included).with_load(other_after),
            RangeCmpResult::StartIncluded { other_before, original_part_which_is_not_included, .. } =>
                StreamBuilder::default().with_drop(original_part_which_is_not_included).with_load(other_before),
            RangeCmpResult::SameStartOriginalShorter { other_after_not_included, .. } =>
                StreamBuilder::default().with_load(other_after_not_included),
            RangeCmpResult::SameStartOtherShorter { original_after_not_included, .. } =>
                StreamBuilder::default().with_drop(original_after_not_included),
            RangeCmpResult::SameEndOriginalShorter { other_before_not_included, .. } =>
                StreamBuilder::default().with_load(other_before_not_included),
            RangeCmpResult::SameEndOtherShorter { original_before_not_included, .. } =>
                StreamBuilder::default().with_drop(original_before_not_included)
        };
        self.stream.refresh_flatten(stream_builder, visible_cell_range, &self.grid_view);
        self.residence = new_residence;

        true
    }

    fn update_residence_manga(&mut self, visible_page_range: Range<usize>, max_page_count: usize) -> ShouldStream {
        let proximity = self.proximity;
        let lookahead = self.lookahead;
        let mut new_residence = self.manga.residence.clone();

        let range_starts_diff = visible_page_range.start.saturating_sub(self.manga.residence.start);
        let range_ends_diff = self.manga.residence.end.saturating_sub(visible_page_range.end);

        // Proximal end
        if range_ends_diff <= proximity {
            new_residence.end = visible_page_range.end.add(lookahead).min(max_page_count);
        }
        // Distal start
        if range_starts_diff >= lookahead + proximity {
            new_residence.start = visible_page_range.start.saturating_sub(lookahead);
        }
        // Proximal start
        if range_starts_diff <= proximity {
            new_residence.start = visible_page_range.start.saturating_sub(lookahead);
        }
        // Distal end
        if range_ends_diff >= lookahead + proximity {
            new_residence.end = visible_page_range.end.add(lookahead).min(max_page_count);
        }

        let cmp = self.manga.residence.compare(&new_residence);
        let stream_builder = match cmp {
            RangeCmpResult::RangeEmpty |
            RangeCmpResult::CompletelyTheSame |
            RangeCmpResult::CompletelyIncluded { .. } |
            RangeCmpResult::MiddleIncluded { .. } =>
                return false,
            RangeCmpResult::NotIncludedBelow |
            RangeCmpResult::NotIncludedAbove =>
                StreamBuilder::default().with_drop(self.manga.residence.clone()).with_load(new_residence.clone()),
            RangeCmpResult::EndIncluded { other_after, original_part_which_is_not_included, .. } =>
                StreamBuilder::default().with_drop(original_part_which_is_not_included).with_load(other_after),
            RangeCmpResult::StartIncluded { other_before, original_part_which_is_not_included, .. } =>
                StreamBuilder::default().with_drop(original_part_which_is_not_included).with_load(other_before),
            RangeCmpResult::SameStartOriginalShorter { other_after_not_included, .. } =>
                StreamBuilder::default().with_load(other_after_not_included),
            RangeCmpResult::SameStartOtherShorter { original_after_not_included, .. } =>
                StreamBuilder::default().with_drop(original_after_not_included),
            RangeCmpResult::SameEndOriginalShorter { other_before_not_included, .. } =>
                StreamBuilder::default().with_load(other_before_not_included),
            RangeCmpResult::SameEndOtherShorter { original_before_not_included, .. } =>
                StreamBuilder::default().with_drop(original_before_not_included)
        };
        self.manga.stream.refresh_manga(stream_builder, visible_page_range);
        self.manga.residence = new_residence;

        true
    }

    #[hotpath::measure]
    fn central_panel_grid(&mut self, ui: &mut egui::Ui) {
        self.stream_textures_stepped();

        self.tag_win(ui);

        let background_resp = ui.scope_builder(
            egui::UiBuilder::new().sense(egui::Sense::click()),
            |ui| self.grid_view(ui)
        )
        .response;

        if background_resp.clicked() {
            self.reset_grid_entries_selection();
        }
    }

    fn central_panel_details(&mut self, ui: &mut egui::Ui) {
        if requested_go_back(ui) {
            self.pop_dir();
        }

        self.stream_textures_stepped();

        self.details_view(ui);
    }

    #[hotpath::measure]
    fn central_panel_manga(&mut self, ui: &mut egui::Ui) {
        if requested_go_back(ui) {
            self.manga.reset();
            self.view_kind = ViewKind::Details;

            return
        }

        self.stream_textures_stepped();

        if let Some(opacity) = self.get_animation_opacity(ui) {
            ui.set_opacity(opacity);
        }

        if let Some(viewport) = self.manga.flagged_scale.take() {
            let scale = self.manga.scale / 100.;
            let viewport_half_height = viewport.height().div_euclid(2.);
            let viewport_offset = viewport.top();
            let viewport_pivot = viewport_offset + viewport_half_height;

            let pivot_page_visible_i = self.manga.view[self.manga.visible_view.clone()]
                .partition_point(|page_info| page_info.offset < viewport_pivot)
                .saturating_sub(1);
            let pivot_page_i = self.manga.visible_view.start + pivot_page_visible_i;
            let pivot_page_info = &self.manga.view[pivot_page_i];
            let pivot_page_inset_px = viewport_pivot - pivot_page_info.offset;
            let pivot_page_inset_pc = pivot_page_inset_px / pivot_page_info.extent.height;

            for page_info in self.manga.view.drain(..) {
                if let ImageStateManga::Ready { .. } = page_info.image_state {
                    self.to_thanatos.send(Soul::ImageState(page_info.image_state.into())).unwrap();
                }
            }

            let limit = self.wgpu.device.limits().max_texture_dimension_2d as f32;
            let mut page_scaled_offset = 0_f32;
            for page in self.manga.archive_pages.iter() {
                let max_width_scale = limit / page.extent.width;
                let max_height_scale = limit / page.extent.height;
                let scale = scale.min(max_width_scale).min(max_height_scale);
                let width_scaled = page.extent.width.mul(scale).round();
                let height_scaled = page.extent.height.mul(scale).round();
                let extent = [width_scaled, height_scaled].into();

                self.manga.view.push(ViewPageInfo { image_kind: page.image_kind, archive_i: page.index, offset: page_scaled_offset, extent, image_state: default!() });

                page_scaled_offset += height_scaled;
            }
            let view_width = self.manga.archive_pages_width.mul(scale).round()
                .div(2.).ceil().mul(2.); // Avoid subpixel alignment
            self.manga.view_extent = [view_width, page_scaled_offset].into();

            let pivot_page_info = &self.manga.view[pivot_page_i];
            let new_viewport_offset = pivot_page_info.offset +
                pivot_page_inset_pc.mul(pivot_page_info.extent.height) -
                viewport_half_height;
            let viewport_translation = new_viewport_offset - viewport_offset;
            let new_viewport = viewport.translate([0., viewport_translation].into());
            self.manga.scroll_offset.y = new_viewport_offset;

            let start_visible = self.manga.view.partition_point(|page_info| page_info.offset < new_viewport_offset)
                .saturating_sub(1);
            let end_visible = self.manga.view.partition_point(|page_info| page_info.offset <= new_viewport.bottom())
                .min(self.manga.view.len());
            self.manga.residence = start_visible.saturating_sub(self.lookahead)..end_visible.add(self.lookahead).min(self.manga.view.len());
            self.manga.visible_view = start_visible..end_visible;

            self.manga.stream.clear();
            for view_i in self.manga.residence.clone() {
                if self.manga.visible_view.contains(&view_i) {
                    self.manga.stream.load_first.insert(view_i);
                } else {
                    self.manga.stream.load_after.insert(view_i);
                }
            }
            self.stream_manga(ui, self.manga.scale != 100.);
        }

        let scroll_offset_x_centered = self.manga.view_extent.width.sub(self.central_rect.width()).div_euclid(2.).max(0.);
        let (secondary_state, dragging) = ui.input(|state|
            (
                ButtonState::new(state.pointer.button_down(egui::PointerButton::Secondary)),
                state.pointer.is_decidedly_dragging()
            )
        );
        let scroll_area_info = match secondary_state {
            ButtonState::Up => {
                if self.manga.secondary_was_down.take() {
                    ui.send_viewport_cmd(egui::ViewportCommand::CursorVisible(true));
                }
                let (scroll_source, scroll_multiplier) = match self.manga.scroll_kind {
                    ScrollKind::EaseInOut =>
                        (ScrollSource::ALL, self.scroll_multiplier),
                    ScrollKind::SpringDamper => {
                        self.manga.spring_damper.step(ui);

                        (ScrollSource::DRAG | ScrollSource::SCROLL_BAR, self.manga.spring_damper.multiplier)
                    }
                };
                let scroll_offset_y = self.manga.scroll_offset_y_anchor.take().unwrap_or(self.manga.scroll_offset.y);

                ScrollAreaInfo {
                    scroll_source,
                    drag_by: egui::PointerButton::Primary,
                    stop_kinesis: false,
                    scroll_offset: [scroll_offset_x_centered, scroll_offset_y + self.manga.spring_damper.delta].into(),
                    scroll_multiplier: [1.0, scroll_multiplier].into()
                }
            },
            ButtonState::Down => {
                if dragging {
                    ui.send_viewport_cmd(egui::ViewportCommand::CursorVisible(false));
                }
                let stop_kinesis = !self.manga.secondary_was_down;
                let (scroll_source, scroll_multiplier) = match self.manga.scroll_kind {
                    ScrollKind::EaseInOut =>
                        (ScrollSource::DRAG | ScrollSource::MOUSE_WHEEL, self.scroll_multiplier),
                    ScrollKind::SpringDamper => {
                        match stop_kinesis {
                            true => self.manga.spring_damper.stop(),
                            false => self.manga.spring_damper.step(ui)
                        }

                        (ScrollSource::DRAG, self.manga.spring_damper.multiplier)
                    }
                };
                self.manga.scroll_offset_y_anchor.get_or_insert(self.manga.scroll_offset.y);
                self.manga.secondary_was_down = true;

                ScrollAreaInfo {
                    scroll_source,
                    drag_by: egui::PointerButton::Secondary,
                    stop_kinesis,
                    scroll_offset: self.manga.scroll_offset + [0., self.manga.spring_damper.delta].into(),
                    scroll_multiplier: [1.0, scroll_multiplier].into()
                }
            }
        };
        let ScrollAreaInfo { scroll_source, drag_by, stop_kinesis, scroll_offset, scroll_multiplier } = scroll_area_info;
        let new_scroll_offset = egui::ScrollArea::both()
            .auto_shrink(false)
            .scroll_source(scroll_source)
            .drag_by(drag_by)
            .stop_kinesis(stop_kinesis)
            .scroll_offset(scroll_offset)
            .horizontal_scroll_bar_visibility(ScrollBarVisibility::AlwaysHidden)
            .wheel_scroll_multiplier(scroll_multiplier)
            .show_viewport(ui, |ui, viewport| {
                ui.set_min_size([self.manga.view_extent.width.max(self.central_rect.width()), self.manga.view_extent.height].into());

                self.show_viewport_manga(ui, viewport)
            })
            .state.offset;

        if new_scroll_offset != self.manga.scroll_offset {
            egui::Popup::close_all(ui);
        }

        self.manga.scroll_offset = new_scroll_offset;
    }

    fn show_viewport_manga(&mut self, ui: &mut egui::Ui, viewport: egui::Rect) {
        ui.set_clip_rect(self.central_rect);
        ui.spacing_mut().item_spacing = egui::Vec2::splat(0.0);

        ui.with_layout(egui::Layout::top_down(egui::Align::Center), |ui| {
            // Note: partition_point gives the index of the point at which the predicate ceases to be true (ie. the index of the next partition)
            let start_visible = self.manga.view.partition_point(|page_info| page_info.offset < viewport.min.y)
                .saturating_sub(1); // ..so sat sub to go back to the index where page_info.offset < viewport.min.y
            let end_visible = self.manga.view.partition_point(|page_info| page_info.offset <= viewport.max.y)
                .min(self.manga.view.len());
            self.manga.visible_view = start_visible..end_visible;

            let viewport_top_delta = viewport.top().sub(self.manga.prev_viewport_top).abs();
            let delta_this_frame = viewport_top_delta > 0.;

            let hyperspace = self.manga.some_prev_delta && delta_this_frame && // Scrolling
                viewport_top_delta > 340.; // Speed limit

            if !hyperspace && self.update_residence_manga(self.manga.visible_view.clone(), self.manga.view.len()) {
                self.stream_manga(ui, self.manga.scale != 100.);
            }

            if self.poll_ready.is_ready() {
                ui.add_space(self.manga.view[start_visible].offset);

                for view_i in self.manga.visible_view.clone() {
                    let page_extent = self.manga.view[view_i].extent;
                    let (page_rect, _) = ui.allocate_exact_size([ui.min_size().x, page_extent.height].into(), egui::Sense::hover());

                    let image_resp = try_add_image_manga(ui, &mut self.manga.view[view_i].image_state, page_rect);

                    if let Some(image_resp) = image_resp {
                        self.context_menu_manga(viewport, &image_resp);
                    }
                }
            }

            self.manga.prev_viewport_top = viewport.top();
            self.manga.some_prev_delta = delta_this_frame;

            // let info = vec![
            //     format!("visible : {:?}", start_visible..end_visible),
            //     format!("resi    : {:?}", self.manga.residence),
            //     format!("viewport: {}", viewport),
            // ];

            // egui::Window::new("").show(ui, |ui| {
            //     ui.set_width(550.);
            //     ui.style_mut().override_font_id = Some(egui::FontId::monospace(18.0));

            //     for i in info {
            //         ui.label(i);
            //     }
            // });
        });
    }

    fn context_menu_manga(&mut self, viewport: egui::Rect, resp: &egui::Response) {
        let close_behaviour = match resp.clicked_elsewhere() {
            true => egui::PopupCloseBehavior::CloseOnClickOutside,
            false => egui::PopupCloseBehavior::IgnoreClicks
        };

        egui::Popup::context_menu(resp)
            .close_behavior(close_behaviour)
            .show(|ui| {
                ui.menu_button("Scale", |ui| self.scale_submenu_manga(ui, viewport));
                ui.menu_button("Filter", |ui| self.filter_submenu_manga(ui));
                ui.menu_button("Scroll", |ui| self.scroll_submenu_common(ui, Stage::Manga));
            });
    }

    fn scale_submenu_manga(&mut self, ui: &mut egui::Ui, viewport: egui::Rect) {
        const SCALE_MIN: f32 = 50.;
        const SCALE_MAX: f32 = 300.;
        const SCALE_STEP: f32 = 25.;

        ui.set_min_width(MANGA_CONTEXT_MENU_MIN_WIDTH);

        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 5.;

            let drag_scale_resp = ui.add(egui::DragValue::new(&mut self.manga.scale)
                .range(SCALE_MIN..=SCALE_MAX)
                .fixed_decimals(0)
                .speed(0.25));
            if drag_scale_resp.drag_started() {
                self.manga.scale_drag_anchor = self.manga.scale;
            }
            if drag_scale_resp.dragged() {
                self.manga.scale = self.manga.scale.div(SCALE_STEP).floor() * SCALE_STEP;
            }
            if drag_scale_resp.drag_stopped() && self.manga.scale_drag_anchor != self.manga.scale {
                self.manga.flagged_scale = Some(viewport);
            }

            ui.separator();

            let drag_scale_extent = drag_scale_resp.rect.size();
            if ui.add_sized(drag_scale_extent, egui::Button::new("-25")).clicked() {
                self.manga.flag_scale(ui, self.manga.scale.sub(SCALE_STEP).max(SCALE_MIN), viewport);
            };
            if ui.add_sized(drag_scale_extent, egui::Button::new("+25")).clicked() {
                self.manga.flag_scale(ui, self.manga.scale.add(SCALE_STEP).min(SCALE_MAX), viewport);
            };
        });

        ui.separator();

        ui.with_layout(egui::Layout::top_down_justified(egui::Align::Center), |ui| {
            if ui.button("50").clicked() {
                self.manga.flag_scale(ui, SCALE_MIN, viewport);
            }
            if ui.button("100").clicked() {
                self.manga.flag_scale(ui, 100., viewport);
            }
            if ui.button("150").clicked() {
                self.manga.flag_scale(ui, 150., viewport);
            }
            if ui.button("200").clicked() {
                self.manga.flag_scale(ui, 200., viewport);
            }
            if ui.button("250").clicked() {
                self.manga.flag_scale(ui, 250., viewport);
            }
            if ui.button("300").clicked() {
                self.manga.flag_scale(ui, SCALE_MAX, viewport);
            }
        });
    }

    fn filter_submenu_manga(&mut self, ui: &mut egui::Ui) {
        ui.set_min_width(MANGA_CONTEXT_MENU_MIN_WIDTH);

        if ui.radio(self.manga.filter == fir::FilterType::Box, "Box").clicked() {
            self.manga.filter = fir::FilterType::Box;
        }
        if ui.radio(self.manga.filter == fir::FilterType::Bilinear, "Bilinear").clicked() {
            self.manga.filter = fir::FilterType::Bilinear;
        }
        if ui.radio(self.manga.filter == fir::FilterType::Custom(blackman_filter_fir()), "Blackman 3").clicked() {
            self.manga.filter = fir::FilterType::Custom(blackman_filter_fir());
        }
        if ui.radio(self.manga.filter == fir::FilterType::CatmullRom, "Catmull-Rom").clicked() {
            self.manga.filter = fir::FilterType::CatmullRom;
        }
        if ui.radio(self.manga.filter == fir::FilterType::Gaussian, "Gaussian").clicked() {
            self.manga.filter = fir::FilterType::Gaussian;
        }
        if ui.radio(self.manga.filter == fir::FilterType::Hamming, "Hamming").clicked() {
            self.manga.filter = fir::FilterType::Hamming;
        }
        if ui.radio(self.manga.filter == fir::FilterType::Lanczos3, "Lanczos 3").clicked() {
            self.manga.filter = fir::FilterType::Lanczos3;
        }
        if ui.radio(self.manga.filter == fir::FilterType::Mitchell, "Mitchell").clicked() {
            self.manga.filter = fir::FilterType::Mitchell;
        }
    }

    fn scroll_submenu_common(&mut self, ui: &mut egui::Ui, stage: Stage) {
        let (scroll_kind, spring_damper) = match stage {
            Stage::Grid | Stage::Details => (&mut self.scroll_kind, &mut self.spring_damper),
            Stage::Manga => (&mut self.manga.scroll_kind, &mut self.manga.spring_damper)
        };

        ui.set_min_width(MANGA_CONTEXT_MENU_MIN_WIDTH);

        if ui.radio(*scroll_kind == ScrollKind::SpringDamper, "Spring-Damper").clicked() {
            *scroll_kind = ScrollKind::SpringDamper;
        }
        if ui.radio(*scroll_kind == ScrollKind::EaseInOut, "Ease In-Out").clicked() {
            *scroll_kind = ScrollKind::EaseInOut;
        }

        ui.separator();

        egui::Grid::new("grid")
            .num_columns(2)
            .show(ui, |ui| {
                match scroll_kind {
                    ScrollKind::EaseInOut => {
                        ui.label("Distance:");

                        self.scroll_multiplier_display.clear();
                        write!(self.scroll_multiplier_display, "{}", self.scroll_multiplier).unwrap();
                        let multiplier_edit_resp = egui::TextEdit::singleline(&mut self.scroll_multiplier_edit)
                            .hint_text(&self.scroll_multiplier_display)
                            .show(ui)
                            .response;
                        if multiplier_edit_resp.lost_focus() && let Ok(multiplier) = self.scroll_multiplier_edit.parse::<f32>() {
                            self.scroll_multiplier_edit.clear();
                            self.scroll_multiplier = multiplier;
                        }
                        ui.end_row();
                    },
                    ScrollKind::SpringDamper => {
                        spring_damper.update_display();

                        ui.checkbox(&mut spring_damper.should_smooth, "Smooth");
                        ui.end_row();

                        ui.label("Distance:");
                        let multiplier_edit_resp = egui::TextEdit::singleline(&mut spring_damper.multiplier_edit)
                            .hint_text(&spring_damper.multiplier_display)
                            .show(ui)
                            .response;
                        if multiplier_edit_resp.lost_focus() && let Ok(multiplier) = spring_damper.multiplier_edit.parse::<f32>() {
                            spring_damper.multiplier_edit.clear();
                            spring_damper.multiplier = multiplier;
                        }
                        ui.end_row();

                        ui.label("Stiffness:");
                        let stiffness_edit_resp = egui::TextEdit::singleline(&mut spring_damper.stiffness_edit)
                            .hint_text(&spring_damper.stiffness_display)
                            .show(ui)
                            .response;
                        if stiffness_edit_resp.lost_focus() && let Ok(omega) = spring_damper.stiffness_edit.parse::<f32>() {
                            spring_damper.stiffness_edit.clear();
                            spring_damper.update_stiffness(omega);
                        }
                        ui.end_row();

                        ui.label("Bounce:");
                        let bounce_edit_resp = egui::TextEdit::singleline(&mut spring_damper.bounce_edit)
                            .hint_text(&spring_damper.bounce_display)
                            .show(ui)
                            .response;
                        if bounce_edit_resp.lost_focus() && let Ok(bounce) = spring_damper.bounce_edit.parse::<f32>() {
                            spring_damper.bounce_edit.clear();
                            spring_damper.update_bounce(bounce);
                        }
                        ui.end_row();
                    }
                }
            });
    }

    fn tag_win(&mut self, ui: &mut egui::Ui) {
        if let Some(PendingTagOp { tag, op }) = self.tag_win_button_pending_tag_op.take() {
            let tag_is_active = self.active_tag.as_ref().map(|active_tag| active_tag == &tag).unwrap_or(false);

            match op {
                TagOp::Rename => {
                    let set = self.tags.remove(&tag).unwrap();
                    let tag: Rc<str> = Rc::from(self.tag_win_rename_edit.as_str());

                    if tag_is_active {
                        self.active_tag = Some(tag.clone());
                    }
                    self.tags.insert(tag, set);

                    self.tag_win_rename_edit.clear();
                },
                TagOp::Remove => {
                    self.tags.remove(&tag);

                    if tag_is_active {
                        self.reset_grid_view(ui);
                    }
                }
            }
        }

        let max_rect = ui.max_rect();
        let tag_win_rect = egui::Rect::from_min_size(
            max_rect.min,
            [250.0, (max_rect.height() - FRAME_INNER_MARGIN).max(0.0)].into()
        );

        let tag_win_resp = self.tag_win_should_open.and_then(|| {
            egui::Window::new("tag_win")
                .fixed_rect(tag_win_rect)
                .title_bar(false)
                .fade_in(true)
                .fade_out(true)
                .show(ui.ctx(), |ui| {
                    ui.with_layout(egui::Layout::top_down_justified(egui::Align::Center), |ui| {
                        ui.heading("Tags");

                        ui.separator();

                        self.tag_win_buttons(ui);

                        ui.take_available_space();
                    });
                })
        })
        .map(|shown| shown.response);

        if !self.tag_win_button_menu_is_open {
            let hover_pos = ui.ctx().input(|state| state.pointer.hover_pos());

            match hover_pos {
                Some(hover_pos) => {
                    self.tag_win_cursor_checked = false;

                    if let Some(resp) = tag_win_resp.as_ref() {
                        match resp.contains_pointer() {
                            true => self.tag_win_time_stamp = Some(now!()),
                            false => if hover_pos.x > resp.rect.right() {
                                self.tag_win_time_stamp = None;
                                self.tag_win_should_open = false;
                            }
                        }

                        if resp.clicked() {
                            self.reset_grid_entries_selection();
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
                                self.tag_win_time_stamp = Some(now!());
                                self.tag_win_should_open = true;
                            }

                            self.tag_win_cursor_checked = true;
                        }
                    }
                }
            }

            if let Some(tag_win_time_stamp) = self.tag_win_time_stamp && tag_win_time_stamp.elapsed() > Duration::from_secs(3) {
                self.tag_win_time_stamp = None;
                self.tag_win_should_open = false;
            }
        }
    }

    fn tag_win_buttons(&mut self, ui: &mut egui::Ui) {
        let all_button_resp = ui.button("All");

        if all_button_resp.clicked() && self.active_tag.is_some() {
            self.reset_grid_view(ui);
        }
        if self.active_tag.is_none() {
            all_button_resp.highlight();
        }

        loan!(self.tags, tags => {
            for (tag, set) in tags.iter() { if !set.is_empty() {
                let tag_button_resp = ui.button(tag.as_ref());

                egui::Popup::context_menu(&tag_button_resp)
                    .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                    .show(|ui| {
                        self.tag_win_button_menu_is_open = true;

                        let tag_rename_edit_resp = egui::TextEdit::singleline(&mut self.tag_win_rename_edit)
                            .hint_text("Rename")
                            .show(ui)
                            .response;

                        tag_rename_edit_resp.request_focus();

                        if ui.input(|state| state.key_pressed(egui::Key::Enter)) {
                            if !self.tag_win_rename_edit.is_empty() {
                                self.tag_win_button_pending_tag_op = Some(PendingTagOp { tag: tag.clone(), op: TagOp::Rename });

                                ui.close();
                            } else {
                                tag_rename_edit_resp.request_focus();
                            }
                        }

                        let tag_remove_button_resp = ui.button("Remove");
                        if tag_remove_button_resp.clicked() {
                            self.tag_win_button_pending_tag_op = Some(PendingTagOp { tag: tag.clone(), op: TagOp::Remove });

                            ui.close();
                        }
                    })
                    .inspect(|tag_button_menu| if tag_button_menu.response.should_close() {
                        self.tag_win_time_stamp = Some(now!());
                        self.tag_win_button_menu_is_open = false;
                    });

                // Switch tag view
                if tag_button_resp.clicked() && self.active_tag.as_ref().is_none_or(|active_tag| active_tag != tag) {
                    self.stream.flatten_drop(self.residence.clone(), &self.grid_view);
                    populate_grid_view(&mut self.grid_view, &self.grid_entries, set);

                    let ResetResidence { visible_cell_count, .. } = self.reset_residence();
                    self.stream.flatten_load(self.residence.clone(), 0..visible_cell_count, &self.grid_view);
                    self.stream(ui);

                    self.reset_grid_entries_selection();
                    self.active_tag = Some(tag.clone());
                    self.animate_bool = false;
                }

                if let Some(active_tag) = self.active_tag.as_ref() && active_tag == tag {
                    tag_button_resp.highlight();
                }
            } }
        });
    }

    fn grid_view(&mut self, ui: &mut egui::Ui) {
        if requested_clear_selection(ui) && !egui::Popup::is_any_open(ui) {
            self.reset_grid_entries_selection();
        }
        if requested_go_back(ui) && self.active_tag.is_some() {
            self.reset_grid_view(ui);
        }

        let GridViewCellCounts { row: row_cell_count, max: max_cell_count } = match self.grid_view_pending_op.take() {
            Some(GridViewOp::Reset) => self.reset_grid_view(ui),
            Some(GridViewOp::Refresh) => self.refresh_grid_view(ui),
            _ => {
                let row_cell_count = (ui.available_width() - self.grid_cell_size.x).div(self.grid_cell_space.x).ceil() as usize;
                let max_cell_count = self.grid_view.len();

                GridViewCellCounts { row: row_cell_count.clamp(1, max_cell_count), max: max_cell_count }
            }
        };
        let max_row_count = max_cell_count.div_ceil(row_cell_count);
        let scroll_area_height = max_row_count as f32 * self.grid_cell_space.y - GRID_IMAGE_SPACING.y;

        if self.poll_ready.is_ready() {
            if let Some(opacity) = self.get_animation_opacity(ui) {
                ui.set_opacity(opacity);
            }

            ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                ui.spacing_mut().item_spacing = GRID_IMAGE_SPACING;

                let scroll_source = match self.scroll_kind {
                    ScrollKind::EaseInOut => ScrollSource::SCROLL_BAR | ScrollSource::MOUSE_WHEEL,
                    ScrollKind::SpringDamper => {
                        self.spring_damper.step(ui);
                        self.grid_scroll_offset += self.spring_damper.delta;
                        self.grid_scroll_offset = self.grid_scroll_offset.clamp(0., scroll_area_height - self.central_rect.height());

                        ScrollSource::SCROLL_BAR
                    }
                };

                let grid_scroll_offset = egui::ScrollArea::new([false, true])
                    .auto_shrink(false)
                    .scroll_source(scroll_source)
                    .wheel_scroll_multiplier([1.0, self.scroll_multiplier].into())
                    .vertical_scroll_offset(self.grid_scroll_offset)
                    .show_rows(ui, self.grid_cell_size.y, max_row_count, |ui, row_range| {
                        if self.update_residence(&row_range, row_cell_count, max_cell_count) {
                            self.stream(ui);
                        }

                        let available_rect = ui.available_rect_before_wrap();

                        #[allow(clippy::cast_precision_loss)]
                        let table_width = row_cell_count as f32 * self.grid_cell_space.x - GRID_IMAGE_SPACING.x;
                        let table_rect = egui::Rect::from_center_size(
                            available_rect.center(),
                            [table_width, available_rect.height()].into()
                        );

                        ui.scope_builder(egui::UiBuilder::new().max_rect(table_rect), |ui| {
                            let table_row_count = row_range.end - row_range.start;
                            self.grid_table(ui, row_range.start, max_cell_count, table_row_count, row_cell_count);
                        });

                        for rect in self.grid_cell_highlights.selected.drain(..) {
                            highlight_rect(ui, rect);
                        }
                        if let Some(rect) = self.grid_cell_highlights.hovered.take() {
                            highlight_rect(ui, rect);
                        }
                    })
                    .state.offset.y;

                if grid_scroll_offset != self.grid_scroll_offset {
                    egui::Popup::close_all(ui);
                }
                self.grid_scroll_offset = grid_scroll_offset;
            });
        }
    }

    fn grid_table(&mut self, ui: &mut egui::Ui, row_start: usize, max_cell_count: usize, row_count: usize, row_cell_count: usize) {
        egui_extras::TableBuilder::new(ui)
            .striped(false)
            .vscroll(false)
            .cell_layout(egui::Layout::top_down(egui::Align::Center))
            .columns(egui_extras::Column::initial(self.grid_cell_size.x).at_most(self.grid_cell_size.x), row_cell_count)
            .body(|body| {
                body.rows(self.grid_cell_size.y, row_count, |mut row| {
                    self.grid_view_i = (row_start + row.index()) * row_cell_count;

                    while row.col_index() < row_cell_count && self.grid_view_i < max_cell_count {
                        row.col(|ui| self.grid_cell(ui));

                        self.grid_view_i += 1
                    }
                });
            });
    }

    fn grid_cell(&mut self, ui: &mut egui::Ui) {
        self.grid_entry_i = self.grid_view[self.grid_view_i];
        let grid_entry_info = &self.grid_entries[self.grid_entry_i];
        let image_states = get_image_states_mut(&mut self.images, grid_entry_info.image_i);

        let cell_resp = match image_states {
            Some(ImageStates { grid: grid_state, .. }) =>
                try_add_image(ui, grid_state, grid_entry_info.stem.as_ref()),
            None => add_label(ui, grid_entry_info.stem.as_ref())
        };

        // Select cell or switch view
        let ctrl_held = ui.input(|state| state.modifiers.ctrl);
        if cell_resp.clicked() {
            match ctrl_held {
                true => {
                    match self.grid_entries_selection.contains(&self.grid_entry_i) {
                        true => self.grid_entries_selection.remove(&self.grid_entry_i),
                        false => self.grid_entries_selection.insert(self.grid_entry_i)
                    };

                    self.grid_entries_selection_kind = match self.grid_entries_selection.len() {
                        0 => None,
                        1 => Some(SelectionKind::Single),
                        _ => Some(SelectionKind::Multi)
                    }
                },
                false => {
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

                    self.reset_grid_entries_selection();
                    self.view_kind = ViewKind::Details;
                }
            }
        }

        self.grid_cell_context_menu(ui, &cell_resp);

        // Defer highlighting cells incase selection changes during layout
        if self.grid_entries_selection.contains(&self.grid_entry_i) ||
            cell_resp.hovered() && self.grid_entries_selection.is_empty()
        {
            self.grid_cell_highlights.selected.push(cell_resp.rect)
        }
    }

    fn grid_cell_context_menu(&mut self, ui: &mut egui::Ui, cell_resp: &egui::Response) {
        let (single_open_memory, multi_open_memory) = match cell_resp.secondary_clicked() {
            true => match self.grid_entries_selection_kind {
                Some(SelectionKind::Multi) if self.grid_entries_selection.contains(&self.grid_entry_i) =>
                    (None, Some(egui::SetOpenCommand::Bool(true))),
                Some(SelectionKind::Multi) => {
                    self.grid_cell_tags_menu_selection.clear();
                    self.grid_entries_selection.clear();
                    self.grid_entries_selection.insert(self.grid_entry_i);
                    self.grid_entries_selection_kind = Some(SelectionKind::Single);

                    (Some(egui::SetOpenCommand::Bool(true)), None)
                },
                Some(SelectionKind::Single) if self.grid_entries_selection.contains(&self.grid_entry_i) =>
                    (Some(egui::SetOpenCommand::Bool(true)), None),
                Some(SelectionKind::Single) => {
                    self.grid_entries_selection.clear();
                    self.grid_entries_selection.insert(self.grid_entry_i);

                    (Some(egui::SetOpenCommand::Bool(true)), None)
                },
                None => {
                    self.grid_entries_selection.insert(self.grid_entry_i);
                    self.grid_entries_selection_kind = Some(SelectionKind::Single);

                    (Some(egui::SetOpenCommand::Bool(true)), None)
                }
            },
            false => (None, None)
        };

        let close_behaviour = match cell_resp.clicked_elsewhere() {
            true => egui::PopupCloseBehavior::CloseOnClickOutside,
            false => egui::PopupCloseBehavior::IgnoreClicks
        };

        egui::Popup::new(ui.make_persistent_id("single"), ui.ctx().clone(), egui::PopupAnchor::PointerFixed, cell_resp.layer_id)
            .kind(egui::PopupKind::Menu)
            .layout(egui::Layout::top_down_justified(egui::Align::Min))
            .style(egui::containers::menu::menu_style)
            .gap(0.0)
            .open_memory(single_open_memory)
            .close_behavior(close_behaviour)
            .show(|ui| {
                if ui.button("Image").clicked() {
                    ui.close();

                    let pick_image_file = rfd::FileDialog::new()
                        .add_filter("images", IMAGE_EXTS)
                        .pick_file();

                    if let Some(path) = pick_image_file {
                        self.pick_image(ui, path).unwrap_or_else(|err| {
                            error!("{}: failed to pick image: {}", module_path!(), err);
                        });
                    }
                }

                self.grid_cell_sort_submenu(ui);
                self.grid_cell_tag_submenus(ui);
                self.grid_cell_scroll_submenu(ui);
            });

        egui::Popup::new(ui.make_persistent_id("multi"), ui.ctx().clone(), egui::PopupAnchor::PointerFixed, cell_resp.layer_id)
            .kind(egui::PopupKind::Menu)
            .layout(egui::Layout::top_down_justified(egui::Align::Min))
            .style(egui::containers::menu::menu_style)
            .gap(0.0)
            .open_memory(multi_open_memory)
            .close_behavior(close_behaviour)
            .show(|ui| self.grid_cell_tag_submenus(ui));
    }

    fn pick_image(&mut self, ctx: &egui::Context, path: PathBuf) -> Res<()> {
        let grid_entry_info = &mut self.grid_entries[self.grid_entry_i];
        let grid_entry_stem = grid_entry_info.stem.as_ref();
        let pick_image_ext = path.get_file_ext()?;

        let (new_image_file_name,
            prev_image_file_name
        ) = match grid_entry_info.image_i {
            Some(image_i) => { // Entry references an existing image
                let (prev_image_file_name, image_states) = self.images.get_index_mut(image_i).unwrap();

                match &mut image_states.ref_count {
                    ref_count @ 2.. => { // Multiple entries reference the image
                        let mut s = String::new();
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

        let cache_readies = CacheReadies::new();
        let (image_i, _) = self.images.insert_full(
            new_image_file_name.clone(),
            ImageStates::new_none_check_cache(cache_readies.clone())
        );
        grid_entry_info.image_i = Some(image_i);

        let ferry_image_info = FerryImageInfo {
            image_file_name: new_image_file_name.clone(),
            expected_metadata: None,
            grid_entry_i: self.grid_entry_i,
            signal_cache_readies: cache_readies,
            wait_cache_readies: CacheReadies::NONE,
            signal_tex_ready: None
        };
        let ferry_images_info = FerryImagesInfo {
            ctx,
            thread_pool: &self.thread_pool,
            metadata_sx: None,
            image_dirs: self.image_dirs,
            base_image_kind: BaseImageKind::Pick { path: path.clone() },
            grid_cell_extent: self.grid_cell_size.into(),
            details_cell_extent: self.details_cell_size.into(),
            grid_relay: self.image_sx.clone(),
            details_relay: self.image_sx.clone(),
            ferry_image_infos: vec![ferry_image_info],
            error_sx: self.error_sx.clone()
        };
        ferry_images(ferry_images_info);

        let base_image_path = self.image_dirs.base.join(new_image_file_name.as_ref());
        let grid_entry_i = self.grid_entry_i;
        let pick_image_metadata_sx = self.pick_image_metadata_sx.clone();
        let image_dirs = self.image_dirs;
        self.thread_pool.spawn_fifo(move || {
            (|| -> Res<_> {
                fs::copy(&path, &base_image_path)?;

                let base_image_file = File::open(base_image_path.as_path())?;
                let metadata = base_image_file.metadata()?;
                let metadata = Arc::new(Metadata {
                    created: metadata.created()?,
                    modified: metadata.modified()?,
                    len: metadata.len()
                });
                pick_image_metadata_sx.send(MetadataInfo { grid_entry_i, metadata }).unwrap();

                if let Some(prev_image_file_name) = prev_image_file_name {
                    let paths = [
                        image_dirs.base.join(prev_image_file_name.as_ref()),
                        image_dirs.grid.join(prev_image_file_name.as_ref()).with_added_extension("webp"),
                        image_dirs.details.join(prev_image_file_name.as_ref()).with_added_extension("webp")
                    ];
                    for path in paths {
                        fs::remove_file(&path)?;
                    }
                }

                Ok(())
            })()
            .unwrap_or_else(|err| error!("{}: failure caching picked image: {}", module_path!(), err));
        });

        Ok(())
    }

    fn grid_cell_sort_submenu(&mut self, ui: &mut egui::Ui) {
        let grid_entry_info = &mut self.grid_entries[self.grid_entry_i];

        let mut sort_grid_view = false;
        ui.menu_button("Sort name", |ui| {
            let sort_name_edit_resp = egui::TextEdit::singleline(&mut self.sort_name_edit)
                .hint_text(grid_entry_info.sort_name.as_deref().unwrap_or("New"))
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

    fn grid_cell_tag_submenus(&mut self, ui: &mut egui::Ui) {
        ui.menu_button("Tags", |ui| {
            // Add tag
            let new_tag_edit_resp = egui::TextEdit::singleline(&mut self.new_tag_edit)
                .hint_text("New")
                .show(ui)
                .response;

            if ui.input(|state| state.key_pressed(egui::Key::Enter)) {
                if !self.new_tag_edit.is_empty() {
                    if let Some(selection_kind) = self.grid_entries_selection_kind.as_ref() {
                        let tag_entry = self.tags.entry(Rc::from(self.new_tag_edit.as_str()));
                        match selection_kind {
                            SelectionKind::Single => tag_entry
                                .and_modify(|set| _ = set.insert(self.grid_entry_i))
                                .or_insert([self.grid_entry_i].into_iter().collect()),
                            SelectionKind::Multi => tag_entry
                                .and_modify(|set| set.extend(self.grid_entries_selection.iter()))
                                .or_insert(self.grid_entries_selection.iter().copied().collect())
                        };
                    }

                    self.new_tag_edit.clear();
                }

                new_tag_edit_resp.request_focus();
            }

            ui.separator();

            match self.grid_entries_selection_kind {
                Some(SelectionKind::Single) => self.grid_cell_single_selection_tags_submenu(ui),
                Some(SelectionKind::Multi) => self.grid_cell_multi_selection_tags_submenu(ui),
                _ => ()
            }
        });
    }

    fn grid_cell_scroll_submenu(&mut self, ui: &mut egui::Ui) {
        ui.menu_button("Scroll", |ui| self.scroll_submenu_common(ui, Stage::Grid));
    }

    fn grid_cell_single_selection_tags_submenu(&mut self, ui: &mut egui::Ui) {
        loan!(self.tags, mut tags => {
            for (tag, set) in tags.iter_mut() { if !set.is_empty() {
                let mut tag_checked = set.contains(&self.grid_entry_i);
                let tag_checkbox_resp = ui.checkbox(&mut tag_checked, tag.as_ref());

                if tag_checkbox_resp.clicked() {
                    self.handle_tag_checkbox(ui, tag, set, tag_checked);
                }
            } }
        });
    }

    fn handle_tag_checkbox(&mut self, ui: &mut egui::Ui, tag: &Rc<str>, set: &mut BTreeSet<usize>, tag_checked: bool) {
        match tag_checked {
            true => _ = set.insert(self.grid_entry_i),
            false => {
                set.remove(&self.grid_entry_i);
                if set.is_empty() {
                    self.tag_win_button_pending_tag_op = Some(PendingTagOp { tag: tag.clone(), op: TagOp::Remove });
                }

                let tag_is_active = self.active_tag.as_ref().map(|active_tag| active_tag == tag).unwrap_or(false);
                if tag_is_active {
                    self.grid_view_pending_op = match set.is_empty() {
                        true => Some(GridViewOp::Reset),
                        false => Some(GridViewOp::Refresh)
                    };

                    ui.close();
                }
            }
        }
    }

    fn grid_cell_multi_selection_tags_submenu(&mut self, ui: &mut egui::Ui) {
        let add_button_resp = ui.button("Add");
        let remove_button_resp = ui.button("Remove");

        ui.separator();

        for (tag, set) in self.tags.iter() { if !set.is_empty() {
            let mut tag_checked = self.grid_cell_tags_menu_selection.contains(tag);
            let tag_checkbox_resp = ui.checkbox(&mut tag_checked, tag.as_ref());

            if tag_checkbox_resp.clicked() {
                match tag_checked {
                    true => self.grid_cell_tags_menu_selection.insert(tag.clone()),
                    false => self.grid_cell_tags_menu_selection.remove(tag)
                };
            }
        } }

        if add_button_resp.clicked() {
            for tag in self.grid_cell_tags_menu_selection.drain() {
                let set = self.tags.get_mut(&tag).unwrap();

                set.extend(self.grid_entries_selection.iter());
            }
        }

        if remove_button_resp.clicked() {
            loan!(self.grid_cell_tags_menu_selection, mut checked_tags => {
                for tag in checked_tags.drain() {
                    let set = self.tags.get_mut(&tag).unwrap();

                    for grid_entry_i in self.grid_entries_selection.iter() {
                        set.remove(grid_entry_i);
                    }
                    if set.is_empty() {
                        self.tag_win_button_pending_tag_op = Some(PendingTagOp { tag: tag.clone(), op: TagOp::Remove });
                    }

                    let tag_is_active = self.active_tag.as_ref().map(|active_tag| active_tag == &tag).unwrap_or(false);
                    if tag_is_active {
                        self.grid_view_pending_op = match set.is_empty() {
                            true => Some(GridViewOp::Reset),
                            false => Some(GridViewOp::Refresh)
                        };

                        ui.close();
                    }
                }
            });
        }
    }

    fn details_view(&mut self, ui: &mut egui::Ui) {
        let image_states = get_image_states_from_grid_entry_mut(&mut self.images, &self.grid_entries, self.details_grid_entry_i);

        // Subdivisions
        let middle_subd_width = 2.0 * FRAME_INNER_MARGIN + SEPARATOR_WIDTH;
        let side_subd_width = (ui.available_width() - middle_subd_width).div_euclid(2.0).max(0.0);
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
                Some(details_state) => try_add_image(ui, details_state, dir_name),
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
                    let top_padding = remaining_space.div_euclid(2.0).max(0.0);

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
                let mut push_dir = None;

                for (i, dir_entry_info) in self.details_dir_entries.iter().enumerate() {
                    let resp = ui.button(dir_entry_info.stem.as_str());

                    if resp.hovered() {
                        self.details_hovered_dir_entry_i = i;
                    }

                    if resp.clicked() {
                        match dir_entry_info.file_kind {
                            FileKind::Dir => push_dir = Some(dir_entry_info.path.clone()),
                            FileKind::Manga => self.view_kind = ViewKind::InitManga { selected_details_dir_entry_i: i },
                            _ => {
                                let discord_activity_info = self.discord_enabled.then(|| self.make_discord_activity_info(dir_entry_info));

                                open_media(
                                    dir_entry_info.path.clone(),
                                    dir_entry_info.file_kind,
                                    self.maintain_sample_rate,
                                    self.override_glsl_shaders,
                                    discord_activity_info,
                                    self.discord_display_kind,
                                    self.error_sx.clone()
                                );
                            }
                        }
                    }
                }

                if let Some(dir) = push_dir {
                    self.push_dir(dir);
                }
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

    fn make_discord_activity_info(&self, dir_entry_info: &DirEntryInfo) -> config::DiscordActivityInfo {
        let grid_entry_info = &self.grid_entries[self.details_grid_entry_i];

        let details = match self.discord_details_edit.is_empty() {
            true => match self.discord_watching {
                Watching::TV => grid_entry_info.stem.to_string(),
                _ => dir_entry_info.stem.clone()
            },
            false => self.discord_details_edit.clone()
        };
        let state = (self.discord_watching == Watching::TV && grid_entry_info.file_kind == FileKind::Dir).then(|| {
            match self.discord_state_edit.is_empty() {
                true => dir_entry_info.stem.clone(),
                false => self.discord_state_edit.clone()
            }
        });

        config::DiscordActivityInfo {
            app_id: match self.discord_watching {
                Watching::Movie => self.discord_app_ids.movies.unwrap().to_string(), // App ID is Some when Discord is enabled
                Watching::TV => self.discord_app_ids.tv.unwrap().to_string(),
                Watching::Words => self.discord_app_ids.words.unwrap().to_string()
            },
            activity: config::DiscordActivity::Watching,
            details,
            state,
            large_image: match self.discord_watching {
                Watching::TV => Some(to_discord_asset_name(grid_entry_info.stem.as_ref())),
                _ => Some(to_discord_asset_name(dir_entry_info.stem.as_str()))
            }
        }
    }

    fn details_context_menu(&mut self, total_alloc_resp: &egui::Response) {
        let close_behaviour = match total_alloc_resp.secondary_clicked() {
            true => egui::PopupCloseBehavior::IgnoreClicks,
            false => egui::PopupCloseBehavior::CloseOnClickOutside
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

            let grid_entry_info = &self.grid_entries[self.details_grid_entry_i];
            let dir_entry_info = &self.details_dir_entries[self.details_hovered_dir_entry_i];

            let details_hint_text = match self.discord_watching {
                Watching::TV => grid_entry_info.stem.as_ref(),
                _ => dir_entry_info.stem.as_str()
            };
            let details_text_edit = egui::TextEdit::singleline(&mut self.discord_details_edit).hint_text(details_hint_text);

            ui.add(details_text_edit);

            ui.end_row();

            if self.discord_watching == Watching::TV && grid_entry_info.file_kind == FileKind::Dir {
                ui.label("State");

                let state_text_edit = egui::TextEdit::singleline(&mut self.discord_state_edit).hint_text(dir_entry_info.stem.as_str());

                ui.add(state_text_edit);
            }
        });
    }
}

pub fn begin(kind: GuiKind) -> Res<(), { loc_var!(Gui) }> {
    let config = config::get().read()?;

    let icon_data = eframe::icon_data::from_png_bytes(include_bytes!("../../../assets/icon.png"))?;
    let mut viewport = egui::ViewportBuilder::default()
        .with_icon(icon_data);
    if let GuiKind::MediaBrowser = kind && let Some(extent) = config.media_browser.as_ref().and_then(|mb| mb.window_inner_size) {
        viewport = viewport.with_inner_size(extent);
    }

    let mut wgpu_setup_create_new = egui_wgpu::WgpuSetupCreateNew::without_display_handle();
    wgpu_setup_create_new.instance_descriptor.backends = wgpu::Backends::VULKAN;
    wgpu_setup_create_new.instance_descriptor.flags = wgpu::InstanceFlags::empty();
    let wgpu_setup = egui_wgpu::WgpuSetup::CreateNew(wgpu_setup_create_new);
    let wgpu_options = egui_wgpu::WgpuConfiguration {
        surface: egui_wgpu::SurfaceConfig::LOW_LATENCY,
        wgpu_setup,
        ..default!()
    };
    let native_options = eframe::NativeOptions {
        viewport,
        renderer: eframe::Renderer::Wgpu,
        wgpu_options,
        centered: true,
        ..default!()
    };

    eframe::run_native(
        "Ogos",
        native_options,
        Box::new(|cctx| {
            // Match window background to egui
            let window = cctx.winit_window().unwrap();
            if let RawWindowHandle::Win32(hnd) = window.window_handle().unwrap().as_raw() {
                fix_background_brush(hnd);
            }

            cctx.egui_ctx.options_mut(|options| options.reduce_texture_memory = true);
            cctx.egui_ctx.set_pixels_per_point(1.0);
            cctx.egui_ctx.global_style_mut(|style| {
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
                GuiKind::Info { msg } => {
                    cctx.egui_ctx.global_style_mut(|style| style.wrap_mode = Some(egui::TextWrapMode::Wrap));

                    Box::new(Info::new(msg))
                },
                GuiKind::MediaBrowser => {
                    let win_inner_size = window.inner_size();
                    let win_inner_size = Extent2dU::new(win_inner_size.width, win_inner_size.height);
                    Box::new(MediaBrowser::new(&cctx.egui_ctx, cctx.wgpu_render_state.as_ref().unwrap(), win_inner_size)?)
                }
            };

            Ok(app)
        })
    )?;

    Ok(())
}
