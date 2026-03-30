use ogos_common::*;
use ogos_core::*;
use ogos_err::*;
use ogos_mki::*;

use const_format::*;
use discord_rich_presence::activity as drpa;
use once_cell::sync::*;
// use log::*;
use serde::{
    de::*,
    *
};
use std::{
    collections::*,
    fmt,
    fs,
    ops::*,
    sync::*,
    time::*
};

pub const CONFIG_FILE_NAME: &str = "config.ron";

pub static CONFIG: OnceCell<RwLock<Config>> = OnceCell::new();

macro_rules! impl_name {
    ($name:ident, $lt:lifetime) => {
        impl<$lt> $name<$lt> {
            pub const NAME: &'static str = map_ascii_case!(Case::Snake, stringify!($name));
        }
    };
    ($name:ident) => {
        impl $name {
            pub const NAME: &str = map_ascii_case!(Case::Snake, stringify!($name));
        }
    };
}

fn deserialize_key<'de, D>(deserializer: D) -> Result<Key, D::Error> where
    D: Deserializer<'de>
{
    BindVar::deserialize(deserializer)?.try_as_key().map_err(D::Error::custom)
}

fn deserialize_hotkey_prefix<'de, D>(deserializer: D) -> Result<Vec<Key>, D::Error> where
    D: Deserializer<'de>
{
    struct KeysVisitor;

    impl<'de> Visitor<'de> for KeysVisitor {
        type Value = Vec<Key>;

        fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
            formatter.write_str("a sequence of keys")
        }

        fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error> where
            A: SeqAccess<'de>
        {
            let mut keys = Vec::new();
            while let Some(key) = seq.next_element::<BindVar>()? {
                keys.push(key.try_as_hotkey_prefix().map_err(A::Error::custom)?);
            };

            if keys.is_empty() {
                Err(A::Error::custom(ErrVar::MissingHotkeyPrefix))?;
            }

            Ok(keys)
        }
    }

    let keys = deserializer.deserialize_seq(KeysVisitor)?;

    Ok(keys)
}

fn deserialize_tasks<'de, D>(deserializer: D) -> Result<HashMap<Key, Task>, D::Error> where
    D: Deserializer<'de>
{
    struct TasksVisitor;

    impl<'de> Visitor<'de> for TasksVisitor {
        type Value = HashMap<Key, Task>;

        fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
            formatter.write_str("a map of tasks")
        }

        fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error> where
            A: serde::de::MapAccess<'de>
        {
            let mut tasks: HashMap<Key, Task> = HashMap::with_capacity(map.size_hint().unwrap_or_default());

            while let Some((key, task)) = map.next_entry::<BindVar, Task>()? {
                let key = key.try_as_key().map_err(A::Error::custom)?;

                tasks.insert(key, task);
            }

            Ok(tasks)
        }
    }

    let tasks = deserializer.deserialize_map(TasksVisitor)?;

    Ok(tasks)
}

fn make_input_event_map(from: BindVar, to: BindVar, click_dur_ms: Option<u64>) -> ResVar<InputEventMap> {
    let from = from.try_as_input_event()?;
    let to = to.try_as_input_event()?;

    Ok(match (from, to, click_dur_ms) {
        (InputEvent::MouseWheel(_), InputEvent::Keyboard(_), Some(click_dur_ms)) |
        (InputEvent::MouseWheel(_), InputEvent::MouseButton(_), Some(click_dur_ms)) => {
            InputEventMap::WheelClick { from, to, dur: Duration::from_millis(click_dur_ms) }
        },
        (InputEvent::Keyboard(_), InputEvent::Keyboard(_), _) |
        (InputEvent::Keyboard(_), InputEvent::MouseButton(_), _) |
        (InputEvent::MouseButton(_), InputEvent::Keyboard(_), _) |
        (InputEvent::MouseButton(_), InputEvent::MouseButton(_), _) => {
            InputEventMap::PressMirror { from, to }
        },
        _ => Err(ErrVar::InvalidInputEventMap { from, to })?
    })
}

fn deserialize_click_map<'de, D>(deserializer: D) -> Result<InputEventMap, D::Error> where
    D: Deserializer<'de>
{
    struct ClickMapVisitor;

    impl<'de> Visitor<'de> for ClickMapVisitor {
        type Value = InputEventMap;

        fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
            formatter.write_str("a click map")
        }

        fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error> where
            A: serde::de::MapAccess<'de>
        {
            let mut dur = None;
            let (mut from, mut to) = (None, None);

            for _ in 0..2 {
                match map.next_key::<BindVar>()? {
                    Some(BindVar::Dur) => dur = Some(map.next_value::<u64>()?),
                    Some(from_) => {
                        from = Some(from_);
                        to = Some(map.next_value::<BindVar>()?);
                    },
                    None => Err(A::Error::custom(ErrVar::MissingClickParams))?
                }
            }

            from.zip(to).ok_or(ErrVar::MissingClickParams)
                .and_then(|(from, to)| make_input_event_map(from, to, dur))
                .map_err(A::Error::custom)
        }
    }

    let click_map = deserializer.deserialize_map(ClickMapVisitor)?;

    Ok(click_map)
}

fn deserialize_input_event_maps<'de, D>(deserializer: D) -> Result<Vec<InputEventMap>, D::Error> where
    D: Deserializer<'de>
{
    struct InputEventMapsVisitor;

    impl<'de> Visitor<'de> for InputEventMapsVisitor {
        type Value = Vec<InputEventMap>;

        fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
            formatter.write_str("a map of key/button maps")
        }

        fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error> where
            A: serde::de::MapAccess<'de>
        {
            let mut input_event_maps: Vec<InputEventMap> = Vec::with_capacity(map.size_hint().unwrap_or_default());

            while let Some(from) = map.next_key::<BindVar>()? {
                match from {
                    BindVar::Click => {
                        let click_map = map.next_value::<ClickMap>()?.0;

                        input_event_maps.push(click_map);
                    },
                    _ => {
                        let to = map.next_value::<BindVar>()?;

                        input_event_maps.push(make_input_event_map(from, to, None).map_err(A::Error::custom)?);
                    }
                }
            }

            Ok(input_event_maps)
        }
    }

    let input_event_maps = deserializer.deserialize_map(InputEventMapsVisitor)?;

    Ok(input_event_maps)
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Op {
    Equals,
    Contains
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Against {
    Caption,
    Class
}

#[derive(Clone, Default, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum WindowRelation {
    #[default]
    This,
    TopLevelFree,
    TopLevelOwned
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Criteria<'a> {
    #[serde(default)]
    pub relation: WindowRelation,
    pub against: Against,
    #[serde(borrow)]
    pub text: Vec<&'a str>,
    pub op: Op
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShiftConstraint<'a> {
    #[serde(borrow)]
    pub criteria: Criteria<'a>
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CenterConstraint<'a> {
    #[serde(borrow)]
    pub criteria: Criteria<'a>
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AttributesConstraint<'a> {
    #[serde(borrow)]
    pub criteria: Criteria<'a>,
    #[serde(default)]
    pub disable_border: bool,
    #[serde(default)]
    pub disable_round_corners: bool
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AnchorConstraint<'a> {
    #[serde(borrow)]
    pub criteria: Criteria<'a>,
    pub relative: AnchorRelative
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Constraints<'a> {
    #[serde(borrow)]
    pub anchor: Option<AnchorConstraint<'a>>,
    pub attributes: Option<AttributesConstraint<'a>>,
    pub center: Option<CenterConstraint<'a>>,
    pub shift: Option<ShiftConstraint<'a>>
}

#[derive(Clone, Copy, Deserialize)]
#[serde(deny_unknown_fields, from = "[u32; 2]")]
pub struct Stride {
    pub x: u32,
    pub y: u32
}
impl Default for Stride {
    fn default() -> Self {
        Self { x: 1, y: 1 }
    }
}
impl From<[u32; 2]> for Stride {
    fn from(value: [u32; 2]) -> Self {
        Self {
            x: value[0],
            y: value[1]
        }
    }
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WindowShift<'a> {
    #[serde(default)]
    pub enable_immersive_dark_mode: bool,
    #[serde(rename = "interval_s")]
    pub interval_dur: u32,
    #[serde(rename = "leeway_px")]
    pub leeway: u32,
    #[serde(default, rename = "stride_px")]
    pub stride: Stride,
    #[serde(borrow)]
    pub constraints: HashMap<&'a str, Constraints<'a>>
}
impl<'a> WindowShift<'a> {
    pub fn get_shift_constraint(&self, exe: &str) -> Option<&ShiftConstraint<'_>> {
        self.constraints.get(exe)
            .and_then(|constraints| {
                constraints.shift.as_ref()
            })
    }
}
impl_name!(WindowShift, 'a);

const fn hitbox_entry_inset_px() -> u16 { 10 }
const fn hitbox_exit_taskbar_offset_px() -> u16 { 100 }
const fn hitbox_exit_jump_list_offset_px() -> u16 { 60 }

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HitboxEntry {
    pub side: Option<Side>,
    #[serde(default = "hitbox_entry_inset_px")]
    pub inset_px: u16,
    pub cursor_snap_offset_px: Option<i32>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HitboxExit {
    #[serde(default = "hitbox_exit_taskbar_offset_px")]
    pub taskbar_offset_px: u16,
    #[serde(default = "hitbox_exit_jump_list_offset_px")]
    pub jump_list_offset_px: u16,
    pub cursor_snap_offset_pc: Option<u32>
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Taskbar {
    pub hitbox_entry: HitboxEntry,
    pub hitbox_exit: HitboxExit
}
impl_name!(Taskbar);

#[derive(Clone, Copy, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PixelCleaning {
    pub let_walk_away: bool
}

const fn reshade_layer_path() -> &'static str { r"C:\ProgramData\ReShade\ReShade64.json" }

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Reshade<'a> {
    pub profile: &'a str,
    #[serde(default = "reshade_layer_path")]
    pub layer_path: &'a str,
    pub preset_path: &'a str,
    pub settings_path: &'a str
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Mpv<'a> {
    pub sdr_profile: &'a str,
    pub hdr_profile: &'a str,
    pub default_glsl_shaders: Option<&'a str>,
    pub override_glsl_shaders: Option<&'a str>,
    pub reshade: Option<Reshade<'a>>
}
impl_name!(Mpv, 'a);

const fn scroll_multiplier() -> f32 { 1.0 }

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MediaBrowser<'a> {
    #[serde(borrow)]
    pub dirs: Vec<&'a str>,
    pub window_inner_size: Option<Extent2dU>,
    pub grid_cell_width: u32,
    pub details_cell_width: u32,
    #[serde(default = "scroll_multiplier")]
    pub scroll_multiplier: f32
}
impl_name!(MediaBrowser, 'a);

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiscordActivity {
    Competing,
    Listening,
    Playing,
    Watching
}
impl From<DiscordActivity> for drpa::ActivityType {
    fn from(value: DiscordActivity) -> Self {
        match value {
            DiscordActivity::Competing => Self::Competing,
            DiscordActivity::Listening => Self::Listening,
            DiscordActivity::Playing => Self::Playing,
            DiscordActivity::Watching => Self::Watching
        }
    }
}

pub struct DiscordActivityInfo {
    pub app_id: String,
    pub activity: DiscordActivity,
    pub details: String,
    pub state: Option<String>,
    pub large_image: Option<String>
}
impl DiscordActivityInfo {
    pub fn as_view(&self) -> DiscordActivityInfoView<'_> {
        DiscordActivityInfoView {
            app_id: self.app_id.as_str(),
            activity: self.activity,
            details: self.details.as_str(),
            state: self.state.as_deref(),
            large_image: self.large_image.as_deref()
        }
    }
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DiscordActivityInfoView<'a> {
    pub app_id: &'a str,
    pub activity: DiscordActivity,
    pub details: &'a str,
    pub state: Option<&'a str>,
    pub large_image: Option<&'a str>
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GameInfo<'a> {
    pub proc: &'a str,
    pub url: Option<&'a str>,
    pub args: Option<Vec<&'a str>>,
    pub cursor_size: Option<usize>,
    pub res: Option<Extent2dU>
}

#[derive(Deserialize)]
#[serde(transparent)]
pub struct Games<'a>(#[serde(borrow)] pub HashMap<&'a str, GameInfo<'a>>);
impl_name!(Games, 'a);

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DitherInfo {
    pub bit_depth: DitherBitDepth,
    pub state: DitherState,
    pub mode: DitherMode,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DisplayModeInfo<'a> {
    pub color_bit_depth: ColorBitDepth,
    pub dither: DitherInfo,
    #[serde(borrow)]
    pub novideo_srgb: Option<NovideoSrgbInfo<'a>>
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DisplayModes<'a> {
    #[serde(borrow)]
    pub sdr: DisplayModeInfo<'a>,
    pub hdr: DisplayModeInfo<'a>
}
impl_name!(DisplayModes, 'a);

#[derive(Clone, Copy, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiscordDisplayKind {
    #[default]
    AppName,
    State,
    Details
}
impl From<DiscordDisplayKind> for drpa::StatusDisplayType {
    fn from(value: DiscordDisplayKind) -> Self {
        match value {
            DiscordDisplayKind::AppName => Self::Name,
            DiscordDisplayKind::State => Self::State,
            DiscordDisplayKind::Details => Self::Details
        }
    }
}

#[derive(Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DiscordAppIds<'a> {
    pub movies: Option<&'a str>,
    pub tv: Option<&'a str>,
    pub words: Option<&'a str>
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Discord<'a> {
    #[serde(borrow)]
    pub app_ids: DiscordAppIds<'a>,
    #[serde(default)]
    pub display_kind: DiscordDisplayKind
}

#[derive(Clone, Copy, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Qmk<'a> {
    pub layer: u8,
    pub layout_path: &'a str
}

#[derive(Clone, Copy, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Underscore {
    #[serde(deserialize_with = "deserialize_key")]
    pub act_on: Key,
    #[serde(deserialize_with = "deserialize_key")]
    pub while_pressed: Key
}

#[derive(Deserialize)]
#[serde(transparent)]
struct ClickMap(
    #[serde(deserialize_with = "deserialize_click_map")]
    InputEventMap
);

#[derive(Clone, Copy)]
pub enum InputEventMap {
    // PressMirror { from: MouseWheel(Wheel), .. } won't make it past config
    PressMirror { from: InputEvent, to: InputEvent },
    WheelClick { from: InputEvent, to: InputEvent, dur: Duration }
}
impl InputEventMap {
    pub fn requires_mouse_hook(&self) -> bool {
        matches!(self,
            InputEventMap::PressMirror { from: InputEvent::MouseButton(_), .. } |
            InputEventMap::WheelClick { .. }
        )
    }
}

#[derive(Clone, Deserialize)]
#[serde(transparent)]
pub struct InputEventMaps(
    #[serde(deserialize_with = "deserialize_input_event_maps")]
    pub Vec<InputEventMap>
);
impl Deref for InputEventMaps {
    type Target = Vec<InputEventMap>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Task {
    BeginPixelCleaning,
    LetWalkAway,
    GoToSleep,
    ToggleDisplayMode,
    PrintWindowInfo
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Hotkeys {
    #[serde(deserialize_with = "deserialize_hotkey_prefix")]
    pub prefix: Vec<Key>,
    #[serde(deserialize_with = "deserialize_tasks", rename = "triggers")]
    pub tasks: HashMap<Key, Task>
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Binds<'a> {
    pub hotkeys: Option<Hotkeys>,
    #[serde(borrow)]
    pub maps: Option<HashMap<&'a str, InputEventMaps>>,
    pub underscore: Option<Underscore>,
    pub qmk: Option<Qmk<'a>>
}
impl_name!(Binds, 'a);

const fn eq_apo_master_config_path() -> &'static str { r"C:\Program Files\EqualizerAPO\config\config.txt" }

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EqApo<'a> {
    #[serde(default = "eq_apo_master_config_path")]
    pub master_config_path: &'a str,
    pub custom_config_paths: HashMap<&'a str, &'a str>
}
impl_name!(EqApo, 'a);

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct App<'a> {
    #[serde(rename = "app")]
    pub path: &'a str,
    #[serde(borrow)]
    pub args: Vec<&'a str>
}
impl<'a> App<'a> {
    pub const FFPROBE:          &'static str = "ffprobe.exe";
    pub const MPV:              &'static str = "mpv.exe";
    pub const NOVIDEO_SRGB:     &'static str = "novideo_srgb.dll";
    pub const SKIF:             &'static str = "SKIF.exe";
    pub const WALLPAPER_ENGINE: &'static str = "wallpaper64.exe";
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Audio<'a> {
    #[serde(borrow)]
    pub endpoint_apps: Option<HashMap<&'a str, App<'a>>>,
    pub eq_apo: Option<EqApo<'a>>
}

const fn epic() -> &'static str { r"C:\Program Files (x86)\Epic Games\Launcher\Portal\Binaries\Win64\EpicGamesLauncher.exe" }
const fn gog() -> &'static str { r"C:\Program Files (x86)\GOG Galaxy\GalaxyClient.exe" }
const fn steam() -> &'static str { r"C:\Program Files (x86)\steam\steam.exe" }

fn app_paths<'a>() -> AppPaths<'a> {
    AppPaths {
        epic: epic(),
        gog: gog(),
        steam: steam(),
        ..default!()
    }
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppPaths<'a> {
    #[serde(default = "epic")]
    pub epic: &'a str,
    pub ffprobe: Option<&'a str>,
    #[serde(default = "gog")]
    pub gog: &'a str,
    pub mpv: Option<&'a str>,
    pub novideo_srgb: Option<&'a str>,
    pub skif: Option<&'a str>,
    #[serde(default = "steam")]
    pub steam: &'a str
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config<'a> {
    #[serde(default = "app_paths", borrow, rename = "apps")]
    pub app_paths: AppPaths<'a>,
    pub audio: Option<Audio<'a>>,
    pub binds: Option<Binds<'a>>,
    #[serde(default)]
    pub discord: Discord<'a>,
    pub display_modes: Option<DisplayModes<'a>>,
    pub games: Option<Games<'a>>,
    pub media_browser: Option<MediaBrowser<'a>>,
    pub mpv: Option<Mpv<'a>>,
    pub pixel_cleaning: Option<PixelCleaning>,
    pub taskbar: Option<Taskbar>,
    pub window_shift: Option<WindowShift<'a>>
}

pub fn load<'a>() -> Res1<Config<'a>> {
    let current_exe_dir = unsafe { CURRENT_EXE_DIR.get_unchecked() };

    // let config_str = fs::read_to_string(current_exe_dir.join("config.json"))?;
    // let config_str = Box::leak(Box::new(config_str));
    // let config = serde_json::from_str::<Config>(config_str)?;

    let config_str = fs::read_to_string(current_exe_dir.join(CONFIG_FILE_NAME))?;
    let config_str = config_str.leak();
    let config = ron::Options::default()
        .with_default_extension(ron::extensions::Extensions::IMPLICIT_SOME)
        .with_default_extension(ron::extensions::Extensions::UNWRAP_NEWTYPES)
        .with_default_extension(ron::extensions::Extensions::UNWRAP_VARIANT_NEWTYPES)
        .from_str::<Config>(config_str)?;

    Ok(config)
}

pub fn get<'a>() -> &'static RwLock<Config<'a>> {
    unsafe { CONFIG.get_unchecked() }
}
