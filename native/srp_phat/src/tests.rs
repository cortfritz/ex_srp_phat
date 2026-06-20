//! Acceptance tests against synthesized geometry.
//!
//! These reproduce the Augur oracle's synthetic input — a Morlet wavelet at
//! each emplacement, delayed by the *true* geometric TDOA — and assert the
//! recovered ECEF position lands within tolerance. This is the parity target
//! the standalone library must clear.

use crate::codec::{Frames, Geometry, Opts};
use crate::solve::solve;
use crate::wgs84::{latlon_alt_to_ecef, slant_range};
use std::f64::consts::PI;

const FS: f64 = 4_000.0;
const C: f64 = 343.0;
const BASE: f64 = 1_200.0;
const SIGMA: f64 = 50.0;
const LEN: usize = 2_600;

// (lat, lon, alt) emplacements from the Augur pipeline fixture.
fn emplacements() -> Vec<[f64; 3]> {
    vec![
        latlon_alt_to_ecef(35.0000, -106.0000, 1_600.0),
        latlon_alt_to_ecef(35.0006, -106.0000, 1_600.0),
        latlon_alt_to_ecef(35.0000, -106.0008, 1_600.0),
        latlon_alt_to_ecef(35.0004, -106.0004, 1_670.0),
    ]
}

fn wavelet(n: usize, center: f64, freq: f64) -> f64 {
    let t = n as f64 - center;
    let env = (-0.5 * (t / SIGMA).powi(2)).exp();
    env * (2.0 * PI * freq * t / FS).cos()
}

/// Build a frame-set: one Morlet per (source, freq), delayed by the geometric
/// TDOA relative to emplacement 0.
fn synth(emps: &[[f64; 3]], sources: &[([f64; 3], f64)]) -> Frames {
    let d0: Vec<f64> = sources
        .iter()
        .map(|(src, _)| slant_range(emps[0], *src))
        .collect();

    let mut data = vec![0.0f64; emps.len() * LEN];
    for (ei, emp) in emps.iter().enumerate() {
        for n in 0..LEN {
            let mut acc = 0.0;
            for (si, (src, freq)) in sources.iter().enumerate() {
                let center = BASE + (slant_range(*emp, *src) - d0[si]) / C * FS;
                acc += wavelet(n, center, *freq);
            }
            data[ei * LEN + n] = acc;
        }
    }
    Frames {
        n_emp: emps.len(),
        n_samples: LEN,
        data,
    }
}

fn geometry(emps: &[[f64; 3]]) -> Geometry {
    Geometry {
        sample_rate_hz: FS,
        ecef: emps.to_vec(),
    }
}

fn default_opts() -> Opts {
    Opts {
        grid_extent_m: 300.0,
        coarse_res_m: 20.0,
        fine_res_m: 2.0,
        min_peak_ratio: 0.5,
        max_sources: 4,
    }
}

#[test]
fn single_source_recovered_within_tolerance() {
    let emps = emplacements();
    let source = latlon_alt_to_ecef(35.0003, -106.0003, 1_720.0);
    let frames = synth(&emps, &[(source, 600.0)]);

    let sources = solve(&frames, &geometry(&emps), &default_opts()).unwrap();
    assert!(!sources.is_empty(), "no source recovered");

    // Strongest by confidence.
    let best = sources
        .iter()
        .max_by(|a, b| a.confidence.partial_cmp(&b.confidence).unwrap())
        .unwrap();

    let err = slant_range(best.ecef, source);
    assert!(err < 50.0, "position error {err} m exceeds 50 m");
    assert!(best.confidence > 0.0, "confidence not positive");
    assert_eq!(
        best.dominant_hz.round() as i64,
        600,
        "dominant_hz = {}",
        best.dominant_hz
    );
}

#[test]
fn two_distinct_band_sources_separated() {
    let emps = emplacements();
    let src_a = latlon_alt_to_ecef(35.0003, -106.0003, 1_720.0);
    let src_b = latlon_alt_to_ecef(34.9997, -105.9995, 1_650.0);
    let frames = synth(&emps, &[(src_a, 600.0), (src_b, 300.0)]);

    let opts = Opts {
        min_peak_ratio: 0.35,
        max_sources: 4,
        ..default_opts()
    };
    let sources = solve(&frames, &geometry(&emps), &opts).unwrap();
    assert!(
        sources.len() >= 2,
        "expected >=2 sources, got {}",
        sources.len()
    );

    // Each true source has a recovered peak within tolerance. The stretch bar is
    // looser than the 50 m single-source bar: in a two-source field each source's
    // TDOAs are associated from per-pair peaks that the *other* source perturbs,
    // so a few extra meters of spread is expected.
    for src in [src_a, src_b] {
        let nearest = sources
            .iter()
            .map(|s| slant_range(s.ecef, src))
            .fold(f64::INFINITY, f64::min);
        assert!(
            nearest < 60.0,
            "no recovered source within 60 m of {src:?} (nearest {nearest} m)"
        );
    }
}

#[test]
fn combination_cost_stays_bounded() {
    // The localizer enumerates TDOA combinations across reference pairs:
    // max_sources^(n_emp - 1). Guard that the worst realistic case stays within
    // the MAX_COMBOS cap that solve() actually enforces.
    let max_sources = crate::codec::MAX_SOURCES_CAP.min(4);
    let n_emp = 4usize;
    let worst = max_sources.pow((n_emp - 1) as u32);
    assert!(
        worst <= crate::solve::MAX_COMBOS,
        "combination count {worst} exceeds cap {}",
        crate::solve::MAX_COMBOS
    );
}
