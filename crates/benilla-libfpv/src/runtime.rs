//! libfpv loaded and initialised: [`Libfpv`] owns the function table and the one drone, routes
//! libfpv's log into benilla's, and answers libfpv's collision rays with benilla's world through
//! [`Libfpv::tick`].

use std::cell::Cell;
use std::ffi::{c_char, c_int, c_void, CString};
use std::path::{Path, PathBuf};

use bevy::prelude::*;

use crate::ffi::{self, Api, FpvFrame};

/// Yards per metre: benilla's world unit is the yard.
pub const UNITS_PER_METER: f32 = 1.093_613_3;

/// Shown in F8 > About next to libfpv's own build.
pub const HOST_NAME: &str = "WoW FPV (benilla-libfpv)";
pub const HOST_URL: &str = "https://github.com/Jepharion/benilla";

/// `LIBFPV_DLL` names the DLL; else `libfpv.dll` beside this executable.
pub fn dll_path() -> PathBuf {
    if let Some(p) = std::env::var_os("LIBFPV_DLL") {
        return PathBuf::from(p);
    }
    std::env::current_exe()
        .ok()
        .and_then(|e| e.parent().map(|d| d.join("libfpv.dll")))
        .unwrap_or_else(|| PathBuf::from("libfpv.dll"))
}

/// How to bring libfpv up.
#[derive(Clone, Debug)]
pub struct InitOptions {
    /// `FPV_TIMING_MANUAL`: the host steps the simulation with [`Libfpv::step`].
    pub manual_timing: bool,
    /// `fpv_init_flags`.
    pub flags: u32,
    /// The About tab's build line: version and commit.
    pub host_build: String,
}

/// libfpv, initialised, with at most one drone.
#[derive(Resource)]
pub struct Libfpv {
    api: Api,
    drone: *mut ffi::FpvDrone,
    pub data_dir: PathBuf,
}

// SAFETY: libfpv's API is thread-safe (serialised internally, libfpv.h:33-35); the drone pointer
// is only handed back to libfpv.
unsafe impl Send for Libfpv {}
unsafe impl Sync for Libfpv {}

impl Libfpv {
    /// Loads the DLL at `dll`, checks its ABI and initialises it, with the shared data folder
    /// (`fpv_shared_data_dir`, which `LIBFPV_DATA_DIR` overrides), benilla's axes in yards, and
    /// its log in benilla's.
    pub fn init(dll: &Path, opts: &InitOptions) -> Result<Self, String> {
        let api = Api::load(dll)?;
        let version = Api::string(unsafe { (api.fpv_version_string)() });

        let mut buf = [0 as c_char; 1024];
        // SAFETY: the buffer and its size; needs no init.
        let r = unsafe { (api.fpv_shared_data_dir)(buf.as_mut_ptr(), buf.len() as u32) };
        if r != ffi::FPV_OK {
            return Err(format!("fpv_shared_data_dir: {}", result_name(&api, r)));
        }
        let data_dir = PathBuf::from(Api::string(buf.as_ptr()));

        let c_data = cstring(&data_dir.to_string_lossy());
        let c_name = cstring(HOST_NAME);
        let c_build = cstring(&opts.host_build);
        let c_url = cstring(HOST_URL);
        // SAFETY: a zeroed config is then filled by libfpv's own defaults.
        let mut cfg: ffi::FpvGlobalConfig = unsafe { std::mem::zeroed() };
        unsafe { (api.fpv_global_config_defaults)(&mut cfg) };
        cfg.data_dir = c_data.as_ptr();
        // X right, Y up, -Z forward (north) in yards: benilla's Bevy world (NOTES Q1).
        cfg.coords =
            unsafe { (api.fpv_coords_preset)(ffi::FPV_COORDS_Y_UP_RIGHT_HANDED, UNITS_PER_METER) };
        cfg.log = Some(log_to_benilla);
        cfg.log_user = std::ptr::null_mut();
        cfg.log_level = ffi::FPV_LOG_INFO;
        cfg.flags = opts.flags;
        if opts.manual_timing {
            cfg.timing = ffi::FPV_TIMING_MANUAL;
        }
        cfg.host_name = c_name.as_ptr();
        cfg.host_build = c_build.as_ptr();
        cfg.host_url = c_url.as_ptr();

        // SAFETY: every pointer in `cfg` outlives the call; libfpv copies the strings.
        let r = unsafe { (api.fpv_global_init)(&cfg) };
        if r != ffi::FPV_OK {
            return Err(format!(
                "fpv_global_init: {} ({})",
                result_name(&api, r),
                Api::string(unsafe { (api.fpv_last_error)() })
            ));
        }
        // The callback answers a miss until a tick lends it the world (`RaycastScope`), so it is
        // safe to register now, also where libfpv calls it outside a tick.
        unsafe { (api.fpv_world_set_raycast_callback)(Some(raycast), std::ptr::null_mut()) };
        info!(
            "libfpv: {version} loaded from {}, data in {}",
            dll.display(),
            data_dir.display()
        );
        Ok(Self {
            api,
            drone: std::ptr::null_mut(),
            data_dir,
        })
    }

    pub fn api(&self) -> &Api {
        &self.api
    }

    /// The last error raised on this thread, for a report.
    pub fn last_error(&self) -> String {
        Api::string(unsafe { (self.api.fpv_last_error)() })
    }

    fn check(&self, what: &str, r: ffi::fpv_result) -> Result<(), String> {
        if r == ffi::FPV_OK {
            Ok(())
        } else {
            Err(format!(
                "{what}: {} ({})",
                result_name(&self.api, r),
                self.last_error()
            ))
        }
    }

    /// Creates the drone with its frame's bottom at `position`, facing `rotation`, the pilot at
    /// `pilot` (feet). `world` answers the rays libfpv may cast while creating it.
    pub fn create_drone(
        &mut self,
        position: Vec3,
        rotation: Quat,
        pilot: Vec3,
        world: &mut dyn FnMut(Vec3, Dir3, f32) -> Option<RayHit>,
    ) -> Result<(), String> {
        if !self.drone.is_null() {
            return Err("a drone already exists".into());
        }
        // SAFETY: zeroed then filled by libfpv's defaults.
        let mut desc: ffi::FpvDroneDesc = unsafe { std::mem::zeroed() };
        unsafe { (self.api.fpv_drone_desc_defaults)(&mut desc) };
        desc.position = position.to_array();
        desc.rotation = rotation.to_array();
        desc.pilot_position = pilot.to_array();
        let mut drone = std::ptr::null_mut();
        let r = with_world(world, || unsafe {
            (self.api.fpv_drone_create)(&desc, &mut drone)
        });
        self.check("fpv_drone_create", r)?;
        self.drone = drone;
        Ok(())
    }

    pub fn has_drone(&self) -> bool {
        !self.drone.is_null()
    }

    /// Teleports the drone, zero velocity.
    pub fn reset_drone(
        &mut self,
        position: Vec3,
        rotation: Quat,
        world: &mut dyn FnMut(Vec3, Dir3, f32) -> Option<RayHit>,
    ) -> Result<(), String> {
        let (p, q) = (position.to_array(), rotation.to_array());
        let r = with_world(world, || unsafe {
            (self.api.fpv_drone_reset)(self.drone, p.as_ptr(), q.as_ptr())
        });
        self.check("fpv_drone_reset", r)
    }

    /// Fresh battery, motors, props and video transmitter (the standalone's R does this, then a
    /// reset).
    pub fn repair(&mut self) -> Result<(), String> {
        let r = unsafe { (self.api.fpv_drone_repair)(self.drone) };
        self.check("fpv_drone_repair", r)
    }

    /// Sets the drone's velocity at once, yards per second.
    pub fn push(&mut self, velocity: Vec3) -> Result<(), String> {
        let v = velocity.to_array();
        let r = unsafe { (self.api.fpv_drone_push)(self.drone, v.as_ptr(), 1) };
        self.check("fpv_drone_push", r)
    }

    /// Host channels, AETR + AUX1..AUX6, -1..1.
    pub fn set_channels(&self, channels: &[f32; ffi::FPV_CHANNEL_COUNT]) {
        unsafe { (self.api.fpv_input_set_channels)(channels.as_ptr(), channels.len() as u32) };
    }

    /// `FPV_TIMING_MANUAL`: advances the simulation by `seconds`.
    pub fn step(&self, seconds: f32) -> Result<(), String> {
        let r = unsafe { (self.api.fpv_step)(seconds) };
        self.check("fpv_step", r)
    }

    pub fn set_view(&self, view: ffi::fpv_view) {
        unsafe { (self.api.fpv_set_view)(view) };
    }

    /// Where the drone is heard from in the pilot view: the game camera (host coords), or `None`
    /// for libfpv's own (the pilot's head, turning to follow the drone).
    pub fn set_audio_listener(&self, pose: Option<(Vec3, Quat)>) {
        match pose {
            Some((p, q)) => {
                let (p, q) = (p.to_array(), q.to_array());
                unsafe { (self.api.fpv_audio_set_listener)(p.as_ptr(), q.as_ptr()) };
            }
            None => unsafe {
                (self.api.fpv_audio_set_listener)(std::ptr::null(), std::ptr::null())
            },
        }
    }

    /// Advances presentation by `dt` and fills `frame`; libfpv's collision rays during it are
    /// answered by `world`.
    pub fn tick(
        &mut self,
        dt: f32,
        frame: &mut FpvFrame,
        world: &mut dyn FnMut(Vec3, Dir3, f32) -> Option<RayHit>,
    ) -> Result<(), String> {
        let drone = self.drone;
        let r = with_world(world, || unsafe {
            (self.api.fpv_drone_tick)(drone, dt, frame)
        });
        self.check("fpv_drone_tick", r)
    }

    /// The overlay's draw data for this frame (also the collision view, with the overlay
    /// closed), copied out of libfpv's buffers.
    pub fn overlay(&self, dt: f32) -> Option<DrawData> {
        let p = unsafe { (self.api.fpv_overlay_render)(dt) };
        DrawData::copy(p)
    }

    pub fn set_overlay_display(&self, width: f32, height: f32, ui_scale: f32) {
        unsafe { (self.api.fpv_overlay_set_display)(width, height, ui_scale) };
    }

    /// The OSD of `frame` as draw data filling a `width` × `height` viewport at its style's
    /// aspect, centred, copied out of libfpv's buffers.
    pub fn osd(
        &self,
        frame: &FpvFrame,
        width: f32,
        height: f32,
        style: ffi::fpv_osd_style,
    ) -> Option<DrawData> {
        let p = unsafe { (self.api.fpv_osd_draw_data)(frame, width, height, style) };
        DrawData::copy(p)
    }

    /// The overlay is open and takes the mouse and keyboard; the game must ignore them.
    pub fn overlay_wants_input(&self) -> bool {
        unsafe { (self.api.fpv_overlay_wants_input)() != 0 }
    }

    /// A key edge, by libfpv's key code (`ffi::FPV_KEY_*` or an uppercase ASCII letter or digit).
    pub fn key(&self, key: i32, down: bool) {
        unsafe { (self.api.fpv_input_key)(key, c_int::from(down)) };
    }

    /// Typed text for the overlay's fields, UTF-8.
    pub fn text(&self, utf8: &str) {
        let s = cstring(utf8);
        unsafe { (self.api.fpv_overlay_text)(s.as_ptr()) };
    }

    /// The cursor, in the overlay's display coordinates.
    pub fn mouse_move(&self, x: f32, y: f32) {
        unsafe { (self.api.fpv_overlay_mouse_move)(x, y) };
    }

    /// A mouse button edge: 0 left, 1 right, 2 middle.
    pub fn mouse_button(&self, button: i32, down: bool) {
        unsafe { (self.api.fpv_overlay_mouse_button)(button, c_int::from(down)) };
    }

    pub fn mouse_wheel(&self, dx: f32, dy: f32) {
        unsafe { (self.api.fpv_overlay_mouse_wheel)(dx, dy) };
    }

    /// libfpv's drone sound (the motors and Betaflight's beeper) for the audio thread: given the
    /// output's sample rate, configures stereo and returns the fill that pulls it
    /// (`fpv_audio_read`, made to be called from an audio thread; INTEGRATION.md "Audio").
    pub fn audio_source(&self) -> impl FnOnce(u32) -> benilla_app::HostFill + Send + 'static {
        let (configure, read) = (self.api.fpv_audio_configure, self.api.fpv_audio_read);
        move |rate| {
            let r = unsafe { configure(rate, 2) };
            if r != ffi::FPV_OK {
                warn!("libfpv: fpv_audio_configure({rate}, 2) = {r}");
            } else {
                info!("libfpv: drone sound at {rate} Hz, stereo");
            }
            // The DLL is pinned for the process (fpv_global_init), so `read` stays valid.
            Box::new(move |buf: &mut [[f32; 2]]| {
                unsafe { read(buf.as_mut_ptr().cast::<f32>(), buf.len() as u32) };
            })
        }
    }

    /// Configures the drone sound for [`Self::audio_read`] on this thread, for a test with no
    /// audio device.
    pub fn audio_configure(&self, rate: u32) -> Result<(), String> {
        let r = unsafe { (self.api.fpv_audio_configure)(rate, 2) };
        self.check("fpv_audio_configure", r)
    }

    /// Pulls the next stereo frames of the drone sound into `buf`.
    pub fn audio_read(&self, buf: &mut [[f32; 2]]) {
        unsafe { (self.api.fpv_audio_read)(buf.as_mut_ptr().cast::<f32>(), buf.len() as u32) };
    }

    /// A button of the game's in the overlay's Fly tab, by our own non-zero `id`; an empty label
    /// removes it.
    pub fn set_overlay_action(&self, id: u32, label: &str) {
        let s = cstring(label);
        unsafe { (self.api.fpv_overlay_set_action)(id, s.as_ptr()) };
    }

    /// The next overlay button clicked, or `None`.
    pub fn poll_overlay_action(&self) -> Option<u32> {
        let id = unsafe { (self.api.fpv_overlay_poll_action)() };
        (id != 0).then_some(id)
    }

    /// Moves the pilot (the LOS camera, the video link's receiver, the ear), host coords of
    /// the feet.
    pub fn set_pilot_position(&self, feet: Vec3) -> Result<(), String> {
        let p = feet.to_array();
        let r = unsafe { (self.api.fpv_drone_set_pilot_position)(self.drone, p.as_ptr()) };
        self.check("fpv_drone_set_pilot_position", r)
    }

    /// Opens or closes the overlay (F8 does it too, inside libfpv).
    pub fn set_overlay_open(&self, open: bool) {
        unsafe { (self.api.fpv_overlay_set_open)(c_int::from(open)) };
    }

    /// A texture's straight-alpha RGBA, width, height and revision, or `None` when it is missing
    /// or still at `known` (no copy then).
    pub fn texture_if_changed(
        &self,
        id: u64,
        known: Option<u32>,
    ) -> Option<(Vec<u8>, u32, u32, u32)> {
        // SAFETY: zeroed out struct for libfpv to fill.
        let mut t: ffi::FpvTexture = unsafe { std::mem::zeroed() };
        let r = unsafe { (self.api.fpv_texture_get)(id, &mut t) };
        if r != ffi::FPV_OK || t.rgba.is_null() || known == Some(t.revision) {
            return None;
        }
        let n = t.width as usize * t.height as usize * 4;
        // SAFETY: libfpv documents `rgba` as width * height * 4 bytes, valid until the next call.
        let rgba = unsafe { std::slice::from_raw_parts(t.rgba, n) }.to_vec();
        Some((rgba, t.width, t.height, t.revision))
    }

    /// A texture's straight-alpha RGBA, width, height and revision.
    pub fn texture(&self, id: u64) -> Option<(Vec<u8>, u32, u32, u32)> {
        // SAFETY: zeroed out struct for libfpv to fill.
        let mut t: ffi::FpvTexture = unsafe { std::mem::zeroed() };
        let r = unsafe { (self.api.fpv_texture_get)(id, &mut t) };
        if r != ffi::FPV_OK || t.rgba.is_null() {
            return None;
        }
        let n = t.width as usize * t.height as usize * 4;
        // SAFETY: libfpv documents `rgba` as width * height * 4 bytes, valid until the next call.
        let rgba = unsafe { std::slice::from_raw_parts(t.rgba, n) }.to_vec();
        Some((rgba, t.width, t.height, t.revision))
    }
}

impl Drop for Libfpv {
    fn drop(&mut self) {
        if !self.drone.is_null() {
            unsafe { (self.api.fpv_drone_delete)(self.drone) };
        }
        unsafe { (self.api.fpv_global_terminate)() };
    }
}

fn result_name(api: &Api, r: ffi::fpv_result) -> String {
    Api::string(unsafe { (api.fpv_result_string)(r) })
}

fn cstring(s: &str) -> CString {
    CString::new(s.replace('\0', "")).unwrap_or_default()
}

/// libfpv's log, which may arrive on its own threads, into benilla's (tracing is thread-safe).
unsafe extern "C" fn log_to_benilla(
    _user: *mut c_void,
    level: ffi::fpv_log_level,
    msg: *const c_char,
) {
    let _ = std::panic::catch_unwind(|| {
        let text = Api::string(msg);
        match level {
            ffi::FPV_LOG_ERROR => error!(target: "libfpv", "libfpv: {text}"),
            ffi::FPV_LOG_WARN => warn!(target: "libfpv", "libfpv: {text}"),
            ffi::FPV_LOG_INFO => info!(target: "libfpv", "libfpv: {text}"),
            _ => debug!(target: "libfpv", "libfpv: {text}"),
        }
    });
}

// ── Collision rays ────────────────────────────────────────────────────────────────────────────

/// What a ray hit: yards along it, the surface normal (facing the ray's origin), and libfpv's
/// material slot.
#[derive(Clone, Copy, Debug)]
pub struct RayHit {
    pub distance: f32,
    pub normal: Vec3,
    pub material: u8,
}

/// The world a tick may cast into, as a type-erased pointer to the caller's `&mut dyn FnMut`.
type WorldPtr = *mut (dyn FnMut(Vec3, Dir3, f32) -> Option<RayHit> + 'static);

thread_local! {
    /// Lent for the length of one libfpv call on this thread (`with_world`); `None` otherwise,
    /// so a ray outside a call is a miss.
    static WORLD: Cell<Option<WorldPtr>> = const { Cell::new(None) };
}

/// Runs `f` with `world` answering libfpv's rays on this thread, then takes it back.
fn with_world<R>(
    world: &mut dyn FnMut(Vec3, Dir3, f32) -> Option<RayHit>,
    f: impl FnOnce() -> R,
) -> R {
    /// Restores the previous lender even if `f` unwinds.
    struct Restore(Option<WorldPtr>);
    impl Drop for Restore {
        fn drop(&mut self) {
            WORLD.with(|w| w.set(self.0));
        }
    }
    // SAFETY: the pointer's lifetime is erased only for the cell; it is put back before `world`'s
    // borrow ends, by `Restore`, and only read inside `f`.
    let ptr: WorldPtr = unsafe { std::mem::transmute(world as *mut _) };
    let _restore = Restore(WORLD.with(|w| w.replace(Some(ptr))));
    f()
}

/// libfpv's raycast callback: the lent world's answer, or a miss. Never unwinds into C: a panic
/// in the world's ray is a miss.
unsafe extern "C" fn raycast(
    _user: *mut c_void,
    origin: *const f32,
    direction: *const f32,
    max_distance: f32,
    out_hit: *mut ffi::FpvRaycastHit,
) -> c_int {
    let result = std::panic::catch_unwind(|| {
        if origin.is_null() || direction.is_null() || out_hit.is_null() {
            return 0;
        }
        let Some(world) = WORLD.with(|w| w.get()) else {
            return 0;
        };
        // SAFETY: libfpv passes three floats each.
        let (o, d) = unsafe {
            (
                Vec3::from_slice(std::slice::from_raw_parts(origin, 3)),
                Vec3::from_slice(std::slice::from_raw_parts(direction, 3)),
            )
        };
        let Ok(dir) = Dir3::new(d) else { return 0 };
        if !o.is_finite() || max_distance.is_nan() || max_distance <= 0.0 {
            return 0;
        }
        // SAFETY: lent by `with_world` on this thread, and alive for the libfpv call we are in.
        let Some(hit) = (unsafe { (*world)(o, dir, max_distance) }) else {
            return 0;
        };
        // Facing the ray's origin, as libfpv wants it.
        let n = if hit.normal.dot(*dir) > 0.0 {
            -hit.normal
        } else {
            hit.normal
        };
        // SAFETY: libfpv's out parameter.
        unsafe {
            *out_hit = ffi::FpvRaycastHit {
                distance: hit.distance,
                normal: n.to_array(),
                material: hit.material,
                object_id: 0,
            };
        }
        1
    });
    result.unwrap_or(0)
}

// ── Draw data ─────────────────────────────────────────────────────────────────────────────────

/// One draw list, copied out of libfpv.
#[derive(Clone, Debug, Default)]
pub struct DrawList {
    pub vertices: Vec<ffi::FpvDrawVertex>,
    pub indices: Vec<u32>,
    pub commands: Vec<ffi::FpvDrawCmd>,
}

/// libfpv's draw data (overlay or OSD), owned.
#[derive(Clone, Debug, Default)]
pub struct DrawData {
    pub display_size: [f32; 2],
    pub lists: Vec<DrawList>,
}

impl DrawData {
    fn copy(p: *const ffi::FpvDrawData) -> Option<Self> {
        if p.is_null() {
            return None;
        }
        // SAFETY: libfpv's draw data stays valid until its next render call; every slice below
        // is the count libfpv gives with its pointer.
        unsafe {
            let d = &*p;
            let lists = slice(d.lists, d.list_count)
                .iter()
                .map(|l| DrawList {
                    vertices: slice(l.vertices, l.vertex_count).to_vec(),
                    indices: slice(l.indices, l.index_count).to_vec(),
                    commands: slice(l.commands, l.command_count).to_vec(),
                })
                .collect();
            Some(Self {
                display_size: d.display_size,
                lists,
            })
        }
    }
}

/// # Safety
/// `ptr` must point to `n` valid `T`s, or `n` must be 0.
unsafe fn slice<'a, T>(ptr: *const T, n: u32) -> &'a [T] {
    if ptr.is_null() || n == 0 {
        &[]
    } else {
        unsafe { std::slice::from_raw_parts(ptr, n as usize) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call(origin: [f32; 3], dir: [f32; 3]) -> (c_int, ffi::FpvRaycastHit) {
        let mut hit = ffi::FpvRaycastHit::default();
        let r = unsafe {
            raycast(
                std::ptr::null_mut(),
                origin.as_ptr(),
                dir.as_ptr(),
                10.0,
                &mut hit,
            )
        };
        (r, hit)
    }

    #[test]
    fn a_ray_outside_a_call_is_a_miss() {
        assert_eq!(call([0.0; 3], [0.0, -1.0, 0.0]).0, 0);
    }

    #[test]
    fn a_lent_world_answers_and_its_normal_faces_the_origin() {
        // A floor at y = -2 whose collider reports its normal pointing down, away from the ray.
        let mut world = |o: Vec3, d: Dir3, _max: f32| {
            (d.y < 0.0).then_some(RayHit {
                distance: o.y + 2.0,
                normal: Vec3::NEG_Y,
                material: 3,
            })
        };
        let (r, hit) = with_world(&mut world, || call([0.0; 3], [0.0, -1.0, 0.0]));
        assert_eq!(r, 1);
        assert_eq!(hit.distance, 2.0);
        assert_eq!(hit.normal, [0.0, 1.0, 0.0]);
        assert_eq!(hit.material, 3);
        // Taken back after the call.
        assert_eq!(call([0.0; 3], [0.0, -1.0, 0.0]).0, 0);
    }

    #[test]
    fn a_panicking_world_is_a_miss_and_does_not_unwind() {
        let mut world = |_: Vec3, _: Dir3, _: f32| -> Option<RayHit> { panic!("boom") };
        let (r, _) = with_world(&mut world, || call([0.0; 3], [1.0, 0.0, 0.0]));
        assert_eq!(r, 0);
    }

    #[test]
    fn bad_input_is_a_miss() {
        let mut world = |_: Vec3, _: Dir3, _: f32| {
            Some(RayHit {
                distance: 1.0,
                normal: Vec3::Y,
                material: 0,
            })
        };
        with_world(&mut world, || {
            assert_eq!(call([0.0; 3], [0.0, 0.0, 0.0]).0, 0, "zero direction");
            assert_eq!(
                call([f32::NAN, 0.0, 0.0], [0.0, -1.0, 0.0]).0,
                0,
                "NaN origin"
            );
        });
    }
}
