//! Keyframes of a CSP animation folder's track: which cel is shown when.
//! The track's mixer is an external chunk holding a zlib "binc" stream: a
//! string table (type and key names, cel names) and byte-packed properties of
//! the form (key, type, payload, 8 zero bytes). Only the cel curve is read.
use std::io::Read;

const MAGIC: &[u8] = b"cmt 0100binc";
const MAX_INFLATED: u64 = 1024 * 1024;
const MAX_KEYS: usize = 4096;

/// Cel keyframes in the clip's own time units (`rate` per second).
#[derive(Debug, PartialEq)]
pub(crate) struct Keys {
    pub times: Vec<f64>,
    pub cels: Vec<String>,
    pub rate: f64,
}
impl Keys {
    /// The cel held at timeline frame `frame` (constant interpolation), or
    /// None before the first key or on an empty key.
    pub(crate) fn cel_at(&self, frame: f64, frame_rate: f64) -> Option<&str> {
        let time = frame * self.rate / frame_rate;
        let index = self.times.iter().rposition(|t| *t <= time + 1e-6)?;
        Some(self.cels[index].as_str()).filter(|cel| !cel.is_empty())
    }
}

/// Parse an external chunk body: u64 BE remaining length, u32 LE zlib
/// length, then the zlib stream.
pub(crate) fn keys(body: &[u8]) -> Result<Keys, String> {
    let declared = body
        .get(..8)
        .map(|b| u64::from_be_bytes(b.try_into().unwrap()))
        .ok_or("Truncated track mixer")?;
    let compressed = body
        .get(8..12)
        .map(|b| u32::from_le_bytes(b.try_into().unwrap()) as usize)
        .ok_or("Truncated track mixer")?;
    if declared != body.len() as u64 - 8 || compressed != body.len() - 12 {
        return Err("Track mixer lengths disagree".into());
    }
    let mut raw = Vec::new();
    flate2::read::ZlibDecoder::new(&body[12..])
        .take(MAX_INFLATED + 1)
        .read_to_end(&mut raw)
        .map_err(|e| format!("Track mixer: {e}"))?;
    if raw.len() as u64 > MAX_INFLATED || !raw.starts_with(MAGIC) {
        return Err("Unsupported track mixer stream".into());
    }
    parse(&raw)
}

fn u32_at(data: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(
        data.get(at..at.checked_add(4)?)?.try_into().ok()?,
    ))
}

fn parse(raw: &[u8]) -> Result<Keys, String> {
    let count = u32_at(raw, 16).ok_or("Truncated track strings")? as usize;
    if count > 4096 {
        return Err("Track string table exceeds limits".into());
    }
    let mut strings = Vec::with_capacity(count);
    let mut at = 20;
    for _ in 0..count {
        let length = *raw.get(at).ok_or("Truncated track strings")? as usize;
        let text = raw
            .get(at + 1..at + 1 + length)
            .ok_or("Truncated track strings")?;
        strings.push(String::from_utf8_lossy(text).into_owned());
        at += 1 + length;
    }
    let body = &raw[at..];
    let index = |name: &str| strings.iter().position(|s| s == name).map(|i| i as u32);
    let (frame, tag, rate, start) = (
        index("Frame").ok_or("Track has no keyframes")?,
        index("Tag").ok_or("Track has no cel names")?,
        index("Rate").ok_or("Track has no rate")?,
        index("Start"),
    );
    let type_of = |at: usize| u32_at(body, at).and_then(|t| strings.get(t as usize));
    let mut times = Vec::new();
    let mut cels = Vec::new();
    let mut rates = Vec::new();
    // Properties are byte-packed, so try every offset; each match must end in
    // the 8 zero bytes that close a property.
    for at in 0..body.len().saturating_sub(12) {
        let Some(key) = u32_at(body, at) else { break };
        if ![frame, tag, rate].contains(&key) && Some(key) != start {
            continue;
        }
        let Some(kind) = type_of(at + 4) else {
            continue;
        };
        let closed = |end: usize| body.get(end..end + 8) == Some(&[0; 8][..]);
        match (key, kind.as_str()) {
            (k, "Single[]" | "Double[]") if k == frame => {
                let width = if kind == "Single[]" { 4 } else { 8 };
                let Some(n) = u32_at(body, at + 8).map(|n| n as usize) else {
                    continue;
                };
                let end = at + 12 + n * width;
                if n == 0 || n > MAX_KEYS || !closed(end) {
                    continue;
                }
                let values = body[at + 12..end].chunks_exact(width).map(|c| {
                    if width == 4 {
                        f64::from(f32::from_le_bytes(c.try_into().unwrap()))
                    } else {
                        f64::from_le_bytes(c.try_into().unwrap())
                    }
                });
                times.push(values.collect::<Vec<_>>());
            }
            (k, "String[]") if k == tag => {
                let Some(n) = u32_at(body, at + 8).map(|n| n as usize) else {
                    continue;
                };
                let end = at + 12 + n * 4;
                if n == 0 || n > MAX_KEYS || !closed(end) {
                    continue;
                }
                let names: Option<Vec<String>> = (0..n)
                    .map(|i| {
                        strings
                            .get(u32_at(body, at + 12 + i * 4)? as usize)
                            .cloned()
                    })
                    .collect();
                if let Some(names) = names {
                    cels.push(names);
                }
            }
            (k, "Double") if k == rate || Some(k) == start => {
                if !closed(at + 16) {
                    continue;
                }
                let value = f64::from_le_bytes(body[at + 8..at + 16].try_into().unwrap());
                if k == rate {
                    rates.push(value);
                } else if value != 0.0 {
                    return Err("Track clip starts after the timeline start".into());
                }
            }
            _ => {}
        }
    }
    let ([times], [cels]) = (times.as_slice(), cels.as_slice()) else {
        return Err("Track has no single cel keyframe curve".into());
    };
    let rate = rates.first().copied().ok_or("Track has no rate")?;
    if times.len() != cels.len()
        || !(rate.is_finite() && rate > 0.0)
        || rates.iter().any(|r| *r != rate)
        || times.iter().any(|t| !t.is_finite())
        || times.windows(2).any(|w| w[0] > w[1])
    {
        return Err("Track keyframes are inconsistent".into());
    }
    Ok(Keys {
        times: times.clone(),
        cels: cels.clone(),
        rate,
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::io::Write;

    /// A mixer body shaped like CSP's: string table, then properties, with a
    /// one-byte Byte property first so later ones are not word-aligned.
    pub(crate) fn mixer(times: &[f32], cels: &[&str], rate: f64) -> Vec<u8> {
        let mut strings: Vec<String> = [
            "null",
            "Byte",
            "Double",
            "Single[]",
            "String[]",
            "Rate",
            "Start",
            "Frame",
            "Tag",
            "BlendEnable",
            "",
        ]
        .map(String::from)
        .to_vec();
        let mut id = |name: &str| {
            let at = strings.iter().position(|s| s == name).unwrap_or_else(|| {
                strings.push(name.into());
                strings.len() - 1
            });
            at as u32
        };
        let mut body = Vec::new();
        let word = |body: &mut Vec<u8>, w: u32| body.extend(w.to_le_bytes());
        word(&mut body, id("BlendEnable"));
        word(&mut body, id("Byte"));
        body.push(1);
        body.extend([0; 8]);
        for key in ["Start", "Rate"] {
            word(&mut body, id(key));
            word(&mut body, id("Double"));
            body.extend(if key == "Rate" { rate } else { 0.0 }.to_le_bytes());
            body.extend([0; 8]);
        }
        word(&mut body, id("Frame"));
        word(&mut body, id("Single[]"));
        word(&mut body, times.len() as u32);
        for t in times {
            body.extend(t.to_le_bytes());
        }
        body.extend([0; 8]);
        let names: Vec<u32> = cels.iter().map(|c| id(c)).collect();
        word(&mut body, id("Tag"));
        word(&mut body, id("String[]"));
        word(&mut body, names.len() as u32);
        for n in names {
            word(&mut body, n);
        }
        body.extend([0; 8]);
        let mut raw = MAGIC.to_vec();
        raw.extend([0; 4]);
        raw.extend((strings.len() as u32).to_le_bytes());
        for s in &strings {
            raw.push(s.len() as u8);
            raw.extend(s.as_bytes());
        }
        raw.extend(body);
        let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::fast());
        z.write_all(&raw).unwrap();
        let z = z.finish().unwrap();
        let mut out = ((z.len() + 4) as u64).to_be_bytes().to_vec();
        out.extend((z.len() as u32).to_le_bytes());
        out.extend(z);
        out
    }

    #[test]
    fn constant_keys_pick_the_cel_held_at_a_frame() {
        // Drawn on twos at 30 fps in 1/60 s units, like the observed sample.
        let body = mixer(&[0., 4., 8., 66.], &["1", "2", "6a", "7b"], 60.0);
        let keys = keys(&body).unwrap();
        assert_eq!(keys.rate, 60.0);
        assert_eq!(keys.cel_at(0.0, 30.0), Some("1"));
        assert_eq!(keys.cel_at(3.0, 30.0), Some("2"));
        assert_eq!(keys.cel_at(32.0, 30.0), Some("6a"));
        assert_eq!(keys.cel_at(37.0, 30.0), Some("7b"));
        // One key per frame at 6 fps.
        let six = keys_of(&mixer(&[0., 10., 110.], &["1", "2", "12"], 60.0));
        assert_eq!(six.cel_at(11.0, 6.0), Some("12"));
        assert_eq!(six.cel_at(10.0, 6.0), Some("2"));
        // An empty key shows nothing.
        let blank = keys_of(&mixer(&[0., 10.], &["1", ""], 60.0));
        assert_eq!(blank.cel_at(1.0, 6.0), None);
    }
    fn keys_of(body: &[u8]) -> Keys {
        keys(body).unwrap()
    }
    #[test]
    fn damaged_or_ambiguous_mixers_are_rejected() {
        let body = mixer(&[0., 4.], &["1", "2"], 60.0);
        for end in 0..body.len() {
            assert!(keys(&body[..end]).is_err(), "{end}");
        }
        assert!(keys(&mixer(&[4., 0.], &["1", "2"], 60.0)).is_err());
        assert!(keys(&mixer(&[0., 4.], &["1"], 60.0)).is_err());
        assert!(keys(&mixer(&[0.], &["1"], 0.0)).is_err());
        let mut bad = body.clone();
        bad[0] ^= 1;
        assert!(keys(&bad).is_err());
    }
}
