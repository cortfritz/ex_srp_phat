//! SRP-PHAT response field + closed-form TDOA multilateration.
//!
//! The SRP response power at a candidate is `Σ_{i<j} g_ij(τ_ij)` with
//! `τ_ij = (‖p − eᵢ‖ − ‖p − eⱼ‖) / c · fs`. [`score_sharp`] point-samples it
//! (razor-sharp, the genuine-source discriminator); [`response_field`] pools it
//! over a coarse grid (windowed-max keeps the sharp peak's height reachable from
//! a coarse cell) to get the field statistics that normalize confidence.
//!
//! Localization itself is closed form: [`multilaterate`] solves the
//! range-difference equations from a set of measured TDOAs. The caller
//! enumerates TDOA combinations across reference pairs (each pair's correlation
//! peaks, one per source) and keeps the combinations whose [`score_sharp`] is
//! high — that both localizes and disambiguates overlapping sources without
//! separating them. [`linearized_covariance`] gives the position covariance.
//!
//! All geometry is recentered at emplacement 0 so squared terms never suffer
//! catastrophic cancellation from raw ~5e6 m ECEF coordinates.

use crate::gcc_phat::CorrTable;
use rayon::prelude::*;

/// Speed of sound (m/s, ~20 °C). MUST match the synthesizer / oracle.
pub const SPEED_OF_SOUND: f64 = 343.0;

#[inline]
fn dist(a: [f64; 3], b: [f64; 3]) -> f64 {
    let dx = a[0] - b[0];
    let dy = a[1] - b[1];
    let dz = a[2] - b[2];
    (dx * dx + dy * dy + dz * dz).sqrt()
}

#[inline]
fn unit_from(emp: [f64; 3], p: [f64; 3], r: f64) -> [f64; 3] {
    [
        (p[0] - emp[0]) / r,
        (p[1] - emp[1]) / r,
        (p[2] - emp[2]) / r,
    ]
}

/// Pooled SRP score for `cand`: each pair contributes the max correlation over
/// a ±`half_win`-sample lag window around the candidate's predicted lag.
pub fn score_pooled(
    table: &CorrTable,
    emp_local: &[[f64; 3]],
    cand: [f64; 3],
    fs: f64,
    half_win: i64,
) -> f64 {
    let n = emp_local.len();
    let ranges: Vec<f64> = emp_local.iter().map(|&e| dist(cand, e)).collect();
    let mut s = 0.0;
    for i in 0..n {
        for j in (i + 1)..n {
            let tau = (ranges[i] - ranges[j]) / SPEED_OF_SOUND * fs;
            s += table.lookup_pooled(i, j, tau, half_win);
        }
    }
    s
}

/// Sharp (point-sampled) SRP score. The whitened peak is razor-sharp, so this
/// is near-zero except within centimeters of a true source — making it the
/// reliable discriminator between genuine localizations and grid artifacts.
pub fn score_sharp(table: &CorrTable, emp_local: &[[f64; 3]], cand: [f64; 3], fs: f64) -> f64 {
    let n = emp_local.len();
    let ranges: Vec<f64> = emp_local.iter().map(|&e| dist(cand, e)).collect();
    let mut s = 0.0;
    for i in 0..n {
        for j in (i + 1)..n {
            let tau = (ranges[i] - ranges[j]) / SPEED_OF_SOUND * fs;
            s += table.lookup(i, j, tau);
        }
    }
    s
}

fn grid(center: [f64; 3], extent: f64, res: f64) -> Vec<[f64; 3]> {
    let n = (extent / res).floor().max(0.0) as i64;
    let mut out = Vec::with_capacity(((2 * n + 1) as usize).pow(3));
    for ix in -n..=n {
        for iy in -n..=n {
            for iz in -n..=n {
                out.push([
                    center[0] + ix as f64 * res,
                    center[1] + iy as f64 * res,
                    center[2] + iz as f64 * res,
                ]);
            }
        }
    }
    out
}

/// Half-window (lag samples) that guarantees the nearest grid node of size
/// `res` can reach any peak inside its cell: the cube-corner displacement times
/// the worst-case lag sensitivity `2·fs/c`.
pub fn pool_half_win(res: f64, fs: f64) -> i64 {
    (1.8 * res * fs / SPEED_OF_SOUND).ceil() as i64
}

fn evaluate(
    table: &CorrTable,
    emp_local: &[[f64; 3]],
    cands: &[[f64; 3]],
    fs: f64,
    half_win: i64,
) -> Vec<f64> {
    cands
        .par_iter()
        .map(|&pos| score_pooled(table, emp_local, pos, fs, half_win))
        .collect()
}

fn mean_score(field: &[f64]) -> f64 {
    if field.is_empty() {
        0.0
    } else {
        field.iter().sum::<f64>() / field.len() as f64
    }
}

/// Coarse SRP response-surface statistics over the search cube, used to
/// normalize a localized source's confidence (peak height relative to the
/// field). The grid is the SRP-PHAT response evaluated with windowed-max
/// pooling so the coarse cells still register the sharp whitened peaks.
pub struct ResponseField {
    pub field_mean: f64,
    pub field_max: f64,
}

/// Evaluate the pooled SRP response over a coarse grid and return its mean/max.
pub fn response_field(
    table: &CorrTable,
    emp_local: &[[f64; 3]],
    center: [f64; 3],
    fs: f64,
    grid_extent_m: f64,
    coarse_res_m: f64,
) -> ResponseField {
    let cands = grid(center, grid_extent_m, coarse_res_m);
    let field = evaluate(
        table,
        emp_local,
        &cands,
        fs,
        pool_half_win(coarse_res_m, fs),
    );
    ResponseField {
        field_mean: mean_score(&field),
        field_max: field.iter().copied().fold(f64::NEG_INFINITY, f64::max),
    }
}

/// Closed-form TDOA multilateration in geometry recentered at emplacement 0
/// (so `emp_local[0] = origin`). `rd[i] = dist_i - dist_0` are the measured
/// range differences (one per reference pair). Returns the source position in
/// the recentered frame, or `None` if the linear system is singular or has no
/// positive-range solution. Same exact, non-iterative method as the oracle.
pub fn multilaterate(emp_local: &[[f64; 3]], rd: &[f64]) -> Option<[f64; 3]> {
    let n = emp_local.len();
    // Linear system m_i · s = g_i - h_i · r0, with m_i = 2·p_i, g_i = |p_i|²−rd_i²,
    // h_i = 2·rd_i. Solve s = u + v·r0, then close with |s| = r0.
    let mut ata = [[0.0f64; 3]; 3];
    let mut atg = [0.0f64; 3];
    let mut ath = [0.0f64; 3];
    for i in 1..n {
        let pi = emp_local[i];
        let ri = rd[i];
        let m = [2.0 * pi[0], 2.0 * pi[1], 2.0 * pi[2]];
        let gi = pi[0] * pi[0] + pi[1] * pi[1] + pi[2] * pi[2] - ri * ri;
        let hi = 2.0 * ri;
        for a in 0..3 {
            atg[a] += m[a] * gi;
            ath[a] += m[a] * hi;
            for b in 0..3 {
                ata[a][b] += m[a] * m[b];
            }
        }
    }

    let inv = invert_3x3(&ata)?;
    let u = mat_vec(&inv, atg);
    let v = {
        let w = mat_vec(&inv, ath);
        [-w[0], -w[1], -w[2]]
    };

    let a = dot(v, v) - 1.0;
    let b = 2.0 * dot(u, v);
    let c = dot(u, u);

    let roots = positive_roots(a, b, c);
    if roots.is_empty() {
        return None;
    }

    // s is in the recentered frame (p0 = origin); pick the root with the
    // smallest range-difference residual.
    roots
        .into_iter()
        .map(|r0| [u[0] + v[0] * r0, u[1] + v[1] * r0, u[2] + v[2] * r0])
        .min_by(|s1, s2| {
            rd_residual(emp_local, rd, *s1).total_cmp(&rd_residual(emp_local, rd, *s2))
        })
}

fn rd_residual(emp_local: &[[f64; 3]], rd: &[f64], s: [f64; 3]) -> f64 {
    let r0 = dist(s, emp_local[0]);
    (1..emp_local.len())
        .map(|i| {
            let f = (dist(s, emp_local[i]) - r0) - rd[i];
            f * f
        })
        .sum()
}

/// `σ²·(JᵀJ)⁻¹` for the range-difference system at the solution — the position
/// covariance (row-major 3×3), or `None` if rank-deficient.
pub fn linearized_covariance(
    emp_local: &[[f64; 3]],
    rd: &[f64],
    pos: [f64; 3],
) -> Option<[f64; 9]> {
    let n = emp_local.len();
    let r0 = dist(pos, emp_local[0]);
    let u0 = unit_from(emp_local[0], pos, r0);
    let mut jtj = [[0.0f64; 3]; 3];
    let mut resid_sq = 0.0;
    for i in 1..n {
        let ri = dist(pos, emp_local[i]);
        let ui = unit_from(emp_local[i], pos, ri);
        let row = [ui[0] - u0[0], ui[1] - u0[1], ui[2] - u0[2]];
        let f = (ri - r0) - rd[i];
        resid_sq += f * f;
        for a in 0..3 {
            for b in 0..3 {
                jtj[a][b] += row[a] * row[b];
            }
        }
    }
    let sigma2 = (resid_sq / (n - 1).max(1) as f64).max(1.0e-6);
    invert_spd_3x3(&jtj).map(|inv| {
        let mut cov = [0.0f64; 9];
        for (k, c) in cov.iter_mut().enumerate() {
            *c = inv[k] * sigma2;
        }
        cov
    })
}

#[inline]
fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

#[inline]
fn mat_vec(m: &[f64; 9], v: [f64; 3]) -> [f64; 3] {
    [
        m[0] * v[0] + m[1] * v[1] + m[2] * v[2],
        m[3] * v[0] + m[4] * v[1] + m[5] * v[2],
        m[6] * v[0] + m[7] * v[1] + m[8] * v[2],
    ]
}

/// Real positive roots of `a·x² + b·x + c = 0` (handles the linear case).
fn positive_roots(a: f64, b: f64, c: f64) -> Vec<f64> {
    if a.abs() < 1.0e-12 {
        if b.abs() < 1.0e-12 {
            return vec![];
        }
        let x = -c / b;
        return if x > 0.0 { vec![x] } else { vec![] };
    }
    let disc = b * b - 4.0 * a * c;
    if disc < 0.0 {
        return vec![];
    }
    let sq = disc.sqrt();
    [(-b + sq) / (2.0 * a), (-b - sq) / (2.0 * a)]
        .into_iter()
        .filter(|&x| x > 0.0)
        .collect()
}

/// General 3×3 inverse (row-major output), `None` if near-singular.
fn invert_3x3(m: &[[f64; 3]; 3]) -> Option<[f64; 9]> {
    let det = m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1])
        - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
        + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0]);
    if det.abs() < 1.0e-12 {
        return None;
    }
    let inv_det = 1.0 / det;
    Some([
        (m[1][1] * m[2][2] - m[1][2] * m[2][1]) * inv_det,
        (m[0][2] * m[2][1] - m[0][1] * m[2][2]) * inv_det,
        (m[0][1] * m[1][2] - m[0][2] * m[1][1]) * inv_det,
        (m[1][2] * m[2][0] - m[1][0] * m[2][2]) * inv_det,
        (m[0][0] * m[2][2] - m[0][2] * m[2][0]) * inv_det,
        (m[0][2] * m[1][0] - m[0][0] * m[1][2]) * inv_det,
        (m[1][0] * m[2][1] - m[1][1] * m[2][0]) * inv_det,
        (m[0][1] * m[2][0] - m[0][0] * m[2][1]) * inv_det,
        (m[0][0] * m[1][1] - m[0][1] * m[1][0]) * inv_det,
    ])
}

/// Invert a symmetric 3×3 matrix, returning `None` unless it is
/// positive-definite (all leading principal minors > 0). Output is row-major.
fn invert_spd_3x3(m: &[[f64; 3]; 3]) -> Option<[f64; 9]> {
    let m00 = m[0][0];
    let minor2 = m[0][0] * m[1][1] - m[0][1] * m[1][0];
    let det = m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1])
        - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
        + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0]);

    if !(m00 > 0.0 && minor2 > 0.0 && det > 1.0e-18) {
        return None;
    }

    let inv_det = 1.0 / det;
    Some([
        (m[1][1] * m[2][2] - m[1][2] * m[2][1]) * inv_det,
        (m[0][2] * m[2][1] - m[0][1] * m[2][2]) * inv_det,
        (m[0][1] * m[1][2] - m[0][2] * m[1][1]) * inv_det,
        (m[1][2] * m[2][0] - m[1][0] * m[2][2]) * inv_det,
        (m[0][0] * m[2][2] - m[0][2] * m[2][0]) * inv_det,
        (m[0][2] * m[1][0] - m[0][0] * m[1][2]) * inv_det,
        (m[1][0] * m[2][1] - m[1][1] * m[2][0]) * inv_det,
        (m[0][1] * m[2][0] - m[0][0] * m[2][1]) * inv_det,
        (m[0][0] * m[1][1] - m[0][1] * m[1][0]) * inv_det,
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grid_has_expected_cardinality() {
        let cands = grid([0.0, 0.0, 0.0], 10.0, 5.0);
        assert_eq!(cands.len(), 125);
    }

    #[test]
    fn spd_inverse_identity() {
        let id = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
        let inv = invert_spd_3x3(&id).unwrap();
        assert_eq!(inv, [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0]);
    }

    #[test]
    fn non_spd_returns_none() {
        let neg = [[-1.0, 0.0, 0.0], [0.0, -1.0, 0.0], [0.0, 0.0, -1.0]];
        assert!(invert_spd_3x3(&neg).is_none());
    }

    #[test]
    fn pool_half_win_scales_with_resolution() {
        assert!(pool_half_win(20.0, 4000.0) > pool_half_win(5.0, 4000.0));
    }
}
