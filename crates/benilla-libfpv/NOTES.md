# libfpv in Benilla: notes

Findings from reading the code, with file:line references. Newest findings go at the top of each section.

**Revisions read:**
- Benilla `e2bad3c` (2026-11-22)
- libfpv `ed4739d` (2026-11-22), version 1.1.3, ABI 1

Line numbers refer to those revisions.

## M3 status: checks pass headless (2026-11-23)

**The bindings** (`src/ffi.rs`):
- **Structs:** every non-optional-build struct in `libfpv.h` is mirrored, 31 in all, `#[repr(C)]`. C enums are their 4-byte integer type, with the values as constants.
- **Functions:** `Api` resolves all 83 DLL exports by name through `libloading` at load, then checks `fpv_abi_version() == 1`.
- **Layout test:** `every_struct_matches_the_c_layout` compares size, alignment and every field offset against `src/ffi_layout.txt`, and fails on any C field the Rust side doesn't mirror. All match. `fpv_frame` is 4624 bytes, align 8, with `sim_time` at offset 8.
  - The table is printed by `tools/libfpv_layout.c`, compiled with MSVC from `D:\VS\Community` against the pinned header.
  - MSVC and the DLL's MinGW-w64 agree for these plain x86_64 C structs.
  - Regenerating it: open a shell with `vcvars64.bat` (the VS Installer folder must be on `PATH` for its `vswhere`), run `cl /std:c11 /I <libfpv>\include /DLIBFPV_STATIC tools\libfpv_layout.c`, then run the exe by full path; cmd here won't search the current folder.
- **Dependency:** `libloading = "0.8"`, approved by the user. It resolves to the 0.8.9 already in the lock.

**The runtime** (`src/runtime.rs`, `Libfpv`):
- **Loading:** `LIBFPV_DLL` names the DLL, else `libfpv.dll` beside the exe.
- **Data folder:** `fpv_shared_data_dir`, which `LIBFPV_DATA_DIR` overrides. Tests point it at a scratch folder so the pilot's `%APPDATA%\libfpv` is never touched.
- **Config:**
  - Coords: `FPV_COORDS_Y_UP_RIGHT_HANDED`, 1.0936133 yd/m.
  - Log: libfpv's log goes to benilla's tracing under target `libfpv`, prefixed `libfpv:`. It's thread-safe and catches panics.
  - About tab: `host_name` "WoW FPV (benilla-libfpv)"; `host_build` is `BuildId::summary()`; `host_url` is the `origin` fork URL. Confirm that URL with the user.
- **The raycast callback:**
  - It's registered once at init, with a null user pointer.
  - Each libfpv call that may cast (create, reset, tick) lends the world through a thread-local for that call only. Outside a lend it answers a miss, so PR #4's casts inside create, reset and set-callback are safe either way.
  - It never unwinds into C (`catch_unwind`, a miss on panic).
  - It rejects null or non-finite input, and flips the normal to face the ray origin.
  - Unit tests cover the miss outside a call, a hit with the flipped normal, a panic, and bad input.
- **Draw data** is copied out of libfpv's buffers each call.
- **`src/draw_cpu.rs`** rasterises draw data on the CPU onto a captured frame (vertex colour × texture, straight alpha, clip rects). It's an instrument for x-ray captures until the M5 renderer.

**Headless flights** (`src/flight_test.rs`): `benilla-libfpv --worldview --flight-test <park|wall> [--capture <dir>]`.
- **Setup:**
  - `FPV_TIMING_MANUAL` at a fixed 1/60 s per frame, with `NO_DEVICE_INPUT | NO_CONFIGURATOR | NO_LENS_DISTORTION`.
  - Each frame: `fpv_input_set_channels`, then `fpv_step`, then `fpv_drone_tick`, the order `examples/minimal.c` uses.
  - Frames are skipped until `sim_time` is 50 ms past its first reading.
  - The drone is the default preset, "Freestyler (5" 6S)", on a fresh data folder. It's made on the Trade District street, from a ground ray.
  - The camera follows `frame.chase_camera` (FOV through `FovZoom`, near plane through `set_nearclip`), ordered `.after(WorldStage::Input).before(CameraPoseSet)` per M1.
- **`park` PASS:**
  - Disarmed drift over 1–6 s: 0.000 m (limit 0.05 m).
  - With the throttle raised while disarmed, `arming_disable_text` = `"THROTTLE"`. Throttle down, it reads `""`, and the drone arms 0.02 s after AUX1 goes on.
  - Armed-idle drift: 0.000 m.
- **`wall` PASS:**
  - The wall: the nearest of 32 headings where level rays at 0.3, 0.8 and 1.5 yd all hit an upright face within 0.75 yd of each other. The drone is made 6 yd short of it and pushed with `fpv_drone_push` at 10 m/s level plus 4 m/s up.
  - The result: an impact at 8.93 m/s, normal `[-0.36, 0.20, -0.91]` facing back. It got no closer than 5.1 cm (0.056 yd) to the face, measured every frame at its own height and line. It rebounded and rested 5.90 yd along.
  - Two earlier wall candidates taught the finder: a street rising ahead (at 1 yd up the "wall" was the slope), and a low curb under the 0.3 yd probe (the drone stopped at it correctly, with libfpv's planes on the curb).
- **X-ray:** with `LIBFPV_COLLISION_VIEW=xray` and `--capture`, `wall-impact-xray.png` shows the legend "no triangles: the game uses the raycast callback", yellow raycast planes on the wall's face at the strike, and planes on the floor under it.
- **Deterministic:** two runs of each scenario gave identical per-half-second logs (park 20 lines, wall 8, 0 differing).
- **Prebuilt check:** `fpv_minimal.exe` from the build folder also flies headless (arms at about 2.75 s sim, climbs).

**Gates:** `scripts/check.sh` escalated to `gates.sh` again, because `Cargo.lock` gained `libloading`.
- **Passed:** fmt, deviation-why, clippy, test, doctest, test-no-install, player-clippy, player-tests, enforcer, enforcer-no-install, plus `benilla-libfpv --worldview` under `WOW_WORLDVIEW_CHECK=10`.
- **Not run:** the Python gates (doc-links, message-catalog, pass-span-lint), with no Python here, and reference, with no benilla-re checkout.

**Open for later:**
- **Q3b** (the player's own collider) can't arise in the worldview, which has no player. Settle it in M8.
- **Materials:** not used yet. Slot 0 is libfpv's default (friction 0.3). INTEGRATION.md advises about 0.6 for stone and grass, and `FPV_MATERIAL_KILLS_PROPS` for water via benilla's Liquid layer. Decide after M4 flights.

## M2 status: done (2026-11-23)

**The binaries:** the user chose a prebuilt Windows build, so WSL is skipped. It's at `C:\Users\<user>\Documents\wow-libfpv\libfpv-win64`.
- **Files:** `libfpv.dll`, `libfpv.def`, `fpv_standalone.exe`, `fpv_minimal.exe`, `libfpv.LICENSE.txt` (GPL-3.0) and `README.txt`.
- **The build:** "libfpv 1.1.3 (commit ed4739d), x86_64 Windows build, cross-compiled with MinGW-w64 posix 13.2".
- **Verified:** the DLL's own strings carry `ed4739d72e2d04091a0e30b2a908864a00b678e0` and `1.1.3`.

**The source:** `D:\benilla\libfpv`, cloned from `https://github.com/hammerill/libfpv` and checked out at `ed4739d72e2d` ("Make websocket listen on localhost only (#11)", 2026-11-22).
- Submodules are left uninitialized. The bindings need only `include/libfpv.h`, and the DLL is prebuilt.

**The header matches the DLL:**
- `LIBFPV_ABI_VERSION 1` (`libfpv.h:68`).
- All 83 exports in `libfpv.def` are declared in the header.
- The header's 9 further functions are compiled only with an optional build flag (`libfpv.h:16-24`). That flag is off in this build, so M3 binds only the 83.

**Licence settled:** `libfpv.h:5` reads `GPL-3.0-or-later OR MIT`; a default build is GPL-3.0-or-later (`:7-11`, `README.md:295-303`). The crate's `GPL-3.0-or-later` is right.

**Checked by the user:** they flew `fpv_standalone.exe` with the RadioMaster Pocket after the F8 radio setup, and checked the camera, respawn and Esc keys. The keys: F8 is radio setup, V the camera, R respawn and Esc quit. Betaflight Configurator connects manually at `ws://localhost:5761`.

## M1 status: done, pending the user's live watch (2026-11-22)

**What was built**, on branch `libfpv`:
- **Fork change 1:** `benilla_world::worldview::run_with(build, extend)`. `run` is now `launch(build, None)`. The extension runs on the fully built viewer, just before `app.run()`, and marks the build `extended`, as the client's `run_with` does (`benilla-app/src/lib.rs:223`, `:267-270`).
- **The crate:** `crates/benilla-libfpv`, GPL-3.0-or-later. Check that licence against libfpv's own LICENSE in M2; the clone isn't on this PC.
  - The binary runs `--worldview` (via `run_with`), or the full client by default.
  - The library holds the plugins.
  - It sits inside Benilla's workspace, so path dependencies pick up the root `[patch]` and `[profile]`. Cargo ignores those sections in a member manifest, so `extended_launcher.rs`'s copies apply only to a crate outside the workspace.
  - Dependencies: `benilla-app`, `benilla-world`, `benilla-assets` (for `wow_to_bevy`, which `benilla-world` doesn't re-export) and `bevy`. All are already in the workspace and lock, so no new third-party crates.
- **The roll test** (`src/roll_test.rs`):
  - `--worldview --roll-test <site>` flies the path live, forward and back, rolling the whole time.
  - `--capture <dir>` instead writes a still burst plus flat and rolled pairs at 9 poses, every 45°. The rolled frame is turned back upright and masked to the central disc, then diffed with `benilla-visual diff-dir`.
  - Sites: `streets`, `cathedral`, `open` (Westfall) and `over-city`.
  - The eye is set by a ground probe through `WorldCollision::ray_body`. A site with no heading takes the clearest of 32 level rays.
  - It logs each change of `CameraInteriorClaim`, with the group's path, flags and bounds.

**The camera contract, the main finding:**
- A plugin that owns the world camera must write `Transform` **after `WorldStage::Input` and before `benilla_world::view::CameraPoseSet`.**
- `publish_camera_pose` (`view.rs:406-411`) copies `Transform` into `GlobalTransform` early, so the same frame's PVS (`wmo_portal/mod.rs:254-262`), interior claim and model visibility (`model_render/visibility.rs`) key on it. It is ordered only `.after(WorldStage::Input)`.
- If the writer is unordered against it, some frames get culled for the previous writer's view, here the worldview's `fly` (`worldview.rs`, which rewrites rotation every frame).
- **Measured:** with the camera still, before the fix the frame flipped between two states every few frames (`benilla-visual flicker` on 8 shots: mean 4.45, 3.5% of pixels, a tree and a building behind the Trade District houses popping). After `.before(CameraPoseSet)`: max swing 4, 0.00%.
- The same contract applies in the full client (M4 and M8): order after the rig's `seat_camera` and `fly_free` and before `CameraPoseSet`.

**Roll results** (1600×900, `play` profile, the RX 7700 XT):

| Site | Still burst (8 shots) | Pairs at 0°, 90°, 180°, 270°, 360° (MAE) | Pairs at 45° multiples (MAE, % px > 8) |
|---|---|---|---|
| `streets` (Trade District, eye z 96.6, heading 45) | max swing 4, 0.00% | 0.000 / 0.14 / 0.12 / 0.08 / 0.000 | 0.50–0.60, 1.1–1.3% |
| `cathedral` (nave, eye z 113.9, auto heading 157.5°, 60 yd clear) | max swing 28, 0.05% (candle flames) | 0.001 / 0.06 / 0.10 / 0.09 / 0.001 | 0.38–0.40, 0.8% |
| `open` (Westfall, eye z 83.5, 30 yd up) | max swing 55, 0.02% | 0.001 / 0.10 / 0.11 / 0.08 / 0.002 | 0.51–0.62, 1.7–2.2% |
| `over-city` (Trade District, eye z 194) | max swing 103, 0.08% | 0.008 / 0.02 / 0.01 / 0.18 / 0.000 | 0.10–1.05, 0.5–2.0% |

- Read from the diff images: the 45° residue is edge and alpha-test foliage outlines from bilinear resampling. No object is missing or added in any rolled pair, and sky, clouds, fog and the horizon stay consistent.
- **Roll needs no fix in Benilla.** This confirms Q4 from source.
- Not yet covered: weather, particles and billboard-heavy scenes; none was in frame. Check them in M4 live flights.

**Open issue I-1, the camera over Stormwind drawn as fog.** This is the M0 blue screen, now measured. It's Benilla's own, pre-existing, and not caused by roll (the over-city pairs diff as clean as the rest).
- **Measured:** from z 400 down to the street (z 96.6) at (-8830, 620), `CameraInteriorClaim` is group 285 of `world/wmo/azeroth/buildings/stormwind/stormwind.wmo`.
  - Flags `0xaa41`: INTERIOR `0x2000` and EXTERIOR_LIT `0x40`, no EXTERIOR `0x8`.
  - Model-space bounds z −28.3..43.5.
- **Why it claims from that high:** the down-ray's broad phase (`wmo_portal/seed.rs:117-124`) bounds the group's column in XY and requires only its bottom at or below the eye. The 1760 yd ray, cited as the client's `0x6be250`, takes the highest face below.
- **What it looks like:** at street level the claim draws correctly. At z 194 only the nearest roofs and some trees show, and the rest is fog colour. Hypothesis, not read yet: a claimed camera draws with the building's MFOG (`WmoPortalInstance` interior-fog gate, `wmo_portal/mod.rs:179-181`), whose end distance is far shorter than the drop to the ground.
- **Matters for:** any drone climbing over a city.
- **Not fixed:** it's Benilla core and a fidelity question. Settle it against the reference first: does 1.12.1 claim a camera 100 yd above group 285? A Stormwind gryphon departure in the real client is a natural witness. Then raise it upstream with this repro: `benilla-libfpv --worldview --roll-test over-city`, or `benilla-worldview` with `WOW_WORLDVIEW_AT=-8830,620,140`.

**Gates** (`scripts/check.sh`, which escalated to `scripts/gates.sh` because the new crate changes `Cargo.lock`):
- **Passed:** fmt, deviation-why, clippy, test (1262 s), doctest, test-no-install, player-clippy, player-tests, enforcer, enforcer-no-install, plus `benilla-libfpv --worldview` under `WOW_WORLDVIEW_CHECK=10`.
- **The first round failed clippy** on the `launch` signature in `worldview.rs` (`type_complexity`). It was fixed with an `Extension` alias, as in `benilla-app`.
- **Not run:**
  - doc-links, message-catalog and pass-span-lint: they're Python scripts, and Python isn't installed.
  - reference: no `benilla-re` checkout.
- **Data-gated tests:** the gate found the install but no addon corpus, so it didn't set `BENILLA_REQUIRE_DATA`.

**Environment for M2:** WSL isn't installed and isn't needed: the user supplied a prebuilt Windows build (see M2).

## M0 status: done (2026-11-22)

- **From source:** Q1–Q6 answered below. Q7 is partly answered, Q8 needs the WoW install, and Q9 is answered by the user.
- **No drift:** upstream `samwhosung/benilla` main is still `e2bad3c` (`git ls-remote`, 2026-11-22). Spot-checked citations below against the tree, and all still match: `lib.rs:223`, `:232`, `collision.rs:135`, `:157-161`, `:241`, `view.rs:86`, `:246`, `:250`, `:290`, `:302`, `:333`, `:343`, `worldview.rs:51`, `state.rs:336`, `controller.rs:482-485`, `camera.rs:1685`, `:2025`, `coords.rs:10-17`, `movement.rs:10-11`, `one_sided.rs:233`, `terrain_stream.rs:373`.
- **The user's PC:**
  - Windows 10 Education 19045, Ryzen 5 7600X, 31 GB, Radeon RX 7700 XT (Vulkan, driver 26.10.1).
  - Rust 1.98.1 (from `rust-toolchain.toml`) with the MSVC host.
  - The install is reached through the repo-root `WoW` link to `D:\benilla\Nostalrius\World of Warcraft Classic`, and `WOW_DATA` is unset. `WoW.exe` is 1.12.1.5875.
- **Build:** `cargo build --profile play -p benilla-worldview -p benilla` passes, taking 7 min cold.
- **Worldview runs:**
  - `WOW_UNATTENDED=1 WOW_NOSOUND=1 WOW_WORLDVIEW_CHECK=40 target/play/benilla-worldview.exe` exits 0 with "WORLDVIEW_CHECK ok". The log has no WARN or ERROR lines.
  - A shot at the default anchor (Northshire) renders terrain, the abbey, trees and fog correctly.
  - A shot outside the Stormwind gate (`WOW_WORLDVIEW_AT=-9120,420,110`) renders the walls, the gate and the city behind them correctly.
- **Open issue for M1, a blue screen over the Trade District.**
  - **Repro:** `WOW_WORLDVIEW_AT=-8830,620,140` (camera at about z 200) and `-8830,620,300` (z 360).
  - **What shows:** a flat blue-grey screen with a few faint trees and no terrain or city WMO.
  - **What it isn't:** streaming. It follows the camera in the worldview (`terrain_stream.rs:704`, `ViewFocus::camera()`), and the gate shot proves the data loads.
  - **Suspects:** an interior or underwater decision made over the city WMO's bounds: WMO interior/portal fog, or the liquid submersion test (`liquid/query.rs:668-704`). Neither is read yet.
  - **Why it matters:** M1's first two test sites are Stormwind streets and the cathedral, so pin it down there first. Check whether the stock client shows the same view before calling it a bug; it may be a camera position the reference also treats as inside.
- **Q3 confirmed:** `CollisionLayer` (`collision.rs:41`) and `camera_filter` (`:166`) are `pub(crate)`. A drone filter that includes the WMO camera collider is a fork change.

## Architecture decision: a crate on top, plus a small fork

Benilla's rules say a feature 1.12.1 doesn't have lives in a crate of its own, started through `benilla_app::run_with`, or in a fork. See `docs/CONTRIBUTING.md:38-51` and `crates/benilla-app/examples/extended_launcher.rs`. The integration goes in its own crate, `crates/benilla-libfpv`, and touches Benilla's crates only where a crate on top can't reach.

**What a crate on top can reach today:**
- `benilla_app::run_with(build, extend)` adds plugins to the full client (`crates/benilla-app/src/lib.rs:223`).
- `benilla_app::config_dir()` gives the crate's own state folder (`lib.rs:232`).
- `benilla_world::collision::WorldCollision` is a public `SystemParam`, and `ray_body` is public (`collision.rs:135`, `:241`).
- `benilla_world::view::FovZoom` and `ViewDistance::set_nearclip` are public (`view.rs:333`, `:250`). That's enough to set the FOV and near plane without touching Benilla. See Q5.
- `benilla_world::view::WorldCamera` marks the world camera (`view.rs:290`).

**What it can't reach (needs fork changes):**

1. **Offline flying. Done in M1:** `benilla_world::worldview::run_with`. Before, `benilla_world::worldview::run` built its app internally and took no extension (`worldview.rs:51` at `e2bad3c`).
2. **Detaching the body in the full client.** `Player` is `pub(crate)` (`benilla-app/src/player/state.rs:336`). Its `detached` flag is only toggled by the dev chord (`player/controller.rs:482-485`). Detached is what makes streaming follow the camera (Q6). Proposed: a small public resource or event that asks for detach and attach. Needed from M4 onwards in the full client.
3. **Owning the camera in the full client.** The rig writes the camera `Transform` in `seat_camera` (`player/camera.rs:1685`) and `fly_free` (`:2025`). A plugin ordered after those can overwrite the `Transform` in the same frame. Try that first; ask for a hook only if ordering isn't enough. The M1 worldview showed the ordering works, provided the writer also runs before `view::CameraPoseSet` (see M1).

Keep each fork change small and separate, so rebasing on new Benilla stays easy.

## Q1. Coordinates: answered

- **libfpv's model** (`libfpv.h:118-159`): internally metres, Y up, X right, Z forward. The host names its right, up and forward axes and `units_per_meter` (`fpv_coord_system`, `:141-146`).
- **Preset:** `FPV_COORDS_Y_UP_RIGHT_HANDED` is X right, Y up, −Z forward (`:149-150`). That's exactly Bevy.
- **WoW vs Bevy axes:** `wow_to_bevy(p) = (-p.y, p.z, -p.x)` (`benilla-assets/src/coords.rs:10-17`). WoW +X is north and +Y is west. So Bevy −Z is north, +X is east, and +Y is up.
- **North:** libfpv uses the host forward axis as north and the right axis as east (`libfpv.h:927-933`). The preset's forward (−Z) is already WoW north.
- **Use:** `fpv_coords_preset(FPV_COORDS_Y_UP_RIGHT_HANDED, 1.0936133)`. That's yards per metre (1 / 0.9144).

## Q2. Frame and camera fields: answered

**`fpv_camera`** (`libfpv.h:455-475`):
- `position[3]`, `rotation[4]`, and unit vectors `forward[3]`, `up[3]`, `right[3]`.
- `fov_y_degrees`, `aspect_hint`, `near_plane` (host units).
- `distortion_k1`, `distortion_k2`, `distortion_zoom`.

The rotation maps the host's forward axis to the view direction. For a −Z-forward host that is exactly the camera's world rotation (`:126-129`), so in Bevy the camera `Transform.rotation` is that quaternion as-is.

**`fpv_frame`** (`:678-738`):
- `struct_size`, `sim_time` (a double), `flags`, `arming_disable_flags`, `arming_disable_text[96]`.
- `position`, `rotation`, `velocity`, `angular_velocity`, `speed_kmh`, `altitude`.
- Three cameras: `camera`, `chase_camera`, `los_camera`.
- `motors[4]`, `battery`, `video`, `impact_count` and `impacts[16]`.
- The OSD grid (`osd[16*30]`) and `osd_revision`.
- `channels[10]`, `wind[3]`, portal and cargo fields, and `video_feed`.

**Flags** (`:504-538`): ARMED, FAILSAFE, CRASHED, BEEPER, PAUSED, ON_GROUND, MODE_ANGLE, MODE_HORIZON, TURTLE, OSD_DISABLED, RADIO_ACTIVE, HEARD_FROM_PILOT.

**Rules:**
- Only one drone can exist per process. A second `fpv_drone_create` returns `FPV_ERROR_DRONE_EXISTS` (`:97-98`, `:381-383`).
- Structs carry `struct_size`. Set it with the `*_defaults()` helpers or `sizeof` (`:47-49`).

## Q3. Ray hits: answered

**`fpv_raycast_hit`** (`libfpv.h:328-333`):
- `distance` (host units), `normal[3]` (facing the ray origin), `material` (u8), `object_id` (u32).
- **The normal is required:** each hit becomes a temporary plane.
- **The `_ex` variant** (`:345-360`) adds `struct_size`, `flags`, and an object pose for moving things.

**When the callback runs:**
- The header says it runs on the host thread inside `fpv_drone_tick`, never on the simulation thread (`:324-327`). The code agrees: the only call site is `src/client/frame.cpp:507`, reached from the tick.
- PR #4 would also run it inside `fpv_drone_create`, `fpv_drone_reset` and `fpv_world_set_raycast_callback`. It isn't merged at `ed4739d`. Write the Rust side so either behaviour is safe.
- General rule: callbacks run on the thread that called into libfpv, except the log callback, which runs on libfpv's threads (`:33-40`, `:172-173`).

**Benilla side:**
- `ray_body` returns avian's `RayHitData`, which includes `normal` (`collision.rs:241-243`).
- It is **one-sided**: it skips triangles facing away from the ray (`collision/one_sided.rs:233-298`). That matches the reference, but it means a drone that ever ends up inside a wall won't collide from there.
- It uses `body_filter()`: the Default layer (terrain, doodads, GameObjects) plus the WMO **walk** collider (`collision.rs:157-161`). The walk collider skips WMO DETAIL faces; the camera collider keeps them but skips NOCAMCOLLIDE faces (`collision.rs:1-5`, `:45-48`).
- A drone ideally hits both. That needs a filter of Default + Walk + Camera. `one_sided::cast_ray` and the layer enum are `pub(crate)`, so this is either a fork change (a public `ray_drone`) or raw avian layer bits. Start with `ray_body`.
- **Liquid:** a separate layer (`collision.rs:49-53`). Use it later for `FPV_MATERIAL_KILLS_PROPS` on water.
- **Not yet checked:** whether the player's own unit has a collider on the Default layer (`entities/display.rs:527-539` builds model hulls). If it does, skip it in the callback, as INTEGRATION.md advises (`:1271-1275`).

## Q4. Readers of the camera's yaw and pitch: answered, roll looks safe

- **`FlyCam`** is only read inside the player module: `player/camera*.rs`, `player/controller.rs`, `player.rs`, and the probe `capture/probe_cam.rs`.
- **Culling is built from the full matrix.** `benilla-world` culls with `Frustum::from_clip_from_world`, built from `proj.get_clip_from_view() * cam_t.to_matrix().inverse()` (`exterior_cull.rs:104-127`, `:155`, `:295-296`). Roll is included.
- **Billboards** use the camera's `forward`, `right` and `up` (`billboard.rs:313`, `:434`, `:554`).
- **The submersion test** uses the camera's full rotation (`liquid/query.rs:668-704`).

Nothing found assumes a level horizon, so M1 is expected to pass. It still has to be shown: sky and clouds weren't read line by line.

## Q5. Projection: answered

- **Worldview:** the projection is created in `spawn_view_camera` (`worldview.rs:255-290`).
- **Every frame, two systems rewrite it:**
  - `stamp_world_fov` sets `fov = world_fovy(aspect) * FovZoom` (`view.rs:343-363`). Bevy's `fov` is vertical, like libfpv's.
  - `stamp_near_clip` sets `near = ViewDistance.nearclip` (`view.rs:302-316`).
- **Drone mode, FOV:** set `FovZoom = fov_y_radians / world_fovy(aspect)`. Don't write `Projection` directly; the stamps would overwrite it.
- **Drone mode, near plane:** `ViewDistance::set_nearclip(v)` clamps to `NEARCLIP_RANGE = 0.01..=0.33` yd (`view.rs:86`). That covers libfpv's 0.02–0.2 m (0.022–0.219 yd).
- **Far plane:** `CAM_FAR` = 3000 yd (`view.rs:246`).

## Q6. Streaming: answered

- **What it follows:** `ViewFocus` decides (`terrain_stream.rs:320-395`; `detached` is at `:373`). With a live body attached, streaming follows the body. **Detached (free-fly), it follows the camera**, and so does a cinematic.
- **Who sets it:** `publish_view_focus`, from `Player.detached` (`player/world_focus.rs:55-110`). The worldview uses `ViewFocus::camera()`.
- **Consequence:**
  - In the worldview, terrain already streams around the drone.
  - In the full client, detaching the body during a flight does the same (fork change 2 above). That's the client-side half of libfpv's stand-in.
  - The server-side half is M8.

## Q7. Movement packets: partly answered

- The protocol models `JUMPING` (0x2000, with a fall/jump tail) and `SWIMMING` (0x200000) (`benilla-protocol/src/messages/movement.rs:10-11`, `:25-26`).
- 1.12 has no flying flag. How vmangos treats a character holding a position in the air without movement flags is still unknown. That's an M8 question: test with GM mode and read vmangos's movement handler.

## Q8. Key conflicts: not answered

- The stock keybindings come from the player's own `Bindings.xml` via FrameXML. They aren't in the repo, so check them against the install in M5.
- **libfpv's standard keys** (INTEGRATION.md `:1385-1435`, `:558-584`):
  - F8: overlay
  - F7: launch, or back to the drone
  - F: back to the pilot, drone left flying
  - Y: move here
  - Enter, W/S/A/D and the arrows: keyboard flying
  - Esc: closes the overlay
- **Benilla's own:** free-fly is on the dev chord + F (`controller.rs:482`), not bare F.

## Q9. libfpv version pin: answered (2026-11-22)

- **The user's answer:** they run no other libfpv game. gtasa-fpv was a proof of concept they found online and don't use.
- **So nothing else uses `%APPDATA%\libfpv`**, and there's no other game's Betaflight to match (`libfpv.h:235-247`).
- **Decision:** pin the local clone at `ed4739d` (1.1.3), and set `data_dir` from `fpv_shared_data_dir()`, the default.
- **Revisit** if the user installs another libfpv game later. Either pin that game's commit, or give Benilla its own data folder.

## Other findings for later milestones

- **Lens:** INTEGRATION.md `:8-52` has the shader.
  - Benilla's `final_pass.rs` is plumbing for the FFXGlow combine, not a ready-made lens hook. The lens is probably its own post-process on the world camera; decide in M6.
  - `docs/renderers/fpv_video_gl3.h` does the lens plus the optional video-feed effects (`FPV_INIT_VIDEO_FEED`).
- **Overlay and OSD** (`libfpv.h:1363-1462`):
  - Vertices are pixel position (top-left origin), UV, and colour as RGBA8 with R in the lowest byte. Commands have a clip rect, a texture id (1 = overlay font, 2 = OSD font), and index/vertex offsets.
  - Textures are straight-alpha RGBA with a revision counter.
  - Call `fpv_overlay_render` every frame, even with the overlay closed, because the collision view draws through it (INTEGRATION.md `:1122-1134`).
  - Set the display with `fpv_overlay_set_display(w, h, ui_scale)`.
- **Audio** (`libfpv.h:1471-1490`): `fpv_audio_configure(rate, 2)`, then `fpv_audio_read` gives **interleaved floats**. The guide's 16-bit figure is about its waveOut example. Benilla's mixer is kira (patched; see `extended_launcher.rs`).
- **Pilot** (`libfpv.h:896-922`, INTEGRATION.md `:552-635`, `:1490-1623`): call `fpv_pilot_set_support(FPV_PILOT_STAND_IN | FPV_PILOT_POSE, 20*upm, 10*upm)` once after init. Read the player's choice with `fpv_pilot_get_config`. The stand-in is expected on by default in every game with a character.
- **No game UI in FPV** (INTEGRATION.md `:327-376`): hide all of Benilla's FrameXML UI over the FPV view.
- **About tab** (`libfpv.h:224-230`, INTEGRATION.md `:1014-1038`): fill in `host_name`, `host_build` (generated at build time) and `host_url`.
- **Naming** (INTEGRATION.md `:1040-1070`): in-game identifiers, the crate, folders and log tags use the word `libfpv`. Players' visible name can say "WoW FPV".
- **Licensing** (`libfpv.h:7-11`, INTEGRATION.md `:1071-1120`):
  - The default libfpv build is GPL-3.0, so a crate linking it must ship under GPL-3.0-compatible terms. Benilla's MIT/Apache code is compatible.
  - A split build (`LIBFPV_SPLIT_PROCESS`) puts Betaflight in a separate `libfpv-server` process. The library itself is then MIT as well.
- **Presets:** `assets/presets/quad/betafpv-air65-ii.json` with its own `.diff.txt` is the closest to the Air75 II. Also `fluid75.json`.
- **Diff import:** `fpv_import_betaflight_diff` (`libfpv.h:1502-1506`) imports a real quad's `diff all`. It forces PWM and ARM on AUX1.
- **Testing:** `FPV_TIMING_MANUAL` plus `fpv_step` (`libfpv.h:180-183`, `:1235-1237`) gives deterministic headless tests.
- **Benilla's run rules** (AGENTS.md, `docs/METHOD.md`):
  - Unattended runs set `WOW_UNATTENDED=1` and `WOW_NOSOUND=1`.
  - Scripted logins use `.probe-identity`, never the player's account.
  - The WoW install is read-only.
  - Gates are `scripts/check.sh` per round and `scripts/gates.sh` before landing.
