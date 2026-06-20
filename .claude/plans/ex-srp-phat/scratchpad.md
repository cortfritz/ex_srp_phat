# Scratchpad — ex_srp_phat

## Key decision (Phase 2): grid-search SRP → closed-form multilateration

The brief specifies "SRP-PHAT grid search; grid peaks = sources." **Pure volumetric
grid search does NOT work for this geometry** and was abandoned after measurement:

- The whitened PHAT correlation peak is ~1 sample wide ≈ 0.043 m in position space.
  A 20 m coarse grid moves the inter-pair lag by hundreds of samples per step, so the
  grid **steps over** the peak: point-sampling at grid nodes near the true source
  scored ~1700 while the true source point-scored ~3534.
- **Gaussian-smoothing** the correlation to bridge grid steps destroyed contrast
  (σ≈233 → true 0.0005 vs background 0.0002) → argmax landed on noise (210 m error).
- **Max-pooling** preserved peak height but the window needed to bridge a coarse cell
  (~420 samples) created a huge plateau; with this near-coplanar array the plateau
  extends far in the ill-conditioned vertical direction → argmax drifted to the cube
  corner (~400 m error).
- Gauss-Newton from a 3-equation reference-only system diverged (348 km) under the
  vertical ill-conditioning.

**What works (current design):** the brief's own "critical gotcha" — *recenter geometry
at a reference emplacement before forming squared terms* — describes closed-form TDOA
multilateration, not grid search. So:
1. GCC-PHAT pairwise correlations (`gcc_phat::CorrTable`).
2. Read sub-sample TDOAs from correlation peaks (`global_peak` / `top_peaks`).
3. **Enumerate TDOA combinations** across reference pairs (each pair contributes one
   peak per source) → `srp::multilaterate` (recentered linear system + |s|=r0 closure,
   exact, oracle-identical) for each hypothesis.
4. Keep combinations whose **sharp point-SRP score** is high (the reliable
   discriminator); dedup, threshold, cap at `max_sources`. This localizes AND
   disambiguates overlapping sources without separating them.
5. SRP grid is retained only as the **confidence field** (pooled response-surface
   mean/max → "normalized SRP peak sharpness").

Result: single-source recovered exactly (well under the 50 m oracle bar); two
distinct-band sources both recovered. **README must document this** (combo-enumeration
+ closed-form, with SRP for detection/confidence) rather than implying pure grid search.

## Parity bar (from Augur oracle, confirmed by reading the repo)
- Real oracle test asserts `dist(src.ecef, source) < 50.0` m (Euclidean ECEF), not the
  0.002° in the brief. We target 50 m.
- `dominant_hz` must round to 600; oracle builds `source_ref = "band-#{round(freq)}"`
  in its adapter (we expose `dominant_hz`, no source_ref in the lib).
- Speed of sound 343.0; slant range = Euclidean chord.

## WGS-84
Vendored in `wgs84.rs` (a=6378137.0, f=1/298.257223563, e²=0.00669437999014132).
Exposed via `latlon_to_ecef_nif`/`ecef_to_latlon_nif` so the Elixir parity fixture can
build geometry without an external geodesy dep.

## Binary layout (Rust `codec` ↔ Elixir `ExSrpPhat.Codec`) — keep in lock-step
- frames: `[n_emp:u32][n_samples:u32]` then row-major f64 PCM
- geometry: `[fs:f64][n_emp:u32]` then n_emp×3 f64 ECEF
- opts: `[grid_extent][coarse_res][fine_res][min_peak_ratio]:f64` then `[max_sources:u32]`
- results: `[n_src:u32]` then per source 20×f64:
  xyz(3) vel(3) radial(1) has_vel(1) cov(9) has_cov(1) confidence(1) dominant_hz(1)
