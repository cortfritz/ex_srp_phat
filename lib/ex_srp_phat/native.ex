defmodule ExSrpPhat.Native do
  @moduledoc """
  NIF entry points, loaded via `RustlerPrecompiled`.

  Do not call this module directly — use `ExSrpPhat.localize/3`, which validates
  inputs and packs PCM + geometry into binaries before crossing the NIF boundary.

  Precompiled binaries are downloaded from GitHub release assets; set
  `EX_SRP_PHAT_BUILD=1` to force a source build (requires a Rust toolchain). The
  `:dev` and `:test` environments of this library always build from source.
  """

  version = Mix.Project.config()[:version]

  use RustlerPrecompiled,
    otp_app: :ex_srp_phat,
    crate: "srp_phat",
    base_url: "https://github.com/cortfritz/ex_srp_phat/releases/download/v#{version}",
    # Compile-time only: `Mix.env()` here selects source vs precompiled build at
    # build time (the standard RustlerPrecompiled pattern) — it does not leak the
    # environment into runtime application code.
    force_build:
      System.get_env("EX_SRP_PHAT_BUILD") in ["1", "true"] or
        Mix.env() in [:dev, :test],
    version: version,
    nif_versions: ["2.15"],
    # Exactly the targets built by .github/workflows/release.yml. Consumers on
    # any other platform fall back to a source build (needs a Rust toolchain).
    targets: [
      "aarch64-apple-darwin",
      "x86_64-apple-darwin",
      "x86_64-unknown-linux-gnu",
      "aarch64-unknown-linux-gnu",
      "x86_64-unknown-linux-musl",
      "aarch64-unknown-linux-musl",
      "x86_64-pc-windows-msvc"
    ]

  @doc """
  Run a single SRP-PHAT solve.

  All three arguments are little-endian packed binaries (see `ExSrpPhat.Codec`):

    * `frames_bin` — header + row-major `f64` PCM, one row per emplacement
    * `geometry_bin` — sample rate + `f64` ECEF triples, aligned to `frames_bin`
    * `opts_bin` — packed grid bounds / resolution / `max_sources`

  Returns `{:ok, results_bin}` or `{:error, atom}`. The NIF is overridden at load
  time; this stub only runs if the native library failed to load.
  """
  def localize_nif(_frames_bin, _geometry_bin, _opts_bin),
    do: :erlang.nif_error(:nif_not_loaded)

  @doc "WGS-84 geodetic (deg, deg, m HAE) → ECEF `{x, y, z}` meters."
  def latlon_to_ecef_nif(_lat_deg, _lon_deg, _alt_m),
    do: :erlang.nif_error(:nif_not_loaded)

  @doc "WGS-84 ECEF meters → geodetic `{lat_deg, lon_deg, alt_m}`."
  def ecef_to_latlon_nif(_x, _y, _z),
    do: :erlang.nif_error(:nif_not_loaded)
end
