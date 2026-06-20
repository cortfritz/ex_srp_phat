# ExSrpPhat

Elixir NIF bindings (Rustler / RustlerPrecompiled) for a Rust **GCC-PHAT → SRP-PHAT**
acoustic source localizer over a known microphone-array geometry.

`ExSrpPhat.localize/3` fuses a single time-aligned frame-set (one PCM frame per
emplacement) against the array's known WGS-84 **ECEF** geometry into zero or more
localized `ExSrpPhat.Source` structs. The NIF runs on a **dirty CPU scheduler**; PCM and
geometry cross the boundary as packed binaries (never term lists), and the grid search is
`rayon`-parallel.

## I/O contract

```elixir
@spec localize(frames, geometry, opts) :: {:ok, [ExSrpPhat.Source.t()]} | {:error, term()}

# frames:   [%{emplacement_id: term(), samples: [float()]}]   # one frame per emplacement, equal length
# geometry: %{sample_rate_hz: pos_integer(),
#             emplacements: [%{emplacement_id: term(), ecef: {x, y, z}}]}  # meters, WGS-84 ECEF
# opts:     keyword — :grid_extent_m, :coarse_res_m, :fine_res_m, :min_peak_ratio, :max_sources
```

Frames are matched to geometry by `emplacement_id` (order-independent); the two id sets
must be identical. Input is fully validated in Elixir before crossing the NIF — bad input
returns `{:error, reason}` (e.g. `:too_few_emplacements`, `:emplacement_mismatch`,
`:unequal_frame_lengths`, `{:invalid_sample_rate, v}`).

Each detected source:

```elixir
%ExSrpPhat.Source{
  ecef:                {x, y, z},          # meters ECEF
  velocity_mps:        {vx, vy, vz} | nil, # nil in v0 (Doppler deferred)
  radial_velocity:     float() | nil,      # nil in v0
  position_covariance: [9 floats] | nil,   # 3x3 ECEF row-major, from the TDOA system
  confidence:          0.0..1.0,           # normalized SRP peak sharpness
  dominant_hz:         float()             # dominant frequency of the steered peak signal
}
```

### Source identity

This library is library-neutral and deliberately assigns **no** cross-solve `source_ref`.
Stable identity is the caller's concern — `dominant_hz` is exposed so an adapter can
reproduce a convention such as `"band-#{round(dominant_hz)}"`.

## Coordinate frame & units

Positions are **WGS-84 ECEF meters**. Two rules the math depends on:

- **Slant range is straight-line Euclidean (chord) distance**, `sqrt(dx²+dy²+dz²)` — sound
  travels the chord, not a great-circle/surface arc. Emplacements at different altitudes
  are handled correctly.
- Geometry is **recentered at a reference emplacement** before any squared term is formed
  (raw ECEF coords are ~5e6 m; `|p_i|² − |p_0|²` in raw coordinates loses all precision to
  catastrophic cancellation). The solve runs locally and shifts back.

Speed of sound is `343.0 m/s` (~20 °C). Use `ExSrpPhat.Geo` to convert lat/lon/alt → ECEF
(WGS-84: `a = 6_378_137.0` m, `f = 1/298.257223563`); the forward transform is exact
closed form, the inverse uses Bowring's method.

| Field suffix / name | Unit |
|---|---|
| `ecef`, `_m`, `position_covariance` | meters (m², row-major, for covariance) |
| `velocity_mps`, `radial_velocity` | meters/second |
| `sample_rate_hz`, `dominant_hz` | hertz |
| `confidence` | unitless, 0.0–1.0 |

## Algorithm

1. **GCC-PHAT** per emplacement pair: real FFT each channel (`realfft`), conj-multiply,
   phase-transform whitening (divide by magnitude), inverse FFT → a sharp correlation vs
   lag, robust to the source spectrum.
2. **Localization** by closed-form TDOA multilateration. Sub-sample TDOAs are read from the
   correlation peaks; combinations across reference pairs (one peak per source) are each
   solved with the recentered linear system closed by the `|s| = r₀` constraint, and the
   point-sampled SRP score keeps the genuine intersections — localizing overlapping sources
   without separating them. (A volumetric grid search is unusable here: the whitened peak is
   ~centimeters wide in position space, far sharper than any affordable grid; the brief's
   "recenter before squaring" gotcha is precisely the closed-form method.)
3. **Confidence** from the pooled SRP response surface (`rayon`-parallel coarse grid):
   normalized peak sharpness relative to the field.
4. **Covariance** from the linearized range-difference system (`σ²·(JᵀJ)⁻¹`).
5. **dominant_hz** from the steered-and-summed peak signal's strongest spectral bin.

Doppler / velocity estimation is deferred (returns `nil`) — a single time-aligned
frame-set does not constrain it.

## Install

From hex (precompiled binaries, no Rust toolchain required):

```elixir
def deps do
  [{:ex_srp_phat, "~> 0.1.0"}]
end
```

Or pin the git dependency directly:

```elixir
{:ex_srp_phat, github: "cortfritz/ex_srp_phat", tag: "v0.1.0"}
```

## Precompiled binaries vs. source builds

Releases ship precompiled NIF binaries for common platforms; `rustler_precompiled`
downloads and checksum-verifies the right one at compile time, so consumers do **not** need
a Rust toolchain. To force a source build (needs a Rust toolchain):

```bash
EX_SRP_PHAT_BUILD=1 mix deps.compile ex_srp_phat --force
```

The `:dev` and `:test` environments of this library always build from source.

## Development

```bash
# Rust
cd native/srp_phat && cargo test && cargo clippy -- -D warnings && cargo fmt --check
# Elixir
EX_SRP_PHAT_BUILD=1 mix compile --warnings-as-errors && mix format --check-formatted && mix test
```

## v0.1 scope

Single-frame localization of one or more sources, confidence, position covariance, and
dominant frequency. Deferred: Doppler / velocity (`velocity_mps`, `radial_velocity` return
`nil`).

## License

MIT — see [LICENSE](LICENSE).
