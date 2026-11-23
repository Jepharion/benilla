# libfpv in Benilla: notes

Findings from reading the code, with file:line references. Newest findings go at the top of each section.

**Revisions read:**
- Benilla `e2bad3c` (2026-11-22)
- libfpv `ed4739d` (2026-11-22), version 1.1.3, ABI 1

Line numbers refer to those revisions.

## M8 status: built and checked unattended against the local vmangos; waiting on the user's flights (2026-11-23)

**The local server (set up 2026-11-23), outside the repo:**
- vmangos `c13b2217` (the user's Windows build, MySQL) in `D:enillavmangosRelease`, with configs from that revision's `mangosd.conf.dist.in` and `realmd.conf.dist.in`: everything on 127.0.0.1, `DataDir = "../data"`, `mmap.enabled = 0` (no NPC pathfinding until mmaps are generated, which takes hours), remote admin on 127.0.0.1:3443. Warden is off and `Anticheat.Enable = 0` by default (relevant to Q7).
- Map data extracted from the 1.12 install (read only): `datamaps`, `data875dbc` (this build wants dbc under the build number), `datavmaps`.
- MariaDB 10.11.10 portable in `D:enillamariadb` (the user chose it over an installer; checksum verified), localhost:3306, with the user's dumps from `D:enillaDBmysql-dump` (world migrations up to 2026-11-21).
- Realm 1 "Benilla libfpv", build 5875. Test account `libfpvtest`, GM level 5 (the highest any command needs), character `Fpvpilot` (human warrior, Northshire), created with `WOW_PROBE_CHARCREATE`. Passwords are in `D:enillamariadbCREDENTIALS.txt` and `D:enillavmangosTEST-ACCOUNT.txt`; `.probe-identity` names the account.
- `D:enillavmangosstart-server.cmd` starts all three. A scripted login as `Fpvpilot` reached the world and held 90 s.
- A `mangosd` whose console input ends quits, so a script must not start it with stdin from a file.

**Done:**
- **Fork change 2:** `benilla_app::HostBody` (`benilla-app/src/player/host_body.rs`), a public resource. `request: Some(true | false)` detaches or attaches the body. `active`, `detached` and `feet` are the client's, rewritten each frame.
  - It sets the same `Player::detached` the dev chord's free-fly sets (`player/controller.rs:482-485`): the avatar and its server position stay put, and streaming follows the camera (`player/world_focus.rs:94`). That's the "detach" half of M8, with the character as the pilot standing at home.
  - `sync_host_body` runs in `PreUpdate`, so a request applies before the frame's player systems. In `Update` it made six unordered pairs with the other `Player` writers, and the schedule test's ceiling (`game_plugins.rs:982`) refused it. It isn't gated on the world, so out of the world it reads "not active" (a request is dropped then), and with no `Player` at all (a harness) it reads the default. Two unit tests.
- **Fork change 5:** `benilla_app::HostInput` (`benilla-app/src/ui_script/records.rs`), a public resource of the keys (or the whole keyboard) and the mouse a crate on top takes. Every game-side key and click goes through one function, `collect_input_records` (`ui_script/records.rs`): the bindings dispatch (`bindings.rs:1027`) and the UI's key feed (`ui_script/input.rs:207`) both read its `InputRecords`. So it drops the taken records there, presses and releases alike; a cursor move still records. The other keyboard readers are an idle timer (`ui_chat/idle.rs:58`), a loading-screen check and test harnesses. It's an `Option<Res<…>>`, so nothing changes without it. One unit test.
- **The full-client plugin, `client.rs`** (the binary's default mode, `run_with`):
  - **F7** launches: the first time, libfpv starts and the drone is made on the ground 1.5 yd ahead of the character, facing the camera's heading; the character is the pilot (`fpv_drone_set_pilot_position(HostBody.feet)` every frame). Then `HostBody.request = Some(true)`, the UI hides, and the world camera is the drone's (`camera::apply`, the M1 contract: `.after(WorldStage::Input).before(CameraPoseSet)`). In the client every rig writer (`control` in `PlayerControlSet`, cinematics after it) runs inside `WorldStage::Input`, and `publish_camera_pose` runs `.after(WorldStage::Input)` (`benilla-world/src/view.rs:406-411`), so no camera hook (fork change 3) is needed.
  - **F** goes back: the body attaches, the UI shows, the game's `FovZoom` and near plane are put back, the lens comes off; the drone flies on with `FPV_VIEW_PILOT` and the game camera as the ear (`fpv_audio_set_listener`).
  - **R** and **P** in FPV, as in `--fly`. Leaving the world while in FPV goes back to the pilot.
  - **Keys:** F7 and F8 are always taken (stock: F7 free, F8 TOGGLEBAG1); in FPV, or while the overlay wants input, every key and the mouse; a key pressed while taken stays taken until released.
  - **The UI:** the UI camera (Bevy's public `IsDefaultUiCamera`) is pointed at render layer 31, which nothing draws on. Its quads are `Mesh2d` entities on the UI layers (`ui_pass.rs:187`), while the world it composites is a per-camera draw (`FfxBackdrop`, `world_backdrop.rs`), so the world stays and the UI goes. Switching the camera off instead would black out the screen: the UI pass writes the swapchain (`ui_pass.rs:1-6`).
  - The input forwarding is shared with `--fly` (`fly::InputBridge`).
- **Checked unattended** (`LIBFPV_CLIENT_TEST=<dir>`, logged in as `Fpvpilot`): 10 s in the world it launches as F7 does, photographs, goes back as F does, photographs and exits. The log: "host body: detached", view Fpv, every key taken; then "attached", view Pilot, keys back. `client-fpv.png`: Northshire Abbey from the ground through the analog feed and fisheye, Betaflight's boot screen in the OSD, none of the game's UI. `client-pilot.png`: the UI back, the character standing where it was.
- **Seen there, for later:** in the pilot view the drone wasn't drawn in the world. Done below (the drone's model).
- **The user's check of part 1:** F7 works cleanly, no keys leak. Asked for: solid black bars (they were translucent), R respawning at the pilot, three times the video range, and Xiggs Fuselighter's flying machine as the drone's model. Northshire-and-beyond flying is still to be tried.
  - **The bars.** In the client the world camera runs only the glow's blur passes (before the lens). The combine, `lerp(screen, blur, haze) + glow·blur²` (`benilla-world/src/ffx_glow.rs:1-6`), runs as the UI camera's backdrop draw, after the lens. So it added unwarped glow over the bars and the fisheye. The plugin clears `FfxSwitches.master` in FPV and puts the switches back on F; they're written only on a CVar change (`benilla-app/src/cvars/mod.rs`), so nothing overwrites them in between. Measured: the brightest pixel in the left bar went from 57 to 0. A real FPV camera has no WoW glow anyway.
  - **R** respawns on the ground just ahead of the character, at the launch heading (`launch_point`, shared with F7).
  - **Range** (`src/presets.rs`, files in `presets/`). The default 5" already flies a 400 mW VTX (about 3 km in the open, `assets/presets/vtx/tbs-unify-pro32-hv.json`), so in town obstruction is what breaks the picture. Free-space loss grows with distance², so 3× the range is 9× the power, +9.5 dB. `libfpv-wow-long-range-vtx` is that VTX at 3,600 mW (`range` 9,000 m), and `libfpv-wow-freestyler-5-long-range` is the Freestyler 5" with it. They're written into the player's preset folder before `fpv_global_init`, when missing or changed. They're `buildIn: false`, so libfpv keeps them (`assets/presets/README.md`). libfpv's own presets and default are untouched; the player picks "Freestyler (5" 6S, WoW FPV long range)" in F8 > Drone. libfpv reads `powerMw` and `range` at `src/shared/config/preset.cpp:186-205`. Two unit tests.
  - **The model.** "Xiggs Fuselighter's Flyingmachine" is creature 8446, display 8011 → `CreatureModelData` 1091, `Creature\FlyingMachineCreature\FlyingMachineCreature.mdx` (read from the extracted `CreatureDisplayInfo.dbc` and `CreatureModelData.dbc`; Doctor Weavil's Flying Machine, 15553, uses it too). Benilla has no public way to place a model: `benilla_world::terrain_stream::spawn::spawn_model_entities` is the streaming path's, with materials, light, forms and animation state. Next step: a small fork change exposing "this model at this transform", or Benilla's creature rendering by display id.
- **The drone's model** (the user's choice: Xiggs Fuselighter's flying machine; size from the quad flown, always far smaller than a character):
  - **Fork change 6:** `benilla_app::HostModel { display_id, scale }` (`benilla-app/src/entities/host_model.rs`). A `PreUpdate` system gives such an entity the guid-less `NetEntity` a mount child has (`entities/mount.rs:41-60`), and Benilla's ordinary creature path builds and animates it. With no `Guid`, nothing that needs a server object (nameplates, targeting, tooltips, the object list) reaches it. After the build its render scale is its `Transform`'s: only a guid's `SCALE_X` eases in (`entities/live_display.rs:187-205`). One unit test.
  - **`client.rs`:** the first F7 spawns it, display 8011. `fit_model` measures it once its parts' count holds still (the descendants' `Aabb`s in its own frame): 18 parts, 6.15 × 3.83 × 5.16 yd at scale 1. `place_model` puts the centre of those bounds on the drone with the drone's full attitude, at `motor diagonal × 1.55 / width`. The diagonal comes live from `frame.motors[i].position`, so a preset change in F8 resizes it; ×1.55 is the props' reach past the motors (5": 225 mm → 355 mm; a 65 mm whoop with 31 mm props, ×1.48). The Freestyler logs "0.35 m across (motor diagonal 225 mm)". It's hidden in the drone's own view.
  - **Facing:** a unit model faces −Z at zero yaw (`net/motion.rs:41`, `wire_yaw`), as libfpv's drone does, so no correction. A first reading of a half-hidden photo said otherwise; a −90° turn then showed the nose sideways, and without it the photo from behind shows the tail with its two engine pods and the exhaust towards the camera.
  - **Test switches:** `LIBFPV_TEST_MODEL_SCALE` enlarges it and `LIBFPV_TEST_LAUNCH_AHEAD` launches further out, for a photo (`LIBFPV_CLIENT_TEST`).
  - **The user asked about sounds for other players.** A stock client only plays what the server sends or its own data, so other players can't hear libfpv's motors. They won't see this model either; it exists on this client only. What they'll see is the character, once it moves with the drone on the server.
- **The stand-in, and the drone seen by other players** (the user: "Could we also make the drone visible to other players? That would be a big deal"), per INTEGRATION.md "Standing in for the drone" (`:1490-1630`) and "The player's choices" (`:605-640`):
  - **The design.** In FPV, once the drone is past the player's stand-in distance (libfpv's 20 m, ending within 10 m, changeable in F8 > Fly > Pilot; `fpv_pilot_set_support(FPV_PILOT_STAND_IN, ...)` after create), the **server's** character goes with the drone. The world then loads, populates and reacts around it. **This client's** body stays home as itself, which is the pilot stand-in INTEGRATION.md requires, with no extra model.
    - To other players the character is the drone: a GM `.modify morph 8011` (the flying machine) and `.modify scale` at the model's size, then `.demorph` and `.modify scale 1` at the end.
    - Our own body keeps its look meanwhile, and for 3 s after the demorph is sent, so the server's answer doesn't rebuild it.
    - The pilot's view (F), or back within the return distance, ends it. While detached and not standing in, the home position streams, so the server's character is home.
  - **Fork change 7:** `benilla_app::HostChat(String)` (`benilla-app/src/player/host_chat.rs`), a message sent as a /say in `PostUpdate`, as the dev "land" sends `.go xyz` (`player/land.rs:82`).
  - **Fork change 8**, on `HostBody` (`player/host_body.rs`):
    - `stand_in: Option<(Vec3, f32)>` is streamed while detached by `stream_stand_in` (`PreUpdate`, after the sync): a `MSG_MOVE_HEARTBEAT` with no movement flags, at most every 50 ms when it moves or turns, else every 1 s. It sets `last_pos`, so on re-attach Benilla's own at-rest check (`movement_net.rs:274`) sends the body's real position.
    - `keep_look: bool` makes `entities/live_display.rs` skip display swaps and scale eases on our own (`Embodied`) body.
    - `place: Option<Vec3>` sets the body's feet ("move here").
    - `facing` is published.
    - The sync keeps the crate's fields (`..host.clone()`).
    - Two more unit tests: the fields survive the sync; a detached body streams a flagless heartbeat, sets `last_pos`, and stops once attached.
  - **Y** in FPV, or the F8 > Fly button "Move the pilot here (Y)": `HostBody.place` at the ground under the drone, and `fpv_gps_set_home` there. `fpv_gps_set_home(feet)` at launch.
  - **Q7, answered on this server:** flagless heartbeats hold the character in the air: 37 yd up, no fall damage, no correction, no disconnect. With Warden off and `Anticheat.Enable = 0` (the shipped config), vmangos relocates the mover to each one. GM rights are needed only for the disguise.
  - **The server's answers:** `.modify scale 0.1` → "You set the size 0.10 of [Fpvpilot]"; `.modify scale 1` → "size 1.00". `.modify morph` answers nothing, and works (below).
  - **The RBAC tables are empty** (`realmd.rbac_*`), so commands go by GM level. The level is read from `account_access` (the watcher's `.go` was refused at `account.gmlevel` 3 alone and accepted once `account_access` said 3). `account create` is console-only with `Ra.Restricted = 1`, so the watcher's account went in by SQL with its SRP6 verifier computed as vmangos does.
  - **The second client** (`LIBFPV_WATCH=<s>`, `LIBFPV_WATCH_AT=x,y,z`, logged in as `libfpvwatch` / `Fpvwatcher`; `.probe-identity` declares it only while it logs in, then `libfpvtest` again): it logs every other player ten times a second as it draws them.
    - **The run** (`LIBFPV_TEST_LAUNCH_AHEAD=40`, a scripted flight: arm in angle mode, climb, pitch forward): the watcher drew Fpvpilot at home as display 49, scale 1. At the launch it was at the drone (37 m out) as **display 8011, scale 0.1**. It climbed with the drone to 125 yd, evenly: 46 samples at 0.1 s, mean step 0.79 yd, max 1.40 yd, none frozen. After F it was home as 49, scale 1 again. Nothing in the pilot's log about a disconnect or a correction.
    - **The first two runs saw nothing,** and the cause was the layout, not the client: a new character's first login sits in the intro cinematic (`HostBody.active` false), and from Northshire's start the drone was about 127 yd away, outside the server's visibility.
  - **What others see and don't:** position and heading (and the morph and scale), not pitch or roll: a unit has none on the wire. The scale is the model's at the stand-in's start; the first one in the run came before the model was measured, so it used the 0.1 fallback (the measured 5" is 0.062). Whether vmangos takes less than 0.1 is untested.
  - **Not done:** damage to the character as damage to the drone, keeping the character from dying, and NPC aggro. Benilla doesn't see damage per frame here yet. The piloting pose (`FPV_PILOT_POSE`) isn't declared.

**Found for later steps:**
- **Benilla's dev "land"** (`player/land.rs`): after free-fly, the dev chord + G sends a GM `.go xyz` to the camera's spot and waits for the teleport, then re-attaches. For M8's Y ("move here") with GM mode on, that's a working path to move the character to the drone. Q7 decides whether movement packets can do it without GM.

**The first plan for M8, all done above:** the full-client plugin (part 1), the keys (fork change 5), hiding the UI (part 1), the pilot (the stand-in, `fpv_drone_set_pilot_position(HostBody.feet)`, `fpv_gps_set_home`), and the server side with a second client (the stand-in). Left: the piloting pose, damage to the drone, and the user's own flights.


## M7 status: built and pre-checked, waiting on the user's check (2026-11-23)

**The drone's sound** (libfpv's motors and Betaflight's beeper), per INTEGRATION.md "Audio" (`docs/INTEGRATION.md:1649-1700`) and `libfpv.h:1470-1490`:
- **Fork change 4** (commit `e72782be`): `benilla_app::play_host_stream`. See "What it can't reach" below.
- **`Libfpv::audio_source()`** returns what `play_host_stream` calls with the output's sample rate. It runs `fpv_audio_configure(rate, 2)` and returns a fill that calls `fpv_audio_read` on the audio thread, as libfpv intends ("Pull interleaved float samples from your audio thread"). It copies the two function pointers out, valid for the process since `fpv_global_init` pins the DLL.
- **`--fly`** starts it once the drone exists and holds the handle in a resource (`DroneSound`). No listener is set and the view is `FPV_VIEW_FPV`, so the drone is heard onboard. The pilot-view listener (`fpv_audio_set_listener`) is M8's, with F.
- **Volumes:** libfpv's defaults. The player sets the motors and the beeps in F8 > Camera & sound. In the full client the stream also sits under Benilla's master volume and limiter.

**Pre-check, `--flight-test hud`** (no device: the test pulls a frame's worth of sound each frame at 48 kHz, as an audio thread would, and analyses the last half second at each shot):
- **Just after arming** (t 1.5 s, motors 2,300–4,200 rpm): RMS 0.0097, peak 0.015, a square wave. Read as Betaflight's arming beep.
- **Climbing** (t 3.0 s, motors 11,220 rpm, 187 rev/s): RMS 0.0020 (about −54 dBFS), peak 0.012, largest sample-to-sample step 0.0029, so no clicks or crackle, all finite. The autocorrelation pitch is about 676 Hz, between 3 and 4 × the rotor rate, in the range of the props' blade-pass tone.
- **A first try that reconfigured the synth on each pull** read much quieter at hover. Pulling continuously is what live playback does.
- **`--fly` under `$WOW_NOSOUND`:** the stream is requested once the drone is up and Benilla's side reports "silent", so the wiring runs end to end. The device path needs a listener, so it's the user's check.

**For the user's check:** the motors at the right pitch as the throttle moves, no crackle, the arming and other beeps, and the loudness (libfpv's default mix is quiet against a full-scale signal).

## M6 status: done, the user checked the lens; the video feed and the pilot fix wait on their check (2026-11-23)

**The user's check of the lens:**
- It "seems good". The F8 fisheye strength works.
- **Feel:** "think it feels better", to be investigated. The user flies lighter quads than this default 5" (their last preset), so the inertia may feel odd. The Air65 II preset (closest to their Air75 II) is in F8 > Drone; a tuned Air75 II preset is M9.
- **Asked for** the analog static the standalone shows: libfpv's video feed, M6's optional item. Done below.
- **I-3:** the user reads it as the camera on the ground clipping into it at the wider FOV, "not a big deal", since it doesn't happen in flight. Low priority.

**The lens and the video feed** (`src/video.rs`, `src/video_signal.wgsl`, `src/video_present.wgsl`), a port of `docs/renderers/fpv_video_gl3.h` line for line (INTEGRATION.md "Lens distortion" and "The video feed"). It replaced the first lens pass (`lens.wgsl`), and `lens.rs` keeps only the FOV fit:
- **`video::view_for(frame, screen aspect, OsdChipCache)`** gives a `VideoView` (an extracted component on the world camera; `camera::set_view` adds it or removes it) and the FOV to render at.
  - **Modes:** lens only (no feed flags), analog, or digital, each its own pipeline (shader defs `VIDEO_ANALOG`, `VIDEO_DIGITAL`), as in the reference.
  - **FOV:** with the resolution effect on, libfpv's own FOV, the lens working on the camera's aspect (INTEGRATION.md: "so the FOV is exact"). Otherwise M6's `lens::fit` recompute for the screen's aspect.
  - **The OSD chip's picture:** 360 × rows·18, built on the CPU from `frame.osd` and a copy of the OSD font, as `fpv_video_gl3__osd_chip` does, and rebuilt only when the grid, the row count or the font changes (unit-tested).
- **The node** sits on the world camera, between `Tonemapping` and `EndMainPassPostProcessing` (after benilla's FFXGlow, `ffx_glow.rs:1200-1208`):
  - **The signal:** into one of two `Rgba8Unorm` textures at the feed's raster (720 × 240/288 analog, the mode's pixels digital, the screen's with the resolution off). Only when the field (or digital frame) changes, the flags change, or there is no signal yet, so the feed runs at the link's rate. The other texture is the history digital breakup reuses, and a frozen digital frame is held.
  - **Uploads:** the line table (2 × `line_count` texels) for analog noise, and the OSD picture when its `Arc` is new.
  - **The goggles:** every frame, the signal into the picture's rectangle, `display_aspect` (or `picture_aspect`) fitted to the screen with black bars, the "sharp bilinear" scaler, and an analog field bobbed half a line on odd parity.
  - **State lives in a `prepare` system** (`RenderSystems::PrepareResources`): the plan, the current texture, and the uniforms laid out by `ShaderType` as the WGSL structs. The node only encodes the two passes, binding this frame's `post_process_write` source as the scene.
- **Colour space:** the reference does its signal maths on the gamma-encoded picture a GL framebuffer holds. The scene here is linear HDR, so the signal shader encodes it to sRGB as it samples, and the goggles shader decodes it back.
- **Coordinates:** the GL maths is kept (uv and p y-up, lines from the top). Only the samples flip, with `textureSampleLevel` throughout (non-uniform control flow) and `textureLoad` for the line table and the OSD.
- **The analog OSD in the signal:** `FPV_VIDEO_FEED_OSD_IN_SIGNAL` puts it in the picture, and then `fly::publish` doesn't draw it again over the top.
- **Live flight** passes `FPV_INIT_VIDEO_FEED`. The pilot turns the effects on or off in F8 > Camera > "Video feed"; both are on by default. Its log line adds the feed: system, effects, raster, RSSI and CNR.
- **Pipelines** are queued at `RenderStartup`: three signal modes, plus present for HDR and for 8-bit.

**Pre-check, `--flight-test hud --capture`** (the hud test now passes `FPV_INIT_VIDEO_FEED`), default 5" preset, NTSC analog, 1280×720:
- **The picture:** a 4:3 analog picture with black bars at the sides. Soft analog resolution, colour bleeding along the lines, the camera's edge halos, and the barrel lens. It renders at libfpv's 112.5° (the projection reads 112.48°).
- **The OSD:** in the signal, `ARMED` and the elements in the chip's blocky white look, softened with the picture. In `hud-overlay.png` the F8 window is undistorted over it.
- **The noise, with `LIBFPV_TEST_PILOT_M` putting the pilot behind the drone** in the Trade District street: 30, 80 and 130 m show the picture with a faint grain; 170, 250 and 1500 m show the receiver's full snow. The obstruction through the buildings makes the fade steep, so a still didn't catch the torn, sparkling middle; the user sees it live.
- **Without the resolution effect:** the M6 lens-only path is the same shader in mode 0.

**The user's first look at the feed: "unpilotable pretty fast", and the standalone only showed the effects "occasionally, without the range limit".** The link is modelled from the pilot (`fpv_drone_set_pilot_position`, the receiver), and `--fly` left the pilot 2 yd behind the spawn. Stormwind's walled streets block the link within a street or two (the stills above: snow from 170 m behind the buildings).
- **The standalone** sets a pilot only with `--pilot X,Z` (`examples/standalone/main.cpp:29,656,809-812`) and flies an open, flat level, so the link rarely leaves line of sight.
- **The fix, in `fly.rs`:** the pilot follows the drone. Whenever the drone is more than `PILOT_FOLLOW_M` (30 m) from the pilot, the pilot moves to the ground under it (a body ray down, 500 yd reach). The link then fades and breaks up from walls and corners close by, as in a small level, and never runs out of range.
  - **Y**, or the overlay's Fly tab button "Move the pilot here (Y)", moves the pilot there now (libfpv's standard "move here", CLAUDE.md "Standard keys").
  - **The Fly tab's second button** toggles following off, for a pilot who wants range to matter (LOS flying from one spot, M8's stand-in). `fpv_overlay_set_action` / `fpv_overlay_poll_action`.
  - **Not checked live:** the follow needs a moving drone, so the user checks it in flight.

**M6's first lens pass,** kept for the record: `lens::fit` at 16:9 rendered at 114.5° against libfpv's 112.5° at its 4:3 hint. The guide's "about 119°" depends on the preset and the fisheye strength; the formula is unit-tested.

**Open issue I-3, fog-coloured ground at the bottom corners on the ground. Found here, not fixed.** With the drone parked (camera about 6.6 cm over the street) and the wider FOV, the bottom tenth of the frame shows a flat fog colour, `[84, 135, 164]`, instead of the street.
- In Stormwind it's a straight horizontal cut. At Westfall (`LIBFPV_TEST_SITE=open`, drone rolled on a hillside) it's a straight cut parallel to the tilted horizon.
- **Measured, at the same pose, without the warp:**
  - Near 0.33 yd or 0.01 yd: the band is identical. So it's not the near plane: the projection reads the set near (0.057 yd), and the world shaders only discard at the far clip (`terrain.wgsl:169-175`, `static_gx.wgsl:278-283`).
  - FOV 100° (M4's): no band. FOV 114.5°: the band.
  - No liquid under the eye (`WOW_FOG_DUMP`: "no surface covers the eye's XY"). The scene fog there is `[77, 120, 143]`, start 88, end 350.
  - The lens warp only curves the cut.
- **Reading:** rays more than roughly 26° below horizontal, hitting ground a few cm away, show fog instead of the ground. M4's 100° bottom edge stopped at about 25°.
- **Not explained yet:** which benilla pass draws fog there. Candidates are the sky or fog elements drawn over close ground; not read.
- **Impact:** in flight the bottom rays reach distant ground and the frames are clean (`hud-overlay.png`, airborne). The user confirms it doesn't show while flying. Low priority.

## M5 status: done, the user checked it (2026-11-23)

**The renderer** (`src/draw.rs`, `src/draw.wgsl`), after `docs/renderers/fpv_render_gl3.h`. It was rewritten after the user's check (below):
- **The camera:** a `Camera2d` at order 100, marked `FpvOverlayCamera`, with `Tonemapping::None` (tonemapping would shift the UI's colours) and `Msaa::Off`. It clears its own texture to transparent, then composites onto the window with `PREMULTIPLIED_ALPHA_BLENDING`. Straight-alpha triangles drawn over transparent black leave premultiplied colour.
- **The node:** `FpvOverlayNode` sits on the Core2d graph between `EndMainPass` and `StartMainPassPostProcessing`, and runs only on that camera.
  - It uploads libfpv's vertices raw. `fpv_draw_vertex` is a GPU vertex layout as it stands: `Float32x2` position, `Float32x2` uv and `Unorm8x4` colour, a 20-byte stride (unit-tested).
  - Every list goes into one vertex buffer and one index buffer.
  - Each command draws with a real scissor rect (its clip rect scaled to the target's pixels), its texture's bind group and its base vertex.
- **The blend:** libfpv's own, `SrcAlpha / OneMinusSrcAlpha` on colour and `One / OneMinusSrcAlpha` on alpha.
- **Colour:**
  - The vertex shader linearises the sRGB bytes of the colour.
  - The textures are `Rgba8UnormSrgb`, uploaded once per `fpv_texture_get` revision. The OSD font uses a nearest sampler, as the reference does; the overlay font is linear.
  - Blending runs in linear space, where the GL reference blends in sRGB, so edges are slightly softer. Accepted.
- **The pipeline:** queued at `RenderStartup`, so it compiles behind the loading, never on the frame that first shows the overlay.
- **The draw:** `fly::publish` fills `FpvDraw` each frame: the OSD (`fpv_osd_draw_data`, in the pilot's style from `frame.video_feed.osd_style`, analog for an unknown value), then the overlay (`fpv_overlay_render`, every frame). Uploads are cleared in `First`, after the render world extracted them. `fpv_overlay_set_display` gets the window's logical size and its scale factor; M4 passed 1.0.
- **Why it was rewritten:** the first version registered `Material2dPlugin::<FpvDrawMaterial>` and clipped in its shader. Benilla's gate test `pipe_warm::menagerie::every_material_lane_has_a_warm_contributor` scans every `.rs` file in the workspace for material-plugin registrations, and a lane without a pipe_warm contributor fails it, since its first sight is a render-thread stall. The crate sits in benilla's workspace, so the rule applies to it.
  - Exempting it in benilla would be a fork change made only for this crate. Hiding the registration from the scan would game the check.
  - The node keeps the spirit of the rule (its pipeline is queued at startup) and is closer to the reference renderer (real scissor rects). The test passes again.
  - The user's M5 check ran on the first version. The `hud` capture after the rewrite matches it element for element.

**Input** (`src/input.rs`, `fly::forward_input` in `PreUpdate` after `InputSystems`):
- **Keys:**
  - By physical position: Bevy's `KeyCode` maps to libfpv's codes, letters and digits as uppercase ASCII (unit-tested).
  - On edges only, skipping repeats, and only while the window has focus.
  - A key already down when forwarding starts is a baseline: its release sends nothing.
  - Losing focus releases every key sent down.
- **While `fpv_overlay_wants_input()`:**
  - Typed text goes to `fpv_overlay_text`, printable characters only.
  - The mouse buttons are cleared from Bevy's input, so the viewer's mouse look never grabs the cursor.
  - The OS cursor is hidden; the overlay draws its own (INTEGRATION.md "Input").
  - Keys pressed then are swallowed from this mode's own R/P until released.
- **The mouse:** position, buttons and wheel always go to libfpv, the wheel in lines (pixels ÷ 40).
- **Not needed in the worldview:** keeping Esc from reaching the game, and stopping the game's own UI keys. There's no game here; both are M8's.

**Q8 answered** from the install's `WTF\DefaultBindings.wtf`, read through the patch chain (`benilla-formats --example dump_file`, output kept out of the repo; benilla resolves defaults from it at `bindings/store.rs:33-35`):
- F is ASSISTTARGET, F8 TOGGLEBAG1, R REPLY, P TOGGLESPELLBOOK, V NAMEPLATES, ENTER OPENCHAT, ESCAPE TOGGLEGAMEMENU.
- W/A/S/D and the arrows move.
- F7 and Y are unbound.
- `Interface\FrameXML\Bindings.xml` names the 235 actions but holds no keys.
- For M8: F8, R, P, Enter and Esc must not reach the client while libfpv takes them, and F is assist, so control returns only after release, per INTEGRATION.md.

**Pre-check, `--flight-test hud --capture` PASS** (the climb, through the real renderer):
- `hud-osd.png`: Betaflight's crisp `ARMED` in the picture.
- `hud-overlay.png` (opened with `fpv_overlay_set_open`): the F8 window complete, with its tabs, live motor and battery readouts, buttons, checkbox and slider. Over the full OSD (speed, ANGL, 25.0 V and 4.16 V, current, flight timer, `LIBFPV`), translucent, with the colours right.

**The user's check:** "everything works, F8 and the OSD show fine": the overlay (radio, rates, CLI), the OSD, and the switches.

## M4 status: done, the user flew it (2026-11-23)

**The FPV camera** (`src/camera.rs`, `camera::apply`):
- **Transform:** `frame.camera` position, and rotation as-is (it maps the host's −Z to the view, which is Bevy's camera rotation).
- **FOV:** through `FovZoom = radians(fov_y) / world_fovy(aspect)`. `Projection` is never written.
- **Near plane, per frame:**
  - 9 collision rays from the eye, through the frustum's centre, its edge midpoints and its corners, at the window's aspect.
  - Each hit's depth along the view axis is its distance divided by the ray's length per unit of depth.
  - The near plane is 0.8 × the closest depth, clamped to 0.02–0.2 m (INTEGRATION.md "Coordinates and the camera"). It's set through `ViewDistance::set_nearclip`; benilla's own range, 0.01–0.33 yd, holds that clamp (unit-tested).
  - With nothing within reach it sits at 0.2 m.

**Live flight** (`src/fly.rs`): `benilla-libfpv --worldview --fly [site]`, default `streets`.
- **Init:**
  - `FPV_TIMING_THREAD`, with the simulation thread at 1000 Hz.
  - SDL device input on, and the Configurator on at `ws://localhost:5761`.
  - Flags: `FPV_INIT_NO_LENS_DISTORTION` only.
  - The data folder is the shared one, so the pilot's radio mapping and tune from `fpv_standalone` carry over.
- **Spawn:** the drone is made after the start has streamed in, on the ground, facing the site's heading, with the pilot 2 yd behind.
- **Every frame:** `fpv_set_view(FPV)`, then `fpv_drone_tick(real dt)` lending the world, then `fpv_overlay_render` (not drawn until M5). Frames are skipped until `sim_time` is 50 ms past its first reading, then `camera::apply`. Ordering is per M1.
- **R:** `fpv_drone_repair` plus `fpv_drone_reset` to the start, as the standalone's R does.
- **Log:** the state every 2 s (pose, speed, altitude, armed, angle mode, FOV, near plane, battery, arm text, impacts).
- **Smoke test** (scratch data folder, `WOW_WORLDVIEW_CHECK=40`): the thread, SDL 3.4.18 input and the Configurator all came up, the drone sat on the street, near 0.086 yd, and "WORLDVIEW_CHECK ok".

**Headless pre-check, `--flight-test climb` PASS:**
- **The flight:** angle mode, armed at 1.02 s, climb from 1.5 s, then throttle −0.5, which still climbs with this preset. Top 23.6 m. The FPV camera stays within 5 cm of the drone.
- **On the ground:**
  - Camera up `[0.05, 0.90, 0.43]` (upright), forward pitched up 25.3° (the preset's camera uptilt).
  - FOV 100°, the 5" preset's default.
  - The probes found the street 9.9 cm away, so the near plane was 7.9 cm.
- **Shots:** in `fpv-ground.png` the street runs to the frame's bottom edge with no clipping. `fpv-climb.png` shows a level horizon over the square. `fpv-hold.png`, at about 20 m, shows the rooftops, walls and spires normally.
- **Known from I-1:** climbing about 100 yd over the city brings the fog. That's benilla's claim, not M4.

**What the user checks:** fly Stormwind offline with the Pocket, and report on feel, latency and popping. There's no OSD or F8 overlay yet (M5) and no lens (M6).

**The user's first flight report** (2026-11-23), on the default 5" preset (their last choice):
- **Feel:** "floaty", especially when dropping it. Falling armed, the camera shakes or jitters now and then. Falling disarmed, it shakes a lot more, which may be the tumble. To measure: drop timing against free fall, and the camera's frame-to-frame motion.
- **Popping:** high over Stormwind, buildings pop in and out. Outside the city you can fly high and nothing pops, though the inner city pops when looked at. This is I-1's claim, which the user judges faithful (below).
- **Latency:** "felt fine, no noticeable lag".
- **Collision bug, I-2:** see below.
- **Answers:** the About URL `github.com/Jepharion/benilla` is the user's fork (keep it), and the git author `Cristi` stays.

**Investigating the report:**
- **Gravity is right.** `--flight-test drop` / `drop-armed` lift the drone 10 m and let it go:
  - Disarmed: 10.05 m in 1.450 s, against 1.431 s in a vacuum (ratio 1.013). The 0.2 s samples follow ½gt² within a few cm.
  - Armed at idle: 1.483 s (ratio 1.037); the idling props add drag.
  - In threaded timing (`LIBFPV_TEST_THREADED=1`: libfpv's own thread, each frame presented at its real duration, as in `--fly`): 1.466 s and 1.499 s.
- **The presentation is smooth.** Threaded, in the air:
  - The camera turns at 0–2°/s in a level drop.
  - Each frame's camera step strays at most 4.3 mm from velocity × frame time, and the largest step (0.213 m) is one 60 fps frame at landing speed, so there was no hitch.
  - So the jitter the user saw is the flight itself: the tumble disarmed, prop wash armed at low throttle, in acro (`angle false` in the user's log).
  - "Floaty" fits scale (Stormwind is oversized) and the unwidened 100° view with no lens (M6). Revisit the feel after M6.
- **I-2, the gatehouse walls. Open, deferred by the user.**
  - **Where:** the user marked it with P while flying through the walls. WoW [-8893.3, 570.7, 111.0], [-8900.5, 564.6, 108.0], [-8911.5, 554.6, 107.5], heading 143°: the gatehouse at the end of the Valley of Heroes, about 12–15 yd over the street.
  - **What the probe found:** `--probe` from the marks finds no face ahead at the drone's height. The photo from mark 2 shows the gatehouse's sloped stone roofs, a wooden palisade, a beam and a banner.
  - **The user's read:** it's the stone walls, which players can't reach in the game, so their collision is probably missing in the data. There are likely many such places.
  - **What it isn't:** Q3. A trial `ray_solid` (default + Walk + Camera layers) found the same faces as `ray_body` there, and only 2 more covered points in a 24 × 24 survey near the Valley, so it was reverted and benilla-world is unchanged. Q3 stays open.
  - **Likely fix later:** give the drone collision built from the render geometry where the collision data has none.
- **Tools added:**
  - `--fly`: **P** logs a `MARK` line with the drone's position and heading.
  - `--scan x0,y0,x1,y1[,z]`: covered spots on a 3 yd grid, from up rays starting at z, with wall counts in 8 directions.
  - `--probe x,y,z,heading[,shot.png]`: level rays from a spot at five heights, plus a photo.

**Gates this round:**
- `check.sh` now always escalates to the full chain, because this branch's `Cargo.lock` differs from `main` (since M1).
- The first run failed `benilla-app` `sound::mixer::tests::a_streamed_bed_loops_seamlessly`. That's upstream code, a timing test that waits in 1 ms sleeps. It passed 5 of 5 alone, so it's flaky under the full suite's load. The chain was rerun.
- **The rerun passed:** fmt, deviation-why, clippy, test (the mixer test included), doctest, test-no-install, player-clippy, player-tests, enforcer, enforcer-no-install, plus `benilla-libfpv --worldview`.
- **Not run:** the Python gates and reference, as before.
- **Follow-up round** (drop tests, `--scan`, `--probe`, the P key, `ray_solid` reverted): the full chain passed again through test-no-install, then player-clippy, player-tests and the three enforcers. The Python gates were not run.

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

**Closed: judged faithful (2026-11-23).** The user checked period footage of the 1.12 client: the city fogs out from above or from far away, as benilla draws it. So it's not a benilla bug, and there's nothing to raise upstream.

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
2. **Detaching the body in the full client. Done in M8:** `benilla_app::HostBody` (see M8). Before: `Player` is `pub(crate)` (`benilla-app/src/player/state.rs:336`). Its `detached` flag is only toggled by the dev chord (`player/controller.rs:482-485`). Detached is what makes streaming follow the camera (Q6). Proposed: a small public resource or event that asks for detach and attach. Needed from M4 onwards in the full client.
3. **Owning the camera in the full client.** The rig writes the camera `Transform` in `seat_camera` (`player/camera.rs:1685`) and `fly_free` (`:2025`). A plugin ordered after those can overwrite the `Transform` in the same frame. Try that first; ask for a hook only if ordering isn't enough. The M1 worldview showed the ordering works, provided the writer also runs before `view::CameraPoseSet` (see M1).
4. **The drone's sound. Done in M7:** `benilla_app::play_host_stream(world, make)` (`benilla-app/src/sound/host.rs`, with `Mixer::play_host`, `sound/mixer.rs`). The mixer is `pub(crate)` (`sound/mixer.rs:83`), and the worldview opens no mixer at all. The function plays a host-filled stereo stream on the main track: on the client's mixer (`SoundOutput`, `sound/mod.rs:239`) when there is one, else on a mixer it opens, under the same silence rules (`$WOW_NOSOUND`, `$WOW_CAPTURE`, no device). The fill runs on the audio thread in blocks of 1024 frames from a preallocated buffer, so the audio thread doesn't allocate. Dropping the `HostStream` handle ends the sound. Two unit tests. With it `benilla-libfpv` needs no kira dependency of its own.

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
