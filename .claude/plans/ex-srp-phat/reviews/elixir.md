# Code Review: ex_srp_phat Elixir Surface

## Summary
- **Status**: ⚠️ Changes Requested
- **Issues Found**: 5 (1 BLOCKER, 2 WARNING, 2 SUGGESTION)

---

## BLOCKER

### 1. `has_vel` flag decodes wrong field — velocity silently suppressed (`codec.ex:150`)

The Rust `encode_results` uses **`radial_velocity.is_some()`** to set `has_vel`, not `velocity.is_some()`:

```rust
// Rust codec.rs lines 148-156
push(s.radial_velocity.unwrap_or(0.0), &mut out);
push(
    if s.radial_velocity.is_some() { 1.0 } else { 0.0 },
    &mut out,
);
```

The Elixir unpack uses `has_vel == 1.0` to gate **both** `velocity_mps` and `radial_velocity`:

```elixir
# codec.ex:90-91
velocity_mps: if(has_vel == 1.0, do: {vx, vy, vz}, else: nil),
radial_velocity: if(has_vel == 1.0, do: radial, else: nil),
```

However, `has_cov` is set independently from `covariance.is_some()`. The result: a source can have `velocity` without `radial_velocity` (or vice-versa) on the Rust side, but the Elixir side treats them as co-present, using the wrong sentinel. In the Rust test fixture (`results_encode_shape_and_flags`), `SourceOut` with `velocity: Some(...)` and `radial_velocity: Some(7.0)` will set `has_vel = 1.0`; a source with `velocity: Some(...)` but `radial_velocity: None` would set `has_vel = 0.0`, causing Elixir to return `velocity_mps: nil` even though velocity data is in the buffer.

**Fix:** Add a separate `has_vel` flag that tracks `velocity.is_some()` in the Rust encoder, OR unpack two flags. Since v0 intentionally returns nil for both, this is a correctness landmine for the first future version that populates velocity — flag now before the layout is frozen.

---

## WARNING

### 2. `validate_sample_rate` accepts floats, but `pack_geometry` sends float to Rust as f64 — mismatched spec (`ex_srp_phat.ex:73`, `codec.ex:39`)

`validate_sample_rate/1` accepts `is_float(fs)` and returns `{:ok, fs}`. Then `pack_geometry/2` encodes it as `sample_rate_hz * 1.0::float-little-64`. The Rust decoder stores it as an `f64`. Rust solver code almost certainly does integer math on sample_rate (e.g., `(sample_rate_hz as usize)` for TDOA delay bins). Passing `4000.5` would be silently accepted and could produce subtly wrong delay grids. The `@type geometry` spec says `pos_integer()` for `sample_rate_hz`, and the public doc says "sample rate", implying integer Hz.

**Fix:** Reject floats in `validate_sample_rate/1`. Remove the `is_float` clause or coerce to integer with a guard (e.g., `trunc(fs) == fs`). The `pack_geometry` spec `@spec pack_geometry(number(), ...)` is too loose — tighten to `pos_integer()`.

### 3. `validate_lengths/1` pipes into `Enum.uniq/1` without naming the pipe subject (`ex_srp_phat.ex:116`)

```elixir
case Enum.map(channels, &length/1) |> Enum.uniq() do
```

The pipe starts mid-expression after `case`. This is a style violation (`case expr |>` is hard to parse; the pipe's data subject is the `case` scrutinee, not a named value). More critically, an empty `channels` list (guarded upstream by `length(emplacements) < 2` but not by frame count) would produce `[]`, matching `[_single]` — no, actually `[]` matches neither `[_single]` nor `_`, so it falls through to `_ -> {:error, :unequal_frame_lengths}`. This is correct behavior but the empty case is not documented.

**Fix (style):**
```elixir
defp validate_lengths(channels) do
  lengths = Enum.map(channels, &length/1) |> Enum.uniq()
  case lengths do
    [_single] -> :ok
    _ -> {:error, :unequal_frame_lengths}
  end
end
```

---

## SUGGESTION

### 4. `x * 1.0` float coercion is brittle for `Decimal` inputs (`codec.ex` throughout, `geo.ex`)

`s * 1.0` coerces integers to float but will crash with a `%Decimal{}` value (raises `ArithmeticError`). The public `@type` says `number()`, which in Elixir means integer or float — not Decimal. If callers might pass Decimal (common in financial/scientific BEAM code), this will produce confusing errors.

**Fix:** Either document explicitly that `Decimal` is not supported (since this is DSP, not money), or add a `to_float/1` guard. The current behaviour is acceptable if the type restriction is made explicit in `@spec` and `@doc`.

### 5. `mix.exs`: `Mix.env()` leaks into `native.ex` at compile time (`native.ex:21`)

```elixir
force_build:
  System.get_env("EX_SRP_PHAT_BUILD") in ["1", "true"] or
    Mix.env() in [:dev, :test],
```

`CLAUDE.md` explicitly forbids leaking `Mix.env/0` into app code. However, `RustlerPrecompiled`'s `use` macro evaluates its options at **compile time** during `mix compile`, so this call happens at compile time inside a `use` macro — not at runtime. This is a known and accepted pattern for Rustler/RustlerPrecompiled. Flag as SUGGESTION rather than BLOCKER, but it violates the project CLAUDE.md rule literally and should have a comment explaining the compile-time exemption.

---

## Codec Layout Verification

Comparing Elixir `unpack_sources/3` against Rust `encode_results`:

| Field | Rust order | Elixir order | Match? |
|-------|-----------|-------------|--------|
| xyz (3×f64) | 1-3 | x,y,z | ✅ |
| vel (3×f64) | 4-6 | vx,vy,vz | ✅ |
| radial (1×f64) | 7 | radial | ✅ |
| has_vel (1×f64) | 8 | has_vel | ✅ |
| cov (9×f64) | 9-17 | c0..c8 | ✅ |
| has_cov (1×f64) | 18 | has_cov | ✅ |
| confidence (1×f64) | 19 | confidence | ✅ |
| dominant_hz (1×f64) | 20 | dominant_hz | ✅ |

Total: 20×f64 = 160 bytes per source. Layout is byte-for-byte correct.

The **semantic bug** (BLOCKER #1) is not a layout mismatch but a flag-semantics mismatch: in the Rust encoder, `has_vel` is set from `radial_velocity.is_some()`, not `velocity.is_some()`. Elixir uses that same flag to gate both `velocity_mps` and `radial_velocity`.
