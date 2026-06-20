defmodule ExSrpPhat.Source do
  @moduledoc """
  One localized acoustic source returned by `ExSrpPhat.localize/3`.

  All positions/velocities are in **WGS-84 ECEF meters**. This library is
  library-neutral: it deliberately does **not** assign a cross-solve
  `source_ref`. Stable source identity is the caller's concern — `dominant_hz`
  is exposed precisely so an adapter can reproduce a convention such as
  `"band-\#{round(dominant_hz)}"`.

  ## Fields

    * `ecef` — `{x, y, z}` source position, meters ECEF.
    * `velocity_mps` — `{vx, vy, vz}` ECEF velocity, or `nil` when not estimated.
    * `radial_velocity` — line-of-sight Doppler velocity (m/s), or `nil`.
    * `position_covariance` — 3×3 ECEF covariance as a row-major list of 9 floats,
      derived from SRP peak sharpness, or `nil`.
    * `confidence` — normalized SRP peak sharpness in `0.0..1.0`.
    * `dominant_hz` — dominant frequency (Hz) of the steered peak signal.
  """

  @enforce_keys [:ecef, :confidence, :dominant_hz]
  defstruct ecef: nil,
            velocity_mps: nil,
            radial_velocity: nil,
            position_covariance: nil,
            confidence: 0.0,
            dominant_hz: nil

  @type vec3 :: {float(), float(), float()}
  @type t :: %__MODULE__{
          ecef: vec3(),
          velocity_mps: vec3() | nil,
          radial_velocity: float() | nil,
          position_covariance: [float()] | nil,
          confidence: float(),
          dominant_hz: float()
        }
end
