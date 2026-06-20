//! Orchestrates one solve: GCC-PHAT → SRP grid search → per-source attributes
//! (ECEF position, dominant frequency, confidence, position covariance).

use crate::codec::{Frames, Geometry, Opts, SourceOut};
use crate::gcc_phat::CorrTable;
use crate::srp::{self, SPEED_OF_SOUND};
use crate::wgs84;
use realfft::RealFftPlanner;

#[derive(Debug)]
pub enum SolveError {
    /// Need at least two emplacements to form a pair.
    TooFewEmplacements,
    /// Frame count and geometry emplacement count disagree.
    ShapeMismatch,
    /// Non-finite or non-positive sample rate.
    BadSampleRate,
}

pub fn solve(
    frames: &Frames,
    geometry: &Geometry,
    opts: &Opts,
) -> Result<Vec<SourceOut>, SolveError> {
    let n_emp = frames.n_emp;
    if n_emp < 2 || geometry.ecef.len() < 2 {
        return Err(SolveError::TooFewEmplacements);
    }
    if n_emp != geometry.ecef.len() {
        return Err(SolveError::ShapeMismatch);
    }
    let fs = geometry.sample_rate_hz;
    if !fs.is_finite() || fs <= 0.0 {
        return Err(SolveError::BadSampleRate);
    }

    // Recenter geometry at emplacement 0 to keep squared terms precise.
    let origin = geometry.ecef[0];
    let emp_local: Vec<[f64; 3]> = geometry
        .ecef
        .iter()
        .map(|&p| wgs84::recenter(p, origin))
        .collect();

    // Search around the array centroid.
    let centroid = centroid(&emp_local);

    let table = CorrTable::compute(frames);
    let max_sources = opts.max_sources.max(1);

    // SRP response-surface statistics, used to normalize confidence (the brief's
    // "normalized SRP peak sharpness").
    let field = srp::response_field(
        &table,
        &emp_local,
        centroid,
        fs,
        opts.grid_extent_m,
        opts.coarse_res_m,
    );

    // Localize by enumerating TDOA hypotheses: each reference pair (0,i)
    // contributes its strongest correlation peaks (one per source); every
    // combination is solved in closed form, and the sharp SRP score keeps the
    // genuine intersections while rejecting cross-source (mismatched) combos.
    let k = max_sources;
    let pair_peaks: Vec<Vec<f64>> = (1..n_emp)
        .map(|i| {
            let peaks = table.top_peaks(0, i, k, 5);
            if peaks.is_empty() {
                vec![table.global_peak(0, i)]
            } else {
                peaks
            }
        })
        .collect();

    let conf_win = srp::pool_half_win(opts.fine_res_m, fs);
    let mut candidates: Vec<Candidate> = Vec::new();
    for combo in combinations(&pair_peaks, MAX_COMBOS) {
        let rd: Vec<f64> = std::iter::once(0.0)
            .chain(combo.iter().map(|&lag| -lag * SPEED_OF_SOUND / fs))
            .collect();
        let Some(pos) = srp::multilaterate(&emp_local, &rd) else {
            continue;
        };
        // Reject implausibly distant intersections (outside the search cube).
        if wgs84::slant_range(pos, centroid) > opts.grid_extent_m * 1.5 {
            continue;
        }
        let sharp = srp::score_sharp(&table, &emp_local, pos, fs);
        let pooled = srp::score_pooled(&table, &emp_local, pos, fs, conf_win);
        candidates.push(Candidate {
            pos,
            covariance: srp::linearized_covariance(&emp_local, &rd, pos),
            sharp,
            pooled,
        });
    }

    // Rank by sharp score, drop anything well below the best, dedup nearby
    // positions, cap at max_sources.
    candidates.sort_by(|a, b| b.sharp.total_cmp(&a.sharp));
    let best_sharp = candidates.first().map(|c| c.sharp).unwrap_or(0.0);
    let keep_threshold = opts.min_peak_ratio * best_sharp;
    let dedup_radius = (2.5 * opts.coarse_res_m).max(30.0);

    let mut kept: Vec<Candidate> = Vec::new();
    for cand in candidates {
        if cand.sharp <= 0.0 || cand.sharp < keep_threshold || kept.len() >= max_sources {
            continue;
        }
        if kept
            .iter()
            .all(|k| wgs84::slant_range(k.pos, cand.pos) > dedup_radius)
        {
            kept.push(cand);
        }
    }

    let denom = (field.field_max - field.field_mean).max(1.0e-9);
    let sources = kept
        .into_iter()
        .map(|cand| SourceOut {
            ecef: wgs84::uncenter(cand.pos, origin),
            velocity: None,
            radial_velocity: None,
            covariance: cand.covariance,
            confidence: ((cand.pooled - field.field_mean) / denom).clamp(0.0, 1.0),
            dominant_hz: dominant_hz(frames, &emp_local, cand.pos, fs),
        })
        .collect();

    Ok(sources)
}

/// Cap on enumerated TDOA combinations (grid-resolution-vs-cost guard).
pub(crate) const MAX_COMBOS: usize = 4096;

struct Candidate {
    pos: [f64; 3],
    covariance: Option<[f64; 9]>,
    sharp: f64,
    pooled: f64,
}

/// Cartesian product of per-pair peak lists (mixed-radix enumeration), capped at
/// `cap` combinations to bound cost.
fn combinations(pair_peaks: &[Vec<f64>], cap: usize) -> Vec<Vec<f64>> {
    let mut total: usize = 1;
    for pp in pair_peaks {
        total = total.saturating_mul(pp.len().max(1));
    }
    let total = total.min(cap);

    let mut out = Vec::with_capacity(total);
    for idx in 0..total {
        let mut rem = idx;
        let mut combo = Vec::with_capacity(pair_peaks.len());
        for pp in pair_peaks {
            let len = pp.len().max(1);
            combo.push(pp[rem % len]);
            rem /= len;
        }
        out.push(combo);
    }
    out
}

fn centroid(points: &[[f64; 3]]) -> [f64; 3] {
    let n = points.len() as f64;
    let mut c = [0.0; 3];
    for p in points {
        c[0] += p[0];
        c[1] += p[1];
        c[2] += p[2];
    }
    [c[0] / n, c[1] / n, c[2] / n]
}

/// Dominant frequency (Hz) of the steered-and-summed signal at `peak`.
///
/// Channels are integer-delay aligned to emplacement 0 so the source component
/// adds coherently, then the magnitude spectrum's strongest non-DC bin sets the
/// frequency.
fn dominant_hz(frames: &Frames, emp_local: &[[f64; 3]], peak: [f64; 3], fs: f64) -> f64 {
    let n_samples = frames.n_samples;
    let n_emp = frames.n_emp;

    let dist = |a: [f64; 3], b: [f64; 3]| {
        let d = [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
        (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt()
    };
    let range0 = dist(peak, emp_local[0]);
    let shifts: Vec<i64> = (0..n_emp)
        .map(|i| ((dist(peak, emp_local[i]) - range0) / SPEED_OF_SOUND * fs).round() as i64)
        .collect();

    let mut steered = vec![0.0f64; n_samples];
    for (i, &shift) in shifts.iter().enumerate() {
        let ch = frames.channel(i);
        for (t, dst) in steered.iter_mut().enumerate() {
            let src = t as i64 + shift;
            if src >= 0 && (src as usize) < n_samples {
                *dst += ch[src as usize];
            }
        }
    }

    let n_fft = (2 * n_samples.max(1)).next_power_of_two();
    let mut planner = RealFftPlanner::<f64>::new();
    let r2c = planner.plan_fft_forward(n_fft);
    let mut input = r2c.make_input_vec();
    input[..n_samples].copy_from_slice(&steered);
    let mut spectrum = r2c.make_output_vec();
    r2c.process(&mut input, &mut spectrum)
        .expect("rfft forward");

    // Strongest non-DC bin.
    let mut best_bin = 1;
    let mut best_mag = f64::NEG_INFINITY;
    for (bin, c) in spectrum.iter().enumerate().skip(1) {
        let mag = c.norm_sqr();
        if mag > best_mag {
            best_mag = mag;
            best_bin = bin;
        }
    }
    best_bin as f64 * fs / n_fft as f64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn combinations_enumerate_cartesian_product() {
        let pairs = vec![vec![1.0, 2.0], vec![3.0], vec![4.0, 5.0]];
        let combos = combinations(&pairs, MAX_COMBOS);
        // 2 × 1 × 2 = 4 combinations.
        assert_eq!(combos.len(), 4);
        assert!(combos.contains(&vec![1.0, 3.0, 4.0]));
        assert!(combos.contains(&vec![2.0, 3.0, 5.0]));
    }

    #[test]
    fn combinations_respect_cap() {
        let pairs = vec![vec![0.0; 100], vec![0.0; 100], vec![0.0; 100]];
        assert_eq!(combinations(&pairs, 50).len(), 50);
    }

    #[test]
    fn centroid_of_unit_cube_corners() {
        let pts = [
            [0.0, 0.0, 0.0],
            [2.0, 0.0, 0.0],
            [0.0, 2.0, 0.0],
            [2.0, 2.0, 0.0],
        ];
        assert_eq!(centroid(&pts), [1.0, 1.0, 0.0]);
    }
}
