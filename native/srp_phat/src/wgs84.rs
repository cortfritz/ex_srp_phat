//! WGS-84 geodetic ↔ ECEF conversion and geometry helpers.
//!
//! Forward (lat/lon/alt → ECEF) is an exact closed form, so it matches any other
//! standard WGS-84 implementation to f64 precision — which is what keeps this
//! library's geometry in lock-step with the Augur oracle's `ExMgrs.Ecef`. The
//! inverse uses Bowring's method; it is only a convenience for reporting and is
//! never on the localization critical path (peaks are compared in ECEF).

/// Semi-major axis (meters).
pub const WGS84_A: f64 = 6_378_137.0;
/// Flattening.
pub const WGS84_F: f64 = 1.0 / 298.257223563;
/// First eccentricity squared, `f(2 - f)` ≈ 0.00669437999014132.
pub const WGS84_E2: f64 = WGS84_F * (2.0 - WGS84_F);

/// Convert geodetic latitude/longitude (degrees) + height (m, HAE) to ECEF (m).
pub fn latlon_alt_to_ecef(lat_deg: f64, lon_deg: f64, alt_m: f64) -> [f64; 3] {
    let lat = lat_deg.to_radians();
    let lon = lon_deg.to_radians();
    let (sin_lat, cos_lat) = lat.sin_cos();
    let (sin_lon, cos_lon) = lon.sin_cos();

    // Prime vertical radius of curvature.
    let n = WGS84_A / (1.0 - WGS84_E2 * sin_lat * sin_lat).sqrt();

    let x = (n + alt_m) * cos_lat * cos_lon;
    let y = (n + alt_m) * cos_lat * sin_lon;
    let z = (n * (1.0 - WGS84_E2) + alt_m) * sin_lat;
    [x, y, z]
}

/// Convert ECEF (m) back to geodetic latitude/longitude (degrees) + height (m).
/// Bowring's closed-form approximation (sub-millimeter for terrestrial points).
pub fn ecef_to_latlon_alt(x: f64, y: f64, z: f64) -> (f64, f64, f64) {
    let a = WGS84_A;
    let e2 = WGS84_E2;
    let b = a * (1.0 - WGS84_F);
    let ep2 = (a * a - b * b) / (b * b);

    let p = (x * x + y * y).sqrt();
    let lon = y.atan2(x);

    // Bowring's auxiliary angle.
    let theta = (z * a).atan2(p * b);
    let (sin_t, cos_t) = theta.sin_cos();
    let lat = (z + ep2 * b * sin_t * sin_t * sin_t).atan2(p - e2 * a * cos_t * cos_t * cos_t);
    let (sin_lat, cos_lat) = lat.sin_cos();
    let n = a / (1.0 - e2 * sin_lat * sin_lat).sqrt();

    // Height: numerically stable away from the poles via p/cos(lat).
    let alt = if cos_lat.abs() > 1.0e-9 {
        p / cos_lat - n
    } else {
        z / sin_lat - n * (1.0 - e2)
    };

    (lat.to_degrees(), lon.to_degrees(), alt)
}

/// Straight-line Euclidean (chord) distance between two ECEF points, in meters.
/// This is the path sound travels — NOT great-circle/surface distance.
#[inline]
pub fn slant_range(a: [f64; 3], b: [f64; 3]) -> f64 {
    let dx = a[0] - b[0];
    let dy = a[1] - b[1];
    let dz = a[2] - b[2];
    (dx * dx + dy * dy + dz * dz).sqrt()
}

/// Subtract `origin` from `p` (recenter geometry to avoid catastrophic
/// cancellation when forming squared terms from raw ~5e6 m ECEF coordinates).
#[inline]
pub fn recenter(p: [f64; 3], origin: [f64; 3]) -> [f64; 3] {
    [p[0] - origin[0], p[1] - origin[1], p[2] - origin[2]]
}

/// Inverse of [`recenter`]: shift a local solution back into ECEF.
#[inline]
pub fn uncenter(p: [f64; 3], origin: [f64; 3]) -> [f64; 3] {
    [p[0] + origin[0], p[1] + origin[1], p[2] + origin[2]]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn e2_matches_canonical_constant() {
        assert!((WGS84_E2 - 0.006_694_379_990_141_32).abs() < 1.0e-15);
    }

    #[test]
    fn forward_inverse_roundtrip() {
        // A point in the Augur fixture neighborhood (central New Mexico).
        let (lat, lon, alt) = (35.0003, -106.0003, 1_720.0);
        let p = latlon_alt_to_ecef(lat, lon, alt);
        let (lat2, lon2, alt2) = ecef_to_latlon_alt(p[0], p[1], p[2]);
        assert!((lat - lat2).abs() < 1.0e-9, "lat {lat} vs {lat2}");
        assert!((lon - lon2).abs() < 1.0e-9, "lon {lon} vs {lon2}");
        assert!((alt - alt2).abs() < 1.0e-4, "alt {alt} vs {alt2}");
    }

    #[test]
    fn ecef_coords_are_large() {
        // Confirms the recenter rationale: raw coords are ~5e6 m.
        let p = latlon_alt_to_ecef(35.0, -106.0, 1_600.0);
        assert!(p[0].abs() > 1.0e6 && p[2].abs() > 1.0e6);
    }

    #[test]
    fn slant_range_is_chord_not_surface() {
        // Two points at the same lat/lon but 120 m apart in altitude: the chord
        // is ~120 m, while any surface metric would call them coincident.
        let a = latlon_alt_to_ecef(35.0, -106.0, 1_600.0);
        let b = latlon_alt_to_ecef(35.0, -106.0, 1_720.0);
        let d = slant_range(a, b);
        assert!((d - 120.0).abs() < 1.0e-3, "chord = {d}");
    }

    #[test]
    fn recenter_uncenter_roundtrip() {
        let p = latlon_alt_to_ecef(35.0006, -106.0, 1_600.0);
        let o = latlon_alt_to_ecef(35.0, -106.0, 1_600.0);
        let local = recenter(p, o);
        // Local coordinates are small (tens of meters), preserving precision.
        assert!(local.iter().all(|c| c.abs() < 1.0e3));
        let back = uncenter(local, o);
        assert!(slant_range(back, p) < 1.0e-6);
    }
}
