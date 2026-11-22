/*
 * Prints the C layout of every libfpv struct the bindings mirror: one line per struct
 * ("S name size align") and per field ("F name.field offset"), as `ffi.rs`'s layout test
 * expects them. Built against the pinned header, x86_64 (MSVC and MinGW-w64 agree for these
 * plain C structs):
 *
 *   cl /nologo /I D:\benilla\libfpv\include /DLIBFPV_STATIC tools\libfpv_layout.c
 *   libfpv_layout.exe > src\ffi_layout.txt
 */
#include <stdalign.h>
#include <stddef.h>
#include <stdio.h>

#include "libfpv.h"

#define S(t) printf("S %s %u %u\n", #t, (unsigned)sizeof(t), (unsigned)_Alignof(t))
#define F(t, f) printf("F %s.%s %u\n", #t, #f, (unsigned)offsetof(t, f))

int main(void) {
    S(fpv_coord_system); F(fpv_coord_system, right); F(fpv_coord_system, up);
    F(fpv_coord_system, forward); F(fpv_coord_system, units_per_meter);

    S(fpv_global_config); F(fpv_global_config, struct_size); F(fpv_global_config, data_dir);
    F(fpv_global_config, coords); F(fpv_global_config, log); F(fpv_global_config, log_user);
    F(fpv_global_config, log_level); F(fpv_global_config, configurator_port);
    F(fpv_global_config, timing); F(fpv_global_config, sim_rate_hz);
    F(fpv_global_config, auto_pause_seconds); F(fpv_global_config, flags);
    F(fpv_global_config, host_name); F(fpv_global_config, host_build); F(fpv_global_config, host_url);

    S(fpv_material); F(fpv_material, restitution); F(fpv_material, friction); F(fpv_material, flags);

    S(fpv_mesh); F(fpv_mesh, positions); F(fpv_mesh, vertex_count); F(fpv_mesh, vertex_stride);
    F(fpv_mesh, indices); F(fpv_mesh, triangle_count); F(fpv_mesh, materials); F(fpv_mesh, material);

    S(fpv_transform); F(fpv_transform, position); F(fpv_transform, rotation); F(fpv_transform, scale);

    S(fpv_raycast_hit); F(fpv_raycast_hit, distance); F(fpv_raycast_hit, normal);
    F(fpv_raycast_hit, material); F(fpv_raycast_hit, object_id);

    S(fpv_raycast_hit_ex); F(fpv_raycast_hit_ex, struct_size); F(fpv_raycast_hit_ex, hit);
    F(fpv_raycast_hit_ex, flags); F(fpv_raycast_hit_ex, object_position);
    F(fpv_raycast_hit_ex, object_rotation);

    S(fpv_drone_desc); F(fpv_drone_desc, struct_size); F(fpv_drone_desc, preset);
    F(fpv_drone_desc, position); F(fpv_drone_desc, rotation); F(fpv_drone_desc, pilot_position);

    S(fpv_cargo); F(fpv_cargo, struct_size); F(fpv_cargo, mass); F(fpv_cargo, offset);
    F(fpv_cargo, radius); F(fpv_cargo, line_length);

    S(fpv_camera); F(fpv_camera, position); F(fpv_camera, rotation); F(fpv_camera, forward);
    F(fpv_camera, up); F(fpv_camera, right); F(fpv_camera, fov_y_degrees);
    F(fpv_camera, aspect_hint); F(fpv_camera, near_plane); F(fpv_camera, distortion_k1);
    F(fpv_camera, distortion_k2); F(fpv_camera, distortion_zoom);

    S(fpv_motor_state); F(fpv_motor_state, rpm); F(fpv_motor_state, thrust_newton);
    F(fpv_motor_state, current_amp); F(fpv_motor_state, temperature_c); F(fpv_motor_state, command);
    F(fpv_motor_state, prop_damage); F(fpv_motor_state, position);
    F(fpv_motor_state, spin_direction); F(fpv_motor_state, flags);

    S(fpv_battery_state); F(fpv_battery_state, voltage); F(fpv_battery_state, cell_voltage);
    F(fpv_battery_state, current_amp); F(fpv_battery_state, mah_drawn);
    F(fpv_battery_state, capacity_mah); F(fpv_battery_state, cell_count);

    S(fpv_impact); F(fpv_impact, position); F(fpv_impact, normal); F(fpv_impact, speed);
    F(fpv_impact, impulse); F(fpv_impact, material); F(fpv_impact, object_id);
    F(fpv_impact, motor_index);

    S(fpv_video_state); F(fpv_video_state, noise); F(fpv_video_state, breakup);
    F(fpv_video_state, signal_strength); F(fpv_video_state, is_digital);

    S(fpv_video_feed); F(fpv_video_feed, flags); F(fpv_video_feed, system);
    F(fpv_video_feed, picture_aspect); F(fpv_video_feed, raster_width);
    F(fpv_video_feed, raster_height); F(fpv_video_feed, luma_cycles);
    F(fpv_video_feed, chroma_cycles); F(fpv_video_feed, subcarrier_cycles);
    F(fpv_video_feed, sharpening); F(fpv_video_feed, field); F(fpv_video_feed, field_parity);
    F(fpv_video_feed, roll); F(fpv_video_feed, seed); F(fpv_video_feed, interference);
    F(fpv_video_feed, block_errors); F(fpv_video_feed, softness); F(fpv_video_feed, frozen);
    F(fpv_video_feed, rssi_dbm); F(fpv_video_feed, cnr_db); F(fpv_video_feed, osd_style);
    F(fpv_video_feed, line_count); F(fpv_video_feed, lines); F(fpv_video_feed, chroma_boost);
    F(fpv_video_feed, chroma_tint); F(fpv_video_feed, hue_sweep);
    F(fpv_video_feed, display_aspect); F(fpv_video_feed, osd_rows);

    S(fpv_frame); F(fpv_frame, struct_size); F(fpv_frame, sim_time); F(fpv_frame, flags);
    F(fpv_frame, arming_disable_flags); F(fpv_frame, arming_disable_text);
    F(fpv_frame, position); F(fpv_frame, rotation); F(fpv_frame, velocity);
    F(fpv_frame, angular_velocity); F(fpv_frame, speed_kmh); F(fpv_frame, altitude);
    F(fpv_frame, camera); F(fpv_frame, chase_camera); F(fpv_frame, los_camera);
    F(fpv_frame, motors); F(fpv_frame, battery); F(fpv_frame, video);
    F(fpv_frame, impact_count); F(fpv_frame, impacts); F(fpv_frame, osd);
    F(fpv_frame, osd_revision); F(fpv_frame, channels); F(fpv_frame, wind);
    F(fpv_frame, portal_passes); F(fpv_frame, last_portal); F(fpv_frame, cargo_flags);
    F(fpv_frame, cargo_position); F(fpv_frame, cargo_velocity); F(fpv_frame, video_feed);

    S(fpv_portal); F(fpv_portal, position); F(fpv_portal, rotation); F(fpv_portal, half_width);
    F(fpv_portal, half_height);

    S(fpv_portal_link); F(fpv_portal_link, a); F(fpv_portal_link, b);

    S(fpv_rc_link_config); F(fpv_rc_link_config, struct_size); F(fpv_rc_link_config, enabled);
    F(fpv_rc_link_config, mode); F(fpv_rc_link_config, max_power_mw);
    F(fpv_rc_link_config, dynamic_power);

    S(fpv_rc_link_state); F(fpv_rc_link_state, struct_size); F(fpv_rc_link_state, enabled);
    F(fpv_rc_link_state, connected); F(fpv_rc_link_state, link_quality);
    F(fpv_rc_link_state, rssi_dbm); F(fpv_rc_link_state, snr_db);
    F(fpv_rc_link_state, tx_power_mw); F(fpv_rc_link_state, sensitivity_dbm);
    F(fpv_rc_link_state, packet_rate_hz);

    S(fpv_pilot_config); F(fpv_pilot_config, struct_size); F(fpv_pilot_config, features);
    F(fpv_pilot_config, stand_in_distance); F(fpv_pilot_config, stand_in_return_distance);

    S(fpv_wind); F(fpv_wind, struct_size); F(fpv_wind, velocity); F(fpv_wind, reference_height);
    F(fpv_wind, turbulence); F(fpv_wind, flags);

    S(fpv_wind_source); F(fpv_wind_source, position); F(fpv_wind_source, direction);
    F(fpv_wind_source, speed); F(fpv_wind_source, radius); F(fpv_wind_source, type);

    S(fpv_input_status); F(fpv_input_status, struct_size); F(fpv_input_status, source);
    F(fpv_input_status, device_name); F(fpv_input_status, needs_setup);
    F(fpv_input_status, device_count);

    S(fpv_control_desc); F(fpv_control_desc, struct_size); F(fpv_control_desc, id);
    F(fpv_control_desc, name); F(fpv_control_desc, label); F(fpv_control_desc, kind);
    F(fpv_control_desc, key); F(fpv_control_desc, gamepad_button);

    S(fpv_draw_vertex); F(fpv_draw_vertex, pos); F(fpv_draw_vertex, uv); F(fpv_draw_vertex, color);

    S(fpv_draw_cmd); F(fpv_draw_cmd, clip_rect); F(fpv_draw_cmd, texture_id);
    F(fpv_draw_cmd, index_offset); F(fpv_draw_cmd, index_count); F(fpv_draw_cmd, vertex_offset);

    S(fpv_draw_list); F(fpv_draw_list, vertices); F(fpv_draw_list, vertex_count);
    F(fpv_draw_list, indices); F(fpv_draw_list, index_count); F(fpv_draw_list, commands);
    F(fpv_draw_list, command_count);

    S(fpv_draw_data); F(fpv_draw_data, display_size); F(fpv_draw_data, list_count);
    F(fpv_draw_data, lists); F(fpv_draw_data, total_vertex_count);
    F(fpv_draw_data, total_index_count);

    S(fpv_texture); F(fpv_texture, width); F(fpv_texture, height); F(fpv_texture, rgba);
    F(fpv_texture, revision);

    S(fpv_configurator_info); F(fpv_configurator_info, struct_size);
    F(fpv_configurator_info, enabled); F(fpv_configurator_info, port);
    F(fpv_configurator_info, client_count); F(fpv_configurator_info, url);
    return 0;
}
