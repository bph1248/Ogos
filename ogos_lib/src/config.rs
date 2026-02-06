use crate::{
    common::*,
    display::*,
    window_foreground::*,
    window_shift::*
};
use ogos_binds::*;
use ogos_err::*;
use ogos_mki::*;

use const_format::*;
use discord_rich_presence::activity as drpa;
// use log::*;
use serde::{
    de::*,
    *
};
use std::{
    collections::*,
    fmt,
    fs,
    sync::*,
    time::*
};

pub(crate) const CONFIG_FILE_NAME: &str = "config.ron";

macro_rules! impl_name {
    ($name:ident, $lt:lifetime) => {
        impl<$lt> $name<$lt> {
            pub(crate) const NAME: &'static str = map_ascii_case!(Case::Snake, stringify!($name));
        }
    };
    ($name:ident) => {
        impl $name {
            pub(crate) const NAME: &str = map_ascii_case!(Case::Snake, stringify!($name));
        }
    };
}

fn deserialize_key<'de, D>(deserializer: D) -> Result<Key, D::Error> where
    D: Deserializer<'de>
{
    BindVar::deserialize(deserializer)?.try_as_key().map_err(D::Error::custom)
}

fn deserialize_keys<'de, D>(deserializer: D) -> Result<Vec<Key>, D::Error> where
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
            let mut keys = Vec::with_capacity(seq.size_hint().unwrap_or_default());

            while let Some(key) = seq.next_element::<BindVar>()? {
                keys.push(key.try_as_key().map_err(A::Error::custom)?);
            };

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
pub(crate) enum Op {
    Equals,
    Contains
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Against {
    Caption,
    Class
}

#[derive(Clone, Default, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum WindowRelation {
    #[default]
    This,
    TopLevelFree,
    TopLevelOwned
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Criteria<'a> {
    #[serde(default)]
    pub(crate) relation: WindowRelation,
    pub(crate) against: Against,
    #[serde(borrow)]
    pub(crate) text: Vec<&'a str>,
    pub(crate) op: Op
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ShiftConstraint<'a> {
    #[serde(borrow)]
    pub(crate) criteria: Criteria<'a>
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CenterConstraint<'a> {
    #[serde(borrow)]
    pub(crate) criteria: Criteria<'a>
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AttributesConstraint<'a> {
    #[serde(borrow)]
    pub(crate) criteria: Criteria<'a>,
    #[serde(default)]
    pub(crate) disable_border: bool,
    #[serde(default)]
    pub(crate) disable_round_corners: bool
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AnchorConstraint<'a> {
    #[serde(borrow)]
    pub(crate) criteria: Criteria<'a>,
    pub(crate) relative: AnchorRelative
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Constraints<'a> {
    #[serde(borrow)]
    pub(crate) anchor: Option<AnchorConstraint<'a>>,
    pub(crate) attributes: Option<AttributesConstraint<'a>>,
    pub(crate) center: Option<CenterConstraint<'a>>,
    pub(crate) shift: Option<ShiftConstraint<'a>>
}

#[derive(Clone, Copy, Deserialize)]
#[serde(deny_unknown_fields, from = "[u32; 2]")]
pub(crate) struct Stride {
    pub(crate) x: u32,
    pub(crate) y: u32
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
pub(crate) struct WindowShift<'a> {
    pub(crate) enable_immersive_dark_mode: bool,
    #[serde(rename = "interval_s")]
    pub(crate) interval_dur: u32,
    #[serde(rename = "leeway_px")]
    pub(crate) leeway: u32,
    #[serde(rename = "stride_px")]
    pub(crate) stride: Stride,
    #[serde(borrow)]
    pub(crate) constraints: HashMap<&'a str, Constraints<'a>>
}
impl<'a> WindowShift<'a> {
    pub(crate) fn get_shift_constraint(&self, exe: &str) -> Option<&ShiftConstraint<'_>> {
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
pub(crate) struct HitboxEntry {
    pub(crate) side: Option<Side>,
    #[serde(default = "hitbox_entry_inset_px")]
    pub(crate) inset_px: u16,
    pub(crate) cursor_snap_offset_px: Option<i32>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct HitboxExit {
    #[serde(default = "hitbox_exit_taskbar_offset_px")]
    pub(crate) taskbar_offset_px: u16,
    #[serde(default = "hitbox_exit_jump_list_offset_px")]
    pub(crate) jump_list_offset_px: u16,
    pub(crate) cursor_snap_offset_pc: Option<u32>
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Taskbar {
    pub(crate) hitbox_entry: HitboxEntry,
    pub(crate) hitbox_exit: HitboxExit
}
impl_name!(Taskbar);

#[derive(Clone, Copy, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PixelCleaning {
    pub(crate) let_walk_away: bool,
    pub(crate) pause_wallpaper_engine: bool
}

const fn reshade_layer_path() -> &'static str { r"C:\ProgramData\ReShade\ReShade64.json" }

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Reshade<'a> {
    pub(crate) profile: &'a str,
    #[serde(default = "reshade_layer_path")]
    pub(crate) layer_path: &'a str,
    pub(crate) preset_path: &'a str,
    pub(crate) settings_path: &'a str
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Mpv<'a> {
    pub(crate) sdr_profile: &'a str,
    pub(crate) hdr_profile: &'a str,
    pub(crate) default_glsl_shaders: Option<&'a str>,
    pub(crate) override_glsl_shaders: Option<&'a str>,
    pub(crate) reshade: Option<Reshade<'a>>
}
impl_name!(Mpv, 'a);

const fn vscroll_multiplier() -> f32 { 1.0 }

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MediaBrowser<'a> {
    #[serde(borrow)]
    pub(crate) dirs: Vec<&'a str>,
    pub(crate) window_inner_size: Option<Extent2dU>,
    pub(crate) grid_cell_width: u32,
    pub(crate) details_cell_width: u32,
    #[serde(default = "vscroll_multiplier")]
    pub(crate) vscroll_multiplier: f32
}
impl_name!(MediaBrowser, 'a);

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum DiscordActivity {
    Competing,
    Listening,
    Playing,
    Watching
}
impl Into<drpa::ActivityType> for DiscordActivity {
    fn into(self) -> drpa::ActivityType {
        match self {
            Self::Competing => drpa::ActivityType::Competing,
            Self::Listening => drpa::ActivityType::Listening,
            Self::Playing => drpa::ActivityType::Playing,
            Self::Watching => drpa::ActivityType::Watching
        }
    }
}

pub(crate) struct DiscordInfo {
    pub(crate) client_id: String,
    pub(crate) activity: DiscordActivity,
    pub(crate) details: String,
    pub(crate) state: Option<String>,
    pub(crate) display_kind: DiscordDisplayKind,
    pub(crate) large_image: Option<String>,
    pub(crate) chess_username: Option<String>
}
impl DiscordInfo {
    pub(crate) fn as_view(&self) -> DiscordInfoView<'_> {
        DiscordInfoView {
            client_id: self.client_id.as_str(),
            activity: self.activity,
            details: self.details.as_str(),
            state: self.state.as_deref(),
            display_kind: self.display_kind,
            large_image: self.large_image.as_deref(),
            chess_username: self.chess_username.as_deref()
        }
    }
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DiscordInfoView<'a> {
    pub(crate) client_id: &'a str,
    pub(crate) activity: DiscordActivity,
    pub(crate) details: &'a str,
    pub(crate) state: Option<&'a str>,
    #[serde(default)]
    pub(crate) display_kind: DiscordDisplayKind,
    pub(crate) large_image: Option<&'a str>,
    pub(crate) chess_username: Option<&'a str>
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct GameInfo<'a> {
    pub(crate) proc: &'a str,
    pub(crate) url: Option<&'a str>,
    pub(crate) args: Option<Vec<&'a str>>,
    pub(crate) cursor_size: Option<usize>,
    pub(crate) res: Option<Extent2dU>,
    pub(crate) discord: Option<DiscordInfoView<'a>>
}

#[derive(Deserialize)]
#[serde(transparent)]
pub(crate) struct Games<'a>(#[serde(borrow)] pub(crate) HashMap<&'a str, GameInfo<'a>>);
impl_name!(Games, 'a);

#[derive(Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Intent {
    Absolute,
    Relative
}

#[derive(Default)]
pub(crate) struct GammaFfi {
    pub(crate) calibrate_gamma: bool,
    pub(crate) gamma_target: i32,
    pub(crate) gamma_value: f64,
    pub(crate) black_output_offset: f64
}

#[derive(Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) enum Gamma {
    #[serde(rename = "srgb")]
    Srgb,
    #[default]
    #[serde(rename = "bt_1886")]
    Bt1886,
    #[serde(rename = "custom")]
    Custom { value: f64, black_output_offset: f64, intent: Intent },
    #[serde(rename = "lstar")]
    Lstar
}
impl Gamma {
    fn target(&self) -> i32 {
        match self {
            Self::Srgb => 0,
            Self::Bt1886 => 1,
            Self::Custom { intent, .. } => {
                match intent {
                    Intent::Absolute => 2,
                    Intent::Relative => 3
                }
            },
            Self::Lstar => 4
        }
    }

    pub(crate) fn as_ffi(&self) -> GammaFfi {
        let calibrate_gamma = true;
        let gamma_target = self.target();

        match self {
            Self::Custom { value, black_output_offset, .. } => GammaFfi {
                calibrate_gamma,
                gamma_target,
                gamma_value: *value,
                black_output_offset: *black_output_offset
            },
            _ => GammaFfi {
                calibrate_gamma,
                gamma_target,
                gamma_value: 0.0,
                black_output_offset: 0.0
            }
        }
    }
}

#[derive(Clone, Copy, Default, Deserialize)]
#[repr(i32)]
pub(crate) enum ColorSpaceTarget {
    #[serde(rename = "bt_709")]
    #[default]
    Bt709 = 0,
    #[serde(rename = "display_p3")]
    DisplayP3,
    #[serde(rename = "adobe_rgb")]
    AdobeRgb,
    #[serde(rename = "bt_2020")]
    Bt2020
}

#[derive(Clone, Default, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
pub(crate) enum PrimariesSource<'a> {
    #[default]
    Edid,
    Profile { path: &'a str }
}
impl<'a> PrimariesSource<'a> {
    pub(crate) fn as_i32(&self) -> i32 {
        match self {
            Self::Edid => 0,
            Self::Profile { .. } => 1
        }
    }
}

const fn novideo_srgb_enable_clamp() -> bool { true }

#[derive(Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct NovideoSrgbInfo<'a> {
    #[serde(default = "novideo_srgb_enable_clamp")]
    pub(crate) enable_clamp: bool,
    #[serde(borrow, rename = "primaries")]
    pub(crate) primaries_source: PrimariesSource<'a>,
    pub(crate) color_space_target: ColorSpaceTarget,
    #[serde(default)]
    pub(crate) gamma: Gamma,
    pub(crate) enable_optimization: bool
}
impl<'a> NovideoSrgbInfo<'a> {
    pub(crate) const NAME: &'static str = "novideo_srgb";
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DitherInfo {
    pub(crate) bit_depth: DitherBitDepth,
    pub(crate) state: DitherState,
    pub(crate) mode: DitherMode,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DisplayModeInfo<'a> {
    pub(crate) color_bit_depth: ColorBitDepth,
    pub(crate) dither: DitherInfo,
    #[serde(borrow)]
    pub(crate) novideo_srgb: Option<NovideoSrgbInfo<'a>>
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DisplayModes<'a> {
    #[serde(borrow)]
    pub(crate) sdr: DisplayModeInfo<'a>,
    pub(crate) hdr: DisplayModeInfo<'a>
}
impl_name!(DisplayModes, 'a);

#[derive(Clone, Copy, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum DiscordDisplayKind {
    #[default]
    Name,
    State,
    Details
}
impl Into<drpa::StatusDisplayType> for DiscordDisplayKind {
    fn into(self) -> drpa::StatusDisplayType {
        match self {
            Self::Name => drpa::StatusDisplayType::Name,
            Self::State => drpa::StatusDisplayType::State,
            Self::Details => drpa::StatusDisplayType::Details
        }
    }
}

#[derive(Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DiscordAppIds<'a> {
    pub(crate) movies: Option<&'a str>,
    pub(crate) tv: Option<&'a str>,
    pub(crate) words: Option<&'a str>
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Discord<'a> {
    #[serde(borrow)]
    pub(crate) app_ids: DiscordAppIds<'a>,
    #[serde(default)]
    pub(crate) display_kind: DiscordDisplayKind
}

#[derive(Clone, Copy, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Qmk<'a> {
    pub(crate) layer: u8,
    pub(crate) layout_path: &'a str
}

#[derive(Clone, Copy, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Underscore {
    #[serde(deserialize_with = "deserialize_key")]
    pub(crate) act_on: Key,
    #[serde(deserialize_with = "deserialize_key")]
    pub(crate) while_pressed: Key
}

#[derive(Deserialize)]
#[serde(transparent)]
struct ClickMap(
    #[serde(deserialize_with = "deserialize_click_map")]
    InputEventMap
);

#[derive(Clone, Copy)]
pub(crate) enum InputEventMap {
    PressMirror { from: InputEvent, to: InputEvent },
    WheelClick { from: InputEvent, to: InputEvent, dur: Duration }
}

#[derive(Clone, Deserialize)]
#[serde(transparent)]
pub(crate) struct InputEventMaps(
    #[serde(deserialize_with = "deserialize_input_event_maps")]
    pub(crate) Vec<InputEventMap>
);

#[derive(Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Task {
    BeginPixelCleaning,
    LetWalkAway,
    GoToSleep,
    ToggleDisplayMode,
    #[cfg(feature = "dbg_window_info")]
    GetForegroundInfo
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Hotkeys {
    #[serde(deserialize_with = "deserialize_keys")]
    pub(crate) prefixes: Vec<Key>,
    #[serde(deserialize_with = "deserialize_tasks")]
    pub(crate) tasks: HashMap<Key, Task>
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Binds<'a> {
    pub(crate) hotkeys: Option<Hotkeys>,
    #[serde(borrow)]
    pub(crate) maps: Option<HashMap<&'a str, InputEventMaps>>,
    pub(crate) underscore: Option<Underscore>,
    pub(crate) qmk: Option<Qmk<'a>>
}
impl_name!(Binds, 'a);

const fn eq_apo_master_config_path() -> &'static str { r"C:\Program Files\EqualizerAPO\config\config.txt" }

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct EqApo<'a> {
    #[serde(default = "eq_apo_master_config_path")]
    pub(crate) master_config_path: &'a str,
    pub(crate) custom_config_paths: HashMap<&'a str, &'a str>
}
impl_name!(EqApo, 'a);

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct App<'a> {
    #[serde(rename = "app")]
    pub(crate) path: &'a str,
    #[serde(borrow)]
    pub(crate) args: Vec<&'a str>
}
impl<'a> App<'a> {
    pub(crate) const FFPROBE:          &'static str = "ffprobe.exe";
    pub(crate) const MPV:              &'static str = "mpv.exe";
    pub(crate) const NOVIDEO_SRGB:     &'static str = "novideo_srgb.dll";
    pub(crate) const SKIF:             &'static str = "SKIF.exe";
    pub(crate) const WALLPAPER_ENGINE: &'static str = "wallpaper64.exe";
}

#[derive(Deserialize)]
#[serde(transparent)]
pub(crate) struct EndpointApps<'a>(#[serde(borrow)] pub(crate) HashMap<&'a str, App<'a>>);

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Audio<'a> {
    #[serde(borrow)]
    pub(crate) endpoint_apps: Option<EndpointApps<'a>>,
    pub(crate) eq_apo: Option<EqApo<'a>>
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
pub(crate) struct AppPaths<'a> {
    #[serde(default = "epic")]
    pub(crate) epic: &'a str,
    pub(crate) ffprobe: Option<&'a str>,
    #[serde(default = "gog")]
    pub(crate) gog: &'a str,
    pub(crate) mpv: Option<&'a str>,
    pub(crate) novideo_srgb: Option<&'a str>,
    pub(crate) skif: Option<&'a str>,
    #[serde(default = "steam")]
    pub(crate) steam: &'a str,
    pub(crate) wallpaper_engine: Option<&'a str>
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Config<'a> {
    #[serde(default = "app_paths", borrow, rename = "apps")]
    pub(crate) app_paths: AppPaths<'a>,
    pub(crate) audio: Option<Audio<'a>>,
    pub(crate) binds: Option<Binds<'a>>,
    #[serde(default)]
    pub(crate) discord: Discord<'a>,
    pub(crate) display_modes: Option<DisplayModes<'a>>,
    pub(crate) games: Option<Games<'a>>,
    pub(crate) media_browser: Option<MediaBrowser<'a>>,
    pub(crate) mpv: Option<Mpv<'a>>,
    pub(crate) pixel_cleaning: Option<PixelCleaning>,
    pub(crate) taskbar: Option<Taskbar>,
    pub(crate) window_shift: Option<WindowShift<'a>>
}

pub(crate) fn load<'a>() -> Res1<Config<'a>> {
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

pub(crate) fn get<'a>() -> &'static RwLock<Config<'a>> {
    unsafe { CONFIG.get_unchecked() }
}
