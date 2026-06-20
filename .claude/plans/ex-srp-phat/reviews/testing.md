# Test Review: ex_srp_phat

## Summary

The test suite is well-structured overall and correctly reproduces the Augur oracle fixture constants (FS=4000, C=343, BASE=1200, SIGMA=50, LEN=2600 match between Elixir and Rust). The 50 m parity assertion is genuine — not trivially passing — because the synthesized signal encodes real geometric TDOAs that the NIF must reconstruct. However, several issues undermine confidence in the parity bar and leave meaningful gaps in coverage.

## Iron Law Violations

None. Both ExUnit modules use `async: true`. No database, no Mox.

## Issues Found

### Critical

- [ ] **BLOCKER — Two-source test uses a looser tolerance (60 m) with no documented justification** (`ex_srp_phat_test.exs` line 99). The parity spec states the Augur oracle bar is 50 m. The single-source test correctly asserts `< 50.0`, but the two-source stretch quietly inflates this to 60 m. If the solver degrades on multi-source inputs the looser bar will mask it. Either document why 60 m is acceptable for this case (grid resolution? source spacing?) or assert ≤ 50 m for both.

- [ ] **BLOCKER — "frame id not present in geometry" validation test builds frames using `geometry()` emplacements before substituting a bad ID** (`ex_srp_phat_test.exs` line 123). The test calls `frames([{source, 600.0}])` which internally invokes the source-signal synthesizer (2600-sample Morlet, all four emplacements). This is a pure input-validation path: the NIF should be rejected before it runs. The overhead is minor but the test makes the expensive fixture call unnecessarily; more importantly, any exception inside `frames/1` would mask the real assertion. Use a cheap hand-crafted frame instead.

- [ ] **BLOCKER — `codec_test.exs` `unpack_results` fixture has a field-count mismatch in `rec1`** (lines 68–82). `rec1` encodes: xyz(3) + vel(3) + radial(1) + has_vel(1) = 8 fields, then cov(9) + has_cov(1) + confidence(1) + dominant_hz(1) = 12 fields → 20 total. But the binary literal is split across two `<<>>` blocks and the first block contains only 8 f64s (xyz + zeros for vel/radial/has_vel). The second block immediately appends 9 cov values then 3 trailing fields. If `Codec.unpack_results/1` reads `has_vel` from the 8th f64 (value `0.0`) to decide whether to set `velocity_mps: nil`, the test happens to pass for the right reason — but the fixture does not explicitly test a record where `has_vel = 1` and the velocity triplet is non-zero. The `rec2` covers that case. The risk: if the byte layout shifts by even one field the test will still decode without error (floats are forgiving) but will silently map values to wrong fields. Add an assertion that `s1.velocity_mps == nil` and `s2.velocity_mps == {4.0, 5.0, 6.0}` (these are present but via pattern match; the concern is that the position `{10.0, 20.0, 30.0}` in `s1.ecef` could be satisfied trivially if unpack reads past the intended offset).

### Warnings

- [ ] **WARNING — Tautological assertion on `band-{round(dominant_hz)}`** (`ex_srp_phat_test.exs` line 83). The line `assert "band-#{round(dominant_hz)}" == "band-600"` adds zero information on top of line 80 (`assert round(dominant_hz) == 600`). It tests only string interpolation, not the Augur adapter binding. Remove it or replace it with an actual call to the adapter function it is supposedly exercising.

- [ ] **WARNING — `grid_cost_stays_bounded` Rust test** (`tests.rs` line 137–145) is a pure arithmetic check on constants — it does not call `solve/3` or any real code path. If `coarse_res_m` is changed in `default_opts()` this test still passes because it computes its own `per_axis` from `extent=300/res=20` literals rather than from `Opts`. This means it will not catch a regression where the actual grid blows up. Either read `Opts` defaults directly or replace with a solver timing assertion or `sources.len() >= 1` check.

- [ ] **WARNING — No test for `sample_rate_hz = NaN` or negative values** (`ex_srp_phat_test.exs` validation block). Only `0` is tested as an invalid sample rate. The Rust `solve` guards `!fs.is_finite() || fs <= 0.0`, so `NaN` and `-1.0` are valid error cases that the Elixir validation layer (or NIF) must also handle. A negative fs produces `{:error, {:invalid_sample_rate, -1}}` or falls through to the NIF — either way, that code path has no coverage.

- [ ] **WARNING — No test for mismatched frame/geometry counts** (e.g., 3 frames vs 4 geometry emplacements). The Rust side returns `SolveError::ShapeMismatch` but the Elixir validation tests do not exercise this path. The `emplacement_mismatch` test checks ID lookup failure, not count disagreement.

- [ ] **WARNING — `codec_test.exs` opts test uses `assert {}` destructuring but the match is complete** (lines 45–48). If defaults change (e.g., `extent` goes from 300.0 to 500.0) the test fails — good. But there is no test that unknown keys in the keyword list are silently ignored (vs raising). Passing `[unknown_key: 99]` should be tested.

- [ ] **WARNING — Rust `truncated_buffer_errors` test** (`codec.rs` line 234) only tests a 2-byte buffer for `decode_frames`. No truncation tests for `decode_geometry` or `decode_opts`. A truncated geometry buffer sent from Elixir would panic or return `Err(Truncated)` — the latter is correct but untested.

### Suggestions

- [ ] **SUGGESTION — Single-source test does not assert `sources` length = 1**. Line 74 asserts `sources != []` then takes the max-confidence element. If the solver returns spurious ghost sources (e.g., multipath artifacts from the synthesized geometry) this would be silently accepted. Asserting `length(sources) == 1` or `length(sources) <= 2` would catch solver regressions.

- [ ] **SUGGESTION — No noise-tolerance test**. The Augur parity bar is set against noiseless synthetic signals. A test adding mild white noise (SNR ~10 dB) would guard against future algorithmic regressions that only manifest with realistic input and would better represent the oracle's real operating condition.

- [ ] **SUGGESTION — `Geo.ecef_to_latlon` round-trip tolerance (1.0e-6 deg lat/lon) is ~11 cm on the surface of the Earth**, which is tighter than the Bowring inversion's documented sub-millimeter accuracy. This is fine, but the altitude tolerance of 1 mm (`1.0e-3 m`) should be noted alongside the Bowring method caveat (it is on the localization critical path for reporting only, not for the 50 m bar).

- [ ] **SUGGESTION — Rust `results_encode_shape_and_flags` test** (`codec.rs` line 207) only checks byte length and `n_src`. It does not decode the buffer and verify field values, meaning the flag byte for `has_vel`/`has_cov` could be wrong and the test would pass. The Elixir `codec_test.exs` does decode, so coverage exists end-to-end, but the unit test at the Rust level is incomplete.

- [ ] **SUGGESTION — Property test opportunity**: `Codec.pack_frames` / `unpack_results` are pure encode/decode functions with well-defined invertibility. StreamData roundtrip properties would catch endianness or field-count regressions far more reliably than the current fixed-fixture approach.
