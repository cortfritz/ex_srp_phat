//! GCC-PHAT: per-channel real FFT, then for each emplacement pair a
//! phase-transform-whitened cross-correlation as a function of lag.
//!
//! For channels `x_i(t) = s(t - D_i)` and `x_j(t) = s(t - D_j)`, the cross
//! spectrum `X_i · conj(X_j)` has phase `exp(-j2πf (D_i - D_j)/N)`; dividing by
//! its magnitude (the PHAT weighting) whitens away the source spectrum, so the
//! inverse FFT is a sharp peak at lag `D_i - D_j` regardless of what `s` is.
//!
//! That whitened peak is essentially a delta — far too sharp for a coarse grid
//! to point-sample. The SRP confidence field therefore uses [`lookup_pooled`],
//! which takes the maximum correlation over a lag window (preserving peak
//! *height*, unlike smoothing which destroys contrast). Localization reads the
//! sub-sample TDOAs directly from [`global_peak`] / [`top_peaks`].
//!
//! [`lookup_pooled`]: CorrTable::lookup_pooled
//! [`global_peak`]: CorrTable::global_peak
//! [`top_peaks`]: CorrTable::top_peaks

use crate::codec::Frames;
use realfft::num_complex::Complex;
use realfft::RealFftPlanner;

const PHAT_EPS: f64 = 1.0e-12;

/// Pairwise PHAT correlations for one frame-set, indexed by lag. Upper-triangular
/// pair order `(0,1),(0,2),…,(0,n-1),(1,2),…`.
pub struct CorrTable {
    pub n_fft: usize,
    pub n_emp: usize,
    pair_corr: Vec<Vec<f64>>,
}

impl CorrTable {
    /// Compute all pairwise PHAT correlations. Channels are zero-padded to the
    /// next power of two ≥ `2·n_samples` so the circular correlation has room
    /// for the full linear lag range.
    pub fn compute(frames: &Frames) -> Self {
        let n_emp = frames.n_emp;
        let n_fft = (2 * frames.n_samples.max(1)).next_power_of_two();

        let mut planner = RealFftPlanner::<f64>::new();
        let r2c = planner.plan_fft_forward(n_fft);
        let c2r = planner.plan_fft_inverse(n_fft);

        let spectra: Vec<Vec<Complex<f64>>> = (0..n_emp)
            .map(|i| {
                let mut input = r2c.make_input_vec();
                let ch = frames.channel(i);
                input[..ch.len()].copy_from_slice(ch);
                let mut spectrum = r2c.make_output_vec();
                r2c.process(&mut input, &mut spectrum)
                    .expect("rfft forward");
                spectrum
            })
            .collect();

        let mut pair_corr = Vec::with_capacity(n_emp * n_emp.saturating_sub(1) / 2);
        for i in 0..n_emp {
            for j in (i + 1)..n_emp {
                let mut cross: Vec<Complex<f64>> = spectra[i]
                    .iter()
                    .zip(spectra[j].iter())
                    .map(|(xi, xj)| {
                        let c = xi * xj.conj();
                        let mag = c.norm();
                        if mag > PHAT_EPS {
                            c / mag
                        } else {
                            Complex::new(0.0, 0.0)
                        }
                    })
                    .collect();

                let mut corr = c2r.make_output_vec();
                c2r.process(&mut cross, &mut corr).expect("rfft inverse");
                pair_corr.push(corr);
            }
        }

        Self {
            n_fft,
            n_emp,
            pair_corr,
        }
    }

    /// Upper-triangular pair index for `i < j`.
    #[inline]
    pub fn pair_index(&self, i: usize, j: usize) -> usize {
        let n = self.n_emp;
        let row_start = i * n - (i * (i + 1)) / 2;
        row_start + (j - i - 1)
    }

    #[inline]
    fn at(&self, corr: &[f64], lag: i64) -> f64 {
        let n = self.n_fft as i64;
        let idx = ((lag % n) + n) % n;
        corr[idx as usize]
    }

    /// Correlation value for pair `(i,j)` at a (possibly fractional) lag, with
    /// linear interpolation and circular wraparound for negative lags.
    #[inline]
    pub fn lookup(&self, i: usize, j: usize, tau: f64) -> f64 {
        let corr = &self.pair_corr[self.pair_index(i, j)];
        let n = self.n_fft;
        let nf = n as f64;
        let mut t = tau % nf;
        if t < 0.0 {
            t += nf;
        }
        let lo = t.floor();
        let frac = t - lo;
        let lo_idx = (lo as usize) % n;
        let hi_idx = (lo_idx + 1) % n;
        corr[lo_idx] * (1.0 - frac) + corr[hi_idx] * frac
    }

    /// Maximum correlation over the lag window `[tau - half_win, tau + half_win]`
    /// (integer lags). Preserves the peak height so the coarse SRP grid keeps
    /// full contrast: the windowed max equals the pair's peak iff that peak's
    /// true lag is reachable from `tau` within `half_win`.
    #[inline]
    pub fn lookup_pooled(&self, i: usize, j: usize, tau: f64, half_win: i64) -> f64 {
        let corr = &self.pair_corr[self.pair_index(i, j)];
        let center = tau.round() as i64;
        let mut best = f64::NEG_INFINITY;
        for lag in (center - half_win)..=(center + half_win) {
            let v = self.at(corr, lag);
            if v > best {
                best = v;
            }
        }
        best
    }

    /// Sub-sample lag of the global correlation peak for pair `(i,j)`, expressed
    /// as a signed lag in `(-n_fft/2, n_fft/2]`. For a single dominant source
    /// this is the unambiguous TDOA `D_i - D_j`.
    pub fn global_peak(&self, i: usize, j: usize) -> f64 {
        let corr = &self.pair_corr[self.pair_index(i, j)];
        let half = (self.n_fft / 2) as i64;
        let mut best_lag = 0i64;
        let mut best_val = f64::NEG_INFINITY;
        for lag in -half..half {
            let v = self.at(corr, lag);
            if v > best_val {
                best_val = v;
                best_lag = lag;
            }
        }
        self.parabolic(corr, best_lag)
    }

    /// The `k` strongest, well-separated correlation peaks for pair `(i,j)` as
    /// sub-sample lags, strongest first. With multiple sources each contributes
    /// a peak here; enumerating combinations across reference pairs is how the
    /// solver disambiguates which TDOA belongs to which source.
    pub fn top_peaks(&self, i: usize, j: usize, k: usize, min_sep: i64) -> Vec<f64> {
        let corr = &self.pair_corr[self.pair_index(i, j)];
        let half = (self.n_fft / 2) as i64;

        // Local maxima with positive correlation.
        let mut maxima: Vec<(i64, f64)> = Vec::new();
        for lag in (-half + 1)..(half - 1) {
            let v = self.at(corr, lag);
            if v > 0.0 && v >= self.at(corr, lag - 1) && v > self.at(corr, lag + 1) {
                maxima.push((lag, v));
            }
        }
        maxima.sort_by(|a, b| b.1.total_cmp(&a.1));

        let mut picked: Vec<i64> = Vec::new();
        for (lag, _) in maxima {
            if picked.len() >= k {
                break;
            }
            if picked.iter().all(|&p| (p - lag).abs() >= min_sep) {
                picked.push(lag);
            }
        }
        picked
            .into_iter()
            .map(|lag| self.parabolic(corr, lag))
            .collect()
    }

    /// Parabolic sub-sample refinement of an integer peak lag.
    #[inline]
    fn parabolic(&self, corr: &[f64], best_lag: i64) -> f64 {
        let cm = self.at(corr, best_lag - 1);
        let c0 = self.at(corr, best_lag);
        let cp = self.at(corr, best_lag + 1);
        let denom = cm - 2.0 * c0 + cp;
        if denom.abs() < 1.0e-18 {
            best_lag as f64
        } else {
            best_lag as f64 + 0.5 * (cm - cp) / denom
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn delayed_pair(n_samples: usize, delay: usize, freq_cyc: f64) -> Frames {
        let mut data = vec![0.0; 2 * n_samples];
        let sigma = 20.0;
        let center0 = (n_samples / 3) as f64;
        for n in 0..n_samples {
            let t0 = n as f64 - center0;
            let env0 = (-0.5 * (t0 / sigma).powi(2)).exp();
            data[n] = env0 * (2.0 * std::f64::consts::PI * freq_cyc * t0).cos();

            let t1 = n as f64 - (center0 + delay as f64);
            let env1 = (-0.5 * (t1 / sigma).powi(2)).exp();
            data[n_samples + n] = env1 * (2.0 * std::f64::consts::PI * freq_cyc * t1).cos();
        }
        Frames {
            n_emp: 2,
            n_samples,
            data,
        }
    }

    #[test]
    fn pair_index_upper_triangular() {
        let g = CorrTable {
            n_fft: 8,
            n_emp: 4,
            pair_corr: vec![vec![0.0; 8]; 6],
        };
        assert_eq!(g.pair_index(0, 1), 0);
        assert_eq!(g.pair_index(0, 3), 2);
        assert_eq!(g.pair_index(1, 2), 3);
        assert_eq!(g.pair_index(2, 3), 5);
    }

    #[test]
    fn peak_at_known_delay() {
        let delay = 13;
        let frames = delayed_pair(512, delay, 0.05);
        let g = CorrTable::compute(&frames);

        // Channel 1 is channel 0 delayed by +13 → peak at lag D_0 - D_1 = -13.
        let lag = g.global_peak(0, 1);
        assert!((lag - (-(delay as f64))).abs() < 0.5, "recovered lag {lag}");

        // top_peaks should surface that same dominant lag first.
        let top = g.top_peaks(0, 1, 2, 5);
        assert!(!top.is_empty() && (top[0] - (-(delay as f64))).abs() < 0.5);
    }

    #[test]
    fn pooled_lookup_recovers_peak_height_within_window() {
        let delay = 13;
        let frames = delayed_pair(512, delay, 0.05);
        let g = CorrTable::compute(&frames);

        // Point-sampling at lag 0 misses the sharp peak at -13...
        let point = g.lookup(0, 1, 0.0);
        // ...but pooling over ±20 captures it.
        let pooled = g.lookup_pooled(0, 1, 0.0, 20);
        assert!(
            pooled > point,
            "pooled {pooled} should exceed point {point}"
        );
    }
}
