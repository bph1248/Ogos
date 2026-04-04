#![allow(non_camel_case_types)]
#![allow(non_upper_case_globals)]

use super::*;

pub type _NV_DITHER_STATE = ::std::os::raw::c_int;
pub use self::_NV_DITHER_STATE as NV_DITHER_STATE;
const _NV_DITHER_STATE_NV_DITHER_STATE_DEFAULT: _NV_DITHER_STATE = 0;
const _NV_DITHER_STATE_NV_DITHER_STATE_ENABLED: _NV_DITHER_STATE = 1;
const _NV_DITHER_STATE_NV_DITHER_STATE_DISABLED: _NV_DITHER_STATE = 2;
const _NV_DITHER_STATE_NV_DITHER_STATE_MAX: _NV_DITHER_STATE = 255;

pub type _NV_DITHER_BITS = ::std::os::raw::c_int;
pub use self::_NV_DITHER_BITS as NV_DITHER_BITS;
const _NV_DITHER_BITS_NV_DITHER_BITS_6: _NV_DITHER_BITS = 0;
const _NV_DITHER_BITS_NV_DITHER_BITS_8: _NV_DITHER_BITS = 1;
const _NV_DITHER_BITS_NV_DITHER_BITS_10: _NV_DITHER_BITS = 2;
const _NV_DITHER_BITS_NV_DITHER_BITS_MAX: _NV_DITHER_BITS = 255;

pub type _NV_DITHER_MODE = ::std::os::raw::c_int;
pub use self::_NV_DITHER_MODE as NV_DITHER_MODE;
const _NV_DITHER_MODE_NV_DITHER_MODE_SPATIAL_DYNAMIC: _NV_DITHER_MODE = 0;
const _NV_DITHER_MODE_NV_DITHER_MODE_SPATIAL_STATIC: _NV_DITHER_MODE = 1;
const _NV_DITHER_MODE_NV_DITHER_MODE_SPATIAL_DYNAMIC_2x2: _NV_DITHER_MODE = 2;
const _NV_DITHER_MODE_NV_DITHER_MODE_SPATIAL_STATIC_2x2: _NV_DITHER_MODE = 3;
const _NV_DITHER_MODE_NV_DITHER_MODE_TEMPORAL: _NV_DITHER_MODE = 4;
const _NV_DITHER_MODE_NV_DITHER_MODE_MAX: _NV_DITHER_MODE = 255;

const fn novideo_srgb_enable_clamp() -> bool { true }

#[derive(Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NovideoSrgbInfo<'a> {
    #[serde(default = "novideo_srgb_enable_clamp", skip)]
    pub enable_clamp: bool,
    #[serde(borrow, rename = "primaries")]
    pub primaries_source: PrimariesSource<'a>,
    pub color_space_target: ColorSpaceTarget,
    #[serde(default)]
    pub gamma: Gamma,
    pub enable_optimization: bool
}
impl<'a> NovideoSrgbInfo<'a> {
    pub const NAME: &'static str = "novideo_srgb";
}

impl<'a> Display for NovideoSrgbInfo<'a> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.enable_clamp {
            true => write!(f, "clamp: {}, optimization: {}, primaries: {}, target: {}, gamma: {}",
                self.enable_clamp, self.enable_optimization, self.primaries_source, self.color_space_target, self.gamma),
            false => write!(f, "clamp: {}", self.enable_clamp)
        }
    }
}

#[derive(Clone, Copy, Deserialize, PartialEq)]
#[serde(try_from = "BindVar")]
pub enum ColorBitDepth {
    Default,
    N6,
    N8,
    N10,
    N12,
    N16
}
impl Deref for ColorBitDepth {
    type Target = NV_BPC;

    fn deref(&self) -> &Self::Target {
        use nvapi_530::*;

        match self {
            Self::Default => &_NV_BPC_NV_BPC_DEFAULT,
            Self::N6 => &_NV_BPC_NV_BPC_6,
            Self::N8 => &_NV_BPC_NV_BPC_8,
            Self::N10 => &_NV_BPC_NV_BPC_10,
            Self::N12 => &_NV_BPC_NV_BPC_12,
            Self::N16 => &_NV_BPC_NV_BPC_16
        }
    }
}
impl Display for ColorBitDepth {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Default => write!(f, "default"),
            Self::N6 => write!(f, "6"),
            Self::N8 => write!(f, "8"),
            Self::N10 => write!(f, "10"),
            Self::N12 => write!(f, "12"),
            Self::N16 => write!(f, "16")
        }
    }
}
impl TryFrom<BindVar> for ColorBitDepth {
    type Error = ErrVar;

    fn try_from(value: BindVar) -> Result<Self, Self::Error> {
        Ok(match value {
            BindVar::Default => Self::Default,
            BindVar::N6 => Self::N6,
            BindVar::N8 => Self::N8,
            BindVar::N10 => Self::N10,
            BindVar::N12 => Self::N12,
            BindVar::N16 => Self::N16,
            _ => Err(ErrVar::FailedColorBitDepthFrom { from: value.as_str().into() })?
        })
    }
}

#[derive(Clone, Copy, Default, Deserialize, IntoStaticStr)]
#[strum(serialize_all = "snake_case")]
#[repr(i32)]
pub enum ColorSpaceTarget {
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
impl Display for ColorSpaceTarget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", Into::<&'static str>::into(self))
    }
}

#[derive(Clone, Copy, PartialEq, IntoStaticStr)]
#[strum(serialize_all = "snake_case")]
pub enum DisplayMode {
    Sdr,
    Hdr
}
impl Display for DisplayMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", Into::<&'static str>::into(self))
    }
}
impl Not for DisplayMode {
    type Output = Self;

    fn not(self) -> Self::Output {
        match self {
            Self::Sdr => Self::Hdr,
            Self::Hdr => Self::Sdr
        }
    }
}

#[derive(Clone, Copy, Deserialize, PartialEq)]
#[serde(try_from = "BindVar")]
pub enum DitherBitDepth {
    N6,
    N8,
    N10
}
impl Deref for DitherBitDepth {
    type Target = NV_DITHER_BITS;

    fn deref(&self) -> &Self::Target {
        match self {
            Self::N6 => &_NV_DITHER_BITS_NV_DITHER_BITS_6,
            Self::N8 => &_NV_DITHER_BITS_NV_DITHER_BITS_8,
            Self::N10 => &_NV_DITHER_BITS_NV_DITHER_BITS_10
        }
    }
}
impl Display for DitherBitDepth {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::N6 => write!(f, "6"),
            Self::N8 => write!(f, "8"),
            Self::N10 => write!(f, "10")
        }
    }
}
impl TryFrom<BindVar> for DitherBitDepth {
    type Error = ErrVar;

    fn try_from(value: BindVar) -> Result<Self, Self::Error> {
        Ok(match value {
            BindVar::N6 => Self::N6,
            BindVar::N8 => Self::N8,
            BindVar::N10 => Self::N10,
            _ => Err(ErrVar::FailedDitherBitDepthFrom { from: value.as_str().into() })?
        })
    }
}

#[derive(Clone, Copy,Deserialize, IntoStaticStr)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum DitherMode {
    SpatialStatic,
    SpatialStatic2x2,
    SpatialDynamic,
    SpatialDynamic2x2,
    Temporal
}
impl Deref for DitherMode {
    type Target = _NV_DITHER_MODE;

    fn deref(&self) -> &Self::Target {
        match self {
            Self::SpatialStatic => &_NV_DITHER_MODE_NV_DITHER_MODE_SPATIAL_STATIC,
            Self::SpatialStatic2x2 => &_NV_DITHER_MODE_NV_DITHER_MODE_SPATIAL_STATIC_2x2,
            Self::SpatialDynamic => &_NV_DITHER_MODE_NV_DITHER_MODE_SPATIAL_DYNAMIC,
            Self::SpatialDynamic2x2 => &_NV_DITHER_MODE_NV_DITHER_MODE_SPATIAL_DYNAMIC_2x2,
            Self::Temporal => &_NV_DITHER_MODE_NV_DITHER_MODE_TEMPORAL
        }
    }
}
impl Display for DitherMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", Into::<&'static str>::into(self))
    }
}

#[derive(Clone, Copy, Deserialize, IntoStaticStr)]
#[serde(rename_all = "snake_case")]
pub enum DitherState {
    Default,
    Enabled,
    Disabled
}
impl Deref for DitherState {
    type Target = _NV_DITHER_STATE;

    fn deref(&self) -> &Self::Target {
        match self {
            Self::Default => &_NV_DITHER_STATE_NV_DITHER_STATE_DEFAULT,
            Self::Enabled => &_NV_DITHER_STATE_NV_DITHER_STATE_ENABLED,
            Self::Disabled => &_NV_DITHER_STATE_NV_DITHER_STATE_DISABLED
        }
    }
}
impl Display for DitherState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", Into::<&'static str>::into(self))
    }
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Intent {
    Absolute,
    Relative
}

#[derive(Default)]
pub struct GammaFfi {
    pub calibrate_gamma: bool,
    pub gamma_target: i32,
    pub gamma_value: f64,
    pub black_output_offset: f64
}

#[derive(Clone, Default, Deserialize, IntoStaticStr)]
#[serde(deny_unknown_fields)]
#[strum(serialize_all = "snake_case")]
pub enum Gamma {
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

    pub fn as_ffi(&self) -> GammaFfi {
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

impl Display for Gamma {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", Into::<&'static str>::into(self))
    }
}

#[derive(Clone, Default, Deserialize, IntoStaticStr)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum PrimariesSource<'a> {
    #[default]
    Edid,
    Profile { path: &'a str }
}
impl<'a> PrimariesSource<'a> {
    pub fn as_i32(&self) -> i32 {
        match self {
            Self::Edid => 0,
            Self::Profile { .. } => 1
        }
    }
}

impl<'a> Display for PrimariesSource<'a> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", Into::<&'static str>::into(self))
    }
}
