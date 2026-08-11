use ogos_common::*;
use ogos_config as config;
use config::*;
use ogos_core::*;
use ogos_discord as discord;
use ogos_err::*;
use ogos_video as video;

use bytemuck::*;
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
use serde::*;
use std::{
    cell::*,
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
        System::Threading::*,
        UI::{
            Shell::*,
            WindowsAndMessaging::*
        }
    }
};

const ASPECT_RATIO_3_2: f32 = 1.5;
const BLACKMAN_SUPPORT: f64 = 3.;
const CELL_STROKE: egui::Stroke = egui::Stroke { width: CELL_STROKE_WIDTH, color: egui::Color32::from_rgb(250, 246, 235) };
const CELL_STROKE_WIDTH: f32 = 3.;
const DEFAULT_FRAME_INNER_MARGIN: f32 = 8.;
const DETAILS_ENTRY_COUNT: usize = 64;
const FRAME_INNER_MARGIN: f32 = 15.;
const GRID_IMAGE_SPACING: egui::Vec2 = egui::vec2(30., 30.);
const IMAGE_EXTS: &[&str] = &["jpg", "jpeg", "png", "webp"];
const PCIE_TRANSFER_LIMIT_MIBS: usize = 3840;
const SEPARATOR_WIDTH: f32 = 2.;
const SUBMENU_MIN_WIDTH: f32 = 180.;

thread_local! {
    static WORKER_THREAD_STATE: RefCell<WorkerThreadState> = RefCell::new({
        let mut resizer = fir::Resizer::new();

        #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
        unsafe {
            if is_x86_feature_detected!("avx2") {
                resizer.set_cpu_extensions(fir::CpuExtensions::Avx2);
            } else if is_x86_feature_detected!("sse4.1") {
                resizer.set_cpu_extensions(fir::CpuExtensions::Sse4_1);
            }
        }

        WorkerThreadState {
            resizer,
            // src_linear: default!(),
            // dst_linear: default!()
        }
    });
}

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

struct AnimationInfo {
    dur: f32,
    kind: AnimationKind,
    target: bool
}
impl From<config::AnimationInfo> for AnimationInfo {
    fn from(value: config::AnimationInfo) -> Self {
        Self {
            dur: value.dur,
            kind: value.kind,
            target: false
        }
    }
}

#[derive(Serialize, Deserialize)]
struct Cache {
    library: BTreeSet<PathBuf>,
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
    tags: Vec<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    bookmark: Option<Pivot>
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
impl From<Extent2dF> for [f32; 2] {
    fn from(value: Extent2dF) -> Self {
        [value.width, value.height]
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
    gen_id_check: Option<GenerationIdCheck>,
    signal_cache_readies: CacheReadies,
    wait_cache_readies: CacheReadies,
    signal_tex_ready: Option<PollReady>
}

struct FerryImagesInfo<'a> {
    ctx: &'a egui::Context,
    thread_pool: &'a Arc<ThreadPool>,
    image_dirs: &'static ImageDirs,
    base_image_kind: BaseImageKind,
    grid_cell_extent: Extent2dF,
    details_cell_extent: Extent2dF,
    grid_ship: mpmc::Sender<ImageResult>,
    details_ship: mpmc::Sender<ImageResult>,
    ferry_image_infos: Vec<FerryImageInfo>,
    error_sx: mpmc::Sender<String>
}

struct FerryImageInfoManga {
    archive_i: usize,
    image_kind: ImageKind,
    view_i: usize,
    scale: Option<ScaleImageManga>,
    gen_id_check: GenerationIdCheck,
    signal_tex_ready: Option<PollReady>
}

struct FerryImagesInfoManga<'a> {
    ctx: &'a egui::Context,
    thread_pool: &'a Arc<ThreadPool>,
    archive_path: Arc<PathBuf>,
    ship: mpmc::Sender<ImageResult>,
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
    ship: mpmc::Sender<ImageResult>,
    signal_cache_ready: CacheReady,
    signal_tex_ready: Option<PollReady>,
    metadata: Option<Metadata>
}

struct FerryImageMangaInfo {
    ctx: egui::Context,
    archive_path: Arc<PathBuf>,
    archive_i: usize,
    image_kind: ImageKind,
    view_i: usize,
    scale: Option<ScaleImageManga>,
    ship: mpmc::Sender<ImageResult>,
    gen_id_check: GenerationIdCheck,
    signal_tex_ready: Option<PollReady>
}

struct FerryCachedImageInfo<'a> {
    ctx: egui::Context,
    path: &'a Path,
    stage: Stage,
    grid_entry_i: usize,
    ship: mpmc::Sender<ImageResult>,
    gen_id_check: &'a Option<GenerationIdCheck>,
    wait_cache_ready: CacheReady,
    signal_tex_ready: Option<PollReady>
}

#[derive(Clone)]
struct GenerationIdCheck {
    id: GenerationId,
    expected: usize
}
impl GenerationIdCheck {
    fn check(&self) -> ResVar<()> {
        if self.id.load(Ordering::Relaxed) != self.expected {
            return Err(ErrVar::Cancel)
        }

        Ok(())
    }
}

struct GridEntryInfo {
    path: PathBuf,
    stem: Rc<str>,
    sort_name: Option<Rc<str>>,
    file_kind: FileKind,
    image_i: Option<usize>,
    metadata: Option<Arc<Metadata>>,
    bookmark: Option<Pivot>
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
    gen_id_check: Option<GenerationIdCheck>,
    signal_tex_ready: Option<PollReady>,
    metadata: Option<Metadata>
}

struct IrisInfo {
    wgpu: egui_wgpu::RenderState,
    tex: wgpu::Texture,
    tex_id: egui::TextureId,
    index: usize,
    gen_id_check: Option<GenerationIdCheck>,
    dst_extent: Extent2dF,
    scaler: Arc<Scaler>,
    ship: mpmc::Sender<ScaledTexManga>,
    to_thanatos: mpmc::Sender<Soul>
}

#[derive(Clone, Default)]
struct GenerationId(Arc<AtomicUsize>);
impl GenerationId {
    fn get_next_check(&self) -> GenerationIdCheck {
        let id = self.clone();
        let expected = id.fetch_add(1, Ordering::Relaxed) + 1;

        GenerationIdCheck { id, expected }
    }
}
impl Deref for GenerationId {
    type Target = Arc<AtomicUsize>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

#[derive(Default)]
struct ImageStates {
    grid: ImageState,
    details: ImageState,
    ref_count: usize,
    gen_id: GenerationId
}
impl ImageStates {
    fn clone_cache_readies_on_should_scale(&mut self) -> CacheReadies {
        CacheReadies {
            grid: self.grid.clone_cache_ready_on_should_scale(),
            details: self.details.clone_cache_ready_on_should_scale()
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
            ..default!()
        }
    }

    fn take_cache_readies(&mut self) -> CacheReadies {
        CacheReadies {
            grid: self.grid.take_cache_ready(),
            details: self.details.take_cache_ready()
        }
    }

    fn take_cache_readies_on_not_should_scale(&mut self) -> CacheReadies {
        CacheReadies {
            grid: self.grid.take_cache_ready_on_not_should_scale(),
            details: self.details.take_cache_ready_on_not_should_scale()
        }
    }

    fn should_scale(&self) -> ShouldScale {
        ShouldScale {
            grid: matches!(self.grid, ImageState::ShouldScale { .. }),
            details: matches!(self.details, ImageState::ShouldScale { .. })
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
    scale_pc: f32,
    scale_drag_anchor: f32,
    flagged_scale: Option<egui::Rect>,
    filter: FilterAccel,
    tint: egui::Rgba,
    sepia_alpha_pc: f32,
    white_level_pc: f32,
    scroll_kind: ScrollKind,
    scroll_offset: egui::Vec2,
    scroll_offset_y_anchor: Option<f32>,
    go_to_scroll_offset_y: Option<f32>,
    spring_damper: SpringDamper,
    secondary_was_down: bool,
    residence: Range<usize>,
    visible_view: Range<usize>,
    stream: Stream,
    to_thanatos: LateInit<mpmc::Sender<Soul>>
}
impl Manga {
    fn new(spring_damper: SpringDamper) -> Self {
        Self {
            scale_pc: 100.,
            filter: FilterAccel::Gpu(FilterKind::Blackman),
            tint: egui::Rgba::WHITE,
            white_level_pc: 100.,
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

        *self = Manga::new(mem::take(&mut self.spring_damper))
    }

    fn flag_scale(&mut self, ui: &mut egui::Ui, scale_pc: f32, viewport: egui::Rect) {
        if scale_pc == 100. && self.scale_pc == 100. {
            return
        }

        self.scale_pc = scale_pc;
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
    image_state: ImageStateManga,
    gen_id: GenerationId
}

struct PartialTex {
    tex: wgpu::Texture,
    tex_id: egui::TextureId,
    captive: Option<(image::RgbaImage, Option<PollReady>)>,
    stage: Stage,
    index: usize,
    gen_id_check: Option<GenerationIdCheck>,
    offset: usize,
    row_size: usize,
    chunk_row_count: usize
}

struct PendingTagOp {
    tag: Rc<str>,
    op: TagOp
}

/// Page coords of the viewport center
#[derive(Clone, Copy, Deserialize, Serialize)]
struct Pivot {
    page_i: usize,
    page_inset_pc: f32
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

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct PushConstants {
    render_pass_kind: u32,
    src_tex_extent: [f32; 2],
    dst_tex_extent: [f32; 2]
}

fn create_sampler_render_pipeline(device: &wgpu::Device, bind_group_layout: &wgpu::BindGroupLayout, shader_module: &wgpu::ShaderModule) -> wgpu::RenderPipeline {
    use wgpu::*;

    let pipeline_layout_desc = PipelineLayoutDescriptor {
        bind_group_layouts: &[Some(bind_group_layout)],
        ..default!()
    };
    let pipeline_layout = device.create_pipeline_layout(&pipeline_layout_desc);

    let vertex_state = VertexState {
        module: shader_module,
        entry_point: Some("vertex_main"),
        compilation_options: default!(),
        buffers: default!()
    };
    let color_target_state = ColorTargetState {
        format: TextureFormat::Rgba8UnormSrgb,
        blend: None,
        write_mask: ColorWrites::ALL
    };
    let fragment_state = FragmentState {
        module: shader_module,
        entry_point: Some("fragment_main"),
        compilation_options: PipelineCompilationOptions {
            constants: &[("0", FilterKind::Bilinear as u32 as f64)],
            zero_initialize_workgroup_memory: default!()
        },
        targets: &[Some(color_target_state)]
    };

    let render_pipeline_desc = RenderPipelineDescriptor {
        label: None,
        layout: Some(&pipeline_layout),
        vertex: vertex_state,
        primitive: default!(), // Right hand coords - WGSL clip space +Y points up
        depth_stencil: None,
        multisample: default!(),
        fragment: Some(fragment_state),
        multiview_mask: None,
        cache: None
    };

    device.create_render_pipeline(&render_pipeline_desc)
}

fn create_blackman_render_pipelines(device: &wgpu::Device, bind_group_layout: &wgpu::BindGroupLayout, shader_module: &wgpu::ShaderModule) -> (wgpu::RenderPipeline, wgpu::RenderPipeline) {
    use wgpu::*;

    let pipeline_layout_desc = PipelineLayoutDescriptor {
        bind_group_layouts: &[Some(bind_group_layout)],
        immediate_size: (mem::size_of::<RenderPassKind>() + 2 * mem::size_of::<Extent2dF>()) as u32,
        ..default!()
    };
    let pipeline_layout = device.create_pipeline_layout(&pipeline_layout_desc);

    let vertex_state = VertexState {
        module: shader_module,
        entry_point: Some("vertex_main"),
        compilation_options: default!(),
        buffers: default!()
    };
    let color_target_state0 = ColorTargetState {
        format: TextureFormat::Rgba8Unorm,
        blend: None,
        write_mask: ColorWrites::ALL
    };
    let color_target_state1 = ColorTargetState {
        format: TextureFormat::Rgba8UnormSrgb,
        blend: None,
        write_mask: ColorWrites::ALL
    };
    let fragment_state0 = FragmentState {
        module: shader_module,
        entry_point: Some("fragment_main"),
        compilation_options: PipelineCompilationOptions {
            constants: &[("0", FilterKind::Blackman as u32 as f64)],
            zero_initialize_workgroup_memory: default!()
        },
        targets: &[Some(color_target_state0)]
    };
    let fragment_state1 = FragmentState {
        module: shader_module,
        entry_point: Some("fragment_main"),
        compilation_options: PipelineCompilationOptions {
            constants: &[("0", FilterKind::Blackman as u32 as f64)],
            zero_initialize_workgroup_memory: default!()
        },
        targets: &[Some(color_target_state1)]
    };

    let render_pipeline0_desc = RenderPipelineDescriptor {
        label: None,
        layout: Some(&pipeline_layout),
        vertex: vertex_state.clone(),
        primitive: default!(),
        depth_stencil: None,
        multisample: default!(),
        fragment: Some(fragment_state0),
        multiview_mask: None,
        cache: None
    };
    let render_pipeline1_desc = RenderPipelineDescriptor {
        label: None,
        layout: Some(&pipeline_layout),
        vertex: vertex_state,
        primitive: default!(),
        depth_stencil: None,
        multisample: default!(),
        fragment: Some(fragment_state1),
        multiview_mask: None,
        cache: None
    };
    let render_pipeline0 = device.create_render_pipeline(&render_pipeline0_desc);
    let render_pipeline1 = device.create_render_pipeline(&render_pipeline1_desc);

    (render_pipeline0, render_pipeline1)
}

struct ResetResidence {
    row_cell_count: usize,
    visible_cell_count: usize
}

struct ScaleImageManga {
    extent: Extent2dF,
    filter: fir::FilterType
}

struct ScaledTexManga {
    tex_id: egui::TextureId,
    index: usize,
    gen_id_check: Option<GenerationIdCheck>,
    extent: Extent2dF
}

struct Scaler {
    nearest_sampler: wgpu::Sampler,
    linear_sampler: wgpu::Sampler,
    bind_group_layout: wgpu::BindGroupLayout,
    sampler_render_pipeline: wgpu::RenderPipeline,
    blackman_render_pipeline0: wgpu::RenderPipeline,
    blackman_render_pipeline1: wgpu::RenderPipeline
}
impl Scaler {
    fn new(device: &wgpu::Device) -> Self {
        use wgpu::*;

        let nearest_sampler_desc = SamplerDescriptor {
            label: None,
            address_mode_u: AddressMode::ClampToEdge,
            address_mode_v: AddressMode::ClampToEdge,
            address_mode_w: AddressMode::ClampToEdge,
            mag_filter: FilterMode::Nearest,
            min_filter: FilterMode::Nearest,
            mipmap_filter: MipmapFilterMode::Nearest,
            lod_min_clamp: 0.,
            lod_max_clamp: 0.,
            compare: None,
            anisotropy_clamp: 1,
            border_color: None
        };
        let linear_sampler_desc = SamplerDescriptor {
            label: None,
            address_mode_u: AddressMode::ClampToEdge,
            address_mode_v: AddressMode::ClampToEdge,
            address_mode_w: AddressMode::ClampToEdge,
            mag_filter: FilterMode::Linear,
            min_filter: FilterMode::Linear,
            mipmap_filter: MipmapFilterMode::Linear,
            lod_min_clamp: 0.,
            lod_max_clamp: 0.,
            compare: None,
            anisotropy_clamp: 1,
            border_color: None
        };
        let nearest_sampler = device.create_sampler(&nearest_sampler_desc);
        let linear_sampler = device.create_sampler(&linear_sampler_desc);

        let bind_group_layout_desc = BindGroupLayoutDescriptor {
            label: None,
            entries: &[
                BindGroupLayoutEntry {
                    binding: 0,
                    visibility: ShaderStages::FRAGMENT,
                    ty: BindingType::Texture {
                        sample_type: TextureSampleType::Float { filterable: true },
                        view_dimension: TextureViewDimension::D2,
                        multisampled: false
                    },
                    count: None
                },
                BindGroupLayoutEntry {
                    binding: 1,
                    visibility: ShaderStages::FRAGMENT,
                    ty: BindingType::Sampler(SamplerBindingType::Filtering),
                    count: None
                }
            ]
        };
        let bind_group_layout = device.create_bind_group_layout(&bind_group_layout_desc);

        let shader_module_desc = wgpu::include_spirv!("../../../assets/scale.spv");
        let shader_module = device.create_shader_module(shader_module_desc);

        let sampler_render_pipeline = create_sampler_render_pipeline(device, &bind_group_layout, &shader_module);
        let blackman_render_pipelines = create_blackman_render_pipelines(device, &bind_group_layout, &shader_module);

        Self {
            nearest_sampler,
            linear_sampler,
            bind_group_layout,
            sampler_render_pipeline,
            blackman_render_pipeline0: blackman_render_pipelines.0,
            blackman_render_pipeline1: blackman_render_pipelines.1
        }
    }
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

#[derive(Default)]
struct SpringDamper {
    multiplier: f32,
    pos: f32,
    vel: f32,
    equilibrium_pos: f32, // Target
    displacement: f32,
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
    fn step(&mut self, ui: &mut egui::Ui, refresh_rate: u32) {
        const EPSILON: f32 = 0.0001;

        let (dt, delta) = ui.input(|i| {
            let dt = i.unstable_dt.min(1. / refresh_rate as f32);

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
        self.displacement = self.pos - self.equilibrium_pos; // Update in equilibrium relative space

        self.pos = self.pos_pos_coef * self.displacement + self.pos_vel_coef * old_vel + self.equilibrium_pos;
        self.vel = self.vel_pos_coef * self.displacement + self.vel_vel_coef * old_vel;

        self.delta = self.pos - old_pos;

        if self.delta.abs() < EPSILON {
            self.stop();
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

struct WorkerThreadState {
    resizer: fir::Resizer,
    // src_linear: Vec<f32>,
    // dst_linear: Vec<f32>
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

enum FilterAccel {
    Cpu(fir::FilterType),
    Gpu(FilterKind)
}
impl Default for FilterAccel {
    fn default() -> Self {
        Self::Cpu(default!())
    }
}

#[repr(u32)]
enum FilterKind {
    Nearest,
    Bilinear,
    Blackman
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
    Png,
    Webp
}

#[derive(Default)]
enum ImageState {
    #[default]
    None,
    NoneCheckCache { cache_ready: CacheReady },
    ShouldScale { cache_ready: CacheReady },
    Ready { tex_id: egui::TextureId, extent: Extent2dF, cache_ready: CacheReady },
    Failed
}
impl ImageState {
    fn clone_cache_ready_on_should_scale(&self) -> CacheReady {
        match self {
            Self::ShouldScale { cache_ready } => cache_ready.clone(),
            _ => None
        }
    }

    fn take_cache_ready(&mut self) -> CacheReady {
        match self {
            Self::NoneCheckCache { cache_ready } |
            Self::ShouldScale { cache_ready } |
            Self::Ready { cache_ready, .. } =>
                mem::take(cache_ready),
            _ => None
        }
    }

    fn take_cache_ready_on_not_should_scale(&mut self) -> CacheReady {
        match self {
            Self::ShouldScale { .. } => None,
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

#[repr(u32)]
#[derive(Clone, Copy)]
enum RenderPassKind {
    Horizontal,
    Vertical
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
    RgbaImage(image::RgbaImage),
    TexId(egui::TextureId)
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
    Manga,
    Restart
}

#[derive(Default, Deserialize, PartialEq)]
enum Watching {
    Movie,
    #[default]
    TV,
    Words
}

fn try_add_image(ui: &mut egui::Ui, image_state: &mut ImageState, text: &str, poll_ready: &PollReady, animation: Option<&mut AnimationInfo>) -> egui::Response {
    match image_state {
        ImageState::Ready { tex_id, extent, .. } if poll_ready.is_ready() => {
            if let Some(animation) = animation {
                let opacity = get_animation_opacity(ui, animation);
                ui.set_opacity(opacity);
            }

            let tex = egui::load::SizedTexture::new(*tex_id, *extent);
            let image = egui::Image::new(tex).sense(egui::Sense::click());

            match extent.orientation() {
                Orientation::Tall => ui.add(image),
                Orientation::Wide => ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| ui.add(image)).inner
            }
        },
        ImageState::Failed => alloc_painted_text(ui, text),
        _ => alloc_painted_text(ui, "...")
    }
}

fn try_add_image_manga(ui: &mut egui::Ui, image_state: &mut ImageStateManga, rect: egui::Rect, tint: egui::Rgba) -> Option<egui::Response> {
    if let ImageStateManga::Ready { tex_id, extent, .. } = image_state {
        let tex = egui::load::SizedTexture::new(*tex_id, *extent);
        let tint: egui::Color32 = tint.into();
        let image = egui::Image::new(tex).sense(egui::Sense::click()).tint(tint);

        let mut ui = ui.new_child(egui::UiBuilder::new().max_rect(rect).layout(egui::Layout::top_down(egui::Align::Center)));

        return Some(ui.add(image))
    }

    None
}

fn alloc_painted_text(ui: &mut egui::Ui, text: &str) -> egui::Response {
    let (space_id, space_rect) = ui.allocate_space(ui.available_size());
    let space_resp = ui.interact(space_rect, space_id, egui::Sense::click());

    let text_place_rect = space_rect.shrink(15.);
    let mut layout_job = egui::text::LayoutJob {
        wrap: egui::text::TextWrapping {
            max_width: text_place_rect.width(),
            ..default!()
        },
        ..default!()
    };
    let font_id = ui.style().text_styles.get(&egui::TextStyle::Body).unwrap().clone();
    let text_color = ui.visuals().text_color();
    layout_job.append(text, 0.0, egui::TextFormat::simple(font_id, text_color));

    let galley = ui.painter().layout_job(layout_job);
    let galley_pos = egui::Align2::CENTER_CENTER.align_size_within_rect(galley.size(), text_place_rect).min;
    ui.painter().galley(galley_pos, galley, text_color);

    space_resp
}

#[hotpath::measure]
fn alloc_texture(wgpu: &egui_wgpu::RenderState, width: u32, height: u32, render_attachment: bool) -> (wgpu::Texture, wgpu::TextureView) {
    let egui_wgpu::RenderState { device, .. } = wgpu;

    let usage = wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::TEXTURE_BINDING;
    let usage = if render_attachment { usage | wgpu::TextureUsages::RENDER_ATTACHMENT } else { usage };
    let tex_desc = wgpu::TextureDescriptor {
        label: None,
        size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage,
        view_formats: &[wgpu::TextureFormat::Rgba8UnormSrgb]
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
        return 1.0
    }

    let pi_x = PI * x;

    pi_x.sin() / pi_x
}

fn blackman(x: f64) -> f64 {
    let t = x.abs();

    if t >= BLACKMAN_SUPPORT {
        return 0.0
    }

    let pi_t = PI * t;
    let window = 0.42 +
        0.5 * (pi_t / BLACKMAN_SUPPORT).cos() +
        0.08 * (2.0 * pi_t / BLACKMAN_SUPPORT).cos();

    sinc(t) * window
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

fn demeter(ctx: egui::Context, wgpu: egui_wgpu::RenderState, port: mpmc::Receiver<ImageInfo>, ship: mpmc::Sender<PartialTex>, chunk_size: usize) {
    for ImageInfo { image, stage, index: view_i, gen_id_check, signal_tex_ready, .. } in port.iter() {
        hotpath::measure_block!(formatcp!("{}::demeter", module_path!()), {
            let image_size = image.as_raw().len();
            let row_size = 4 * image.width() as usize;
            let chunk_row_count = chunk_size.div(row_size).max(1);

            if image_size < chunk_size {
                let offset = image.height() as usize;
                let (tex, tex_id) = alloc_write_texture(&wgpu, &image);

                ship.send(PartialTex { tex, tex_id, captive: Some((image, signal_tex_ready)), stage, index: view_i, gen_id_check, offset, row_size, chunk_row_count }).unwrap();
            } else {
                let (tex, tex_id) = alloc_clear_texture(&wgpu, &image);

                ship.send(PartialTex { tex, tex_id, captive: Some((image, signal_tex_ready)), stage, index: view_i, gen_id_check, offset: 0, row_size, chunk_row_count }).unwrap();
            }
        });

        ctx.request_repaint();
    }
}

fn iris_sampler(info: IrisInfo, filter_mode: wgpu::FilterMode) {
    use wgpu::*;

    let IrisInfo { wgpu, tex: src_tex, tex_id, index, gen_id_check, dst_extent, scaler, ship, to_thanatos } = info;
    let egui_wgpu::RenderState { device, queue, .. } = &wgpu;

    let dst_tex_desc = wgpu::TextureDescriptor {
        label: None,
        size: Extent3d {
            width: dst_extent.width as u32,
            height: dst_extent.height as u32,
            depth_or_array_layers: 1
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: TextureDimension::D2,
        format: TextureFormat::Rgba8UnormSrgb,
        usage: TextureUsages::RENDER_ATTACHMENT | TextureUsages::TEXTURE_BINDING,
        view_formats: &[TextureFormat::Rgba8Unorm]
    };
    let dst_tex = device.create_texture(&dst_tex_desc);

    let src_tex_view_desc = TextureViewDescriptor {
        format: Some(TextureFormat::Rgba8UnormSrgb),
        ..default!()
    };
    let src_tex_view = src_tex.create_view(&src_tex_view_desc);
    let dst_tex_view = dst_tex.create_view(&default!());

    let bind_group_desc = BindGroupDescriptor {
        label: None,
        layout: &scaler.bind_group_layout,
        entries: &[
            BindGroupEntry {
                binding: 0,
                resource: BindingResource::TextureView(&src_tex_view)
            },
            BindGroupEntry {
                binding: 1,
                resource: BindingResource::Sampler(match filter_mode {
                    FilterMode::Nearest => &scaler.nearest_sampler,
                    FilterMode::Linear => &scaler.linear_sampler
                })
            }
        ]
    };
    let bind_group = device.create_bind_group(&bind_group_desc);

    let render_pass_desc = RenderPassDescriptor {
        label: None,
        color_attachments: &[
            Some(RenderPassColorAttachment {
                view: &dst_tex_view,
                depth_slice: None,
                resolve_target: None,
                ops: Operations {
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

    let mut encoder = device.create_command_encoder(&default!());
    {
        let mut render_pass = encoder.begin_render_pass(&render_pass_desc);
        render_pass.set_pipeline(&scaler.sampler_render_pipeline);
        render_pass.set_bind_group(0, &bind_group, default!());
        render_pass.draw(0..3, 0..1);
    }
    let command_buffer = encoder.finish();
    queue.submit([command_buffer]);

    let dst_tex_view_desc = TextureViewDescriptor {
        format: Some(TextureFormat::Rgba8Unorm),
        ..default!()
    };
    let dst_tex_view = dst_tex.create_view(&dst_tex_view_desc);
    let dst_tex_id = register_native_texture(&wgpu, &dst_tex_view);

    ship.send(ScaledTexManga {
        tex_id: dst_tex_id,
        index,
        gen_id_check,
        extent: dst_extent
    })
    .unwrap();

    to_thanatos.send(Soul::TexId(tex_id)).unwrap();
}

fn iris_blackman(info: IrisInfo) {
    use wgpu::*;

    let IrisInfo { wgpu, tex: src_tex, tex_id, index, gen_id_check, dst_extent, scaler, ship, to_thanatos } = info;
    let egui_wgpu::RenderState { device, queue, .. } = &wgpu;

    let dst_tex0_desc = wgpu::TextureDescriptor {
        label: None,
        size: Extent3d {
            width: dst_extent.width as u32,
            height: src_tex.height(),
            depth_or_array_layers: 1
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: TextureDimension::D2,
        format: TextureFormat::Rgba8Unorm,
        usage: TextureUsages::RENDER_ATTACHMENT | TextureUsages::TEXTURE_BINDING,
        view_formats: default!()
    };
    let dst_tex1_desc = wgpu::TextureDescriptor {
        label: None,
        size: Extent3d {
            width: dst_extent.width as u32,
            height: dst_extent.height as u32,
            depth_or_array_layers: 1
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: TextureDimension::D2,
        format: TextureFormat::Rgba8UnormSrgb,
        usage: TextureUsages::RENDER_ATTACHMENT | TextureUsages::TEXTURE_BINDING,
        view_formats: &[TextureFormat::Rgba8Unorm]
    };
    let dst_tex0 = device.create_texture(&dst_tex0_desc);
    let dst_tex1 = device.create_texture(&dst_tex1_desc);

    let src_tex_view_desc = TextureViewDescriptor {
        format: Some(TextureFormat::Rgba8UnormSrgb),
        ..default!()
    };
    let src_tex_view = src_tex.create_view(&src_tex_view_desc);
    let dst_tex_view0 = dst_tex0.create_view(&default!());
    let dst_tex_view1 = dst_tex1.create_view(&default!());

    let bind_group0_desc = BindGroupDescriptor {
        label: None,
        layout: &scaler.bind_group_layout,
        entries: &[
            BindGroupEntry {
                binding: 0,
                resource: BindingResource::TextureView(&src_tex_view)
            },
            BindGroupEntry {
                binding: 1,
                resource: BindingResource::Sampler(&scaler.nearest_sampler)
            }
        ]
    };
    let bind_group1_desc = BindGroupDescriptor {
        label: None,
        layout: &scaler.bind_group_layout,
        entries: &[
            BindGroupEntry {
                binding: 0,
                resource: BindingResource::TextureView(&dst_tex_view0)
            },
            BindGroupEntry {
                binding: 1,
                resource: BindingResource::Sampler(&scaler.nearest_sampler)
            }
        ]
    };
    let bind_group0 = device.create_bind_group(&bind_group0_desc);
    let bind_group1 = device.create_bind_group(&bind_group1_desc);

    let render_pass0_desc = RenderPassDescriptor {
        label: None,
        color_attachments: &[
            Some(RenderPassColorAttachment {
                view: &dst_tex_view0,
                depth_slice: None,
                resolve_target: None,
                ops: Operations {
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
    let render_pass1_desc = RenderPassDescriptor {
        label: None,
        color_attachments: &[
            Some(RenderPassColorAttachment {
                view: &dst_tex_view1,
                depth_slice: None,
                resolve_target: None,
                ops: Operations {
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

    let mut encoder = device.create_command_encoder(&default!());
    {
        let mut render_pass0 = encoder.begin_render_pass(&render_pass0_desc);
        render_pass0.set_pipeline(&scaler.blackman_render_pipeline0);
        render_pass0.set_bind_group(0, &bind_group0, default!());
        let push_constants = PushConstants {
            render_pass_kind: RenderPassKind::Horizontal as u32,
            src_tex_extent: src_tex.as_::<Extent2dF>().into(),
            dst_tex_extent: dst_tex0.as_::<Extent2dF>().into()
        };
        render_pass0.set_immediates(0, bytemuck::bytes_of(&push_constants));
        render_pass0.draw(0..3, 0..1);
    }
    {
        let mut render_pass1 = encoder.begin_render_pass(&render_pass1_desc);
        render_pass1.set_pipeline(&scaler.blackman_render_pipeline1);
        render_pass1.set_bind_group(0, &bind_group1, default!());
        let push_constants = PushConstants {
            render_pass_kind: RenderPassKind::Vertical as u32,
            src_tex_extent: dst_tex0.as_::<Extent2dF>().into(),
            dst_tex_extent: dst_tex1.as_::<Extent2dF>().into()
        };
        render_pass1.set_immediates(0, bytemuck::bytes_of(&push_constants));
        render_pass1.draw(0..3, 0..1);
    }
    let command_buffer = encoder.finish();
    queue.submit([command_buffer]);

    let dst_tex_view_desc = TextureViewDescriptor {
        format: Some(TextureFormat::Rgba8Unorm),
        ..default!()
    };
    let dst_tex_view = dst_tex1.create_view(&dst_tex_view_desc);
    let dst_tex_id = register_native_texture(&wgpu, &dst_tex_view);

    ship.send(ScaledTexManga {
        tex_id: dst_tex_id,
        index,
        gen_id_check,
        extent: dst_extent
    })
    .unwrap();

    to_thanatos.send(Soul::TexId(tex_id)).unwrap();
}

fn hephaestus(ctx: egui::Context, wgpu: egui_wgpu::RenderState, port: mpmc::Receiver<WriteTex>) {
    let mut captive_ = None;

    for WriteTex { tex, captive, offset, row_count, last_write } in port.iter() {
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

fn thanatos(wgpu: egui_wgpu::RenderState, port: mpmc::Receiver<Soul>) {
    let egui_wgpu::RenderState { renderer, ..} = wgpu;

    for soul in port.iter() {
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
                Soul::RgbaImage(image) => drop(image),
                Soul::TexId(tex_id) => renderer.write().free_texture(&tex_id)
            }
        });
    }
}

fn get_animation_opacity(ui: &mut egui::Ui, info: &mut AnimationInfo) -> f32 {
    match info.target {
        true => ui.ctx().animate_bool_with_time_and_easing("animate".into(), true, info.dur, info.kind.as_easing()),
        false => {
            ui.ctx().clear_animations();
            info.target = true; // For future calls

            ui.ctx().animate_bool_with_time_and_easing("animate".into(), false, info.dur, info.kind.as_easing())
        }
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
    let mut buf = Vec::new();
    zip_file.read_to_end(&mut buf)?;

    let image = match image_kind {
        ImageKind::Jpeg => {
            let reader = io::BufReader::new(io::Cursor::new(buf));
            let opts = zune_jpeg::zune_core::options::DecoderOptions::new_fast()
                .jpeg_set_out_colorspace(zune_jpeg::zune_core::colorspace::ColorSpace::RGBA);
            let mut decoder = zune_jpeg::JpegDecoder::new_with_options(reader, opts);

            let decoded = decoder.decode()?;
            let dimensions = decoder.dimensions().unwrap();

            image::RgbaImage::from_vec(dimensions.0 as u32, dimensions.1 as u32, decoded).unwrap()
        },
        ImageKind::Png => {
            let reader = io::BufReader::new(io::Cursor::new(buf));
            let opts = zune_png::zune_core::options::DecoderOptions::new_fast()
                .png_set_add_alpha_channel(true)
                .png_set_decode_animated(false)
                .png_set_strip_to_8bit(true);
            let mut decoder = zune_png::PngDecoder::new_with_options(reader, opts);

            let decoded = decoder.decode()?;
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
        },
        ImageKind::Webp => {
            let (buf, width, height) = zenwebp::oneshot::decode_rgba(&buf).map_err(|err| err.decompose().0)?;

            image::RgbaImage::from_vec(width, height, buf).unwrap()
        }
    };

    Ok(image)
}

#[cfg(feature = "resize")]
#[hotpath::measure]
fn resize_image_common(image: image::RgbaImage, dst_extent: Extent2dF, _filter: fir::FilterType) -> ResVar<image::RgbaImage> {
    use rgb::FromSlice;

    let mut src_linear = image::Rgba32FImage::new(
        image.width(),
        image.height()
    );
    linear_srgb::default::srgb_u8_to_linear_rgba_slice(&image, &mut src_linear);

    let (src_width, src_height) = image.dimensions();
    let Extent2dU { width: dst_width, height: dst_height } = dst_extent.into();
    let mut tmp_linear = image::Rgba32FImage::new(dst_width, src_height);
    let mut dst_linear = image::Rgba32FImage::new(dst_width, dst_height);

    let mut resizer = resize::new(
        src_width as usize,
        src_height as usize,
        dst_width as usize,
        src_height as usize,
        resize::Pixel::RGBAF32,
        resize::Type::Custom(blackman_filter())
    )?;
    resizer.resize(src_linear.as_rgba(), tmp_linear.as_rgba_mut())?;
    let mut resizer = resize::new(
        dst_width as usize,
        src_height as usize,
        dst_width as usize,
        dst_height as usize,
        resize::Pixel::RGBAF32,
        resize::Type::Custom(blackman_filter())
    )?;
    resizer.resize(tmp_linear.as_rgba(), dst_linear.as_rgba_mut())?;

    let mut srgb = image::RgbaImage::new(
        dst_width,
        dst_height
    );
    linear_srgb::default::linear_to_srgb_u8_rgba_slice(&dst_linear, &mut srgb);

    Ok(srgb)
}

#[cfg(not(feature = "resize"))]
#[hotpath::measure]
fn resize_image_common(image: image::RgbaImage, extent: Extent2dF, filter: fir::FilterType) -> ResVar<image::RgbaImage> {
    WORKER_THREAD_STATE.with_borrow_mut(|ts| {
        let Extent2dU { width: dst_width, height: dst_height } = extent.into();

        // ts.src_linear.resize((image.width() * image.height() * 4) as usize, 0.);
        // let mut src_linear = image::Rgba32FImage::from_vec(
        //     image.width(),
        //     image.height(),
        //     mem::take(&mut ts.src_linear)
        // )
        // .unwrap();
        let mut src_linear = image::Rgba32FImage::new(
            image.width(),
            image.height()
        );
        linear_srgb::default::srgb_u8_to_linear_rgba_slice(&image, &mut src_linear);

        // ts.dst_linear.resize((dst_width * dst_height * 4) as usize, 0.);
        // let mut dst_linear = image::Rgba32FImage::from_vec(
        //     dst_width,
        //     dst_height,
        //     mem::take(&mut ts.dst_linear)
        // )
        // .unwrap();
        let mut dst_linear = image::Rgba32FImage::new(
            dst_width,
            dst_height,
        );

        let res = (|| -> ResVar<_> {
            let opts = fir::ResizeOptions { algorithm: fir::ResizeAlg::Convolution(filter), ..default!() };
            ts.resizer.resize(&src_linear, &mut dst_linear, &opts)?;
            ts.resizer.reset_internal_buffers();

            let mut srgb = image::RgbaImage::new(
                dst_width,
                dst_height
            );
            linear_srgb::default::linear_to_srgb_u8_rgba_slice(&dst_linear, &mut srgb);

            Ok(srgb)
        })();
        // ts.src_linear = src_linear.into_raw();
        // ts.dst_linear = dst_linear.into_raw();
        let srgb = res?;

        Ok(srgb)
    })
}

fn ferry_base_image(info: FerryBaseImageInfo) -> Res1<()> {
    let FerryBaseImageInfo { ctx, src_path, dst_path, cell_extent, stage, grid_entry_i, ship, signal_cache_ready, signal_tex_ready, metadata } = info;

    let inner = || -> Res1<image::RgbaImage> {
        let src_image = load_rgba_image(src_path)?;

        let aspect_ratio_v = Extent2dF::from(src_image.dimensions()).aspect_ratio_v();
        let (dst_width, dst_height) = match Orientation::from(aspect_ratio_v) {
            Orientation::Tall => (cell_extent.height.div(aspect_ratio_v).round(), cell_extent.height),
            Orientation::Wide => (cell_extent.width, cell_extent.width.mul(aspect_ratio_v).round())
        };
        let dst_image = resize_image_common(src_image, [dst_width, dst_height].into(), fir::FilterType::Custom(blackman_filter_fir()))?;

        Ok(dst_image)
    };

    match inner() {
        Ok(image) => {
            let image_ = image.clone();

            if ship.send(Ok(ImageInfo { image: image_, stage, index: grid_entry_i, gen_id_check: None, signal_tex_ready, metadata })).is_ok() {
                ctx.request_repaint();
            }

            let image_file = fs::File::create(dst_path)?;
            let encoder = image::codecs::webp::WebPEncoder::new_lossless(image_file);
            let (width, height) = image.dimensions();
            encoder.encode(image.as_raw(), width, height, image::ExtendedColorType::Rgba8)?;

            drop(signal_cache_ready)
        },
        Err(err) => {
            _ = ship.send(Err((stage, grid_entry_i)));

            Err(err)?;
        }
    }

    Ok(())
}

fn ferry_image_manga(info: FerryImageMangaInfo) -> Res1<()> {
    let FerryImageMangaInfo { ctx, archive_path, archive_i, image_kind, view_i, scale, ship, gen_id_check, signal_tex_ready } = info;

    let inner = || -> Res1<image::RgbaImage> {
        gen_id_check.check()?;
        let src_image = load_rgba_image_manga(archive_path, archive_i, image_kind)?;

        let dst_image = if let Some(ScaleImageManga { extent, filter }) = scale {
            gen_id_check.check()?;
            resize_image_common(src_image, extent, filter)?
        } else {
            src_image
        };

        Ok(dst_image)
    };

    match inner() {
        Ok(image) => if ship.send(Ok(ImageInfo { image, stage: Stage::Manga, index: view_i, gen_id_check: Some(gen_id_check), signal_tex_ready, metadata: None })).is_ok() {
            ctx.request_repaint();
        },
        Err(err) => {
            if let Some(signal_tex_ready) = signal_tex_ready {
                signal_tex_ready.mark_done();
            }

            match err.var.as_ref() {
                ErrVar::Cancel => return Ok(()),
                _ => {
                    _ = ship.send(Err((Stage::Manga, view_i)));

                    return Err(err)
                }
            }
        }
    }

    Ok(())
}

fn ferry_cached_image(info: FerryCachedImageInfo) -> Res1<()> {
    let FerryCachedImageInfo { ctx, path, stage, grid_entry_i, ship, gen_id_check, wait_cache_ready, signal_tex_ready } = info;

    let inner = || -> ResVar<image::RgbaImage> {
        if let Some(gen_id_check) = gen_id_check { gen_id_check.check()? }
        let image = load_rgba_image_cached(path)?;

        Ok(image)
    };

    if let Some(wait_cache_ready) = wait_cache_ready {
        wait_cache_ready.wait();
    }

    match inner() {
        Ok(image) => if ship.send(Ok(ImageInfo { image, stage, index: grid_entry_i, gen_id_check: gen_id_check.clone(), signal_tex_ready, metadata: None })).is_ok() {
            ctx.request_repaint();
        },
        Err(err) => {
            if let Some(signal_tex_ready) = signal_tex_ready {
                signal_tex_ready.mark_done();
            }

            match err {
                ErrVar::Cancel => return Ok(()),
                _ => {
                    _ = ship.send(Err((stage, grid_entry_i)));

                    return Err(err.into())
                }
            }
        }
    }

    Ok(())
}

fn ferry_images(info: FerryImagesInfo) {
    let FerryImagesInfo {
        ctx,
        thread_pool,
        image_dirs,
        base_image_kind,
        grid_cell_extent,
        details_cell_extent,
        grid_ship,
        details_ship,
        ferry_image_infos,
        error_sx
    } = info;

    fn handle_err<const ID: u32>(error_sx: mpmc::Sender<String>, err: ErrLoc<ID>) {
        let msg = format!("{}: failed to ferry image: {}", module_path!(), err);
        send_log_err_msg(&error_sx, msg);
    }

    for info in ferry_image_infos {
        let FerryImageInfo {
            image_file_name,
            expected_metadata,
            grid_entry_i,
            gen_id_check,
            signal_cache_readies,
            wait_cache_readies,
            signal_tex_ready
        } = info;

        let ctx = ctx.clone();
        let thread_pool_ = thread_pool.clone();
        let base_image_kind = base_image_kind.clone();
        let grid_ship = grid_ship.clone();
        let details_ship = details_ship.clone();
        let error_sx_high = error_sx.clone();
        let error_sx_low = error_sx.clone();

        thread_pool.enqueue_high(move || {
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
                let metadata = Metadata {
                    created: metadata.created()?,
                    modified: metadata.modified()?,
                    len: metadata.len()
                };
                let metadata_differs = expected_metadata.is_none_or(|expected_metadata| *expected_metadata != metadata);

                match metadata_differs {
                    true => { // Scale grid & details
                        let ferry_base_image_info = FerryBaseImageInfo {
                            ctx: ctx.clone(),
                            src_path: &base_image_path,
                            dst_path: &grid_image_path,
                            cell_extent: grid_cell_extent,
                            stage: Stage::Grid,
                            grid_entry_i,
                            ship: grid_ship,
                            signal_cache_ready: signal_cache_readies.grid,
                            signal_tex_ready,
                            metadata: Some(metadata)
                        };
                        ferry_base_image(ferry_base_image_info)?;

                        thread_pool_.enqueue_low(move || {
                            let ferry_base_image_info = FerryBaseImageInfo {
                                ctx,
                                src_path: &base_image_path,
                                dst_path: &details_image_path,
                                cell_extent: details_cell_extent,
                                stage: Stage::Details,
                                grid_entry_i,
                                ship: details_ship,
                                signal_cache_ready: signal_cache_readies.details,
                                signal_tex_ready: None,
                                metadata: None
                            };
                            ferry_base_image(ferry_base_image_info).unwrap_or_else(|err| handle_err(error_sx_low, err));
                        });
                    },
                    false => {
                        match signal_cache_readies.grid.is_some() {
                            true => { // Scale grid
                                let ferry_base_image_info = FerryBaseImageInfo {
                                    ctx: ctx.clone(),
                                    src_path: &base_image_path,
                                    dst_path: &grid_image_path,
                                    cell_extent: grid_cell_extent,
                                    stage: Stage::Grid,
                                    grid_entry_i,
                                    ship: grid_ship,
                                    signal_cache_ready: signal_cache_readies.grid,
                                    signal_tex_ready,
                                    metadata: None
                                };
                                ferry_base_image(ferry_base_image_info)?
                            },
                            false => {
                                drop(signal_cache_readies.grid);

                                let ferry_cached_image_info = FerryCachedImageInfo {
                                    ctx: ctx.clone(),
                                    path: &grid_image_path,
                                    stage: Stage::Grid,
                                    grid_entry_i,
                                    ship: grid_ship,
                                    gen_id_check: &gen_id_check,
                                    wait_cache_ready: wait_cache_readies.grid,
                                    signal_tex_ready
                                };
                                ferry_cached_image(ferry_cached_image_info)?;
                            }
                        }
                        match signal_cache_readies.details.is_some() {
                            true => { // Scale details
                                thread_pool_.enqueue_low(move || {
                                    let ferry_base_image_info = FerryBaseImageInfo {
                                        ctx,
                                        src_path: &base_image_path,
                                        dst_path: &details_image_path,
                                        cell_extent: details_cell_extent,
                                        stage: Stage::Details,
                                        grid_entry_i,
                                        ship: details_ship,
                                        signal_cache_ready: signal_cache_readies.details,
                                        signal_tex_ready: None,
                                        metadata: None
                                    };
                                    ferry_base_image(ferry_base_image_info).unwrap_or_else(|err| handle_err(error_sx_low, err));
                                });
                            },
                            false => {
                                drop(signal_cache_readies.details);

                                thread_pool_.enqueue_low(move || {
                                    let ferry_cached_image_info = FerryCachedImageInfo {
                                        ctx,
                                        path: &details_image_path,
                                        stage: Stage::Details,
                                        grid_entry_i,
                                        ship: details_ship,
                                        gen_id_check: &gen_id_check,
                                        wait_cache_ready: wait_cache_readies.details,
                                        signal_tex_ready: None
                                    };
                                    ferry_cached_image(ferry_cached_image_info).unwrap_or_else(|err| handle_err(error_sx_low, err));
                                });
                            }
                        }
                    }
                }

                Ok(())
            })()
            .unwrap_or_else(|err| handle_err(error_sx_high, err));
        });
    }
}

fn ferry_images_manga(info: FerryImagesInfoManga) {
    let FerryImagesInfoManga {
        ctx,
        thread_pool,
        archive_path,
        ship,
        ferry_image_infos,
        error_sx
    } = info;

    for info in ferry_image_infos {
        let ctx = ctx.clone();
        let archive_path = archive_path.clone();
        let ship = ship.clone();
        let error_sx = error_sx.clone();

        thread_pool.enqueue_high(move || {
            (|| -> Res<()> {
                let ferry_image_manga_info = FerryImageMangaInfo {
                    ctx,
                    archive_path,
                    archive_i: info.archive_i,
                    image_kind: info.image_kind,
                    view_i: info.view_i,
                    scale: info.scale,
                    ship,
                    gen_id_check: info.gen_id_check,
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

fn stroke_rect(ui: &mut egui::Ui, rect: egui::Rect, clip_rect: Option<egui::Rect>) {
    let mut painter = ui.layer_painter(ui.layer_id());
    if let Some(clip_rect) = clip_rect {
        painter.set_clip_rect(clip_rect);
    }
    painter.rect_stroke(rect, 0.0, CELL_STROKE, egui::StrokeKind::Outside);
}

fn init_residence(max_cell_count: usize, central_size: egui::Vec2, grid_cell_size: egui::Vec2, grid_cell_space: egui::Vec2, lookahead: usize) -> Residence {
    let available_row_cell_count = (central_size.x - grid_cell_size.x).div(grid_cell_space.x).ceil() as usize;
    let available_col_cell_count = central_size.y.div(grid_cell_space.y).ceil() as usize;
    let visible_cell_count = (available_row_cell_count * available_col_cell_count).min(max_cell_count);

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

type Job = Box<dyn FnOnce() + Send + 'static>;

struct ThreadPool {
    high_priority: mpmc::Sender<Job>,
    low_priority: mpmc::Sender<Job>,
}
impl ThreadPool {
    fn enqueue_high<Op: FnOnce() + Send + 'static>(&self, job: Op) {
        self.high_priority.send(Box::new(job)).unwrap();
    }

    fn enqueue_low<Op: FnOnce() + Send + 'static>(&self, job: Op) {
        self.low_priority.send(Box::new(job)).unwrap();
    }
}

fn init_grid_entries(grid_entries: &mut Vec<GridEntryInfo>, cache: &mut Cache, images: &mut IndexMap<Arc<str>, ImageStates>, missing_base_images: &mut Vec<Rc<str>>, grid_cell_size: egui::Vec2, details_cell_size: egui::Vec2) {
    let grid_entry_info_iter = cache.library.iter()
        .map(|dir| dir.read_dir())
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

                let grid_entry_info = match cache.entries.get_mut(&path) {
                    Some(cache_entry_info) => {
                        let sort_name = cache_entry_info.sort_name.clone();
                        let image_i = cache_entry_info.image_i
                            .and_then(|cache_image_i| cache.images.get_index(cache_image_i))
                            .and_then(|image_file_name| images.get_full_mut(image_file_name.as_ref())
                                .tap_none(|| missing_base_images.push(image_file_name.clone()))
                            )
                            .map(|(image_i, _, image_states)| {
                                if cache_entry_info.should_scale.grid || cache.grid_cell_size != grid_cell_size {
                                    image_states.grid = ImageState::ShouldScale { cache_ready: Some(WaitGroup::new()) };
                                }
                                if cache_entry_info.should_scale.details || cache.details_cell_size != details_cell_size {
                                    image_states.details = ImageState::ShouldScale { cache_ready: Some(WaitGroup::new()) };
                                }
                                image_states.ref_count += 1;

                                image_i
                            });
                        let metadata = cache_entry_info.metadata.clone();
                        let bookmark = cache_entry_info.bookmark;

                        GridEntryInfo { path, stem, sort_name, file_kind, image_i, metadata, bookmark }
                    },
                    None => { // This entry is new. If a base image exists, scale and cache it
                        let sort_name = None;
                        let image_i = try_get_image_i(images);
                        let metadata = None;
                        let bookmark = None;

                        if let Some(image_states) = get_image_states_mut(images, image_i) {
                            image_states.grid = ImageState::ShouldScale { cache_ready: Some(WaitGroup::new()) };
                            image_states.details = ImageState::ShouldScale { cache_ready: Some(WaitGroup::new()) };
                        }

                        GridEntryInfo { path, stem, sort_name, file_kind, image_i, metadata, bookmark }
                    }
                };

                Ok(Some(grid_entry_info))
            })
            .unwrap_or_else(|err| {
                error!("{}: failed to read dir entry: {}", module_path!(), err);

                None
            })
        });

    grid_entries.clear();
    grid_entries.extend(grid_entry_info_iter);
}

struct MediaBrowser<'a> {
    wgpu: egui_wgpu::RenderState,
    thread_pool: Arc<ThreadPool>,
    refresh_rate: u32,
    image_dirs: &'static ImageDirs,
    images: IndexMap<Arc<str>, ImageStates>,
    deferred_metadata_sx: mpmc::Sender<MetadataInfo>,
    deferred_metadata_rx: mpmc::Receiver<MetadataInfo>,
    cache_path: PathBuf,
    cache: Cache,
    selected_library_entries: HashSet<usize>,
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
    grid_cell_strokes: Vec<egui::Rect>,
    grid_cell_tags_menu_selection: HashSet<Rc<str>>,
    grid_scroll_offset: f32,
    /// Indices into [`grid_entries`]
    grid_view: Vec<usize>,
    grid_view_i: usize,
    grid_view_pending_op: Option<GridViewOp>,
    lookahead: usize,
    proximity: usize,
    animation: AnimationInfo,
    residence: Range<usize>,
    stream: Stream,
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
    scaler: Arc<Scaler>,
    iris_ship: mpmc::Sender<ScaledTexManga>,
    from_iris: mpmc::Receiver<ScaledTexManga>,
    charon_ship: mpmc::Sender<ImageResult>,
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
        egui::CentralPanel::default()
            .frame(self.frame)
            .show(ui, |ui: &mut egui::Ui| {
                self.central_rect = ui.available_rect_before_wrap();

                match self.view_kind {
                    ViewKind::Grid => self.central_panel_grid(ui),
                    ViewKind::Details => self.central_panel_details(ui),
                    ViewKind::InitManga { selected_details_dir_entry_i } =>
                        if let Err(err) = self.init_manga(ui, selected_details_dir_entry_i) {
                            let msg = format!("{}", err);
                            send_log_err_msg(&self.error_sx, msg);

                            self.view_kind = ViewKind::Details;
                        },
                    ViewKind::WaitManga => self.wait_manga(),
                    ViewKind::Manga => self.central_panel_manga(ui),
                    ViewKind::Restart => {
                        // Backup entry tag indices (akin to cache)
                        let tags = self.tags.keys().cloned().collect::<Vec<_>>();
                        let mut grid_entry_tag_is = vec![Vec::with_capacity(self.tags.len()); self.grid_entries.len()];
                        for (tag_i, (_, set)) in self.tags.iter().enumerate() {
                            for grid_entry_i in set {
                                grid_entry_tag_is[*grid_entry_i].push(tag_i);
                            }
                        }
                        let mut grid_entry_tag_is = grid_entry_tag_is.into_iter().enumerate()
                            .map(|(grid_entry_i, tag_is)| {
                                (mem::take(&mut self.grid_entries[grid_entry_i].path), tag_is)
                            })
                            .collect::<HashMap<_, _>>();

                        // Reset tag sets - indices are invalid
                        for set in self.tags.values_mut() {
                            set.clear();
                        }

                        self.init_grid_entries();

                        // Refill tag sets with new indices
                        for (grid_entry_i, grid_entry_info) in self.grid_entries.iter().enumerate() {
                            if let Some(tag_is) = grid_entry_tag_is.get_mut(&grid_entry_info.path) {
                                for tag_i in tag_is {
                                    let tag = &tags[*tag_i];
                                    let set = self.tags.get_mut(tag);

                                    if let Some(set) = set {
                                        set.insert(grid_entry_i);
                                    }
                                }
                            }
                        }

                        self.reset_grid_view(ui);

                        self.view_kind = ViewKind::Grid;
                    }
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

        let mut grid_entry_tags = vec![Vec::with_capacity(self.tags.len()); self.grid_entries.len()];
        for (tag_i, (_, set)) in self.tags.iter().enumerate() {
            for grid_entry_i in set {
                grid_entry_tags[*grid_entry_i].push(tag_i);
            }
        }

        while let Ok(MetadataInfo { grid_entry_i, metadata }) = self.deferred_metadata_rx.try_recv() {
            self.grid_entries[grid_entry_i].metadata = Some(metadata);
        }

        self.cache.entries.clear();
        for (grid_entry_i, info) in mem::take(&mut self.grid_entries).into_iter().enumerate() {
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
                    tags: mem::take(&mut grid_entry_tags[grid_entry_i]),
                    bookmark: info.bookmark
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
    fn new(ctx: &egui::Context, wgpu: &egui_wgpu::RenderState, refresh_rate: u32, win_inner_extent: Extent2dU) -> Res<Self> {
        let config = config::get().read()?;
        let (grid_cell_width,
            details_cell_width,
            scroll_multiplier,
            lookahead,
            proximity,
            animation
        ) = config.media_browser.as_ref()
            .map(|media_browser_config| {
                (
                    media_browser_config.grid_cell_width.next_multiple_of(2) as f32,
                    media_browser_config.details_cell_width.next_multiple_of(2) as f32,
                    media_browser_config.scroll_multiplier,
                    media_browser_config.lookahead,
                    media_browser_config.proximity,
                    media_browser_config.animation.into()
                )
            })
            .ok_or(ErrVar::MissingConfigOption { name: config::MediaBrowser::NAME })?;

        if lookahead < 2 { Err(ErrVar::InvalidLookahead(lookahead))?; }
        let proximity_range = 1..lookahead;
        if !(proximity_range.contains(&proximity)) { Err(ErrVar::InvalidProximity(proximity))?; }

        let grid_cell_size = egui::vec2(grid_cell_width, grid_cell_width * ASPECT_RATIO_3_2);
        let grid_cell_space = grid_cell_size + GRID_IMAGE_SPACING;
        let details_cell_size = egui::vec2(details_cell_width, details_cell_width * ASPECT_RATIO_3_2);
        let discord_app_ids = config.discord.app_ids.clone();
        let discord_display_kind = config.discord.display_kind;
        let enable_override_glsl_shaders_checkbox = config.mpv.as_ref().map(|mpv_config| mpv_config.override_glsl_shaders.is_some()).unwrap_or(false);

        let (deferred_metadata_sx, deferred_metadata_rx) = mpmc::bounded(1);

        let (high_priority_sx, high_priority_rx) = mpmc::unbounded();
        let (low_priority_sx, low_priority_rx) = mpmc::unbounded();
        let thread_pool = Arc::new(ThreadPool {
            high_priority: high_priority_sx,
            low_priority: low_priority_sx
        });
        let worker_count = 4.min(thread::available_parallelism()?.get());
        for _ in 0..worker_count {
            let high_priority_rx = high_priority_rx.clone();
            let low_priority_rx = low_priority_rx.clone();

            thread::spawn(move || {
                loop {
                    while let Ok(job) = high_priority_rx.try_recv() {
                        job();
                    }

                    if let Ok(job) = low_priority_rx.try_recv() {
                        job();
                    }

                    crossbeam::channel::select_biased! {
                        recv(high_priority_rx) -> res => if let Ok(job) = res { job(); },
                        recv(low_priority_rx) -> res => if let Ok(job) = res { job(); }
                    }
                }
            });
        }

        let ctx_ = ctx.clone();
        let wgpu_ = wgpu.clone();
        let (to_hephaestus, hephaestus_port) = mpmc::unbounded();
        thread::spawn(move || hephaestus(ctx_, wgpu_, hephaestus_port));

        let ctx_ = ctx.clone();
        let wgpu_ = wgpu.clone();
        let chunk_size = PCIE_TRANSFER_LIMIT_MIBS / refresh_rate as usize * 1024_usize.pow(2);
        let (charon_ship, from_charon) = mpmc::unbounded();
        let (to_demeter, demeter_port) = mpmc::unbounded();
        let (demeter_ship, from_demeter) = mpmc::unbounded();
        thread::spawn(move || demeter(ctx_, wgpu_, demeter_port, demeter_ship, chunk_size));

        let (to_thanatos, thanatos_port) = mpmc::unbounded();
        let wgpu_ = wgpu.clone();
        thread::spawn(move || thanatos(wgpu_, thanatos_port));

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

        let scaler = Arc::new(Scaler::new(&wgpu.device));
        let (iris_ship, from_iris) = mpmc::unbounded();

        let frame = egui::Frame::central_panel(&ctx.global_style()).inner_margin(
            egui::Margin::symmetric(FRAME_INNER_MARGIN as i8, FRAME_INNER_MARGIN as i8)
        );
        let win_inner_extent = Extent2dF::from(win_inner_extent);
        let central_size = egui::vec2(
            win_inner_extent.width.sub(2. * FRAME_INNER_MARGIN).max(0.),
            win_inner_extent.height.sub(2. * FRAME_INNER_MARGIN).max(0.),
        );

        let (error_sx, error_rx) = mpmc::unbounded();
        let error_msg = "".to_string();

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

        let mut grid_entries = Vec::new();
        init_grid_entries(&mut grid_entries, &mut cache, &mut images, &mut missing_base_images, grid_cell_size, details_cell_size);

        let mut grid_view = Vec::with_capacity(grid_entries.len());
        grid_view.extend(0..grid_entries.len());
        sort_grid_view(&mut grid_view, &grid_entries);

        drop(config);
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
                        gen_id_check: Some(image_states.gen_id.get_next_check()),
                        signal_cache_readies: image_states.clone_cache_readies_on_should_scale(),
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
            image_dirs,
            base_image_kind: BaseImageKind::Startup,
            grid_cell_extent: grid_cell_size.into(),
            details_cell_extent: details_cell_size.into(),
            grid_ship: resident_grid_sx.clone(),
            details_ship: charon_ship.clone(),
            ferry_image_infos,
            error_sx: error_sx.clone()
        };
        ferry_images(ferry_images_info);

        drop(resident_grid_sx);
        for res in resident_grid_rx.iter() {
            match res {
                Ok(image_info) => {
                    let ImageInfo { image, index, metadata, .. } = image_info;

                    if let Some(metadata) = metadata {
                        let grid_entry_info = &mut grid_entries[index];
                        grid_entry_info.metadata = Some(Arc::new(metadata));
                    }

                    let (tex, tex_id) = alloc_write_texture(wgpu, &image);
                    to_thanatos.send(Soul::RgbaImage(image)).unwrap();

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
            refresh_rate,
            thread_pool,
            image_dirs,
            images,
            deferred_metadata_sx,
            deferred_metadata_rx,
            cache_path,
            cache,
            selected_library_entries: default!(),
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
            grid_cell_strokes: default!(),
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
            scaler,
            iris_ship,
            from_iris,
            charon_ship,
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

    fn assess_partial_tex_ready(&mut self, partial_tex: PartialTex) {
        let PartialTex { tex, tex_id, stage, index, gen_id_check, .. } = partial_tex;
        let extent = tex.as_();

        if let Some(gen_id_check) = gen_id_check.as_ref() && gen_id_check.check().is_err() {
            self.to_thanatos.send(Soul::TexId(tex_id)).unwrap();

            return
        }

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
            Stage::Manga => {
                let make_iris_info = || IrisInfo {
                    wgpu: self.wgpu.clone(),
                    tex,
                    tex_id,
                    index,
                    gen_id_check,
                    dst_extent: self.manga.view[index].extent,
                    scaler: self.scaler.clone(),
                    ship: self.iris_ship.clone(),
                    to_thanatos: self.to_thanatos.clone()
                };

                match self.manga.filter {
                    FilterAccel::Gpu(FilterKind::Nearest) if self.manga.scale_pc != 100. => {
                        let iris_info = make_iris_info();
                        self.thread_pool.enqueue_high(|| iris_sampler(iris_info, wgpu::FilterMode::Nearest));
                    },
                    FilterAccel::Gpu(FilterKind::Bilinear) if self.manga.scale_pc != 100. => {
                        let iris_info = make_iris_info();
                        self.thread_pool.enqueue_high(|| iris_sampler(iris_info, wgpu::FilterMode::Linear));
                    },
                    FilterAccel::Gpu(FilterKind::Blackman) if self.manga.scale_pc != 100. => {
                        let iris_info = make_iris_info();
                        self.thread_pool.enqueue_high(|| iris_blackman(iris_info));
                    },
                    _ => self.manga.view[index].image_state = ImageStateManga::Ready { tex_id, extent }
                }
            }
        }
    }

    fn get_image_states_mut(&mut self, image_i: Option<usize>) -> Option<&mut ImageStates> {
        get_image_states_mut(&mut self.images, image_i)
    }

    fn get_image_states_from_grid_entry_mut(&mut self, grid_entry_i: usize) -> Option<&mut ImageStates> {
        get_image_states_from_grid_entry_mut(&mut self.images, &self.grid_entries, grid_entry_i)
    }

    fn get_pivot(&self) -> Pivot {
        let viewport_offset = self.manga.scroll_offset.y;
        let viewport_half_height = self.central_rect.height().div_euclid(2.);
        let pivot_offset = viewport_offset + viewport_half_height;

        let visible_page_i = self.manga.view[self.manga.visible_view.clone()]
            .partition_point(|page_info| page_info.offset < pivot_offset)
            .saturating_sub(1);
        let page_i = self.manga.visible_view.start + visible_page_i;
        let page_info = &self.manga.view[page_i];
        let page_inset_px = pivot_offset - page_info.offset;
        let page_inset_pc = page_inset_px / page_info.extent.height;

        Pivot {
            page_i,
            page_inset_pc
        }
    }

    fn init_grid_entries(&mut self) {
        init_grid_entries(&mut self.grid_entries, &mut self.cache, &mut self.images, &mut self.missing_base_images, self.grid_cell_size, self.details_cell_size);
    }

    #[hotpath::measure]
    fn init_manga(&mut self, ui: &mut egui::Ui, selected_details_dir_entry_i: usize) -> Res1<()> { //$ Slow
        let dir_entry_info = &self.details_dir_entries[selected_details_dir_entry_i];
        let archive_path = Arc::new(dir_entry_info.path.clone());
        let archive = fs::File::open(archive_path.as_path())?;
        let archive = io::BufReader::new(archive);
        let mut archive = zip::ZipArchive::new(archive)?;

        self.manga.archive_pages.reserve_exact(archive.len());
        self.manga.view.reserve_exact(archive.len());

        for archive_i in 0..archive.len() {
            let page = archive.by_index(archive_i)?;
            let name = page.name().to_string();
            let mut page = io::BufReader::new(page);

            let ext = Path::new(name.as_str()).get_file_ext()?;
            let (image_kind, width, height) = match ext.to_ascii_lowercase().as_str() {
                "jpg" | "jpeg" => {
                    let mut decoder = jpeg_decoder::Decoder::new(page); // jpeg_decoder is used as it doesn't require Seek
                    decoder.read_info()?;
                    let decoder_info = decoder.info().unwrap();

                    (ImageKind::Jpeg, decoder_info.width as f32, decoder_info.height as f32)
                },
                "png" => {
                    let mut buf = [0_u8; 24];
                    page.read_exact(&mut buf)?;

                    let width_slice = buf.get(16..20).unwrap();
                    let height_slice = buf.get(20..24).unwrap();
                    let width = u32::from_be_bytes(width_slice.try_into().unwrap());
                    let height = u32::from_be_bytes(height_slice.try_into().unwrap());

                    (ImageKind::Png, width as f32, height as f32)
                },
                "webp" => {
                    let mut buf = [0_u8; zenwebp::ImageInfo::PROBE_BYTES];
                    page.read_exact(&mut buf)?;

                    let info = zenwebp::ImageInfo::from_bytes(&buf).map_err(|err| err.decompose().0)?;

                    (ImageKind::Webp, info.width as f32, info.height as f32)
                },
                _ => return Err(ErrVar::InvalidImageFormat { archive_i, name }.into())
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
                image_state: default!(),
                gen_id: default!()
            };
            self.manga.view.push(view_page_info);

            view_page_offset += archive_page_info.extent.height;
        }
        self.manga.view_extent = [self.manga.archive_pages_width, view_page_offset].into();

        let visible_page_count = self.init_residence_manga();

        let ferry_image_infos = (0..visible_page_count)
            .map(|view_i| {
                let ViewPageInfo { archive_i, image_kind, ref gen_id, .. } = self.manga.view[view_i];

                FerryImageInfoManga {
                    archive_i,
                    image_kind,
                    view_i,
                    scale: None,
                    gen_id_check: gen_id.get_next_check(),
                    signal_tex_ready: Some(self.poll_ready.clone())
                }
            })
            .collect::<Vec<_>>();
        let ferry_images_info = FerryImagesInfoManga {
            ctx: ui.ctx(),
            thread_pool: &self.thread_pool,
            archive_path: archive_path.clone(),
            ship: self.charon_ship.clone(),
            ferry_image_infos,
            error_sx: self.error_sx.clone()
        };
        ferry_images_manga(ferry_images_info);

        let ferry_image_infos = (visible_page_count..self.manga.residence.end)
            .map(|view_i| {
                let ViewPageInfo { archive_i, image_kind, ref gen_id, .. } = self.manga.view[view_i];

                FerryImageInfoManga {
                    archive_i,
                    image_kind,
                    view_i,
                    scale: None,
                    gen_id_check: gen_id.get_next_check(),
                    signal_tex_ready: None
                }
            })
            .collect::<Vec<_>>();
        let ferry_images_info = FerryImagesInfoManga {
            ctx: ui.ctx(),
            thread_pool: &self.thread_pool,
            archive_path: archive_path.clone(),
            ship: self.charon_ship.clone(),
            ferry_image_infos,
            error_sx: self.error_sx.clone()
        };
        ferry_images_manga(ferry_images_info);

        self.manga.archive.set(archive);
        self.manga.archive_path.set(archive_path);
        self.manga.to_thanatos.set(self.to_thanatos.clone());

        self.view_kind = ViewKind::WaitManga;

        Ok(())
    }

    fn wait_manga(&mut self) {
        self.stream_textures_stepped();

        if self.poll_ready.is_ready() {
            self.animation.target = false;
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
        self.grid_scroll_offset = 0.;
        self.animation.target = false;
        self.active_tag = None;

        GridViewCellCounts { row: row_cell_count, max: self.grid_view.len() }
    }

    fn reset_residence(&mut self) -> ResetResidence {
        let max_cell_count = self.grid_view.len();

        let available_row_cell_count = (self.central_rect.width() - self.grid_cell_size.x).div(self.grid_cell_space.x).ceil() as usize;
            // ui.available_width() - (self.grid_cell_size.x * avail_row_cell_count - GRID_IMAGE_SPACING.x) <= self.grid_cell_size.x
        let available_col_cell_count = self.central_rect.height().div(self.grid_cell_space.y).ceil() as usize;
        let visible_cell_count = (available_row_cell_count * available_col_cell_count).min(max_cell_count);
        let row_cell_count = available_row_cell_count.min(max_cell_count);

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
                        let cache_readies = image_states.take_cache_readies();
                        let gen_id = mem::take(&mut image_states.gen_id);
                        gen_id.fetch_add(1, Ordering::Relaxed);

                        let new_image_states = ImageStates {
                            grid: ImageState::NoneCheckCache { cache_ready: cache_readies.grid },
                            details: ImageState::NoneCheckCache { cache_ready: cache_readies.details },
                            ref_count: 0,
                            gen_id
                        };

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
                            gen_id_check: Some(image_states.gen_id.get_next_check()),
                            signal_cache_readies: image_states.clone_cache_readies_on_should_scale(),
                            wait_cache_readies: image_states.take_cache_readies_on_not_should_scale(),
                            signal_tex_ready: signal_tex_ready.then(|| self.poll_ready.clone())
                        })
                    }

                    None
                })
                .collect::<Vec<_>>();

            FerryImagesInfo {
                ctx,
                thread_pool: &self.thread_pool,
                image_dirs: self.image_dirs,
                base_image_kind: BaseImageKind::Startup,
                grid_cell_extent: self.grid_cell_size.into(),
                details_cell_extent: self.details_cell_size.into(),
                grid_ship: self.charon_ship.clone(),
                details_ship: self.charon_ship.clone(),
                ferry_image_infos,
                error_sx: self.error_sx.clone()
            }
        };

        ferry_images(make_ferry_images_info(&self.stream.load_first, true));
        ferry_images(make_ferry_images_info(&self.stream.load_after, false));
    }

    fn stream_manga(&mut self, ctx: &egui::Context, should_scale: bool) {
        if !self.manga.stream.drop.is_empty() {
            for view_i in self.manga.stream.drop.iter() {
                let page_info = &mut self.manga.view[*view_i];
                page_info.gen_id.fetch_add(1, Ordering::Relaxed);
                let old_image_state = mem::take(&mut page_info.image_state);
                self.to_thanatos.send(Soul::ImageState(old_image_state.into())).unwrap();
            }
        }

        let make_ferry_images_info = |load: &HashSet<usize>, signal_tex_ready: bool| -> FerryImagesInfoManga {
            let ferry_image_infos = load.iter().copied()
                .map(|view_i| {
                    let ViewPageInfo { archive_i, image_kind, extent, ref gen_id, .. } = self.manga.view[view_i];

                    let scale = if should_scale && let FilterAccel::Cpu(filter) = self.manga.filter {
                        Some(ScaleImageManga { extent, filter })
                    } else {
                        None
                    };

                    FerryImageInfoManga {
                        archive_i,
                        image_kind,
                        view_i,
                        scale,
                        gen_id_check: gen_id.get_next_check(),
                        signal_tex_ready: signal_tex_ready.then(|| self.poll_ready.clone())
                    }
                })
                .collect::<Vec<_>>();

            FerryImagesInfoManga {
                ctx,
                thread_pool: &self.thread_pool,
                archive_path: self.manga.archive_path.clone(),
                ship: self.charon_ship.clone(),
                ferry_image_infos,
                error_sx: self.error_sx.clone()
            }
        };

        ferry_images_manga(make_ferry_images_info(&self.manga.stream.load_first, true));
        ferry_images_manga(make_ferry_images_info(&self.manga.stream.load_after, false));
    }

    #[hotpath::measure]
    fn stream_textures_stepped(&mut self) {
        while let Ok(scaled_tex) = self.from_iris.try_recv() {
            let ScaledTexManga { tex_id, index, gen_id_check, extent } = scaled_tex;

            if let Some(gen_id_check) = gen_id_check && gen_id_check.check().is_ok() {
                self.manga.view[index].image_state = ImageStateManga::Ready { tex_id, extent };
            } else {
                self.to_thanatos.send(Soul::TexId(tex_id)).unwrap();
            }
        }

        let mut sentinel = self.chunk_size;
        sentinel -= self.try_write_partial_tex(sentinel);

        while let Ok(mut partial_tex) = self.from_demeter.try_recv() {
            if partial_tex.offset == partial_tex.tex.height() as usize {
                if let Some((image, signal_tex_ready)) = partial_tex.captive.take() {
                    if let Some(signal_tex_ready) = signal_tex_ready {
                        signal_tex_ready.mark_done();
                    }
                    self.to_thanatos.send(Soul::RgbaImage(image)).unwrap();
                }

                self.assess_partial_tex_ready(partial_tex);
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
                Ok(mut image_info) => {
                    if let Some(metadata) = image_info.metadata.take() {
                        let grid_entry_info = &mut self.grid_entries[image_info.index];
                        grid_entry_info.metadata = Some(Arc::new(metadata));
                    }

                    let image_size = image_info.image.as_raw().len();
                    if image_size <= sentinel {
                        // Writable now
                        self.to_demeter.send(image_info).unwrap();
                        sentinel -= image_size;

                        continue
                    }
                    if image_size > self.chunk_size {
                        // Clearable now
                        self.to_demeter.send(image_info).unwrap();

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
            let PartialTex { row_size, chunk_row_count, .. } = *partial_tex;

            let tex_height = tex.height() as usize;
            let remaining_row_count = tex_height.saturating_sub(*offset);
            let sentinel_row_count = sentinel / row_size;
            let write_row_count = chunk_row_count.min(sentinel_row_count).min(remaining_row_count);
            let last_write = write_row_count == remaining_row_count;
            let write_size = write_row_count * row_size;
            let write_tex = WriteTex { tex: tex.clone(), captive: captive.take(), offset: *offset, row_count: write_row_count, last_write };

            self.to_hephaestus.send(write_tex).unwrap();

            if last_write {
                let partial_tex = self.partial_tex_stash.pop_front().unwrap();
                self.assess_partial_tex_ready(partial_tex);
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

        let background_builder = egui::UiBuilder::new().sense(egui::Sense::click());
        let background_resp = ui.scope_builder(background_builder, |ui| self.grid_view(ui)).response;

        if background_resp.clicked() || background_resp.secondary_clicked() {
            self.reset_grid_entries_selection();
        }

        let close_behaviour = match background_resp.clicked_elsewhere() {
            true => egui::PopupCloseBehavior::CloseOnClickOutside,
            false => egui::PopupCloseBehavior::IgnoreClicks
        };

        egui::Popup::context_menu(&background_resp)
            .close_behavior(close_behaviour)
            .show(|ui| {
                self.grid_cell_scroll_submenu(ui);
                self.grid_cell_library_submenu(ui);
            });
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

        let opacity = get_animation_opacity(ui, &mut self.animation);
        ui.set_opacity(opacity);

        if let Some(viewport) = self.manga.flagged_scale.take() {
            let scale = self.manga.scale_pc / 100.;
            let pivot = self.get_pivot();

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

                self.manga.view.push(ViewPageInfo {
                    image_kind: page.image_kind,
                    archive_i: page.index,
                    offset: page_scaled_offset,
                    extent,
                    image_state: default!(),
                    gen_id: default!()
                });

                page_scaled_offset += height_scaled;
            }
            let view_width = self.manga.archive_pages_width.mul(scale).round()
                .div(2.).ceil().mul(2.); // Avoid subpixel alignment
            self.manga.view_extent = [view_width, page_scaled_offset].into();

            let viewport_offset = viewport.top();
            let viewport_half_height = viewport.height().div_euclid(2.);
            let pivot_page_info = &self.manga.view[pivot.page_i];
            let new_viewport_offset = pivot_page_info.offset +
                pivot.page_inset_pc.mul(pivot_page_info.extent.height) -
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
            self.stream_manga(ui, self.manga.scale_pc != 100. && matches!(self.manga.filter, FilterAccel::Cpu(_)));
        }

        if let Some(scroll_offset_y) = self.manga.go_to_scroll_offset_y.take() {
            self.manga.scroll_offset.y = scroll_offset_y;
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
                        self.manga.spring_damper.step(ui, self.refresh_rate);

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
                            false => self.manga.spring_damper.step(ui, self.refresh_rate)
                        }

                        (ScrollSource::DRAG, self.manga.spring_damper.multiplier)
                    }
                };
                self.manga.scroll_offset_y_anchor.get_or_insert(self.manga.scroll_offset.y.round());
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

        match new_scroll_offset == self.manga.scroll_offset {
            true =>
                self.manga.scroll_offset = new_scroll_offset.round(),
            false => {
                self.manga.scroll_offset = new_scroll_offset;

                egui::Popup::close_all(ui)
            }
        }
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

            if self.update_residence_manga(self.manga.visible_view.clone(), self.manga.view.len()) {
                self.stream_manga(ui, self.manga.scale_pc != 100. && matches!(self.manga.filter, FilterAccel::Cpu(_)));
            }

            if self.poll_ready.is_ready() {
                ui.add_space(self.manga.view[start_visible].offset);

                for view_i in self.manga.visible_view.clone() {
                    let page_extent = self.manga.view[view_i].extent;
                    let (page_rect, _) = ui.allocate_exact_size([ui.min_size().x, page_extent.height].into(), egui::Sense::hover());

                    let image_resp = try_add_image_manga(ui, &mut self.manga.view[view_i].image_state, page_rect, self.manga.tint);

                    if let Some(image_resp) = image_resp {
                        self.context_menu_manga(viewport, &image_resp);
                    }
                }
            }

            // let info = &[
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
                ui.menu_button("Tint", |ui| self.tint_submenu_manga(ui));
                ui.separator();
                ui.menu_button("Bookmark", |ui| self.bookmark_submenu_manga(ui));
                ui.menu_button("Scroll", |ui| self.scroll_submenu_common(ui, Stage::Manga));
            });
    }

    fn scale_submenu_manga(&mut self, ui: &mut egui::Ui, viewport: egui::Rect) {
        const SCALE_MIN: f32 = 50.;
        const SCALE_MAX: f32 = 300.;

        ui.scope(|ui| {
            ui.spacing_mut().slider_width = 180.;

            let scale_slider_resp = ui.add(egui::Slider::new(&mut self.manga.scale_pc, SCALE_MIN..=SCALE_MAX)
                .clamping(egui::SliderClamping::Always)
                .fixed_decimals(0)
                .step_by(5.));

            if scale_slider_resp.drag_started() {
                self.manga.scale_drag_anchor = self.manga.scale_pc;
            }
            if scale_slider_resp.drag_stopped() && self.manga.scale_drag_anchor != self.manga.scale_pc {
                self.manga.flagged_scale = Some(viewport);
            }
        });

        ui.separator();

        ui.with_layout(egui::Layout::top_down_justified(egui::Align::Center), |ui| {
            if ui.button("50").clicked() {
                self.manga.flag_scale(ui, SCALE_MIN, viewport);
            }
            if ui.button("75").clicked() {
                self.manga.flag_scale(ui, 75., viewport);
            }
            if ui.button("100").clicked() {
                self.manga.flag_scale(ui, 100., viewport);
            }
            if ui.button("125").clicked() {
                self.manga.flag_scale(ui, 125., viewport);
            }
            if ui.button("150").clicked() {
                self.manga.flag_scale(ui, 150., viewport);
            }
            if ui.button("175").clicked() {
                self.manga.flag_scale(ui, 175., viewport);
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
        egui::Grid::new("grid")
            .num_columns(2)
            .show(ui, |ui| {
                ui.vertical(|ui| {
                    ui.set_min_width(SUBMENU_MIN_WIDTH);

                    ui.label("CPU:");

                    if ui.radio(matches!(self.manga.filter, FilterAccel::Cpu(fir::FilterType::Box)), "Box").clicked() {
                        self.manga.filter = FilterAccel::Cpu(fir::FilterType::Box);
                    }
                    if ui.radio(matches!(self.manga.filter, FilterAccel::Cpu(fir::FilterType::Bilinear)), "Bilinear").clicked() {
                        self.manga.filter = FilterAccel::Cpu(fir::FilterType::Bilinear);
                    }
                    if ui.radio(matches!(self.manga.filter, FilterAccel::Cpu(fir::FilterType::Custom(_))), "Blackman 3").clicked() {
                        self.manga.filter = FilterAccel::Cpu(fir::FilterType::Custom(blackman_filter_fir()));
                    }
                    if ui.radio(matches!(self.manga.filter, FilterAccel::Cpu(fir::FilterType::CatmullRom)), "Catmull-Rom").clicked() {
                        self.manga.filter = FilterAccel::Cpu(fir::FilterType::CatmullRom);
                    }
                    if ui.radio(matches!(self.manga.filter, FilterAccel::Cpu(fir::FilterType::Gaussian)), "Gaussian").clicked() {
                        self.manga.filter = FilterAccel::Cpu(fir::FilterType::Gaussian);
                    }
                    if ui.radio(matches!(self.manga.filter, FilterAccel::Cpu(fir::FilterType::Hamming)), "Hamming").clicked() {
                        self.manga.filter = FilterAccel::Cpu(fir::FilterType::Hamming);
                    }
                    if ui.radio(matches!(self.manga.filter, FilterAccel::Cpu(fir::FilterType::Lanczos3)), "Lanczos 3").clicked() {
                        self.manga.filter = FilterAccel::Cpu(fir::FilterType::Lanczos3);
                    }
                    if ui.radio(matches!(self.manga.filter, FilterAccel::Cpu(fir::FilterType::Mitchell)), "Mitchell").clicked() {
                        self.manga.filter = FilterAccel::Cpu(fir::FilterType::Mitchell);
                    }
                });

                ui.vertical(|ui| {
                    ui.set_min_width(SUBMENU_MIN_WIDTH);

                    ui.label("GPU:");

                    if ui.radio(matches!(self.manga.filter, FilterAccel::Gpu(FilterKind::Nearest)), "Nearest").clicked() {
                        self.manga.filter = FilterAccel::Gpu(FilterKind::Nearest);
                    }

                    if ui.radio(matches!(self.manga.filter, FilterAccel::Gpu(FilterKind::Bilinear)), "Bilinear").clicked() {
                        self.manga.filter = FilterAccel::Gpu(FilterKind::Bilinear);
                    }

                    if ui.radio(matches!(self.manga.filter, FilterAccel::Gpu(FilterKind::Blackman)), "Blackman 3").clicked() {
                        self.manga.filter = FilterAccel::Gpu(FilterKind::Blackman);
                    }
                });

                ui.end_row();
            });
    }

    fn tint_submenu_manga(&mut self, ui: &mut egui::Ui) {
        ui.vertical(|ui| {
            const SEPIA: egui::Rgba = egui::Rgba::from_rgb(1., 0.6, 0.3);
            const WHITE: egui::Rgba = egui::Rgba::WHITE;

            ui.scope(|ui| {
                ui.spacing_mut().slider_width = 200.;

                ui.label("Sepia:");

                let sepia_alpha_slider_resp = ui.add(egui::Slider::new(&mut self.manga.sepia_alpha_pc, 0.0..=100.)
                    .clamping(egui::SliderClamping::Always)
                    .fixed_decimals(0));

                ui.label("White:");

                let white_level_slider_resp = ui.add(egui::Slider::new(&mut self.manga.white_level_pc, 0.0..=100.)
                    .clamping(egui::SliderClamping::Always)
                    .fixed_decimals(0));

                if sepia_alpha_slider_resp.union(white_level_slider_resp).dragged() {
                    let sepia_alpha = self.manga.sepia_alpha_pc / 100.;
                    let mut tint = WHITE.blend(SEPIA.multiply(sepia_alpha));

                    let white_level = (self.manga.white_level_pc / 100.).powf(2.2);
                    tint[0] *= white_level;
                    tint[1] *= white_level;
                    tint[2] *= white_level;

                    self.manga.tint = tint;
                }
            });
        });
    }

    fn bookmark_submenu_manga(&mut self, ui: &mut egui::Ui) {
        ui.set_min_width(SUBMENU_MIN_WIDTH);

        if ui.button("Set").clicked() {
            let pivot = self.get_pivot();

            let grid_entry_info = &mut self.grid_entries[self.details_grid_entry_i];
            grid_entry_info.bookmark = Some(pivot);
        }

        let bookmark = &mut self.grid_entries[self.details_grid_entry_i].bookmark;

        if bookmark.is_some() {
            ui.separator();

            if ui.button("Clear").clicked() {
                *bookmark = None;
            }
        }

        if let Some(pivot) = bookmark && ui.button("Jump").clicked() {
            let viewport_half_height = self.central_rect.height().div_euclid(2.);

            let page_info = &self.manga.view[pivot.page_i];
            let scroll_offset_y = page_info.offset +
                pivot.page_inset_pc.mul(page_info.extent.height) -
                viewport_half_height;

            self.manga.go_to_scroll_offset_y = Some(scroll_offset_y);
        }
    }

    fn scroll_submenu_common(&mut self, ui: &mut egui::Ui, stage: Stage) {
        ui.set_min_width(SUBMENU_MIN_WIDTH);

        let (scroll_kind, spring_damper) = match stage {
            Stage::Grid | Stage::Details => (&mut self.scroll_kind, &mut self.spring_damper),
            Stage::Manga => (&mut self.manga.scroll_kind, &mut self.manga.spring_damper)
        };

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
                            self.scroll_multiplier = multiplier.max(0.1);
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
                            spring_damper.multiplier = multiplier.max(0.1);
                        }
                        ui.end_row();

                        ui.label("Stiffness:");
                        let stiffness_edit_resp = egui::TextEdit::singleline(&mut spring_damper.stiffness_edit)
                            .hint_text(&spring_damper.stiffness_display)
                            .show(ui)
                            .response;
                        if stiffness_edit_resp.lost_focus() && let Ok(omega) = spring_damper.stiffness_edit.parse::<f32>() {
                            spring_damper.stiffness_edit.clear();
                            spring_damper.update_stiffness(omega.max(0.1));
                        }
                        ui.end_row();

                        ui.label("Bounce:");
                        let bounce_edit_resp = egui::TextEdit::singleline(&mut spring_damper.bounce_edit)
                            .hint_text(&spring_damper.bounce_display)
                            .show(ui)
                            .response;
                        if bounce_edit_resp.lost_focus() && let Ok(bounce) = spring_damper.bounce_edit.parse::<f32>() {
                            spring_damper.bounce_edit.clear();
                            spring_damper.update_bounce(bounce.max(0.1));
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
                    self.grid_scroll_offset = 0.;
                    self.active_tag = Some(tag.clone());
                    self.animation.target = false;
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

        if self.grid_view.is_empty() {
            let (_, background_rect) = ui.allocate_space(ui.available_size());

            let msg = "Library is currently empty - right-click to add directories.";
            let font_id = ui.style().text_styles.get(&egui::TextStyle::Body).unwrap().clone();
            let text_color = ui.visuals().text_color();
            let galley = ui.painter().layout_no_wrap(msg.to_string(), font_id, text_color);

            let galley_pos = egui::Align2::CENTER_CENTER.align_size_within_rect(galley.size(), background_rect).min;
            ui.painter().galley(galley_pos, galley, text_color);

            return
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
        let max_scroll_offset = scroll_area_height.sub(self.central_rect.height()).max(0.);

        ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
            ui.spacing_mut().item_spacing = GRID_IMAGE_SPACING;

            let (scroll_source, scroll_offset) = match self.scroll_kind {
                ScrollKind::EaseInOut => {
                    (
                        ScrollSource::SCROLL_BAR | ScrollSource::MOUSE_WHEEL,
                        self.grid_scroll_offset
                    )
                },
                ScrollKind::SpringDamper => {
                    self.spring_damper.step(ui, self.refresh_rate);
                    let scroll_offset = self.grid_scroll_offset + self.spring_damper.delta;
                    let scroll_offset = scroll_offset.clamp(0., max_scroll_offset);

                    (
                        ScrollSource::SCROLL_BAR,
                        scroll_offset
                    )
                }
            };

            let new_grid_scroll_offset = egui::ScrollArea::new([false, true])
                .auto_shrink(false)
                .scroll_source(scroll_source)
                .wheel_scroll_multiplier([1.0, self.scroll_multiplier].into())
                .vertical_scroll_offset(scroll_offset)
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
                })
                .state.offset.y;

            let clip_stroke_rect = (
                new_grid_scroll_offset > 0. &&
                new_grid_scroll_offset < max_scroll_offset
            )
            .then_some(self.central_rect);
            for rect in self.grid_cell_strokes.drain(..) {
                stroke_rect(ui, rect, clip_stroke_rect);
            }

            match new_grid_scroll_offset == self.grid_scroll_offset {
                true =>
                    self.grid_scroll_offset = new_grid_scroll_offset.round(),
                false => {
                    self.grid_scroll_offset = new_grid_scroll_offset;

                    egui::Popup::close_all(ui)
                }
            }
        });
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
                try_add_image(ui, grid_state, &grid_entry_info.stem, &self.poll_ready, Some(&mut self.animation)),
            None => alloc_painted_text(ui, &grid_entry_info.stem)
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
            ui.rect_contains_pointer(cell_resp.rect) && self.grid_entries_selection.is_empty() // Hover
        {
            self.grid_cell_strokes.push(cell_resp.rect)
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
                ui.separator();
                self.grid_cell_scroll_submenu(ui);
                self.grid_cell_library_submenu(ui);
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
            gen_id_check: None,
            signal_cache_readies: cache_readies,
            wait_cache_readies: CacheReadies::NONE,
            signal_tex_ready: None
        };
        let ferry_images_info = FerryImagesInfo {
            ctx,
            thread_pool: &self.thread_pool,
            image_dirs: self.image_dirs,
            base_image_kind: BaseImageKind::Pick { path: path.clone() },
            grid_cell_extent: self.grid_cell_size.into(),
            details_cell_extent: self.details_cell_size.into(),
            grid_ship: self.charon_ship.clone(),
            details_ship: self.charon_ship.clone(),
            ferry_image_infos: vec![ferry_image_info],
            error_sx: self.error_sx.clone()
        };
        ferry_images(ferry_images_info);

        let base_image_path = self.image_dirs.base.join(new_image_file_name.as_ref());
        let grid_entry_i = self.grid_entry_i;
        let deferred_metadata_sx = self.deferred_metadata_sx.clone();
        let image_dirs = self.image_dirs;
        self.thread_pool.enqueue_high(move || {
            (|| -> Res<_> {
                fs::copy(&path, &base_image_path)?;

                let base_image_file = File::open(base_image_path.as_path())?;
                let metadata = base_image_file.metadata()?;
                let metadata = Arc::new(Metadata {
                    created: metadata.created()?,
                    modified: metadata.modified()?,
                    len: metadata.len()
                });
                deferred_metadata_sx.send(MetadataInfo { grid_entry_i, metadata }).unwrap();

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

    fn grid_cell_library_submenu(&mut self, ui: &mut egui::Ui) {
        ui.menu_button("Library", |ui| {
            ui.set_min_width(SUBMENU_MIN_WIDTH);

            let new_button_resp = ui.button("New");
            let remove_button_resp = ui.add_enabled(!self.selected_library_entries.is_empty(), egui::Button::new("Remove"));

            if !self.cache.library.is_empty() {
                ui.separator();

                for (i, dir) in self.cache.library.iter().enumerate() {
                    let mut checked = self.selected_library_entries.contains(&i);
                    let dir_label_resp = ui.checkbox(&mut checked, dir.to_string_lossy());

                    if dir_label_resp.clicked() {
                        match checked {
                            true => self.selected_library_entries.insert(i),
                            false => self.selected_library_entries.remove(&i)
                        };
                    }
                }
            }

            if new_button_resp.clicked() {
                let dirs = rfd::FileDialog::new().pick_folders();

                if let Some(dirs) = dirs {
                    for dir in dirs {
                        self.cache.library.insert(dir);
                    }

                    self.view_kind = ViewKind::Restart;
                }
            }

            if remove_button_resp.clicked() {
                let mut i = 0;
                self.cache.library.retain(|_| {
                    let retain = !self.selected_library_entries.contains(&i);
                    i +=1 ;

                    retain
                });
                self.selected_library_entries.clear();

                self.view_kind = ViewKind::Restart;
            }
        });
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
                Some(details_state) => try_add_image(ui, details_state, dir_name, &self.poll_ready, None),
                None => alloc_painted_text(ui, dir_name)
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
    wgpu_setup_create_new.device_descriptor = Arc::new(|_| {
        wgpu::DeviceDescriptor {
            required_limits: wgpu::Limits {
                max_texture_dimension_2d: 8192,
                max_immediate_size: 32,
                ..default!()
            },
            required_features: wgpu::Features::IMMEDIATES,
            ..default!()
        }
    });
    let wgpu_setup = egui_wgpu::WgpuSetup::CreateNew(wgpu_setup_create_new);
    let wgpu_options = egui_wgpu::WgpuConfiguration {
        surface: egui_wgpu::SurfaceConfig {
            present_mode: wgpu::PresentMode::Fifo,
            desired_maximum_frame_latency: Some(1)
        },
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
                // style.visuals.handle_shape = egui::style::HandleShape::Circle;

                for font_id in style.text_styles.values_mut() {
                    font_id.size = (font_id.size * factor).round();
                }
            });

            let app: Box<dyn eframe::App> = match kind {
                GuiKind::Info { msg } => {
                    cctx.egui_ctx.global_style_mut(|style| style.wrap_mode = Some(egui::TextWrapMode::Wrap));

                    Box::new(Info::new(msg))
                },
                GuiKind::MediaBrowser => {
                    unsafe {
                        let thread_hnd = GetCurrentThread();
                        SetThreadPriority(thread_hnd, THREAD_PRIORITY_ABOVE_NORMAL)?;
                    }

                    let refresh_rate = window.current_monitor().unwrap().refresh_rate_millihertz().unwrap() / 1000;
                    let win_inner_size = window.inner_size();
                    let win_inner_size = Extent2dU::new(win_inner_size.width, win_inner_size.height);

                    Box::new(MediaBrowser::new(&cctx.egui_ctx, cctx.wgpu_render_state.as_ref().unwrap(), refresh_rate, win_inner_size)?)
                }
            };

            Ok(app)
        })
    )?;

    Ok(())
}
