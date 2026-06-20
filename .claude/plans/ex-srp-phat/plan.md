# Plan: `ex_srp_phat` — standalone Rust-NIF SRP-PHAT acoustic localizer

**Slug:** `ex-srp-phat`
**Type:** New Elixir library wrapping an in-tree Rust NIF (rustler). **Not** a Phoenix
feature — no Ecto/LiveView/Oban. Pure DSP library.
**Working dir:** `~/Developer/io/ex_srp_phat` (currently empty except `.claude/`)
**Depth:** deep
**Acceptance:** green `cargo test` + `mix test`, README documenting the I/O + coordinate
contract, tagged `v0.1.0`. **Do NOT modify the Augur repo.**

---

## Context gathered (research is done — see findings below)

Three reference repos were studied. Key facts that drive this plan:

### From `ex_ballistics_engine` (the primary template)
- `use RustlerPrecompiled, otp_app:, crate:, base_url:, force_build:, version:, nif_versions:`
  in a dedicated `lib/<app>/native.ex`. Source build gated by
  `force_build: System.get_env("EX_BALLISTICS_ENGINE_BUILD") in ["1","true"] or Mix.env() in [:dev,:test]`.
- NIF stubs return `:erlang.nif_error(:nif_not_loaded)`.
- Crate at `native/ballistics_nif/`, `crate-type = ["cdylib"]`, `publish = false`,
  `rustler = "~> 0.38"` (mix) / `"0.38"` (cargo), NIF-version features
  `nif_version_2_15/16/17`, `.cargo/config.toml` adds `-C target-feature=-crt-static`
  for musl.
- `#[rustler::nif(schedule = "DirtyCpu")]`, `rustler::init!("Elixir.ExBallisticsEngine.Native")`.
- Types mirrored with `#[derive(NifStruct)] #[module="..."]` and `#[derive(NifUnitEnum)]`.
- Returns `Result<T, NifUnitEnum>` → `{:ok,_}|{:error,atom}`. Elixir validates first.
- CI: `ci.yml` (ubuntu+macos `mix compile --warnings-as-errors`, `mix format --check`,
  `mix test`; rust `cargo fmt --check` + `cargo clippy -- -D warnings`; global
  `EX_BALLISTICS_ENGINE_BUILD: "1"`). `release.yml` on `v*` tags builds 7 targets via
  `philss/rustler-precompiled-action@v1.1.4` → GitHub release + `checksum-*.exs`.
- README order: title/one-liner → API → install (git dep now, hex later, private-repo
  source-build note) → quickstart → **coordinate frame** → **units table** → precompiled
  vs source → scope.
- Versioned via `@version` module attr; tag `v0.1.0` exists. `.formatter.exs` minimal.
  `.gitignore` ignores `native/*/target/` and `/priv/native/`.

### From `ex_mgrs`
- Same shape (RustlerPrecompiled, `native/geoconvert_nif/`). Confirms WGS-84 constants:
  `a = 6_378_137.0`, `f = 1/298.257223563`.
- The embedded `geoconvert` crate has **no ECEF transform** → we vendor our own (forward
  is exact closed-form; trivial to match to f64 precision).

### From the Augur oracle (the parity target — DO NOT modify Augur)
- `Localizer` behaviour callback (mirror this signature exactly, minus Augur types):
  `localize(frames, geometry, opts) :: {:ok, [Source.t()]} | {:error, term()}`
  - `frames :: [%{emplacement_id: term(), samples: [float()]}]`
  - `geometry :: %{sample_rate_hz: pos_integer(), emplacements: [%{emplacement_id: term(), ecef: {x,y,z}}]}`
- **`@speed_of_sound = 343.0`**. Slant range = **Euclidean ECEF chord**
  `sqrt(dx²+dy²+dz²)` (NOT great-circle). Geometry **recentered at `p0`** before forming
  squared terms (catastrophic-cancellation fix); solve local, shift back.
- Oracle `source_ref = "band-#{round(freq)}"`. **Our lib exposes `dominant_hz` instead**
  (Augur's adapter builds the ref later). Parity: `round(dominant_hz) == 600`.
- **Real oracle test tolerance is `dist(src.ecef, source) < 50.0` m** (Euclidean ECEF) +
  `confidence > 0.0` + `source_ref == "band-600"`. This is the bar — tighter than the
  `~0.002°` in the brief. We assert the 50 m ECEF bar.
- Fixture constants (Augur `spectral_localizer_test.exs`):
  `fs=4_000`, `c=343.0`, `base_offset=1_200`, `sigma=50.0`, `len=2_600`, `freq=600.0`.
  - Emplacements (lat,lon,alt): `(35.0000,-106.0000,1600)`, `(35.0006,-106.0000,1600)`,
    `(35.0000,-106.0008,1600)`, `(35.0004,-106.0004,1670)`. Source `(35.0003,-106.0003,1720)`.
  - `delay_samples(ep) = base_offset + (dist(ep,src) - dist(e0,src))/c * fs`
  - `wavelet(n,center,freq): t=n-center; exp(-0.5*(t/sigma)^2) * cos(2*pi*freq*t/fs)`
  - `signal(ep) = Σ_sources wavelet(n, delay_samples(ep,src), freq)` for n in 0..len-1.
- The oracle's *internal* algorithm is spectral peak-pick (Goertzel) + band isolation +
  cross-correlation TDOA + closed-form multilateration. **We are NOT required to copy that
  algorithm** — the brief specifies GCC-PHAT → SRP-PHAT grid search. The bar is matching
  the *output* (recovered position < 50 m, dominant ~600 Hz) on this exact fixture.

Reference reports saved under `.claude/plans/ex-srp-phat/research/` (deletable).

---

## Architecture / approach

```
ex_srp_phat/
├── mix.exs                         # @version "0.1.0", rustler ~>0.38 + rustler_precompiled ~>0.9
├── .formatter.exs  .gitignore  README.md  CHANGELOG.md  LICENSE
├── lib/
│   ├── ex_srp_phat.ex              # public API: localize/3 (validate → pack → NIF → decode)
│   ├── ex_srp_phat/source.ex       # %ExSrpPhat.Source{} struct
│   ├── ex_srp_phat/native.ex       # use RustlerPrecompiled; NIF stubs (binary in/out)
│   └── ex_srp_phat/codec.ex        # Elixir-side pack/unpack of f64 binaries + headers
├── native/srp_phat/
│   ├── Cargo.toml                  # cdylib, publish=false, rustler/rustfft/realfft/ndarray/rayon
│   ├── Cargo.lock
│   ├── .cargo/config.toml          # musl -crt-static
│   └── src/
│       ├── lib.rs                  # #[rustler::nif(schedule="DirtyCpu")] localize_nif/3 (binaries)
│       ├── codec.rs                # decode packed frames+geometry header; encode results buffer
│       ├── wgs84.rs                # lat/lon/alt ↔ ECEF (a, f constants); recenter helpers
│       ├── gcc_phat.rs             # realfft per channel, pairwise conj·whiten·ifft correlation
│       ├── srp.rs                  # coarse-to-fine grid steer-&-sum over GCC-PHAT, peak pick
│       ├── solve.rs               # orchestrate: build sources, confidence, covariance, dominant_hz
│       └── tests.rs               # #[cfg(test)] geometry/multi-source/grid-cost guards
└── .github/workflows/ci.yml (+ release.yml stub for when repo goes public)
```

**Marshalling (the performance contract).** PCM + geometry cross the boundary as **packed
`f64` binaries**, never term lists:
- **Frames buffer**: header `[n_emp:u32][n_samples:u32]` then `n_emp × n_samples` f64 PCM,
  row-major per emplacement (frames are time-aligned & equal length — validate in Elixir).
- **Geometry buffer**: `[sample_rate_hz:f64][n_emp:u32]` then `n_emp × 3` f64 ECEF (x,y,z),
  ordered to match the frames buffer (Elixir aligns by `emplacement_id`).
- **Opts**: pass grid bounds/resolution/max_sources as a small fixed f64/u32 header (or a
  `NifStruct` of scalars — scalars are cheap; only the PCM must be binary).
- **Results buffer**: `[n_src:u32]` then per source a fixed-width record:
  `ecef(3 f64) | velocity(3 f64) | radial_velocity(f64) | has_velocity(f64 flag) |
   covariance(9 f64) | has_cov(f64) | confidence(f64) | dominant_hz(f64)`.
  Elixir `codec.ex` unpacks to `%ExSrpPhat.Source{}` list. Use `nil` where flags are 0.
- Use little-endian `f64`/`u32` consistently (`<<x::float-little-64>>`, `binary` in Rust via
  `rustler::Binary` / return `OwnedBinary`).

**Algorithm (Rust).**
1. **GCC-PHAT** (`gcc_phat.rs`): `realfft` each channel → for each emplacement pair
   `i<j`: `X_i · conj(X_j)`, divide by `|·|` (+ε) for PHAT whitening, IFFT → correlation
   vs lag. Cache per-pair correlations.
2. **SRP grid search** (`srp.rs`): candidate ECEF grid (coarse→fine) around the array
   centroid; **recenter geometry at p0** before any distance/squared term. For each
   candidate, compute per-emplacement slant range (Euclidean chord), convert pairwise
   range-difference → expected lag (`Δrange/c * fs`), gather that pairwise GCC-PHAT value,
   sum over pairs. Parallelize the grid with **rayon**. Peaks = sources; significant-peak
   count (relative threshold + non-max suppression) = source count, capped by
   `max_sources`.
3. **dominant_hz** (`solve.rs`): steer-and-sum the channels at the peak's delays, take the
   bin of max magnitude from its rfft → dominant frequency. Must round to 600 on the
   fixture.
4. **confidence**: normalized SRP peak sharpness (peak vs local background) clamped 0..1.
5. **position_covariance**: from response-surface curvature (Hessian of SRP near the peak)
   → 3×3 ECEF row-major `[9 f64]`. May be `nil` if curvature is degenerate.
6. **velocity_mps / radial_velocity**: single time-aligned frame → `nil` for v0 (Doppler
   deferred; flags set to 0 in the results buffer).

**WGS-84 (`wgs84.rs`).** Forward (exact closed-form):
`N = a/sqrt(1 - e²·sin²φ); X=(N+h)cosφcosλ; Y=(N+h)cosφsinλ; Z=(N(1-e²)+h)sinφ`
with `a=6_378_137.0`, `f=1/298.257223563`, `e²=f(2-f)=0.00669437999014132`. Inverse via
Bowring/iterative (only for convenience; **parity test compares in ECEF**, so inverse
precision is off the critical path). Document the formula + constants in README.

**Elixir API (`ex_srp_phat.ex`).**
```elixir
@spec localize(frames, geometry, opts) :: {:ok, [ExSrpPhat.Source.t()]} | {:error, term()}
```
Validates (non-empty frames, equal sample lengths, ≥3 emplacements, ids in frames ⊆ ids in
geometry, positive sample_rate), aligns frames↔geometry by `emplacement_id`, packs binaries
via `codec.ex`, calls `Native.localize_nif/3`, unpacks results to `%ExSrpPhat.Source{}`.
Returns `{:error, reason}` for bad input *before* crossing the NIF.

`%ExSrpPhat.Source{}`: `ecef`, `velocity_mps` (nil), `radial_velocity` (nil),
`position_covariance` ([9 floats]|nil), `confidence` (0.0..1.0), `dominant_hz`. **No
`source_ref`** (Augur's adapter owns identity).

---

## Tasks

### Phase 0 — Scaffold & conventions  `[rust][mix]`
- [x] `cd ~/Developer/io/ex_srp_phat && git init` (repo not yet initialized).
- [x] `mix.exs`: `@version "0.1.0"`, `@source_url ".../ex_srp_phat"`, `elixir: "~> 1.15"`,
      deps `{:rustler, "~> 0.38", runtime: false, optional: true}`,
      `{:rustler_precompiled, "~> 0.9"}`, `{:ex_doc, "~> 0.34", only: :dev, runtime: false}`;
      `description/0`, `package/0` (files list incl. `native/srp_phat/src`,
      `native/srp_phat/.cargo`, `Cargo.toml`, `Cargo.lock`, `checksum-*.exs`), `docs/0`.
      Mirror ballistics mix.exs exactly.
- [x] `.formatter.exs`, `.gitignore` (ignore `native/*/target/`, `/priv/native/`),
      `LICENSE` (MIT), `CHANGELOG.md`.
- [x] `lib/ex_srp_phat/native.ex`: `use RustlerPrecompiled, otp_app: :ex_srp_phat,
      crate: "srp_phat", base_url: ".../releases/download/v#{version}",
      force_build: System.get_env("EX_SRP_PHAT_BUILD") in ["1","true"] or Mix.env() in [:dev,:test],
      version: version, nif_versions: ["2.15"]`. Stub `localize_nif/3 → :erlang.nif_error(:nif_not_loaded)`.
- [x] `native/srp_phat/Cargo.toml`: `crate-type=["cdylib"]`, `publish=false`, `edition="2021"`,
      deps `rustler="0.38"`, `rustfft="6.4"`, `realfft="3.5"`, `ndarray={version="0.17",features=["rayon"]}`,
      `rayon="1"`; NIF-version features `nif_version_2_15/16/17`.
- [x] `native/srp_phat/.cargo/config.toml`: musl `-C target-feature=-crt-static` (both arches).
- [x] `native/srp_phat/src/lib.rs`: module decls + `rustler::init!("Elixir.ExSrpPhat.Native")`
      + empty `#[rustler::nif(schedule="DirtyCpu")] fn localize_nif(...)` returning a stub error.
- [x] **Verify:** `mix deps.get && EX_SRP_PHAT_BUILD=1 mix compile` builds the cdylib clean.

 — WGS-84 + codec (the boundary)  `[rust]`
- [x] `wgs84.rs`: forward `latlon_alt_to_ecef` (constants above), inverse (Bowring),
      `recenter`/`uncenter` helpers, `slant_range` (Euclidean chord). Rust unit tests:
      round-trip a known point; assert chord ≠ haversine.
- [x] `codec.rs`: decode frames buffer (header + row-major f64) and geometry buffer; encode
      results buffer (fixed-width records + nil flags). Rust unit tests for pack/unpack
      symmetry.
- [x] `lib/ex_srp_phat/codec.ex`: Elixir mirror — pack frames/geometry/opts to binaries,
      unpack results to `%ExSrpPhat.Source{}` (flags→nil). Property-ish test for round-trip.
- [x] `lib/ex_srp_phat/source.ex`: define the struct + `@type t`.

### Phase 2 — GCC-PHAT + SRP grid search  `[rust]`
- [x] `gcc_phat.rs`: per-channel `realfft`; pairwise conj-multiply → PHAT whiten (÷|·|+ε)
      → IFFT → correlation; return per-pair correlation indexed by lag. Test: a synthetic
      pair with known integer delay recovers that lag's peak.
- [x] `srp.rs`: recenter geometry at p0; coarse→fine ECEF grid; steer-&-sum pairwise
      GCC-PHAT at each candidate's range-difference lags; rayon-parallel; non-max
      suppression + relative-significance threshold → peak list (≤ max_sources).
- [x] `solve.rs`: assemble `Source` records — dominant_hz (rfft of steered-summed peak
      signal), confidence (normalized sharpness), covariance (peak-curvature Hessian → [9]),
      velocity/radial = nil for v0.
- [x] Wire `localize_nif/3` in `lib.rs`: `Binary` in → decode → solve → `OwnedBinary` out.
- [x] **Verify:** `cd native/srp_phat && cargo test && cargo clippy -- -D warnings && cargo fmt --check`.

 — Rust acceptance tests (`tests.rs`)  `[rust]`
- [x] Known geometry + synthesized inter-emplacement delays → recovered position within
      tolerance (single source).
- [x] Multi-source separation: two distinct-band sources → two peaks at correct positions.
- [x] Grid-resolution-vs-cost guard: assert candidate count / runtime stays under a bound
      for the coarse→fine schedule (prevents accidental O(n³) blowups).

### Phase 4 — Elixir surface + parity fixture  `[elixir][test]`
- [x] `ex_srp_phat.ex`: `localize/3` with full input validation (see API section),
      align-by-id, pack→NIF→unpack, `{:ok,[Source]} | {:error, reason}`. `@moduledoc` +
      doctest-able quickstart.
- [x] `test/support/fixture.ex` (or inline): reproduce the Augur Morlet fixture **exactly** —
      same constants, `delay_samples`, `wavelet`, `signal`; lat/lon/alt → ECEF via our
      `wgs84` (through the NIF or a thin Elixir helper) so geometry matches the oracle.
- [x] `test/ex_srp_phat_test.exs`: single 600 Hz source →
      `assert {:ok, [%Source{} = s]} = localize(frames, geometry)`;
      `assert ecef_dist(s.ecef, true_src) < 50.0`; `assert s.confidence > 0.0`;
      `assert round(s.dominant_hz) == 600`. (Matches the real oracle bar.)
- [x] Input-validation tests (no NIF crossing): too few emplacements, mismatched sample
      lengths, unknown emplacement_id, non-positive sample_rate → `{:error, _}`.
- [x] **Stretch:** second distinct-band source in the fixture → two `%Source{}`, each within
      tolerance, distinct `dominant_hz`.
- [x] **Verify:** `mix format --check-formatted && mix compile --warnings-as-errors && mix test`.

 — Docs, CI, release  `[docs][ci]`
- [x] `README.md` in ballistics order: one-liner → `localize/3` API + I/O contract →
      install (git dep `tag: "v0.1.0"`, hex later, **private-repo `EX_SRP_PHAT_BUILD=1`
      source-build note**) → quickstart → **coordinate frame** (WGS-84 ECEF meters, slant
      range = Euclidean chord, recenter-at-reference note) → units table → **`%Source{}`
      field reference incl. `dominant_hz` and the no-`source_ref` identity note** →
      precompiled-vs-source → v0.1 scope (Doppler/velocity deferred).
- [x] `.github/workflows/ci.yml`: ubuntu+macos `mix compile --warnings-as-errors`,
      `mix format --check`, `mix test` with `EX_SRP_PHAT_BUILD: "1"`; rust `cargo fmt --check`
      + `cargo clippy -- -D warnings`. Cache `mix.lock` + `Cargo.lock`.
- [x] `.github/workflows/release.yml`: `philss/rustler-precompiled-action` on `v*` tags
      (carry over ballistics' 7-target matrix; fine to keep even while private — it just
      won't run until tagged/public).
- [x] **Verify (final gate):** clean `cargo test` + `mix test`; `git tag v0.1.0`.
      **STOP — do not touch the Augur repo.**

---

## Verification commands
```bash
# Rust
cd ~/Developer/io/ex_srp_phat/native/srp_phat && cargo test && cargo clippy -- -D warnings && cargo fmt --check
# Elixir
cd ~/Developer/io/ex_srp_phat && EX_SRP_PHAT_BUILD=1 mix compile --warnings-as-errors && mix format --check-formatted && mix test
```

## Risks & self-check (deep)
- **Will the SRP grid search actually hit < 50 m on this geometry?** The oracle achieves
  ~50 m via closed-form multilateration; a grid search is limited by its finest cell size.
  *Mitigation:* coarse→fine must refine to ≤ ~10 m near the peak (and/or parabolic
  sub-cell interpolation) so quantization stays well under 50 m. This is the single biggest
  technical risk — validate early with a Rust test on the fixture geometry before building
  the full Elixir surface.
- **dominant_hz must round to exactly 600.** rfft bin spacing at `len=2_600, fs=4_000` is
  ~1.54 Hz — fine enough; but steer-and-sum windowing could bias the peak. Verify the bin
  pick lands in [599.5, 600.5).
- **WGS-84 parity.** Forward transform is exact closed-form → matches Augur's to f64. We
  compare recovered position in **ECEF** (not lat/lon) to keep the inverse transform off the
  critical path. Confirmed safe.
- **What am I NOT copying from the oracle?** Its internal Goertzel/cross-correlation
  algorithm. That's intentional — the brief mandates GCC-PHAT→SRP-PHAT, and the contract is
  output parity, not algorithm parity. Flagged so a reviewer doesn't expect line-by-line
  equivalence.
- **rustler_precompiled vs plain Rustler.** Plan uses `rustler_precompiled` + `force_build`
  (the exact ballistics pattern, and what "gate a source build like EX_BALLISTICS_ENGINE_BUILD=1"
  implies). The brief's `use Rustler, otp_app:, crate:` line is the simpler source-only
  alternative — see Open Questions.

## Open questions (non-blocking; defaults chosen)
1. **Precompiled packaging vs source-only.** *Default:* mirror ballistics
   (`rustler_precompiled` + `force_build` + `release.yml`). Alternative: plain `use Rustler`
   (simpler, no release pipeline) since the repo stays private for now.
2. **f32 vs f64 PCM buffer.** *Default:* f64 end-to-end (matches oracle math, simplest
   parity). f32 halves marshalling cost if profiling later demands it.
3. **Doppler/velocity in v0.** *Default:* `nil` (single time-aligned frame can't estimate
   it; brief marks it optional). Defer to v0.2.
