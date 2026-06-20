defmodule ExSrpPhatTest do
  use ExUnit.Case, async: true

  alias ExSrpPhat.{Geo, Source}

  doctest ExSrpPhat

  # --- Augur parity fixture --------------------------------------------------
  # Reproduces the Augur pipeline/controller test input: a Morlet wavelet at each
  # emplacement, delayed by the *true* geometric TDOA. The bar is the oracle's:
  # recovered ECEF within 50 m of truth, dominant frequency → 600.

  @fs 4_000
  @c 343.0
  @base 1_200
  @sigma 50.0
  @len 2_600

  defp emplacements do
    [
      {:e0, Geo.latlon_to_ecef(35.0000, -106.0000, 1_600.0)},
      {:e1, Geo.latlon_to_ecef(35.0006, -106.0000, 1_600.0)},
      {:e2, Geo.latlon_to_ecef(35.0000, -106.0008, 1_600.0)},
      {:e3, Geo.latlon_to_ecef(35.0004, -106.0004, 1_670.0)}
    ]
  end

  defp dist({x1, y1, z1}, {x2, y2, z2}) do
    :math.sqrt(:math.pow(x1 - x2, 2) + :math.pow(y1 - y2, 2) + :math.pow(z1 - z2, 2))
  end

  defp wavelet(n, center, freq) do
    t = n - center
    env = :math.exp(-0.5 * :math.pow(t / @sigma, 2))
    env * :math.cos(2.0 * :math.pi() * freq * t / @fs)
  end

  # sources :: [{ecef, freq}]
  defp signal(ep_ecef, sources, ref_ecef) do
    d0 = Enum.map(sources, fn {src, _} -> dist(ref_ecef, src) end)

    for n <- 0..(@len - 1) do
      sources
      |> Enum.zip(d0)
      |> Enum.reduce(0.0, fn {{src, freq}, dref}, acc ->
        center = @base + (dist(ep_ecef, src) - dref) / @c * @fs
        acc + wavelet(n, center, freq)
      end)
    end
  end

  defp geometry do
    %{
      sample_rate_hz: @fs,
      emplacements:
        Enum.map(emplacements(), fn {id, ecef} -> %{emplacement_id: id, ecef: ecef} end)
    }
  end

  defp frames(sources) do
    [{_ref_id, ref_ecef} | _] = emplacements()

    Enum.map(emplacements(), fn {id, ecef} ->
      %{emplacement_id: id, samples: signal(ecef, sources, ref_ecef)}
    end)
  end

  describe "localize/3 single source (Augur parity)" do
    test "recovers the source position within 50 m and dominant 600 Hz" do
      source = Geo.latlon_to_ecef(35.0003, -106.0003, 1_720.0)

      assert {:ok, sources} = ExSrpPhat.localize(frames([{source, 600.0}]), geometry())
      assert sources != []

      best = Enum.max_by(sources, & &1.confidence)
      assert %Source{ecef: ecef, confidence: confidence, dominant_hz: dominant_hz} = best

      assert dist(ecef, source) < 50.0, "position error #{dist(ecef, source)} m"
      assert confidence > 0.0
      # round(dominant_hz) == 600 lets the Augur adapter reproduce "band-600".
      assert round(dominant_hz) == 600
    end
  end

  describe "localize/3 two distinct-band sources (stretch)" do
    test "recovers both sources within tolerance" do
      src_a = Geo.latlon_to_ecef(35.0003, -106.0003, 1_720.0)
      src_b = Geo.latlon_to_ecef(34.9997, -105.9995, 1_650.0)

      assert {:ok, sources} =
               ExSrpPhat.localize(frames([{src_a, 600.0}, {src_b, 300.0}]), geometry(),
                 min_peak_ratio: 0.35
               )

      # Looser than the 50 m single-source bar: in a two-source field each
      # source's TDOAs are associated from per-pair peaks the other source
      # perturbs, so a few extra meters of spread is expected.
      for src <- [src_a, src_b] do
        nearest = sources |> Enum.map(&dist(&1.ecef, src)) |> Enum.min()
        assert nearest < 60.0, "no source within 60 m of #{inspect(src)} (nearest #{nearest} m)"
      end
    end
  end

  describe "input validation (never crosses the NIF)" do
    test "fewer than two emplacements" do
      geo = %{sample_rate_hz: 4_000, emplacements: [%{emplacement_id: :a, ecef: {1.0, 2.0, 3.0}}]}
      frames = [%{emplacement_id: :a, samples: [0.0, 1.0]}]
      assert {:error, :too_few_emplacements} = ExSrpPhat.localize(frames, geo)
    end

    test "non-positive sample rate" do
      assert {:error, {:invalid_sample_rate, 0}} =
               ExSrpPhat.localize(
                 frames([{Geo.latlon_to_ecef(35.0003, -106.0003, 1_720.0), 600.0}]),
                 %{
                   geometry()
                   | sample_rate_hz: 0
                 }
               )
    end

    test "frame id not present in geometry" do
      [f0 | rest] = frames([{Geo.latlon_to_ecef(35.0003, -106.0003, 1_720.0), 600.0}])
      bad = [%{f0 | emplacement_id: :ghost} | rest]
      assert {:error, :emplacement_mismatch} = ExSrpPhat.localize(bad, geometry())
    end

    test "unequal frame lengths" do
      [f0 | rest] = frames([{Geo.latlon_to_ecef(35.0003, -106.0003, 1_720.0), 600.0}])
      bad = [%{f0 | samples: Enum.take(f0.samples, 100)} | rest]
      assert {:error, :unequal_frame_lengths} = ExSrpPhat.localize(bad, geometry())
    end

    test "negative sample rate" do
      assert {:error, {:invalid_sample_rate, -1}} =
               ExSrpPhat.localize(
                 frames([{Geo.latlon_to_ecef(35.0003, -106.0003, 1_720.0), 600.0}]),
                 %{geometry() | sample_rate_hz: -1}
               )
    end

    test "float sample rate is rejected (must be a positive integer)" do
      assert {:error, {:invalid_sample_rate, 4000.5}} =
               ExSrpPhat.localize(
                 frames([{Geo.latlon_to_ecef(35.0003, -106.0003, 1_720.0), 600.0}]),
                 %{geometry() | sample_rate_hz: 4000.5}
               )
    end

    test "fewer frames than emplacements" do
      [_dropped | rest] = frames([{Geo.latlon_to_ecef(35.0003, -106.0003, 1_720.0), 600.0}])
      assert {:error, :emplacement_mismatch} = ExSrpPhat.localize(rest, geometry())
    end
  end

  describe "Geo WGS-84 round-trip" do
    test "forward then inverse recovers lat/lon/alt" do
      {x, y, z} = Geo.latlon_to_ecef(35.0003, -106.0003, 1_720.0)
      {lat, lon, alt} = Geo.ecef_to_latlon(x, y, z)
      assert_in_delta lat, 35.0003, 1.0e-6
      assert_in_delta lon, -106.0003, 1.0e-6
      assert_in_delta alt, 1_720.0, 1.0e-3
    end
  end
end
