//! The libfpv C API (`include/libfpv.h` at `ed4739d`, ABI 1), mirrored for loading `libfpv.dll`
//! at runtime: every struct `#[repr(C)]`, every enum as its integer type with its values as
//! constants, and the 83 exported functions resolved into [`Api`] by name.
//!
//! The layout test below checks every struct's size, alignment and field offsets against
//! `ffi_layout.txt`, which `tools/libfpv_layout.c` prints from the real header.

#![allow(non_camel_case_types, dead_code, clippy::upper_case_acronyms)]

use std::ffi::{c_char, c_int, c_void};

/// The header's `LIBFPV_ABI_VERSION`, compared with `fpv_abi_version()` at load.
pub const LIBFPV_ABI_VERSION: u32 = 1;

// ── Results ───────────────────────────────────────────────────────────────────────────────────

pub type fpv_result = i32;
pub const FPV_OK: fpv_result = 0;
pub const FPV_ERROR_INVALID_ARGUMENT: fpv_result = -1;
pub const FPV_ERROR_NOT_INITIALIZED: fpv_result = -2;
pub const FPV_ERROR_ALREADY_INITIALIZED: fpv_result = -3;
pub const FPV_ERROR_DRONE_EXISTS: fpv_result = -4;
pub const FPV_ERROR_NOT_FOUND: fpv_result = -5;
pub const FPV_ERROR_IO: fpv_result = -6;
pub const FPV_ERROR_BUSY: fpv_result = -7;
pub const FPV_ERROR_ABI_MISMATCH: fpv_result = -8;
pub const FPV_ERROR_PARSE: fpv_result = -9;
pub const FPV_ERROR_TIMEOUT: fpv_result = -10;
pub const FPV_ERROR_INTERNAL: fpv_result = -11;

// ── Host world conventions ────────────────────────────────────────────────────────────────────

pub type fpv_axis = u32;
pub const FPV_AXIS_POS_X: fpv_axis = 0;
pub const FPV_AXIS_NEG_X: fpv_axis = 1;
pub const FPV_AXIS_POS_Y: fpv_axis = 2;
pub const FPV_AXIS_NEG_Y: fpv_axis = 3;
pub const FPV_AXIS_POS_Z: fpv_axis = 4;
pub const FPV_AXIS_NEG_Z: fpv_axis = 5;

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FpvCoordSystem {
    pub right: fpv_axis,
    pub up: fpv_axis,
    pub forward: fpv_axis,
    pub units_per_meter: f32,
}

pub type fpv_coords_preset_id = u32;
pub const FPV_COORDS_Y_UP_RIGHT_HANDED: fpv_coords_preset_id = 0;
pub const FPV_COORDS_Y_UP_LEFT_HANDED: fpv_coords_preset_id = 1;
pub const FPV_COORDS_Z_UP_LEFT_HANDED: fpv_coords_preset_id = 2;
pub const FPV_COORDS_Z_UP_RIGHT_HANDED: fpv_coords_preset_id = 3;

// ── Global init ───────────────────────────────────────────────────────────────────────────────

pub type fpv_log_level = u32;
pub const FPV_LOG_DEBUG: fpv_log_level = 0;
pub const FPV_LOG_INFO: fpv_log_level = 1;
pub const FPV_LOG_WARN: fpv_log_level = 2;
pub const FPV_LOG_ERROR: fpv_log_level = 3;

/// May be called from libfpv's own threads.
pub type fpv_log_fn =
    Option<unsafe extern "C" fn(user: *mut c_void, level: fpv_log_level, message: *const c_char)>;

pub type fpv_timing_mode = u32;
pub const FPV_TIMING_THREAD: fpv_timing_mode = 0;
pub const FPV_TIMING_MANUAL: fpv_timing_mode = 1;

pub const FPV_INIT_NO_DEVICE_INPUT: u32 = 1 << 0;
pub const FPV_INIT_NO_CONFIGURATOR: u32 = 1 << 1;
pub const FPV_INIT_NO_AUTO_PAUSE: u32 = 1 << 2;
pub const FPV_INIT_NO_LENS_DISTORTION: u32 = 1 << 3;
pub const FPV_INIT_VIDEO_FEED: u32 = 1 << 4;

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct FpvGlobalConfig {
    pub struct_size: u32,
    pub data_dir: *const c_char,
    pub coords: FpvCoordSystem,
    pub log: fpv_log_fn,
    pub log_user: *mut c_void,
    pub log_level: fpv_log_level,
    pub configurator_port: c_int,
    pub timing: fpv_timing_mode,
    pub sim_rate_hz: f32,
    pub auto_pause_seconds: f32,
    pub flags: u32,
    pub host_name: *const c_char,
    pub host_build: *const c_char,
    pub host_url: *const c_char,
}

// ── World collision ───────────────────────────────────────────────────────────────────────────

pub const FPV_MATERIAL_KILLS_PROPS: u32 = 1 << 0;
pub const FPV_MATERIAL_NO_COLLISION: u32 = 1 << 1;
pub const FPV_MATERIAL_TRANSPARENT_TO_VIDEO: u32 = 1 << 2;

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct FpvMaterial {
    pub restitution: f32,
    pub friction: f32,
    pub flags: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct FpvMesh {
    pub positions: *const f32,
    pub vertex_count: u32,
    pub vertex_stride: u32,
    pub indices: *const u32,
    pub triangle_count: u32,
    pub materials: *const u8,
    pub material: u8,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct FpvTransform {
    pub position: [f32; 3],
    pub rotation: [f32; 4],
    pub scale: [f32; 3],
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct FpvRaycastHit {
    pub distance: f32,
    pub normal: [f32; 3],
    pub material: u8,
    pub object_id: u32,
}

/// Returns non-zero on a hit. Runs on the host thread inside `fpv_drone_tick`.
pub type fpv_raycast_fn = Option<
    unsafe extern "C" fn(
        user: *mut c_void,
        origin: *const f32,
        direction: *const f32,
        max_distance: f32,
        out_hit: *mut FpvRaycastHit,
    ) -> c_int,
>;

pub const FPV_RAYCAST_HIT_POSE: u32 = 1;

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct FpvRaycastHitEx {
    pub struct_size: u32,
    pub hit: FpvRaycastHit,
    pub flags: u32,
    pub object_position: [f32; 3],
    pub object_rotation: [f32; 4],
}

pub type fpv_raycast_ex_fn = Option<
    unsafe extern "C" fn(
        user: *mut c_void,
        origin: *const f32,
        direction: *const f32,
        max_distance: f32,
        out_hit: *mut FpvRaycastHitEx,
    ) -> c_int,
>;

// ── Drone ─────────────────────────────────────────────────────────────────────────────────────

/// Opaque.
#[repr(C)]
pub struct FpvDrone {
    _private: [u8; 0],
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct FpvDroneDesc {
    pub struct_size: u32,
    pub preset: *const c_char,
    pub position: [f32; 3],
    pub rotation: [f32; 4],
    pub pilot_position: [f32; 3],
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct FpvCargo {
    pub struct_size: u32,
    pub mass: f32,
    pub offset: [f32; 3],
    pub radius: f32,
    pub line_length: f32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct FpvCamera {
    pub position: [f32; 3],
    pub rotation: [f32; 4],
    pub forward: [f32; 3],
    pub up: [f32; 3],
    pub right: [f32; 3],
    pub fov_y_degrees: f32,
    pub aspect_hint: f32,
    pub near_plane: f32,
    pub distortion_k1: f32,
    pub distortion_k2: f32,
    pub distortion_zoom: f32,
}

pub const FPV_MOTOR_BURNED_OUT: u32 = 1 << 0;
pub const FPV_MOTOR_DAMAGED: u32 = 1 << 1;
pub const FPV_MOTOR_PROP_BROKEN: u32 = 1 << 2;

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct FpvMotorState {
    pub rpm: f32,
    pub thrust_newton: f32,
    pub current_amp: f32,
    pub temperature_c: f32,
    pub command: f32,
    pub prop_damage: f32,
    pub position: [f32; 3],
    pub spin_direction: f32,
    pub flags: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct FpvBatteryState {
    pub voltage: f32,
    pub cell_voltage: f32,
    pub current_amp: f32,
    pub mah_drawn: f32,
    pub capacity_mah: f32,
    pub cell_count: u32,
}

pub const FPV_DRONE_ARMED: u32 = 1 << 0;
pub const FPV_DRONE_FAILSAFE: u32 = 1 << 1;
pub const FPV_DRONE_CRASHED: u32 = 1 << 2;
pub const FPV_DRONE_BEEPER: u32 = 1 << 3;
pub const FPV_DRONE_PAUSED: u32 = 1 << 4;
pub const FPV_DRONE_ON_GROUND: u32 = 1 << 5;
pub const FPV_DRONE_MODE_ANGLE: u32 = 1 << 6;
pub const FPV_DRONE_MODE_HORIZON: u32 = 1 << 7;
pub const FPV_DRONE_TURTLE: u32 = 1 << 8;
pub const FPV_DRONE_OSD_DISABLED: u32 = 1 << 9;
pub const FPV_DRONE_RADIO_ACTIVE: u32 = 1 << 10;
pub const FPV_DRONE_HEARD_FROM_PILOT: u32 = 1 << 12;

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct FpvImpact {
    pub position: [f32; 3],
    pub normal: [f32; 3],
    pub speed: f32,
    pub impulse: f32,
    pub material: u8,
    pub object_id: u32,
    pub motor_index: i32,
}

pub const FPV_IMPACT_CARGO: i32 = -2;
pub const FPV_MAX_IMPACTS: usize = 16;
pub const FPV_OSD_ROWS: usize = 16;
pub const FPV_OSD_COLS: usize = 30;
/// AETR + AUX1..AUX6.
pub const FPV_CHANNEL_COUNT: usize = 10;

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct FpvVideoState {
    pub noise: f32,
    pub breakup: f32,
    pub signal_strength: f32,
    pub is_digital: u32,
}

pub const FPV_VIDEO_FEED_NOISE: u32 = 1 << 0;
pub const FPV_VIDEO_FEED_RESOLUTION: u32 = 1 << 1;
pub const FPV_VIDEO_FEED_OSD_IN_SIGNAL: u32 = 1 << 2;

pub type fpv_video_system = u32;
pub const FPV_VIDEO_NTSC: fpv_video_system = 0;
pub const FPV_VIDEO_PAL: fpv_video_system = 1;
pub const FPV_VIDEO_DIGITAL: fpv_video_system = 2;

pub const FPV_VIDEO_MAX_LINES: usize = 320;

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct FpvVideoFeed {
    pub flags: u32,
    pub system: u32,
    pub picture_aspect: f32,
    pub raster_width: u32,
    pub raster_height: u32,
    pub luma_cycles: f32,
    pub chroma_cycles: f32,
    pub subcarrier_cycles: f32,
    pub sharpening: f32,
    pub field: u32,
    pub field_parity: u32,
    pub roll: f32,
    pub seed: f32,
    pub interference: [f32; 4],
    pub block_errors: f32,
    pub softness: f32,
    pub frozen: u32,
    pub rssi_dbm: f32,
    pub cnr_db: f32,
    pub osd_style: u32,
    pub line_count: u32,
    pub lines: [u8; FPV_VIDEO_MAX_LINES * 8],
    pub chroma_boost: f32,
    pub chroma_tint: [f32; 2],
    pub hue_sweep: f32,
    pub display_aspect: f32,
    pub osd_rows: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct FpvFrame {
    pub struct_size: u32,
    pub sim_time: f64,
    pub flags: u32,
    pub arming_disable_flags: u32,
    pub arming_disable_text: [c_char; 96],
    pub position: [f32; 3],
    pub rotation: [f32; 4],
    pub velocity: [f32; 3],
    pub angular_velocity: [f32; 3],
    pub speed_kmh: f32,
    pub altitude: f32,
    pub camera: FpvCamera,
    pub chase_camera: FpvCamera,
    pub los_camera: FpvCamera,
    pub motors: [FpvMotorState; 4],
    pub battery: FpvBatteryState,
    pub video: FpvVideoState,
    pub impact_count: u32,
    pub impacts: [FpvImpact; FPV_MAX_IMPACTS],
    pub osd: [u8; FPV_OSD_ROWS * FPV_OSD_COLS],
    pub osd_revision: u32,
    pub channels: [f32; FPV_CHANNEL_COUNT],
    pub wind: [f32; 3],
    pub portal_passes: u32,
    pub last_portal: i32,
    pub cargo_flags: u32,
    pub cargo_position: [f32; 3],
    pub cargo_velocity: [f32; 3],
    pub video_feed: FpvVideoFeed,
}

impl FpvFrame {
    /// All zero but `struct_size`, ready for `fpv_drone_tick`.
    pub fn new() -> Box<Self> {
        // SAFETY: every field is an integer, float, array of them or a struct of them, for
        // which all-zero bits are a valid value.
        let mut f: Box<Self> = unsafe { Box::new(std::mem::zeroed()) };
        f.struct_size = std::mem::size_of::<Self>() as u32;
        f
    }

    /// `arming_disable_text` as a string, empty when armable.
    pub fn arming_disable_text(&self) -> String {
        let bytes: Vec<u8> = self
            .arming_disable_text
            .iter()
            .take_while(|&&c| c != 0)
            .map(|&c| c as u8)
            .collect();
        String::from_utf8_lossy(&bytes).into_owned()
    }
}

pub const FPV_CARGO_CARRIED: u32 = 1 << 0;
pub const FPV_CARGO_ON_LINE: u32 = 1 << 1;
pub const FPV_CARGO_LINE_TAUT: u32 = 1 << 2;

pub type fpv_view = u32;
pub const FPV_VIEW_FPV: fpv_view = 0;
pub const FPV_VIEW_CHASE: fpv_view = 1;
pub const FPV_VIEW_LOS: fpv_view = 2;
pub const FPV_VIEW_PILOT: fpv_view = 3;

// ── Portals ───────────────────────────────────────────────────────────────────────────────────

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct FpvPortal {
    pub position: [f32; 3],
    pub rotation: [f32; 4],
    pub half_width: f32,
    pub half_height: f32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct FpvPortalLink {
    pub a: FpvPortal,
    pub b: FpvPortal,
}

pub const FPV_MAX_PORTAL_LINKS: u32 = 16;

// ── Simulation options, RC link, pilot, GPS, wind ─────────────────────────────────────────────

pub const FPV_SIM_NO_BATTERY_DRAIN: u32 = 1 << 0;
pub const FPV_SIM_NO_DAMAGE: u32 = 1 << 1;

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct FpvRcLinkConfig {
    pub struct_size: u32,
    pub enabled: i32,
    pub mode: u32,
    pub max_power_mw: u32,
    pub dynamic_power: i32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct FpvRcLinkState {
    pub struct_size: u32,
    pub enabled: i32,
    pub connected: i32,
    pub link_quality: f32,
    pub rssi_dbm: f32,
    pub snr_db: f32,
    pub tx_power_mw: f32,
    pub sensitivity_dbm: f32,
    pub packet_rate_hz: f32,
}

pub const FPV_PILOT_STAND_IN: u32 = 1 << 0;
pub const FPV_PILOT_POSE: u32 = 1 << 1;

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct FpvPilotConfig {
    pub struct_size: u32,
    pub features: u32,
    pub stand_in_distance: f32,
    pub stand_in_return_distance: f32,
}

pub const FPV_WIND_NO_SHELTER: u32 = 1 << 0;
pub const FPV_WIND_ENCLOSED: u32 = 1 << 1;

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct FpvWind {
    pub struct_size: u32,
    pub velocity: [f32; 3],
    pub reference_height: f32,
    pub turbulence: f32,
    pub flags: u32,
}

pub const FPV_WIND_SOURCE_ROTOR: u32 = 0;
pub const FPV_WIND_SOURCE_JET: u32 = 1;

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct FpvWindSource {
    pub position: [f32; 3],
    pub direction: [f32; 3],
    pub speed: f32,
    pub radius: f32,
    /// C `type`.
    pub kind: u32,
}

pub const FPV_MAX_WIND_SOURCES: u32 = 32;

// ── Input and game controls ───────────────────────────────────────────────────────────────────

pub type fpv_input_source = u32;
pub const FPV_INPUT_SOURCE_NONE: fpv_input_source = 0;
pub const FPV_INPUT_SOURCE_RADIO: fpv_input_source = 1;
pub const FPV_INPUT_SOURCE_GAMEPAD: fpv_input_source = 2;
pub const FPV_INPUT_SOURCE_KEYBOARD: fpv_input_source = 3;
pub const FPV_INPUT_SOURCE_HOST: fpv_input_source = 4;

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct FpvInputStatus {
    pub struct_size: u32,
    pub source: fpv_input_source,
    pub device_name: [c_char; 128],
    pub needs_setup: c_int,
    pub device_count: c_int,
}

pub const FPV_KEY_NONE: i32 = 0;
pub const FPV_KEY_TAB: i32 = 9;
pub const FPV_KEY_ENTER: i32 = 13;
pub const FPV_KEY_ESCAPE: i32 = 27;
pub const FPV_KEY_SPACE: i32 = 32;
pub const FPV_KEY_LEFT: i32 = 256;
pub const FPV_KEY_RIGHT: i32 = 257;
pub const FPV_KEY_UP: i32 = 258;
pub const FPV_KEY_DOWN: i32 = 259;
pub const FPV_KEY_PAGE_UP: i32 = 260;
pub const FPV_KEY_PAGE_DOWN: i32 = 261;
pub const FPV_KEY_HOME: i32 = 262;
pub const FPV_KEY_END: i32 = 263;
pub const FPV_KEY_INSERT: i32 = 264;
pub const FPV_KEY_DELETE: i32 = 265;
pub const FPV_KEY_BACKSPACE: i32 = 266;
pub const FPV_KEY_LSHIFT: i32 = 267;
pub const FPV_KEY_RSHIFT: i32 = 268;
pub const FPV_KEY_LCTRL: i32 = 269;
pub const FPV_KEY_RCTRL: i32 = 270;
pub const FPV_KEY_LALT: i32 = 271;
pub const FPV_KEY_RALT: i32 = 272;
/// F1..F12 are `FPV_KEY_F1 + n - 1`.
pub const FPV_KEY_F1: i32 = 273;

pub type fpv_control_kind = u32;
pub const FPV_CONTROL_INSTANT: fpv_control_kind = 0;
pub const FPV_CONTROL_SWITCH: fpv_control_kind = 1;
pub const FPV_CONTROL_HOLD: fpv_control_kind = 2;

pub const FPV_GAMEPAD_NONE: i32 = -1;

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct FpvControlDesc {
    pub struct_size: u32,
    pub id: u32,
    pub name: *const c_char,
    pub label: *const c_char,
    pub kind: fpv_control_kind,
    pub key: i32,
    pub gamepad_button: i32,
}

// ── Overlay, OSD and textures ─────────────────────────────────────────────────────────────────

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct FpvDrawVertex {
    pub pos: [f32; 2],
    pub uv: [f32; 2],
    /// RGBA8, R in the lowest byte.
    pub color: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct FpvDrawCmd {
    pub clip_rect: [f32; 4],
    pub texture_id: u64,
    pub index_offset: u32,
    pub index_count: u32,
    pub vertex_offset: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct FpvDrawList {
    pub vertices: *const FpvDrawVertex,
    pub vertex_count: u32,
    pub indices: *const u32,
    pub index_count: u32,
    pub commands: *const FpvDrawCmd,
    pub command_count: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct FpvDrawData {
    pub display_size: [f32; 2],
    pub list_count: u32,
    pub lists: *const FpvDrawList,
    pub total_vertex_count: u32,
    pub total_index_count: u32,
}

pub const FPV_TEXTURE_OVERLAY_FONT: u64 = 1;
pub const FPV_TEXTURE_OSD_FONT: u64 = 2;

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct FpvTexture {
    pub width: u32,
    pub height: u32,
    /// `width * height * 4` bytes, straight alpha.
    pub rgba: *const u8,
    pub revision: u32,
}

pub type fpv_osd_style = u32;
pub const FPV_OSD_STYLE_ANALOG: fpv_osd_style = 0;
pub const FPV_OSD_STYLE_HD: fpv_osd_style = 1;

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct FpvConfiguratorInfo {
    pub struct_size: u32,
    pub enabled: c_int,
    pub port: c_int,
    pub client_count: c_int,
    pub url: [c_char; 64],
}

// ── The function table ────────────────────────────────────────────────────────────────────────

/// Declares [`Api`], one field per exported function, and its loader, which resolves each by its
/// C name and fails on the first missing one.
macro_rules! api {
    ($( fn $name:ident ( $($arg:ident : $ty:ty),* $(,)? ) $(-> $ret:ty)? ; )*) => {
        /// Every libfpv export, resolved from one loaded `libfpv.dll`. The library stays loaded
        /// for as long as this lives (and `fpv_global_init` pins it for the process anyway).
        pub struct Api {
            $( pub $name: unsafe extern "C" fn($($ty),*) $(-> $ret)?, )*
            _lib: libloading::Library,
        }

        impl Api {
            /// Resolves every function from `lib`.
            ///
            /// # Safety
            /// `lib` must be a libfpv build whose exports have the signatures above (checked
            /// only by name here; [`Api::load`] then checks the ABI version).
            unsafe fn resolve(lib: libloading::Library) -> Result<Self, String> {
                $(
                    let $name = *unsafe {
                        lib.get::<unsafe extern "C" fn($($ty),*) $(-> $ret)?>(
                            concat!(stringify!($name), "\0").as_bytes(),
                        )
                    }
                    .map_err(|e| format!("{}: {e}", stringify!($name)))?;
                )*
                Ok(Self { $( $name, )* _lib: lib })
            }

            /// The C names of every function in the table.
            pub const NAMES: &'static [&'static str] = &[$( stringify!($name), )*];
        }
    };
}

api! {
    fn fpv_abi_version() -> u32;
    fn fpv_version_string() -> *const c_char;
    fn fpv_result_string(result: fpv_result) -> *const c_char;
    fn fpv_last_error() -> *const c_char;
    fn fpv_free(ptr: *mut c_void);

    fn fpv_coords_preset(preset: fpv_coords_preset_id, units_per_meter: f32) -> FpvCoordSystem;

    fn fpv_global_config_defaults(config: *mut FpvGlobalConfig);
    fn fpv_shared_data_dir(out: *mut c_char, out_size: u32) -> fpv_result;
    fn fpv_global_init(config: *const FpvGlobalConfig) -> fpv_result;
    fn fpv_global_terminate();
    fn fpv_is_initialized() -> c_int;

    fn fpv_material_set(material_id: u8, material: *const FpvMaterial) -> fpv_result;
    fn fpv_static_surfaces_load(meshes: *const FpvMesh, mesh_count: u32) -> fpv_result;
    fn fpv_static_surfaces_clear() -> fpv_result;
    fn fpv_surface_object_create(
        mesh: *const FpvMesh,
        transform: *const FpvTransform,
        out_object_id: *mut u32,
    ) -> fpv_result;
    fn fpv_surface_object_move(object_id: u32, transform: *const FpvTransform) -> fpv_result;
    fn fpv_surface_object_delete(object_id: u32) -> fpv_result;
    fn fpv_world_set_raycast_callback(callback: fpv_raycast_fn, user: *mut c_void);
    fn fpv_world_set_raycast_callback_ex(callback: fpv_raycast_ex_fn, user: *mut c_void);

    fn fpv_drone_desc_defaults(desc: *mut FpvDroneDesc);
    fn fpv_drone_create(desc: *const FpvDroneDesc, out_drone: *mut *mut FpvDrone) -> fpv_result;
    fn fpv_drone_delete(drone: *mut FpvDrone) -> fpv_result;
    fn fpv_drone_reset(drone: *mut FpvDrone, position: *const f32, rotation: *const f32)
        -> fpv_result;
    fn fpv_drone_repair(drone: *mut FpvDrone) -> fpv_result;
    fn fpv_drone_damage_props(drone: *mut FpvDrone, motor_index: i32, amount: f32) -> fpv_result;
    fn fpv_drone_set_pilot_position(drone: *mut FpvDrone, position: *const f32) -> fpv_result;
    fn fpv_drone_set_preset(drone: *mut FpvDrone, preset: *const c_char) -> fpv_result;
    fn fpv_drone_push(drone: *mut FpvDrone, velocity: *const f32, replace: c_int) -> fpv_result;
    fn fpv_cargo_defaults(cargo: *mut FpvCargo);
    fn fpv_drone_set_cargo(drone: *mut FpvDrone, cargo: *const FpvCargo) -> fpv_result;
    fn fpv_drone_tick(drone: *mut FpvDrone, frame_dt: f32, frame: *mut FpvFrame) -> fpv_result;
    fn fpv_set_view(view: fpv_view);

    fn fpv_portals_set(links: *const FpvPortalLink, count: u32) -> fpv_result;

    fn fpv_sim_set_options(options: u32);
    fn fpv_sim_get_options() -> u32;

    fn fpv_rc_link_config_defaults(config: *mut FpvRcLinkConfig);
    fn fpv_rc_link_set_config(config: *const FpvRcLinkConfig) -> fpv_result;
    fn fpv_rc_link_get_config(config: *mut FpvRcLinkConfig) -> fpv_result;
    fn fpv_rc_link_get_state(state: *mut FpvRcLinkState) -> fpv_result;

    fn fpv_pilot_set_support(
        features: u32,
        stand_in_distance: f32,
        stand_in_return_distance: f32,
    ) -> fpv_result;
    fn fpv_pilot_get_config(config: *mut FpvPilotConfig) -> fpv_result;

    fn fpv_gps_set_home(position: *const f32) -> fpv_result;

    fn fpv_wind_defaults(wind: *mut FpvWind);
    fn fpv_wind_set(wind: *const FpvWind) -> fpv_result;
    fn fpv_wind_set_sources(sources: *const FpvWindSource, count: u32) -> fpv_result;
    fn fpv_wind_add_blast(position: *const f32, radius: f32) -> fpv_result;

    fn fpv_set_paused(paused: c_int);
    fn fpv_is_paused() -> c_int;
    fn fpv_set_time_scale(scale: f32);
    fn fpv_step(seconds: f32) -> fpv_result;

    fn fpv_input_get_status(status: *mut FpvInputStatus) -> fpv_result;
    fn fpv_input_set_channels(channels: *const f32, count: u32);
    fn fpv_input_clear_channels();
    fn fpv_input_key(key: c_int, down: c_int);
    fn fpv_controls_set(controls: *const FpvControlDesc, count: u32) -> fpv_result;
    fn fpv_control_poll(id: u32, presses: *mut u32) -> c_int;

    fn fpv_texture_get(texture_id: u64, out: *mut FpvTexture) -> fpv_result;
    fn fpv_overlay_set_open(open: c_int);
    fn fpv_overlay_is_open() -> c_int;
    fn fpv_overlay_wants_input() -> c_int;
    fn fpv_overlay_set_toggle_key(key: c_int);
    fn fpv_overlay_set_display(width: f32, height: f32, ui_scale: f32);
    fn fpv_overlay_mouse_move(x: f32, y: f32);
    fn fpv_overlay_mouse_button(button: c_int, down: c_int);
    fn fpv_overlay_mouse_wheel(dx: f32, dy: f32);
    fn fpv_overlay_text(utf8: *const c_char);
    fn fpv_overlay_set_action(id: u32, label: *const c_char);
    fn fpv_overlay_poll_action() -> u32;
    fn fpv_overlay_render(frame_dt: f32) -> *const FpvDrawData;

    fn fpv_osd_draw_data(
        frame: *const FpvFrame,
        width: f32,
        height: f32,
        style: fpv_osd_style,
    ) -> *const FpvDrawData;
    fn fpv_osd_render_rgba(
        frame: *const FpvFrame,
        rgba: *mut u8,
        width: u32,
        height: u32,
        stride_bytes: u32,
        style: fpv_osd_style,
    ) -> fpv_result;

    fn fpv_audio_configure(sample_rate: u32, channels: u32) -> fpv_result;
    fn fpv_audio_read(out: *mut f32, frames: u32) -> u32;
    fn fpv_audio_set_volume(volume: f32);
    fn fpv_audio_set_beep_volume(volume: f32);
    fn fpv_audio_set_listener(position: *const f32, rotation: *const f32);

    fn fpv_cli_exec(commands: *const c_char, out_output: *mut *mut c_char) -> fpv_result;
    fn fpv_import_betaflight_diff(diff_text: *const c_char, out_report: *mut *mut c_char)
        -> fpv_result;
    fn fpv_set_default_betaflight_diff(diff_text: *const c_char);
    fn fpv_preset_count() -> u32;
    fn fpv_preset_name(index: u32) -> *const c_char;
    fn fpv_preset_directory() -> *const c_char;
    fn fpv_configurator_get_info(info: *mut FpvConfiguratorInfo) -> fpv_result;
}

impl Api {
    /// Loads `libfpv.dll` from `path`, resolves every export and checks the ABI version.
    pub fn load(path: &std::path::Path) -> Result<Self, String> {
        // SAFETY: loading runs the DLL's initialisers; libfpv's are its own C++ statics.
        let lib = unsafe { libloading::Library::new(path) }
            .map_err(|e| format!("{}: {e}", path.display()))?;
        // SAFETY: a libfpv DLL, its signatures per this header; the ABI is checked next.
        let api = unsafe { Self::resolve(lib) }?;
        // SAFETY: no arguments, no state.
        let abi = unsafe { (api.fpv_abi_version)() };
        if abi != LIBFPV_ABI_VERSION {
            return Err(format!(
                "{}: ABI {abi}, these bindings are ABI {LIBFPV_ABI_VERSION}",
                path.display()
            ));
        }
        Ok(api)
    }

    /// A C string libfpv returned, owned by libfpv, as a Rust string; empty for null.
    pub(crate) fn string(ptr: *const c_char) -> String {
        if ptr.is_null() {
            return String::new();
        }
        // SAFETY: libfpv returns NUL-terminated strings that stay valid until the next call.
        unsafe { std::ffi::CStr::from_ptr(ptr) }
            .to_string_lossy()
            .into_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::mem::{align_of, offset_of, size_of};

    /// `tools/libfpv_layout.c`'s output for the pinned header.
    const LAYOUT: &str = include_str!("ffi_layout.txt");

    fn c_layout() -> (HashMap<String, (usize, usize)>, HashMap<String, usize>) {
        let (mut structs, mut fields) = (HashMap::new(), HashMap::new());
        for line in LAYOUT.lines() {
            let p: Vec<&str> = line.split_whitespace().collect();
            match p.as_slice() {
                ["S", name, size, align] => {
                    structs.insert(
                        name.to_string(),
                        (size.parse().unwrap(), align.parse().unwrap()),
                    );
                }
                ["F", name, off] => {
                    fields.insert(name.to_string(), off.parse().unwrap());
                }
                [] => {}
                _ => panic!("bad layout line {line:?}"),
            }
        }
        (structs, fields)
    }

    /// A field's C name: its Rust name, or the one given where they differ (`type`).
    macro_rules! c_name {
        ($field:ident) => {
            stringify!($field)
        };
        ($field:ident, $c:literal) => {
            $c
        };
    }

    /// Checks one struct's size, alignment and the listed fields' offsets against C, and that
    /// every C field of it is listed.
    macro_rules! check {
        ($s:expr, $f:expr, $seen:expr, $rust:ty, $c:literal { $($field:ident $(as $cname:literal)?),* $(,)? }) => {{
            let (size, align) = $s.get($c).copied().unwrap_or_else(|| panic!("{} not in C layout", $c));
            assert_eq!(size_of::<$rust>(), size, "size of {}", $c);
            assert_eq!(align_of::<$rust>(), align, "align of {}", $c);
            $(
                let cname = c_name!($field $(, $cname)?);
                let key = format!("{}.{}", $c, cname);
                let want = *$f.get(&key).unwrap_or_else(|| panic!("{key} not in C layout"));
                assert_eq!(offset_of!($rust, $field), want, "offset of {key}");
                $seen.push(key);
            )*
            $seen.push(format!("S {}", $c));
        }};
    }

    #[test]
    fn every_struct_matches_the_c_layout() {
        let (s, f) = c_layout();
        let mut seen: Vec<String> = Vec::new();
        check!(s, f, seen, FpvCoordSystem, "fpv_coord_system" { right, up, forward, units_per_meter });
        check!(s, f, seen, FpvGlobalConfig, "fpv_global_config" {
            struct_size, data_dir, coords, log, log_user, log_level, configurator_port, timing,
            sim_rate_hz, auto_pause_seconds, flags, host_name, host_build, host_url });
        check!(s, f, seen, FpvMaterial, "fpv_material" { restitution, friction, flags });
        check!(s, f, seen, FpvMesh, "fpv_mesh" {
            positions, vertex_count, vertex_stride, indices, triangle_count, materials, material });
        check!(s, f, seen, FpvTransform, "fpv_transform" { position, rotation, scale });
        check!(s, f, seen, FpvRaycastHit, "fpv_raycast_hit" { distance, normal, material, object_id });
        check!(s, f, seen, FpvRaycastHitEx, "fpv_raycast_hit_ex" {
            struct_size, hit, flags, object_position, object_rotation });
        check!(s, f, seen, FpvDroneDesc, "fpv_drone_desc" {
            struct_size, preset, position, rotation, pilot_position });
        check!(s, f, seen, FpvCargo, "fpv_cargo" { struct_size, mass, offset, radius, line_length });
        check!(s, f, seen, FpvCamera, "fpv_camera" {
            position, rotation, forward, up, right, fov_y_degrees, aspect_hint, near_plane,
            distortion_k1, distortion_k2, distortion_zoom });
        check!(s, f, seen, FpvMotorState, "fpv_motor_state" {
            rpm, thrust_newton, current_amp, temperature_c, command, prop_damage, position,
            spin_direction, flags });
        check!(s, f, seen, FpvBatteryState, "fpv_battery_state" {
            voltage, cell_voltage, current_amp, mah_drawn, capacity_mah, cell_count });
        check!(s, f, seen, FpvImpact, "fpv_impact" {
            position, normal, speed, impulse, material, object_id, motor_index });
        check!(s, f, seen, FpvVideoState, "fpv_video_state" {
            noise, breakup, signal_strength, is_digital });
        check!(s, f, seen, FpvVideoFeed, "fpv_video_feed" {
            flags, system, picture_aspect, raster_width, raster_height, luma_cycles,
            chroma_cycles, subcarrier_cycles, sharpening, field, field_parity, roll, seed,
            interference, block_errors, softness, frozen, rssi_dbm, cnr_db, osd_style,
            line_count, lines, chroma_boost, chroma_tint, hue_sweep, display_aspect, osd_rows });
        check!(s, f, seen, FpvFrame, "fpv_frame" {
            struct_size, sim_time, flags, arming_disable_flags, arming_disable_text, position,
            rotation, velocity, angular_velocity, speed_kmh, altitude, camera, chase_camera,
            los_camera, motors, battery, video, impact_count, impacts, osd, osd_revision,
            channels, wind, portal_passes, last_portal, cargo_flags, cargo_position,
            cargo_velocity, video_feed });
        check!(s, f, seen, FpvPortal, "fpv_portal" { position, rotation, half_width, half_height });
        check!(s, f, seen, FpvPortalLink, "fpv_portal_link" { a, b });
        check!(s, f, seen, FpvRcLinkConfig, "fpv_rc_link_config" {
            struct_size, enabled, mode, max_power_mw, dynamic_power });
        check!(s, f, seen, FpvRcLinkState, "fpv_rc_link_state" {
            struct_size, enabled, connected, link_quality, rssi_dbm, snr_db, tx_power_mw,
            sensitivity_dbm, packet_rate_hz });
        check!(s, f, seen, FpvPilotConfig, "fpv_pilot_config" {
            struct_size, features, stand_in_distance, stand_in_return_distance });
        check!(s, f, seen, FpvWind, "fpv_wind" {
            struct_size, velocity, reference_height, turbulence, flags });
        check!(s, f, seen, FpvWindSource, "fpv_wind_source" {
            position, direction, speed, radius, kind as "type" });
        check!(s, f, seen, FpvInputStatus, "fpv_input_status" {
            struct_size, source, device_name, needs_setup, device_count });
        check!(s, f, seen, FpvControlDesc, "fpv_control_desc" {
            struct_size, id, name, label, kind, key, gamepad_button });
        check!(s, f, seen, FpvDrawVertex, "fpv_draw_vertex" { pos, uv, color });
        check!(s, f, seen, FpvDrawCmd, "fpv_draw_cmd" {
            clip_rect, texture_id, index_offset, index_count, vertex_offset });
        check!(s, f, seen, FpvDrawList, "fpv_draw_list" {
            vertices, vertex_count, indices, index_count, commands, command_count });
        check!(s, f, seen, FpvDrawData, "fpv_draw_data" {
            display_size, list_count, lists, total_vertex_count, total_index_count });
        check!(s, f, seen, FpvTexture, "fpv_texture" { width, height, rgba, revision });
        check!(s, f, seen, FpvConfiguratorInfo, "fpv_configurator_info" {
            struct_size, enabled, port, client_count, url });

        // Every struct and field the C side printed was checked here.
        let mut missing: Vec<String> = s
            .keys()
            .map(|k| format!("S {k}"))
            .chain(f.keys().cloned())
            .filter(|k| !seen.contains(k))
            .collect();
        missing.sort();
        assert!(missing.is_empty(), "C layout not mirrored: {missing:?}");
    }

    #[test]
    fn the_table_binds_83_distinct_exports() {
        let mut names = Api::NAMES.to_vec();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), 83);
    }
}
