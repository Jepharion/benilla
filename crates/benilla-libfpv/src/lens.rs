//! The fisheye's FOV for the real aspect ratio (docs/INTEGRATION.md "Coordinates and the
//! camera"): libfpv widens `fov_y_degrees` for its `aspect_hint` (4:3), so a screen of another
//! aspect renders at a recomputed FOV for the lens to show the vertical FOV the player chose.
//! The lens itself is drawn by [`crate::video`].

use crate::ffi::FpvCamera;

/// The widest vertical FOV the recomputation may ask for (INTEGRATION.md: "an angle the engine
/// accepts (170°)").
const MAX_FOV_DEG: f32 = 170.0;

/// The fisheye's coefficients: k1 and k2, the zoom after distortion, and the output's aspect.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LensSettings {
    pub k: [f32; 2],
    pub zoom: f32,
    pub aspect: f32,
}

/// `cam`'s lens at an output of `aspect` (width / height), and the vertical FOV to render at, in
/// radians. `None` for the lens when the pilot has the fisheye off (no warp, zoom 1).
pub fn fit(cam: &FpvCamera, aspect: f32) -> (Option<LensSettings>, f32) {
    let fov = cam.fov_y_degrees.to_radians();
    let (k1, k2, zoom) = (cam.distortion_k1, cam.distortion_k2, cam.distortion_zoom);
    let zoom = if zoom > 0.0 { zoom } else { 1.0 };
    if k1 == 0.0 && k2 == 0.0 && zoom == 1.0 {
        return (None, fov);
    }
    // The scale of the image's top-edge centre: r2 = 1 / (1 + a²) with |c| = 1 at the corners.
    let edge = |a: f32| {
        let r2 = 1.0 / (1.0 + a * a);
        (1.0 + k1 * r2 + k2 * r2 * r2) / zoom
    };
    let hint = if cam.aspect_hint > 0.0 {
        cam.aspect_hint
    } else {
        4.0 / 3.0
    };
    let real = 2.0 * ((fov * 0.5).tan() * edge(hint) / edge(aspect)).atan();
    let lens = LensSettings {
        k: [k1, k2],
        zoom,
        aspect,
    };
    (Some(lens), real.min(MAX_FOV_DEG.to_radians()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cam(fov: f32, k1: f32, k2: f32, zoom: f32) -> FpvCamera {
        FpvCamera {
            fov_y_degrees: fov,
            aspect_hint: 4.0 / 3.0,
            distortion_k1: k1,
            distortion_k2: k2,
            distortion_zoom: zoom,
            ..Default::default()
        }
    }

    #[test]
    fn no_fisheye_means_no_pass_and_the_fov_unchanged() {
        let (lens, fov) = fit(&cam(100.0, 0.0, 0.0, 1.0), 16.0 / 9.0);
        assert!(lens.is_none());
        assert!((fov.to_degrees() - 100.0).abs() < 1e-4);
    }

    #[test]
    fn at_the_hint_aspect_the_fov_is_libfpvs() {
        let (lens, fov) = fit(&cam(120.0, 0.3, 0.1, 1.3), 4.0 / 3.0);
        assert!(lens.is_some());
        assert!((fov.to_degrees() - 120.0).abs() < 1e-3);
    }

    #[test]
    fn a_wider_output_than_the_hint_widens_the_vertical_fov() {
        // The top edge's centre sits nearer the image centre on a 16:9 picture (r2 = 0.24
        // against 0.36 at 4:3), where a barrel lens (k > 0) scales less, so the same edge needs
        // a wider render (INTEGRATION.md: the default comes out at about 119° at 16:9).
        let (_, fov) = fit(&cam(120.0, 0.3, 0.1, 1.3), 16.0 / 9.0);
        let edge = |a: f32| {
            let r2 = 1.0 / (1.0 + a * a);
            (1.0 + 0.3 * r2 + 0.1 * r2 * r2) / 1.3
        };
        let want = 2.0 * (60f32.to_radians().tan() * edge(4.0 / 3.0) / edge(16.0 / 9.0)).atan();
        assert!((fov - want).abs() < 1e-5);
        assert!(fov.to_degrees() > 120.0);
    }

    #[test]
    fn the_fov_is_clamped() {
        let (_, fov) = fit(&cam(179.0, 0.0, 0.0, 0.5), 16.0 / 9.0);
        assert!(fov.to_degrees() <= MAX_FOV_DEG + 1e-3);
    }
}
