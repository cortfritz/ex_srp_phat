defmodule ExSrpPhat.Geo do
  @moduledoc """
  WGS-84 geodetic ↔ ECEF conversion, delegated to the Rust NIF so the transform
  matches the solver's geometry exactly (and so callers need no separate geodesy
  dependency).

  Constants: semi-major axis `a = 6_378_137.0` m, flattening
  `f = 1 / 298.257223563`, first eccentricity² `e² = f(2 − f)`. The forward
  transform is exact closed form; the inverse uses Bowring's method.
  """

  alias ExSrpPhat.Native

  @doc "Geodetic latitude/longitude (degrees) + height (m, HAE) → ECEF `{x, y, z}` meters."
  @spec latlon_to_ecef(number(), number(), number()) :: {float(), float(), float()}
  def latlon_to_ecef(lat_deg, lon_deg, alt_m \\ 0.0) do
    Native.latlon_to_ecef_nif(lat_deg * 1.0, lon_deg * 1.0, alt_m * 1.0)
  end

  @doc "ECEF `{x, y, z}` meters → geodetic `{lat_deg, lon_deg, alt_m}`."
  @spec ecef_to_latlon(number(), number(), number()) :: {float(), float(), float()}
  def ecef_to_latlon(x, y, z) do
    Native.ecef_to_latlon_nif(x * 1.0, y * 1.0, z * 1.0)
  end
end
