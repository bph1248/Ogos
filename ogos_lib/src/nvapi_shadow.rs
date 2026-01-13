#![allow(warnings)]

use nvapi_sys::{
    handles::NvPhysicalGpuHandle,
    status::NvAPI_Status
};
use nvapi_sys_new::NvU32;

pub(crate) type _NV_DITHER_STATE = ::std::os::raw::c_int;
pub(crate) use self::_NV_DITHER_STATE as NV_DITHER_STATE;
pub(crate) const _NV_DITHER_STATE_NV_DITHER_STATE_DEFAULT: _NV_DITHER_STATE = 0;
pub(crate) const _NV_DITHER_STATE_NV_DITHER_STATE_ENABLED: _NV_DITHER_STATE = 1;
pub(crate) const _NV_DITHER_STATE_NV_DITHER_STATE_DISABLED: _NV_DITHER_STATE = 2;
pub(crate) const _NV_DITHER_STATE_NV_DITHER_STATE_MAX: _NV_DITHER_STATE = 255;

pub(crate) type _NV_DITHER_BITS = ::std::os::raw::c_int;
pub(crate) use self::_NV_DITHER_BITS as NV_DITHER_BITS;
pub(crate) const _NV_DITHER_BITS_NV_DITHER_BITS_6: _NV_DITHER_BITS = 0;
pub(crate) const _NV_DITHER_BITS_NV_DITHER_BITS_8: _NV_DITHER_BITS = 1;
pub(crate) const _NV_DITHER_BITS_NV_DITHER_BITS_10: _NV_DITHER_BITS = 2;
pub(crate) const _NV_DITHER_BITS_NV_DITHER_BITS_MAX: _NV_DITHER_BITS = 255;

pub(crate) type _NV_DITHER_MODE = ::std::os::raw::c_int;
pub(crate) use self::_NV_DITHER_MODE as NV_DITHER_MODE;
pub(crate) const _NV_DITHER_MODE_NV_DITHER_MODE_SPATIAL_DYNAMIC: _NV_DITHER_MODE = 0;
pub(crate) const _NV_DITHER_MODE_NV_DITHER_MODE_SPATIAL_STATIC: _NV_DITHER_MODE = 1;
pub(crate) const _NV_DITHER_MODE_NV_DITHER_MODE_SPATIAL_DYNAMIC_2x2: _NV_DITHER_MODE = 2;
pub(crate) const _NV_DITHER_MODE_NV_DITHER_MODE_SPATIAL_STATIC_2x2: _NV_DITHER_MODE = 3;
pub(crate) const _NV_DITHER_MODE_NV_DITHER_MODE_TEMPORAL: _NV_DITHER_MODE = 4;
pub(crate) const _NV_DITHER_MODE_NV_DITHER_MODE_MAX: _NV_DITHER_MODE = 255;

#[repr(C)]
#[derive(Default, Clone, Copy, PartialEq)]
pub(crate) struct _NV_GPU_DITHER_CONTROL_V1__bindgen_ty_1 {
    pub(crate) bits: NvU32,
    pub(crate) mode: NvU32
}

#[repr(C)]
#[derive(Default, Clone, Copy, PartialEq)]
pub(crate) struct _NV_GPU_DITHER_CONTROL_V1 {
    pub(crate) version: NvU32,
    pub(crate) state: NV_DITHER_STATE,
    pub(crate) bits: NV_DITHER_BITS,
    pub(crate) mode: NV_DITHER_MODE,
    pub(crate) caps: _NV_GPU_DITHER_CONTROL_V1__bindgen_ty_1
}
pub(crate) type NV_GPU_DITHER_CONTROL = _NV_GPU_DITHER_CONTROL_V1;

pub(crate) type NvAPI_GPU_SetDitherControl_fn = unsafe extern "C" fn(
    hPhysicalGpu: NvPhysicalGpuHandle,
    output_id: NvU32,
    state: NV_DITHER_STATE,
    bits: NV_DITHER_BITS,
    mode: NV_DITHER_MODE
) -> NvAPI_Status;
pub(crate) type NvAPI_GPU_GetDitherControl_fn = unsafe extern "C" fn(
    output_id: NvU32,
    ditherControl: *mut NV_GPU_DITHER_CONTROL
) -> NvAPI_Status;
