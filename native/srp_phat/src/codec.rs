//! Binary marshalling between Elixir and Rust.
//!
//! Everything crosses the NIF boundary as little-endian packed buffers — never
//! term lists — because term-list marshalling of multi-thousand-sample frames
//! dominates runtime. Layouts (all integers `u32`, all reals `f64`, LE):
//!
//! * **frames**: `[n_emp][n_samples]` then `n_emp × n_samples` PCM, row-major
//! * **geometry**: `[sample_rate_hz: f64][n_emp: u32]` then `n_emp × 3` ECEF xyz
//! * **opts**: `[grid_extent_m][coarse_res_m][fine_res_m][min_peak_ratio]` (f64)
//!   then `[max_sources: u32]`
//! * **results**: `[n_src: u32]` then per source a fixed 21×f64 record:
//!   `xyz(3) | vel(3) | has_vel(1) | radial(1) | has_radial(1) | cov(9) |
//!   has_cov(1) | confidence(1) | dominant_hz(1)`

/// Number of f64 fields per encoded source record.
pub const SOURCE_FIELDS: usize = 21;

#[derive(Debug)]
pub enum CodecError {
    Truncated,
    BadShape,
    NonFinite,
}

/// Decoded PCM frame-set, row-major (`n_emp` rows × `n_samples` cols).
pub struct Frames {
    pub n_emp: usize,
    pub n_samples: usize,
    pub data: Vec<f64>,
}

impl Frames {
    /// Borrow the samples for emplacement `i`.
    pub fn channel(&self, i: usize) -> &[f64] {
        &self.data[i * self.n_samples..(i + 1) * self.n_samples]
    }
}

/// Array geometry: sample rate plus one ECEF position per emplacement, ordered
/// to match the rows of [`Frames`].
pub struct Geometry {
    pub sample_rate_hz: f64,
    pub ecef: Vec<[f64; 3]>,
}

/// Grid-search tuning, with the solver's defaults applied on the Elixir side.
pub struct Opts {
    pub grid_extent_m: f64,
    pub coarse_res_m: f64,
    pub fine_res_m: f64,
    pub min_peak_ratio: f64,
    pub max_sources: usize,
}

/// One localized source, pre-encoding.
#[derive(Clone, Debug)]
pub struct SourceOut {
    pub ecef: [f64; 3],
    pub velocity: Option<[f64; 3]>,
    pub radial_velocity: Option<f64>,
    pub covariance: Option<[f64; 9]>,
    pub confidence: f64,
    pub dominant_hz: f64,
}

struct Cursor<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    fn new(buf: &'a [u8]) -> Self {
        Self { buf, pos: 0 }
    }

    fn u32(&mut self) -> Result<u32, CodecError> {
        let end = self.pos + 4;
        let bytes = self.buf.get(self.pos..end).ok_or(CodecError::Truncated)?;
        self.pos = end;
        Ok(u32::from_le_bytes(bytes.try_into().unwrap()))
    }

    fn f64(&mut self) -> Result<f64, CodecError> {
        let end = self.pos + 8;
        let bytes = self.buf.get(self.pos..end).ok_or(CodecError::Truncated)?;
        self.pos = end;
        Ok(f64::from_le_bytes(bytes.try_into().unwrap()))
    }

    /// Like [`f64`](Self::f64) but rejects NaN/∞ — non-finite values from a
    /// crafted buffer otherwise reach the scoring/sort paths and could panic.
    fn finite_f64(&mut self) -> Result<f64, CodecError> {
        let v = self.f64()?;
        if v.is_finite() {
            Ok(v)
        } else {
            Err(CodecError::NonFinite)
        }
    }

    /// Bytes not yet consumed — used to reject an oversized header before it
    /// drives a huge `Vec::with_capacity` (allocation-DoS guard).
    fn remaining(&self) -> usize {
        self.buf.len().saturating_sub(self.pos)
    }
}

/// Upper bound on `max_sources` (also caps enumerated TDOA combinations).
pub const MAX_SOURCES_CAP: usize = 64;

pub fn decode_frames(buf: &[u8]) -> Result<Frames, CodecError> {
    let mut c = Cursor::new(buf);
    let n_emp = c.u32()? as usize;
    let n_samples = c.u32()? as usize;
    let count = n_emp.checked_mul(n_samples).ok_or(CodecError::BadShape)?;
    // Reject an oversized header before allocating: the buffer cannot hold
    // `count` f64s, so this is truncated/hostile — never request the memory.
    if count.checked_mul(8).ok_or(CodecError::BadShape)? > c.remaining() {
        return Err(CodecError::Truncated);
    }
    let mut data = Vec::with_capacity(count);
    for _ in 0..count {
        data.push(c.finite_f64()?);
    }
    Ok(Frames {
        n_emp,
        n_samples,
        data,
    })
}

pub fn decode_geometry(buf: &[u8]) -> Result<Geometry, CodecError> {
    let mut c = Cursor::new(buf);
    let sample_rate_hz = c.finite_f64()?;
    let n_emp = c.u32()? as usize;
    // Each emplacement is 3 f64s — bounds-check before allocating.
    if n_emp.checked_mul(24).ok_or(CodecError::BadShape)? > c.remaining() {
        return Err(CodecError::Truncated);
    }
    let mut ecef = Vec::with_capacity(n_emp);
    for _ in 0..n_emp {
        ecef.push([c.finite_f64()?, c.finite_f64()?, c.finite_f64()?]);
    }
    Ok(Geometry {
        sample_rate_hz,
        ecef,
    })
}

pub fn decode_opts(buf: &[u8]) -> Result<Opts, CodecError> {
    let mut c = Cursor::new(buf);
    let grid_extent_m = c.finite_f64()?;
    let coarse_res_m = c.finite_f64()?;
    let fine_res_m = c.finite_f64()?;
    let min_peak_ratio = c.finite_f64()?;
    let max_sources = (c.u32()? as usize).min(MAX_SOURCES_CAP);
    Ok(Opts {
        grid_extent_m,
        coarse_res_m,
        fine_res_m,
        min_peak_ratio,
        max_sources,
    })
}

pub fn encode_results(sources: &[SourceOut]) -> Vec<u8> {
    let mut out = Vec::with_capacity(4 + sources.len() * SOURCE_FIELDS * 8);
    out.extend_from_slice(&(sources.len() as u32).to_le_bytes());
    let push = |v: f64, out: &mut Vec<u8>| out.extend_from_slice(&v.to_le_bytes());
    let flag = |present: bool| if present { 1.0 } else { 0.0 };
    for s in sources {
        for v in s.ecef {
            push(v, &mut out);
        }
        // velocity (3) + its own presence flag — independent of radial velocity.
        for v in s.velocity.unwrap_or([0.0; 3]) {
            push(v, &mut out);
        }
        push(flag(s.velocity.is_some()), &mut out);
        // radial velocity (1) + its own presence flag.
        push(s.radial_velocity.unwrap_or(0.0), &mut out);
        push(flag(s.radial_velocity.is_some()), &mut out);
        // covariance (9) + presence flag.
        for v in s.covariance.unwrap_or([0.0; 9]) {
            push(v, &mut out);
        }
        push(flag(s.covariance.is_some()), &mut out);
        push(s.confidence, &mut out);
        push(s.dominant_hz, &mut out);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pack_frames(n_emp: u32, n_samples: u32, data: &[f64]) -> Vec<u8> {
        let mut b = Vec::new();
        b.extend_from_slice(&n_emp.to_le_bytes());
        b.extend_from_slice(&n_samples.to_le_bytes());
        for v in data {
            b.extend_from_slice(&v.to_le_bytes());
        }
        b
    }

    #[test]
    fn frames_roundtrip() {
        let data = [1.0, 2.0, 3.0, 4.0, 5.0, 6.0];
        let buf = pack_frames(2, 3, &data);
        let f = decode_frames(&buf).unwrap();
        assert_eq!(f.n_emp, 2);
        assert_eq!(f.n_samples, 3);
        assert_eq!(f.channel(0), &[1.0, 2.0, 3.0]);
        assert_eq!(f.channel(1), &[4.0, 5.0, 6.0]);
    }

    #[test]
    fn geometry_roundtrip() {
        let mut b = Vec::new();
        b.extend_from_slice(&4_000.0f64.to_le_bytes());
        b.extend_from_slice(&2u32.to_le_bytes());
        for v in [1.0f64, 2.0, 3.0, 4.0, 5.0, 6.0] {
            b.extend_from_slice(&v.to_le_bytes());
        }
        let g = decode_geometry(&b).unwrap();
        assert_eq!(g.sample_rate_hz, 4_000.0);
        assert_eq!(g.ecef, vec![[1.0, 2.0, 3.0], [4.0, 5.0, 6.0]]);
    }

    #[test]
    fn results_encode_shape_and_flags() {
        let srcs = vec![
            SourceOut {
                ecef: [10.0, 20.0, 30.0],
                velocity: None,
                radial_velocity: None,
                covariance: Some([1.0; 9]),
                confidence: 0.5,
                dominant_hz: 600.0,
            },
            SourceOut {
                ecef: [1.0, 2.0, 3.0],
                velocity: Some([4.0, 5.0, 6.0]),
                radial_velocity: Some(7.0),
                covariance: None,
                confidence: 0.9,
                dominant_hz: 440.0,
            },
        ];
        let buf = encode_results(&srcs);
        assert_eq!(buf.len(), 4 + 2 * SOURCE_FIELDS * 8);
        let n = u32::from_le_bytes(buf[0..4].try_into().unwrap());
        assert_eq!(n, 2);
    }

    #[test]
    fn truncated_buffer_errors() {
        assert!(matches!(decode_frames(&[0, 0]), Err(CodecError::Truncated)));
        assert!(matches!(
            decode_geometry(&[0, 0]),
            Err(CodecError::Truncated)
        ));
        assert!(matches!(decode_opts(&[0, 0]), Err(CodecError::Truncated)));
    }

    #[test]
    fn oversized_header_rejected_without_allocating() {
        // n_emp = n_samples = u32::MAX but no payload → must error, not OOM-abort.
        let mut frames = Vec::new();
        frames.extend_from_slice(&u32::MAX.to_le_bytes());
        frames.extend_from_slice(&u32::MAX.to_le_bytes());
        assert!(matches!(
            decode_frames(&frames),
            Err(CodecError::Truncated | CodecError::BadShape)
        ));

        let mut geom = Vec::new();
        geom.extend_from_slice(&4_000.0f64.to_le_bytes());
        geom.extend_from_slice(&u32::MAX.to_le_bytes());
        assert!(matches!(
            decode_geometry(&geom),
            Err(CodecError::Truncated | CodecError::BadShape)
        ));
    }

    #[test]
    fn non_finite_samples_rejected() {
        let buf = pack_frames(1, 1, &[f64::NAN]);
        assert!(matches!(decode_frames(&buf), Err(CodecError::NonFinite)));
    }

    #[test]
    fn max_sources_is_clamped() {
        let mut b = Vec::new();
        for v in [300.0f64, 20.0, 2.0, 0.5] {
            b.extend_from_slice(&v.to_le_bytes());
        }
        b.extend_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(decode_opts(&b).unwrap().max_sources, MAX_SOURCES_CAP);
    }
}
