
#![allow(warnings)]

use ogos_binds::*;
use ogos_err::*;

use nvapi_sys::{
    handles::NvPhysicalGpuHandle,
    status::NvAPI_Status,
    *
};
use nvapi_sys_new as nvapi_530;
use nvapi_530::*;
use serde::*;
use std::{
    fmt::{self, *},
    ops::*,
    result::Result
};

pub type _NV_DITHER_STATE = ::std::os::raw::c_int;
pub use self::_NV_DITHER_STATE as NV_DITHER_STATE;
pub const _NV_DITHER_STATE_NV_DITHER_STATE_DEFAULT: _NV_DITHER_STATE = 0;
pub const _NV_DITHER_STATE_NV_DITHER_STATE_ENABLED: _NV_DITHER_STATE = 1;
pub const _NV_DITHER_STATE_NV_DITHER_STATE_DISABLED: _NV_DITHER_STATE = 2;
pub const _NV_DITHER_STATE_NV_DITHER_STATE_MAX: _NV_DITHER_STATE = 255;

pub type _NV_DITHER_BITS = ::std::os::raw::c_int;
pub use self::_NV_DITHER_BITS as NV_DITHER_BITS;
pub const _NV_DITHER_BITS_NV_DITHER_BITS_6: _NV_DITHER_BITS = 0;
pub const _NV_DITHER_BITS_NV_DITHER_BITS_8: _NV_DITHER_BITS = 1;
pub const _NV_DITHER_BITS_NV_DITHER_BITS_10: _NV_DITHER_BITS = 2;
pub const _NV_DITHER_BITS_NV_DITHER_BITS_MAX: _NV_DITHER_BITS = 255;

pub type _NV_DITHER_MODE = ::std::os::raw::c_int;
pub use self::_NV_DITHER_MODE as NV_DITHER_MODE;
pub const _NV_DITHER_MODE_NV_DITHER_MODE_SPATIAL_DYNAMIC: _NV_DITHER_MODE = 0;
pub const _NV_DITHER_MODE_NV_DITHER_MODE_SPATIAL_STATIC: _NV_DITHER_MODE = 1;
pub const _NV_DITHER_MODE_NV_DITHER_MODE_SPATIAL_DYNAMIC_2x2: _NV_DITHER_MODE = 2;
pub const _NV_DITHER_MODE_NV_DITHER_MODE_SPATIAL_STATIC_2x2: _NV_DITHER_MODE = 3;
pub const _NV_DITHER_MODE_NV_DITHER_MODE_TEMPORAL: _NV_DITHER_MODE = 4;
pub const _NV_DITHER_MODE_NV_DITHER_MODE_MAX: _NV_DITHER_MODE = 255;

#[repr(C)]
#[derive(Default, Clone, Copy, PartialEq)]
pub struct _NV_GPU_DITHER_CONTROL_V1__bindgen_ty_1 {
    pub bits: NvU32,
    pub mode: NvU32
}

#[repr(C)]
#[derive(Default, Clone, Copy, PartialEq)]
pub struct _NV_GPU_DITHER_CONTROL_V1 {
    pub version: NvU32,
    pub state: NV_DITHER_STATE,
    pub bits: NV_DITHER_BITS,
    pub mode: NV_DITHER_MODE,
    pub caps: _NV_GPU_DITHER_CONTROL_V1__bindgen_ty_1
}
pub type NV_GPU_DITHER_CONTROL = _NV_GPU_DITHER_CONTROL_V1;

pub type NvAPI_GPU_SetDitherControl_fn = unsafe extern "C" fn(
    hPhysicalGpu: NvPhysicalGpuHandle,
    output_id: NvU32,
    state: NV_DITHER_STATE,
    bits: NV_DITHER_BITS,
    mode: NV_DITHER_MODE
) -> NvAPI_Status;
pub type NvAPI_GPU_GetDitherControl_fn = unsafe extern "C" fn(
    output_id: NvU32,
    ditherControl: *mut NV_GPU_DITHER_CONTROL
) -> NvAPI_Status;

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

#[derive(Clone, Copy,Deserialize)]
#[serde(rename_all = "snake_case")]
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
        match self {
            Self::SpatialStatic => write!(f, "spatial_static"),
            Self::SpatialStatic2x2 => write!(f, "spatial_static2x2"),
            Self::SpatialDynamic => write!(f, "spatial_dynamic"),
            Self::SpatialDynamic2x2 => write!(f, "spatial_dynamic2x2"),
            Self::Temporal => write!(f, "temporal")
        }
    }
}

#[derive(Clone, Copy, Deserialize)]
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
        match self {
            Self::Default => write!(f, "default"),
            Self::Enabled => write!(f, "enabled"),
            Self::Disabled => write!(f, "disabled")
        }
    }
}
