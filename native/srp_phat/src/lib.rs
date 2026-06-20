//! `srp_phat` — Rustler NIF for GCC-PHAT → SRP-PHAT acoustic source localization.
//!
//! The Elixir side packs PCM + geometry into little-endian `f64` binaries (see
//! `ExSrpPhat.Codec`); this crate decodes them, runs the solve, and encodes a
//! packed results binary back. Heavy work runs on a dirty CPU scheduler.

use rustler::{Atom, Binary, Env, OwnedBinary};

mod codec;
mod gcc_phat;
mod solve;
mod srp;
mod wgs84;

#[cfg(test)]
mod tests;

mod atoms {
    rustler::atoms! {
        bad_input,
        solve_failed,
        alloc_failed,
    }
}

fn to_binary<'a>(env: Env<'a>, bytes: &[u8]) -> Option<Binary<'a>> {
    let mut ob = OwnedBinary::new(bytes.len())?;
    ob.as_mut_slice().copy_from_slice(bytes);
    Some(ob.release(env))
}

/// Single SRP-PHAT solve. Args are packed binaries (see `codec`); returns
/// `{:ok, results_bin}` or `{:error, atom}` (rustler encodes `Result<T, E>` as
/// that tuple).
#[rustler::nif(schedule = "DirtyCpu")]
fn localize_nif<'a>(
    env: Env<'a>,
    frames: Binary,
    geometry: Binary,
    opts: Binary,
) -> Result<Binary<'a>, Atom> {
    let frames = codec::decode_frames(frames.as_slice()).map_err(|_| atoms::bad_input())?;
    let geometry = codec::decode_geometry(geometry.as_slice()).map_err(|_| atoms::bad_input())?;
    let opts = codec::decode_opts(opts.as_slice()).map_err(|_| atoms::bad_input())?;

    let sources = solve::solve(&frames, &geometry, &opts).map_err(|_| atoms::solve_failed())?;
    let results = codec::encode_results(&sources);
    to_binary(env, &results).ok_or_else(atoms::alloc_failed)
}

/// WGS-84 geodetic → ECEF. Exposed so callers can build ECEF geometry without a
/// separate geodesy dependency (and so the parity fixture matches exactly).
/// Cheap and pure — not dirty-scheduled.
#[rustler::nif]
fn latlon_to_ecef_nif(lat_deg: f64, lon_deg: f64, alt_m: f64) -> (f64, f64, f64) {
    let p = wgs84::latlon_alt_to_ecef(lat_deg, lon_deg, alt_m);
    (p[0], p[1], p[2])
}

/// WGS-84 ECEF → geodetic (degrees, meters HAE), via Bowring's method.
#[rustler::nif]
fn ecef_to_latlon_nif(x: f64, y: f64, z: f64) -> (f64, f64, f64) {
    wgs84::ecef_to_latlon_alt(x, y, z)
}

rustler::init!("Elixir.ExSrpPhat.Native");
