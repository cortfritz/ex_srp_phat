# Changelog

All notable changes to this project are documented here.

## [0.1.0] - unreleased

Initial release.

- `ExSrpPhat.localize/3` — GCC-PHAT → SRP-PHAT acoustic source localization over a
  known WGS-84 ECEF microphone-array geometry.
- `ExSrpPhat.Source` result struct: `ecef`, `velocity_mps`, `radial_velocity`,
  `position_covariance`, `confidence`, `dominant_hz`.
- Rust NIF (rustler 0.38) with binary marshalling of PCM + geometry, rayon-parallel
  coarse-to-fine grid search, `realfft`/`rustfft` GCC-PHAT.
- Doppler / velocity estimation deferred to a later release (returns `nil`).
