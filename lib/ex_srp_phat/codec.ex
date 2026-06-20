defmodule ExSrpPhat.Codec do
  @moduledoc """
  Packs PCM + geometry + opts into little-endian `f64`/`u32` binaries for the
  NIF, and unpacks the packed results binary back into `ExSrpPhat.Source`
  structs. Mirrors the Rust `codec` module byte-for-byte.

  Layouts (all integers `u32-little`, all reals `float-little-64`):

    * **frames**:   `[n_emp][n_samples]` then `n_emp × n_samples` PCM, row-major
    * **geometry**: `[sample_rate_hz: f64][n_emp: u32]` then `n_emp × 3` ECEF xyz
    * **opts**:     `[grid_extent_m][coarse_res_m][fine_res_m][min_peak_ratio]` (f64)
                    then `[max_sources: u32]`
    * **results**:  `[n_src: u32]` then per source 21×f64:
                    `xyz(3) | vel(3) | has_vel(1) | radial(1) | has_radial(1) |
                     cov(9) | has_cov(1) | confidence(1) | dominant_hz(1)`
  """

  alias ExSrpPhat.Source

  @doc """
  Pack already-aligned channels (a list of equal-length sample lists, in
  emplacement order) into the frames buffer.

  Sample/coordinate values must be native numbers (`integer` or `float`); they
  are coerced to `f64` via `* 1.0`. `Decimal` and other numeric structs are not
  supported.
  """
  @spec pack_frames([[number()]]) :: binary()
  def pack_frames(channels) do
    n_emp = length(channels)
    n_samples = if n_emp == 0, do: 0, else: length(hd(channels))

    header = <<n_emp::unsigned-little-32, n_samples::unsigned-little-32>>
    body = for ch <- channels, s <- ch, into: <<>>, do: <<s * 1.0::float-little-64>>
    <<header::binary, body::binary>>
  end

  @doc """
  Pack the sample rate and a list of `{x, y, z}` ECEF tuples (in emplacement
  order, aligned to `pack_frames/1`) into the geometry buffer.
  """
  @spec pack_geometry(number(), [{number(), number(), number()}]) :: binary()
  def pack_geometry(sample_rate_hz, ecef_list) do
    header = <<sample_rate_hz * 1.0::float-little-64, length(ecef_list)::unsigned-little-32>>

    body =
      for {x, y, z} <- ecef_list, into: <<>> do
        <<x * 1.0::float-little-64, y * 1.0::float-little-64, z * 1.0::float-little-64>>
      end

    <<header::binary, body::binary>>
  end

  @doc """
  Pack grid-search opts. Recognized keys (all optional, defaults shown):

    * `:grid_extent_m` (300.0) — half-extent of the search cube around the centroid
    * `:coarse_res_m` (20.0) — coarse grid cell size
    * `:fine_res_m` (2.0) — finest grid cell size after refinement
    * `:min_peak_ratio` (0.5) — peak significance threshold (fraction of global max)
    * `:max_sources` (4) — cap on returned sources
  """
  @spec pack_opts(keyword()) :: binary()
  def pack_opts(opts \\ []) do
    grid_extent_m = Keyword.get(opts, :grid_extent_m, 300.0)
    coarse_res_m = Keyword.get(opts, :coarse_res_m, 20.0)
    fine_res_m = Keyword.get(opts, :fine_res_m, 2.0)
    min_peak_ratio = Keyword.get(opts, :min_peak_ratio, 0.5)
    max_sources = Keyword.get(opts, :max_sources, 4)

    <<grid_extent_m * 1.0::float-little-64, coarse_res_m * 1.0::float-little-64,
      fine_res_m * 1.0::float-little-64, min_peak_ratio * 1.0::float-little-64,
      max_sources::unsigned-little-32>>
  end

  @doc "Unpack the results buffer into a list of `ExSrpPhat.Source` structs."
  @spec unpack_results(binary()) :: [Source.t()]
  def unpack_results(<<n_src::unsigned-little-32, rest::binary>>) do
    unpack_sources(rest, n_src, [])
  end

  defp unpack_sources(_bin, 0, acc), do: Enum.reverse(acc)

  defp unpack_sources(bin, remaining, acc) do
    <<x::float-little-64, y::float-little-64, z::float-little-64, vx::float-little-64,
      vy::float-little-64, vz::float-little-64, has_vel::float-little-64, radial::float-little-64,
      has_radial::float-little-64, c0::float-little-64, c1::float-little-64, c2::float-little-64,
      c3::float-little-64, c4::float-little-64, c5::float-little-64, c6::float-little-64,
      c7::float-little-64, c8::float-little-64, has_cov::float-little-64,
      confidence::float-little-64, dominant_hz::float-little-64, tail::binary>> = bin

    source = %Source{
      ecef: {x, y, z},
      velocity_mps: if(has_vel == 1.0, do: {vx, vy, vz}, else: nil),
      radial_velocity: if(has_radial == 1.0, do: radial, else: nil),
      position_covariance:
        if(has_cov == 1.0, do: [c0, c1, c2, c3, c4, c5, c6, c7, c8], else: nil),
      confidence: confidence,
      dominant_hz: dominant_hz
    }

    unpack_sources(tail, remaining - 1, [source | acc])
  end
end
