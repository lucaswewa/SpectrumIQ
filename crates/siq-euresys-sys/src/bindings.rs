// The shim's bindings, from `shim/siq_egrabber.h`: regenerate them with `cargo xtask
// regen-bindings`, never by hand.
//
// P2.5 wrote this first version by hand, in bindgen's form, because libclang isn't installed
// (docs/impl-notes/P2.5.md). `tests/layout.rs` checks it against the header.

pub const SIQ_EG_ABI_VERSION: u32 = 1;
pub const SIQ_EG_OK: SiqEgStatus = 0;
pub const SIQ_EG_TIMEOUT: SiqEgStatus = 1;
pub const SIQ_EG_ABORTED: SiqEgStatus = 2;
pub const SIQ_EG_TRUNCATED: SiqEgStatus = 3;
pub const SIQ_EG_INVALID_ARGUMENT: SiqEgStatus = 4;
pub const SIQ_EG_GENTL_ERROR: SiqEgStatus = 5;
pub const SIQ_EG_ERROR: SiqEgStatus = 6;
pub type SiqEgStatus = ::std::os::raw::c_int;
pub const SIQ_EG_INTERFACE: SiqEgModule = 0;
pub const SIQ_EG_DEVICE: SiqEgModule = 1;
pub const SIQ_EG_STREAM: SiqEgModule = 2;
pub const SIQ_EG_REMOTE: SiqEgModule = 3;
pub type SiqEgModule = ::std::os::raw::c_int;
pub const SIQ_EG_FOUND_INTERFACE: SiqEgFound = 0;
pub const SIQ_EG_FOUND_GRABBER: SiqEgFound = 1;
pub const SIQ_EG_FOUND_CAMERA: SiqEgFound = 2;
pub type SiqEgFound = ::std::os::raw::c_int;
#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct SiqEgCounts {
    pub interfaces: i32,
    pub grabbers: i32,
    pub cameras: i32,
}
#[allow(clippy::unnecessary_operation, clippy::identity_op)]
const _: () = {
    ["Size of SiqEgCounts"][::std::mem::size_of::<SiqEgCounts>() - 12usize];
    ["Alignment of SiqEgCounts"][::std::mem::align_of::<SiqEgCounts>() - 4usize];
    ["Offset of field: SiqEgCounts::interfaces"]
        [::std::mem::offset_of!(SiqEgCounts, interfaces) - 0usize];
    ["Offset of field: SiqEgCounts::grabbers"]
        [::std::mem::offset_of!(SiqEgCounts, grabbers) - 4usize];
    ["Offset of field: SiqEgCounts::cameras"]
        [::std::mem::offset_of!(SiqEgCounts, cameras) - 8usize];
};
#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct SiqBufferInfo {
    pub index: u64,
    pub ptr: *mut ::std::os::raw::c_void,
    pub size: u64,
    pub timestamp: u64,
    pub frame_id: u64,
    pub width: u32,
    pub height: u32,
    pub pixfmt: u64,
    pub flags: u32,
    pub pixfmt_namespace: u32,
}
#[allow(clippy::unnecessary_operation, clippy::identity_op)]
const _: () = {
    ["Size of SiqBufferInfo"][::std::mem::size_of::<SiqBufferInfo>() - 64usize];
    ["Alignment of SiqBufferInfo"][::std::mem::align_of::<SiqBufferInfo>() - 8usize];
    ["Offset of field: SiqBufferInfo::index"]
        [::std::mem::offset_of!(SiqBufferInfo, index) - 0usize];
    ["Offset of field: SiqBufferInfo::ptr"][::std::mem::offset_of!(SiqBufferInfo, ptr) - 8usize];
    ["Offset of field: SiqBufferInfo::size"][::std::mem::offset_of!(SiqBufferInfo, size) - 16usize];
    ["Offset of field: SiqBufferInfo::timestamp"]
        [::std::mem::offset_of!(SiqBufferInfo, timestamp) - 24usize];
    ["Offset of field: SiqBufferInfo::frame_id"]
        [::std::mem::offset_of!(SiqBufferInfo, frame_id) - 32usize];
    ["Offset of field: SiqBufferInfo::width"]
        [::std::mem::offset_of!(SiqBufferInfo, width) - 40usize];
    ["Offset of field: SiqBufferInfo::height"]
        [::std::mem::offset_of!(SiqBufferInfo, height) - 44usize];
    ["Offset of field: SiqBufferInfo::pixfmt"]
        [::std::mem::offset_of!(SiqBufferInfo, pixfmt) - 48usize];
    ["Offset of field: SiqBufferInfo::flags"]
        [::std::mem::offset_of!(SiqBufferInfo, flags) - 56usize];
    ["Offset of field: SiqBufferInfo::pixfmt_namespace"]
        [::std::mem::offset_of!(SiqBufferInfo, pixfmt_namespace) - 60usize];
};
pub const SIQ_EG_BUFFER_INCOMPLETE: u32 = 1;
#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct SiqEgCounters {
    pub announced: u64,
    pub queued: u64,
    pub awaiting_delivery: u64,
    pub delivered: u64,
    pub underrun: u64,
    pub started: u64,
}
#[allow(clippy::unnecessary_operation, clippy::identity_op)]
const _: () = {
    ["Size of SiqEgCounters"][::std::mem::size_of::<SiqEgCounters>() - 48usize];
    ["Alignment of SiqEgCounters"][::std::mem::align_of::<SiqEgCounters>() - 8usize];
    ["Offset of field: SiqEgCounters::announced"]
        [::std::mem::offset_of!(SiqEgCounters, announced) - 0usize];
    ["Offset of field: SiqEgCounters::queued"]
        [::std::mem::offset_of!(SiqEgCounters, queued) - 8usize];
    ["Offset of field: SiqEgCounters::awaiting_delivery"]
        [::std::mem::offset_of!(SiqEgCounters, awaiting_delivery) - 16usize];
    ["Offset of field: SiqEgCounters::delivered"]
        [::std::mem::offset_of!(SiqEgCounters, delivered) - 24usize];
    ["Offset of field: SiqEgCounters::underrun"]
        [::std::mem::offset_of!(SiqEgCounters, underrun) - 32usize];
    ["Offset of field: SiqEgCounters::started"]
        [::std::mem::offset_of!(SiqEgCounters, started) - 40usize];
};
#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct SiqEgSystem {
    _unused: [u8; 0],
}
#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct SiqEgGrabber {
    _unused: [u8; 0],
}
unsafe extern "C" {
    pub fn siq_eg_abi_version() -> u32;
    pub fn siq_eg_last_error(buf: *mut ::std::os::raw::c_char, len: usize) -> usize;
    pub fn siq_eg_system_open(
        producer: *const ::std::os::raw::c_char,
        system: *mut *mut SiqEgSystem,
    ) -> SiqEgStatus;
    pub fn siq_eg_system_close(system: *mut SiqEgSystem);
    pub fn siq_eg_discover(system: *mut SiqEgSystem, counts: *mut SiqEgCounts) -> SiqEgStatus;
    pub fn siq_eg_discovery_info(
        system: *mut SiqEgSystem,
        found: SiqEgFound,
        index: i32,
        buf: *mut ::std::os::raw::c_char,
        len: usize,
        needed: *mut usize,
    ) -> SiqEgStatus;
    pub fn siq_eg_open(
        system: *mut SiqEgSystem,
        camera: i32,
        grabber: *mut *mut SiqEgGrabber,
    ) -> SiqEgStatus;
    pub fn siq_eg_close(grabber: *mut SiqEgGrabber);
    pub fn siq_eg_get_int(
        grabber: *mut SiqEgGrabber,
        module: SiqEgModule,
        feature: *const ::std::os::raw::c_char,
        value: *mut i64,
    ) -> SiqEgStatus;
    pub fn siq_eg_get_float(
        grabber: *mut SiqEgGrabber,
        module: SiqEgModule,
        feature: *const ::std::os::raw::c_char,
        value: *mut f64,
    ) -> SiqEgStatus;
    pub fn siq_eg_get_string(
        grabber: *mut SiqEgGrabber,
        module: SiqEgModule,
        feature: *const ::std::os::raw::c_char,
        buf: *mut ::std::os::raw::c_char,
        len: usize,
        needed: *mut usize,
    ) -> SiqEgStatus;
    pub fn siq_eg_set_int(
        grabber: *mut SiqEgGrabber,
        module: SiqEgModule,
        feature: *const ::std::os::raw::c_char,
        value: i64,
    ) -> SiqEgStatus;
    pub fn siq_eg_set_float(
        grabber: *mut SiqEgGrabber,
        module: SiqEgModule,
        feature: *const ::std::os::raw::c_char,
        value: f64,
    ) -> SiqEgStatus;
    pub fn siq_eg_set_string(
        grabber: *mut SiqEgGrabber,
        module: SiqEgModule,
        feature: *const ::std::os::raw::c_char,
        value: *const ::std::os::raw::c_char,
    ) -> SiqEgStatus;
    pub fn siq_eg_execute(
        grabber: *mut SiqEgGrabber,
        module: SiqEgModule,
        command: *const ::std::os::raw::c_char,
    ) -> SiqEgStatus;
    pub fn siq_eg_announce_user_buffers(
        grabber: *mut SiqEgGrabber,
        ptrs: *const *mut ::std::os::raw::c_void,
        size: usize,
        count: usize,
    ) -> SiqEgStatus;
    pub fn siq_eg_start(grabber: *mut SiqEgGrabber, frames: u64) -> SiqEgStatus;
    pub fn siq_eg_stop(grabber: *mut SiqEgGrabber) -> SiqEgStatus;
    pub fn siq_eg_pop(
        grabber: *mut SiqEgGrabber,
        timeout_ms: u64,
        info: *mut SiqBufferInfo,
    ) -> SiqEgStatus;
    pub fn siq_eg_cancel_pop(grabber: *mut SiqEgGrabber) -> SiqEgStatus;
    pub fn siq_eg_requeue(grabber: *mut SiqEgGrabber, index: u64) -> SiqEgStatus;
    pub fn siq_eg_counters(grabber: *mut SiqEgGrabber, counters: *mut SiqEgCounters)
    -> SiqEgStatus;
}
