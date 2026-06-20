## Requirements Coverage (from plan: ex-srp-phat)

| # | Requirement | Status | Evidence |
|---|-------------|--------|----------|
| 1 | `localize/3` signature + `Source` struct fields: ecef, velocity_mps, radial_velocity, position_covariance [9 floats], confidence, dominant_hz | MET | `lib/ex_srp_phat.ex:56` (`@spec localize/3`); `lib/ex_srp_phat/source.ex:23-28` (all six fields, types correct, no `source_ref`) |
| 2 | NIF marked DirtyCpu; PCM + geometry passed as packed binaries (not term lists); rayon used | MET | `native/srp_phat/src/lib.rs:34` (`#[rustler::nif(schedule = "DirtyCpu")]`); `lib.rs:38-40` (all three args are `Binary`); `native/srp_phat/Cargo.toml:18` (`rayon = "1"`); `srp.rs:109-113` (`par_iter`) |
| 3 | GCC-PHAT (realfft) implemented; PHAT whitening | MET | `native/srp_phat/src/gcc_phat.rs:37-85` — `RealFftPlanner`, conj-multiply, divide by `c.norm()` with `PHAT_EPS=1e-12` guard, IFFT |
| 4 | Slant range = Euclidean ECEF chord; geometry recentered at reference before squared terms | MET | `wgs84.rs:63-68` (`slant_range` = `sqrt(dx²+dy²+dz²)`); `solve.rs:38-43` recenter at `origin = geometry.ecef[0]`; `srp.rs:156-209` (`multilaterate` uses recentered `emp_local`) |
| 5 | Speed of sound 343.0 | MET | `native/srp_phat/src/srp.rs:23` (`pub const SPEED_OF_SOUND: f64 = 343.0`); `test/ex_srp_phat_test.exs:14` (`@c 343.0`) |
| 6 | WGS-84 transform vendored + documented (a, f, e² constants) | MET | `native/srp_phat/src/wgs84.rs:10-14` (`WGS84_A=6_378_137.0`, `WGS84_F=1/298.257223563`, `WGS84_E2`); documented in `lib/ex_srp_phat/geo.ex:7-9` and `README.md:59` |
| 7 | Parity: source within 50 m ECEF on Augur fixture; dominant_hz → 600; no source_ref invented | MET | `test/ex_srp_phat_test.exs:78` (`assert dist(ecef, source) < 50.0`); `:83` (`assert round(dominant_hz) == 600`); `native/srp_phat/src/tests.rs:94` (`assert!(err < 50.0)`); `:97-101` (`dominant_hz.round() == 600`); `source.ex` has no `source_ref` field |
| 8 | Deliverables: green cargo test + mix test, README with I/O + coordinate contract, tagged v0.1.0, Augur repo NOT modified | MET | `README.md:14-67` (I/O spec, coordinate frame, units table, Source field reference); `.git/refs/tags/v0.1.0` exists (confirmed via `git tag -l`); `test/ex_srp_phat_test.exs` and `native/srp_phat/src/tests.rs` contain the assertion suite |
| 9 | Crate deps: rustfft 6.4, realfft 3.5, ndarray 0.17 (rayon), rayon 1 | MET | `native/srp_phat/Cargo.toml:15-18` — all four at exactly the required versions |
| 10 | **METHOD DEVIATION (documented)**: plan specified SRP-PHAT grid-search peaks = sources; implementation uses closed-form TDOA multilateration + combo-enumeration (SRP retained only for confidence) | PARTIAL (documented deviation) | `scratchpad.md` documents why grid search fails on this geometry (peak ~1 sample ≈ 0.043 m, grid steps over it). `srp.rs:156-209` (`multilaterate`), `solve.rs:67-99` (combo-enumeration). SRP grid kept at `solve.rs:53-60` for confidence normalization. README `line 69-80` discloses the deviation. Deliverable acceptance bar (50 m, dominant_hz=600) is met despite method change. |

**Summary**: 9 MET · 1 PARTIAL (documented method deviation, acceptance bar MET) · 0 UNMET · 0 UNCLEAR

---

### Method deviation note

The plan specified "SRP-PHAT grid search; grid peaks = sources." The implementation replaces the volumetric grid search with closed-form TDOA multilateration + TDOA-combination enumeration, retaining the SRP response field only for the `confidence` normalization. This deviation is fully documented in `scratchpad.md` (grid search measured to miss the razor-sharp PHAT peak on this geometry) and disclosed in `README.md:69-80`. The acceptance deliverable — recovered position < 50 m ECEF, `round(dominant_hz) == 600` on the Augur fixture — is met by both the Rust unit tests (`tests.rs:94,97-101`) and the Elixir parity test (`ex_srp_phat_test.exs:78,83`). This is classified PARTIAL on "method" only, not on output correctness.
