defmodule ExSrpPhat do
  @moduledoc """
  GCC-PHAT → SRP-PHAT acoustic source localization over a known microphone-array
  geometry, backed by a Rust NIF.

  One **solve** fuses a single time-aligned frame-set (one PCM frame per
  emplacement) against the array's known WGS-84 **ECEF** geometry into zero or
  more localized `ExSrpPhat.Source` structs.

  ## Coordinate frame & units

    * Positions are **WGS-84 ECEF meters**. Slant range is straight-line
      Euclidean (chord) distance — sound travels the chord, not a surface arc.
    * Speed of sound is `343.0 m/s` (~20 °C).
    * Use `ExSrpPhat.Geo` to convert lat/lon/alt → ECEF when building geometry.

  ## Example

      iex> geometry = %{
      ...>   sample_rate_hz: 4_000,
      ...>   emplacements: [
      ...>     %{emplacement_id: :a, ecef: ExSrpPhat.Geo.latlon_to_ecef(35.0, -106.0, 1600.0)},
      ...>     %{emplacement_id: :b, ecef: ExSrpPhat.Geo.latlon_to_ecef(35.0006, -106.0, 1600.0)},
      ...>     %{emplacement_id: :c, ecef: ExSrpPhat.Geo.latlon_to_ecef(35.0, -106.0008, 1600.0)},
      ...>     %{emplacement_id: :d, ecef: ExSrpPhat.Geo.latlon_to_ecef(35.0004, -106.0004, 1670.0)}
      ...>   ]
      ...> }
      iex> frames = Enum.map(geometry.emplacements, &%{emplacement_id: &1.emplacement_id, samples: List.duplicate(0.0, 16)})
      iex> {:ok, sources} = ExSrpPhat.localize(frames, geometry)
      iex> is_list(sources)
      true
  """

  alias ExSrpPhat.{Codec, Native, Source}

  @type vec3 :: {number(), number(), number()}
  @type frame :: %{emplacement_id: term(), samples: [number()]}
  @type emplacement :: %{emplacement_id: term(), ecef: vec3()}
  @type geometry :: %{sample_rate_hz: pos_integer(), emplacements: [emplacement()]}

  @doc """
  Localize acoustic sources from a time-aligned frame-set.

  `frames` is a list of `%{emplacement_id: id, samples: [float]}`; every frame
  must share the same sample length. `geometry` carries the sample rate and one
  ECEF position per emplacement. Frames are matched to geometry by
  `emplacement_id` (order-independent); the two id sets must be identical.

  `opts` (all optional) tune the search — see `ExSrpPhat.Codec.pack_opts/1` for
  keys and defaults (`:grid_extent_m`, `:coarse_res_m`, `:fine_res_m`,
  `:min_peak_ratio`, `:max_sources`).

  Returns `{:ok, [%ExSrpPhat.Source{}]}` or `{:error, reason}`. Input is fully
  validated in Elixir before crossing the NIF boundary.
  """
  @spec localize([frame()], geometry(), keyword()) :: {:ok, [Source.t()]} | {:error, term()}
  def localize(frames, geometry, opts \\ []) do
    with {:ok, fs} <- validate_sample_rate(geometry),
         {:ok, channels, ecef_list} <- align(frames, geometry),
         :ok <- validate_lengths(channels) do
      frames_bin = Codec.pack_frames(channels)
      geometry_bin = Codec.pack_geometry(fs, ecef_list)
      opts_bin = Codec.pack_opts(opts)

      case Native.localize_nif(frames_bin, geometry_bin, opts_bin) do
        {:ok, results_bin} -> {:ok, Codec.unpack_results(results_bin)}
        {:error, reason} -> {:error, reason}
      end
    end
  end

  # `sample_rate_hz` is a positive integer per the type contract; reject anything
  # else (including floats and NaN, which cannot be integers).
  defp validate_sample_rate(%{sample_rate_hz: fs}) when is_integer(fs) and fs > 0, do: {:ok, fs}
  defp validate_sample_rate(%{sample_rate_hz: fs}), do: {:error, {:invalid_sample_rate, fs}}
  defp validate_sample_rate(_), do: {:error, :missing_sample_rate}

  defp align(frames, %{emplacements: emplacements})
       when is_list(frames) and is_list(emplacements) do
    cond do
      length(emplacements) < 2 ->
        {:error, :too_few_emplacements}

      not Enum.all?(frames, &valid_frame?/1) ->
        {:error, :invalid_frame}

      not Enum.all?(emplacements, &valid_emplacement?/1) ->
        {:error, :invalid_emplacement}

      true ->
        frame_ids = MapSet.new(frames, & &1.emplacement_id)
        geo_ids = MapSet.new(emplacements, & &1.emplacement_id)

        if MapSet.equal?(frame_ids, geo_ids) and MapSet.size(frame_ids) == length(frames) do
          by_id = Map.new(frames, &{&1.emplacement_id, &1.samples})
          channels = Enum.map(emplacements, &Map.fetch!(by_id, &1.emplacement_id))
          ecef_list = Enum.map(emplacements, & &1.ecef)
          {:ok, channels, ecef_list}
        else
          {:error, :emplacement_mismatch}
        end
    end
  end

  defp align(_frames, _geometry), do: {:error, :invalid_arguments}

  defp valid_frame?(%{emplacement_id: _, samples: s}) when is_list(s) and s != [], do: true
  defp valid_frame?(_), do: false

  defp valid_emplacement?(%{emplacement_id: _, ecef: {x, y, z}})
       when is_number(x) and is_number(y) and is_number(z),
       do: true

  defp valid_emplacement?(_), do: false

  defp validate_lengths(channels) do
    distinct_lengths = channels |> Enum.map(&length/1) |> Enum.uniq()

    case distinct_lengths do
      [_single] -> :ok
      _ -> {:error, :unequal_frame_lengths}
    end
  end
end
