# Security Audit: ex_srp_phat NIF FFI boundary

## Executive Summary

Threat model: malformed/oversized/adversarial packed binaries crossing the
rustler NIF boundary. A Rust panic on a DirtyCpu scheduler aborts the BEAM
process (whole VM), so any reachable panic/abort is a BLOCKER.

The decoders mostly handle truncation gracefully (`Cursor` returns
`CodecError::Truncated`, mapped to `{:error, :bad_input}`), and `ExSrpPhat.localize/3`
validates and re-packs input, so the *happy path is safe*. But the NIF
(`ExSrpPhat.Native.localize_nif/3`) is directly callable with arbitrary
binaries — the docstring "do not call directly" is not an enforcement
boundary. Anything that constructs `frames_bin`/`geometry_bin`/`opts_bin`
(tests, a future caller, or a process that receives them over the wire)
hits the raw decoders. The dominant risk is **huge-allocation DoS via
attacker-controlled header counts driving `Vec::with_capacity`**, which
aborts (SIGABRT / OOM kill) the entire VM, not just the calling process.

Sobelow/deps.audit not run (no Bash); recommend manually.

## Critical Vulnerabilities

### 1. Huge-allocation DoS in `decode_frames` (`Vec::with_capacity` from header)
- **Severity**: High (BLOCKER for the FFI boundary)
- **Location**: `native/srp_phat/src/codec.rs:94-95`
- **Issue**: `count = n_emp.checked_mul(n_samples)`. Both are `u32` cast to
  `usize`; on 64-bit, max product ≈ 4.29e9 × 4.29e9 ≈ 1.8e19, which fits in
  `usize` — so `checked_mul` almost never trips. `Vec::with_capacity(count)`
  then requests ~1.4e20 bytes and **aborts the process** (allocation failure
  is not a recoverable `Result` — it panics/aborts even on DirtyCpu). The
  per-element `c.f64()?` truncation check happens only *after* the allocation,
  so it never gets a chance to reject the input. A 8-byte header
  (`n_emp=65536, n_samples=65536`) → 4.29e9 elems → 34 GB request → OOM-kill.
- **Fix**: Bound by remaining buffer length before allocating. The body needs
  exactly `count * 8` bytes, so reject early and pre-size safely:
  ```rust
  let count = n_emp.checked_mul(n_samples).ok_or(CodecError::BadShape)?;
  let need = count.checked_mul(8).ok_or(CodecError::BadShape)?;
  if buf.len() - c.pos < need { return Err(CodecError::Truncated); }
  let mut data = Vec::with_capacity(count); // now bounded by real input size
  ```
  (Optionally also cap `n_emp`/`n_samples` to sane maxima.)

### 2. Huge-allocation DoS in `decode_geometry`
- **Severity**: High (BLOCKER)
- **Location**: `native/srp_phat/src/codec.rs:110`
- **Issue**: `Vec::with_capacity(n_emp)` with `n_emp` straight from a `u32`
  header (up to 4.29e9 × 24 bytes ≈ 103 GB). Same abort-before-validate
  pattern as #1.
- **Fix**: Validate `buf.len() - c.pos >= n_emp.checked_mul(24)?` before
  `with_capacity`, mirroring #1.

### 3. `encode_results` capacity from result count is safe, but note `to_binary` abort
- **Severity**: Low/informational
- **Location**: `native/srp_phat/src/lib.rs:26`
- **Issue**: `OwnedBinary::new(len).expect("alloc results binary")` aborts the
  VM if allocation fails. `len` is solver-derived (bounded by `max_sources`),
  not directly attacker-controlled, so low risk — but `max_sources` IS from
  the opts header (#4) and feeds combination/candidate counts.
- **Fix**: Map `None` to `solve_failed` instead of `.expect`.

## Warnings

### 4. Unbounded `max_sources` from opts header
- **Severity**: Medium (WARNING)
- **Location**: `codec.rs:126`, used `solve.rs:49,66`
- **Issue**: `max_sources` (u32, up to 4.29e9) drives `k` for `top_peaks` and
  the candidate/dedup loops. `MAX_COMBOS=4096` caps combination enumeration,
  and `top_peaks` is bounded by available local maxima, so this is not a
  direct allocation bomb — but a huge `k` enlarges per-pair work and the
  `candidates`/`kept` vectors grow with combinations, not `max_sources`, so
  impact is limited. Still, an absurd `max_sources` should be rejected.
- **Fix**: In `decode_opts` or `solve`, clamp `max_sources` to a sane ceiling
  (e.g. `min(max_sources, 64)`). Elixir's `pack_opts` does not validate the
  caller-supplied `:max_sources` value either — it is passed through verbatim
  (`codec.ex:65`).

### 5. `partial_cmp().unwrap()` on possibly-NaN scores
- **Severity**: Medium (WARNING — panic vector)
- **Location**: `solve.rs:103`, `solve.rs:207-208`, `srp.rs:172`,
  `gcc_phat.rs:172`
- **Issue**: `sort_by(|a,b| b.sharp.partial_cmp(&a.sharp).unwrap())` and the
  `min_by(... .partial_cmp(...).unwrap())` in `multilaterate` panic if any
  compared value is `NaN`. Scores derive from FFT/correlation of PCM samples.
  PCM values cross the boundary as raw `f64` with **no finiteness check** —
  an attacker can pack `NaN`/`inf` samples (codec.rs decodes any bit pattern).
  NaN/inf propagates through GCC-PHAT (`norm`, divisions) and SRP scoring,
  yielding NaN scores → `partial_cmp` returns `None` → `.unwrap()` panics →
  **VM abort**. `Elixir.localize/3` only checks `is_number`, which is true for
  no Elixir float NaN literal but does not stop a direct NIF call, and
  `is_number` does not reject `inf` either from a packed binary.
- **Fix**: Sanitize PCM on decode (replace non-finite with 0.0, or reject),
  and replace `partial_cmp().unwrap()` with
  `.partial_cmp(&x).unwrap_or(std::cmp::Ordering::Equal)` /
  `total_cmp`. At minimum use `total_cmp` for the sorts.

### 6. Non-finite geometry / sample-rate reaching math
- **Severity**: Low/Medium (WARNING)
- **Location**: `solve.rs:33-35` (fs checked — good), geometry ECEF unchecked
- **Issue**: `fs` is validated finite/positive (good). ECEF coords are NOT
  finiteness-checked; `inf`/`NaN` propagate into distances and into #5's
  comparisons. `Elixir.valid_emplacement?` checks `is_number` but a direct
  NIF call bypasses it.
- **Fix**: Validate ECEF finiteness in `solve` (return `BadInput`/`bad_input`).

## Non-issues / verified clean

- `Cursor::u32/f64` (codec.rs:75-87): `try_into().unwrap()` is safe — slice is
  always exactly 4/8 bytes from a successful `.get(range)`; not
  attacker-triggerable.
- Integer-delay indexing in `dominant_hz` (solve.rs:200-207) is bounds-checked
  (`src >= 0 && < n_samples`). Good.
- `gcc_phat::at` (line 96-99) uses Euclidean-mod into range — no OOB.
- `unit_from` (srp.rs:34) divides by range `r`. `r==0` only if candidate
  coincides exactly with an emplacement; result is `inf`/`NaN` but only feeds
  covariance (`invert_spd_3x3` rejects non-SPD → `None`), not a panic.
  Divide-by-zero in f64 is not a panic in Rust. SUGGESTION: guard `r` anyway.
- `positive_roots` / `invert_3x3` / `invert_spd_3x3` guard singularities with
  determinant thresholds and return `None`. No division panic.
- No `String.to_atom`, `raw/1`, SQL, `binary_to_term`, secrets — N/A for this
  native library.

## Security Posture

### Input Validation (FFI boundary)
- Status: WARNING
- Truncation handled; **oversized headers and non-finite floats are not**.
  The Elixir gate (`localize/3`) is sound for its own callers but does not
  protect the directly-callable NIF. Treat the Rust decoders as the real
  boundary and validate there.

## Recommendations (priority order)

1. Bound `with_capacity` by remaining buffer bytes in `decode_frames` /
   `decode_geometry` (#1, #2) — fixes the VM-abort DoS.
2. Sanitize/reject non-finite PCM and ECEF on decode (#5, #6).
3. Replace every `partial_cmp().unwrap()` with `total_cmp` or
   `unwrap_or(Ordering::Equal)` (#5).
4. Clamp `max_sources` (#4); replace `OwnedBinary::new(..).expect` (#3).
5. Add an Elixir guard rejecting non-finite floats in samples/ecef before
   packing, as defense-in-depth.

## Tools to Recommend (run manually — no Bash here)
- `mix sobelow --exit medium`
- `mix deps.audit` / `mix hex.audit`
- `cargo +nightly fuzz` or a quick proptest harness on `decode_frames`/
  `decode_geometry` with random byte buffers (would catch #1, #2, #5 fast).
