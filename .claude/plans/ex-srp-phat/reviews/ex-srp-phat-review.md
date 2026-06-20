# Review — ex_srp_phat v0.1.0

**Verdict: REQUIRES CHANGES** (robustness blockers on the NIF boundary).
Functionally complete and the deliverable acceptance bar is met — but the NIF is
directly callable with arbitrary binaries and has crash/DoS vectors worth fixing
before this is relied on as "the production DSP."

Agents: elixir-reviewer, testing-reviewer, security-analyzer, verification-runner,
requirements-verifier (all completed).

## Requirements Coverage (vs plan: ex-srp-phat)

**9 MET · 1 PARTIAL · 0 UNMET.** The single PARTIAL is the *documented* method
deviation: plan said volumetric SRP-PHAT grid search; implementation localizes via
closed-form TDOA multilateration + combo-enumeration (SRP grid kept only for
confidence). Scratchpad documents why grid search fails this geometry. The acceptance
bar (<50 m ECEF + `round(dominant_hz)==600`) is met by both Rust and Elixir tests.
→ PARTIAL (no UNMET) normally means PASS WITH WARNINGS; code-quality blockers below
escalate the overall verdict.

## Verification

All green: `mix compile --warnings-as-errors`, `mix format --check`, `mix test` (16),
`cargo test` (22), `cargo clippy -- -D warnings`, `cargo fmt --check`.

## BLOCKERS

1. **Huge-allocation DoS in `decode_frames`** (`native/srp_phat/src/codec.rs:~94`).
   `checked_mul` rarely trips on 64-bit, so `Vec::with_capacity(n_emp*n_samples)` can
   request ~1e20 bytes and abort the BEAM *before* the per-element truncation check.
   Fix: require `remaining_bytes >= count*8` before allocating.
2. **Huge-allocation DoS in `decode_geometry`** (`codec.rs:~110`). `Vec::with_capacity(n_emp)`
   with `n_emp` straight from the u32 header (up to ~103 GB). Same bounds-first fix.
3. **NaN → panic → VM abort** (`solve.rs:103,207-208`, `srp.rs:172`, `gcc_phat.rs:172`).
   `partial_cmp().unwrap()` panics on NaN; packed `NaN`/`inf` PCM or ECEF reaches the
   scoring/sort paths unchecked. Fix: reject non-finite floats on decode and/or use
   `total_cmp`. (DirtyCpu panic aborts the whole VM.)
4. **`has_vel` flag-semantics mismatch** (`codec.rs` encode vs `codec.ex:~90`). Rust sets
   the single flag from `radial_velocity.is_some()`; Elixir uses it to gate *both*
   `velocity_mps` and `radial_velocity`. Latent only (v0 always emits both `nil`), but
   corrupts output once velocity is populated. Fix: emit independent `has_velocity` /
   `has_radial` flags (record grows to 21 f64; update `SOURCE_FIELDS`, both codecs, and
   `codec_test`).

> Note: the public `ExSrpPhat.localize/3` path builds well-formed binaries and validates
> input, so #1–#3 are only reachable by calling `Native.localize_nif/3` directly with
> hostile data. For a distributable production NIF that's still worth hardening; cheap fixes.

## WARNINGS

- `max_sources` is an unbounded u32 (`codec.rs:~126`) → drives combo enumeration; clamp it.
- `OwnedBinary::new(..).expect(...)` (`lib.rs:26`) aborts on alloc failure; map to an error.
- `validate_sample_rate` accepts floats (`ex_srp_phat.ex:~73`); Rust uses fs in delay-bin
  math — either document float support or reject non-integers.
- Two-source stretch test uses 60 m tolerance vs the 50 m parity bar (`ex_srp_phat_test.exs`);
  tighten or document why the stretch is looser.
- Missing tests: `decode_geometry`/`decode_opts` truncation (only `decode_frames` covered);
  frame-count vs geometry-count mismatch (`ShapeMismatch`); NaN/negative sample rate.
- `grid_cost_stays_bounded` (`tests.rs`) uses hardcoded literals, not the real combo cap /
  `Opts` defaults — won't catch an actual blowup. Rename/retarget to `MAX_COMBOS`.

## SUGGESTIONS

- `"band-#{round(dominant_hz)}" == "band-600"` (`ex_srp_phat_test.exs:83`) is a tautology
  over the prior assertion — drop or replace with an adapter-convention doc note.
- `x * 1.0` coercion crashes on `Decimal`; document the numeric-type contract.
- `Mix.env()` in `native.ex` is the accepted RustlerPrecompiled compile-time pattern but
  violates project CLAUDE.md at a glance — add an explanatory comment.
- Pipe inside `case` scrutinee (`ex_srp_phat.ex:116`) — extract to a named binding.

## Strengths

- Binary marshalling, recentering, slant-range, closed-form multilateration all correct
  and well-documented; `Cursor` decode is bounds-checked elementwise; matrix inverters
  return `None` on singularity; no unsafe, no secrets/SQL/atom-injection concerns.
- Parity with the Augur oracle genuinely demonstrated (not a trivial pass).
