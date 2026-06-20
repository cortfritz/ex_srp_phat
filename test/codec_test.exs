defmodule ExSrpPhat.CodecTest do
  use ExUnit.Case, async: true

  alias ExSrpPhat.{Codec, Source}

  # Build one 21-f64 results record the way the Rust `encode_results` does:
  # xyz(3) | vel(3) has_vel(1) | radial(1) has_radial(1) | cov(9) has_cov(1) |
  # confidence(1) | dominant_hz(1). `nil` velocity/radial/cov set their flag to 0.
  defp record({x, y, z}, vel, radial, cov, confidence, dominant) do
    {vx, vy, vz} = vel || {0.0, 0.0, 0.0}
    c = cov || List.duplicate(0.0, 9)

    fields =
      [x, y, z, vx, vy, vz, flag(vel), radial || 0.0, flag(radial)] ++
        c ++ [flag(cov), confidence, dominant]

    for f <- fields, into: <<>>, do: <<f::float-little-64>>
  end

  defp flag(nil), do: 0.0
  defp flag(_), do: 1.0

  describe "frames packing" do
    test "header + row-major f64 layout round-trips through manual parse" do
      channels = [[1.0, 2.0, 3.0], [4.0, 5.0, 6.0]]
      bin = Codec.pack_frames(channels)

      <<n_emp::unsigned-little-32, n_samples::unsigned-little-32, rest::binary>> = bin
      assert n_emp == 2
      assert n_samples == 3

      floats = for <<f::float-little-64 <- rest>>, do: f
      assert floats == [1.0, 2.0, 3.0, 4.0, 5.0, 6.0]
    end

    test "integer samples are normalized to floats" do
      bin = Codec.pack_frames([[1, 2], [3, 4]])
      <<_::unsigned-little-32, _::unsigned-little-32, rest::binary>> = bin
      assert for(<<f::float-little-64 <- rest>>, do: f) == [1.0, 2.0, 3.0, 4.0]
    end

    test "empty channel list packs a zero header" do
      assert <<0::unsigned-little-32, 0::unsigned-little-32>> = Codec.pack_frames([])
    end
  end

  describe "geometry packing" do
    test "sample rate, count, and xyz triples round-trip" do
      bin = Codec.pack_geometry(4_000, [{1.0, 2.0, 3.0}, {4.0, 5.0, 6.0}])

      <<fs::float-little-64, n::unsigned-little-32, rest::binary>> = bin
      assert fs == 4_000.0
      assert n == 2
      assert for(<<f::float-little-64 <- rest>>, do: f) == [1.0, 2.0, 3.0, 4.0, 5.0, 6.0]
    end
  end

  describe "opts packing" do
    test "defaults are applied and layout is fixed width" do
      bin = Codec.pack_opts([])

      <<extent::float-little-64, coarse::float-little-64, fine::float-little-64,
        ratio::float-little-64, max_sources::unsigned-little-32>> = bin

      assert {extent, coarse, fine, ratio, max_sources} == {300.0, 20.0, 2.0, 0.5, 4}
    end

    test "overrides win" do
      bin = Codec.pack_opts(max_sources: 2, fine_res_m: 1.0)

      <<_::float-little-64, _::float-little-64, fine::float-little-64, _::float-little-64,
        max_sources::unsigned-little-32>> = bin

      assert fine == 1.0
      assert max_sources == 2
    end
  end

  describe "results unpacking" do
    test "decodes sources, mapping flags to nil" do
      # Build a results buffer the way the Rust side does: two sources, the
      # first with covariance but no velocity, the second with velocity but no
      # covariance.
      # ecef | vel(3) has_vel=0 | radial has_radial=0 | cov(9) has_cov=1 | conf dom
      rec1 =
        record(
          {10.0, 20.0, 30.0},
          nil,
          nil,
          [1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0],
          0.5,
          600.0
        )

      # velocity present + radial present, covariance absent
      rec2 =
        record({1.0, 2.0, 3.0}, {4.0, 5.0, 6.0}, 7.0, nil, 0.9, 440.0)

      buf = <<2::unsigned-little-32, rec1::binary, rec2::binary>>

      assert [s1, s2] = Codec.unpack_results(buf)

      assert %Source{
               ecef: {10.0, 20.0, 30.0},
               velocity_mps: nil,
               radial_velocity: nil,
               position_covariance: [1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0],
               confidence: 0.5,
               dominant_hz: 600.0
             } = s1

      assert %Source{
               ecef: {1.0, 2.0, 3.0},
               velocity_mps: {4.0, 5.0, 6.0},
               radial_velocity: 7.0,
               position_covariance: nil,
               confidence: 0.9,
               dominant_hz: 440.0
             } = s2
    end

    test "zero sources yields an empty list" do
      assert [] = Codec.unpack_results(<<0::unsigned-little-32>>)
    end
  end
end
