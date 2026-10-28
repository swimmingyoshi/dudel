//! Cold undo payloads. Each worker owns at most one tile-region input. Decoded
//! bytes can prefetch into nearby actions under a bounded budget. Packed records
//! themselves never retain a second decoded copy.
use std::{io::Write, sync::Arc};

pub(crate) const MAX_REGION_BYTES: usize = crate::pixels::SIDE * crate::pixels::SIDE * 4;
pub(crate) const MIN_REGION_BYTES: usize = 4096;
pub(crate) fn raw_capacity(bytes: &Arc<Vec<u8>>) -> usize {
    bytes.capacity() + std::mem::size_of::<Vec<u8>>() + 2 * std::mem::size_of::<usize>()
}

pub(crate) enum Payload {
    Raw {
        bytes: Arc<Vec<u8>>,
        checked: bool,
        prefetched: bool,
    },
    Packed {
        bytes: Arc<Vec<u8>>,
        raw_len: usize,
        warm_checked: bool,
    },
}
impl Payload {
    pub fn new(bytes: Vec<u8>) -> Self {
        Self::Raw {
            bytes: Arc::new(bytes),
            checked: false,
            prefetched: false,
        }
    }
    #[cfg(test)]
    pub fn len(&self) -> usize {
        match self {
            Self::Raw { bytes, .. } => bytes.len(),
            Self::Packed { raw_len, .. } => *raw_len,
        }
    }
    pub fn cold_candidate(&self) -> Option<&Arc<Vec<u8>>> {
        match self {
            Self::Raw {
                bytes,
                checked: false,
                ..
            } if (MIN_REGION_BYTES..=MAX_REGION_BYTES).contains(&bytes.len()) => Some(bytes),
            _ => None,
        }
    }
    pub fn capacity(&self) -> usize {
        match self {
            Self::Raw { bytes, .. } | Self::Packed { bytes, .. } => raw_capacity(bytes),
        }
    }
    pub fn allocation_identity(&self) -> usize {
        match self {
            Self::Raw { bytes, .. } | Self::Packed { bytes, .. } => Arc::as_ptr(bytes) as usize,
        }
    }
    pub fn prefetched_bytes(&self) -> usize {
        if matches!(
            self,
            Self::Raw {
                prefetched: true,
                ..
            }
        ) {
            self.capacity()
        } else {
            0
        }
    }
    pub fn warm_candidate(&self) -> Option<(&Arc<Vec<u8>>, usize)> {
        if let Self::Packed {
            bytes,
            raw_len,
            warm_checked: false,
        } = self
            && *raw_len <= MAX_REGION_BYTES
        {
            Some((bytes, *raw_len))
        } else {
            None
        }
    }
    pub fn packed(bytes: Box<[u8]>, raw_len: usize) -> Self {
        Self::Packed {
            bytes: Arc::new(bytes.into_vec()),
            raw_len,
            warm_checked: false,
        }
    }
    #[cfg(test)]
    pub fn raw_identity(&self) -> Option<usize> {
        match self {
            Self::Raw { bytes, .. } => Some(Arc::as_ptr(bytes) as usize),
            _ => None,
        }
    }
    pub fn accept(&mut self, source: &Arc<Vec<u8>>, compressed: Option<Box<[u8]>>) -> bool {
        if let Self::Raw { bytes, checked, .. } = self
            && Arc::ptr_eq(bytes, source)
        {
            *checked = true;
            if let Some(compressed) = compressed {
                *self = Self::packed(compressed, source.len());
            }
            true
        } else {
            false
        }
    }
    pub fn matches(&self, source: &Source) -> bool {
        match (self, source) {
            (Self::Raw { bytes, .. }, Source::Raw(input)) => Arc::ptr_eq(bytes, input),
            (
                Self::Packed { bytes, raw_len, .. },
                Source::Packed {
                    bytes: input,
                    raw_len: len,
                },
            ) => raw_len == len && Arc::ptr_eq(bytes, input),
            _ => false,
        }
    }
    pub fn accept_job(&mut self, source: &Source, output: Output) -> bool {
        if !self.matches(source) {
            return false;
        }
        match (source, output) {
            (Source::Raw(bytes), Output::Packed(packed)) => {
                self.accept(bytes, packed);
            }
            (Source::Packed { .. }, Output::Unpacked(Ok(bytes))) => {
                *self = Self::Raw {
                    bytes: Arc::new(bytes),
                    checked: false,
                    prefetched: true,
                };
            }
            (Source::Packed { .. }, Output::Unpacked(Err(_))) | (_, Output::Failed) => match self {
                Self::Raw { checked, .. } => *checked = true,
                Self::Packed { warm_checked, .. } => *warm_checked = true,
            },
            _ => return false,
        }
        true
    }
    pub fn prepare(&mut self, expected: usize) -> Result<(), String> {
        match self {
            Self::Raw { bytes, .. } => {
                if bytes.len() != expected {
                    return Err("Undo pixel payload has an invalid length".into());
                }
            }
            Self::Packed { bytes, raw_len, .. } => {
                if *raw_len != expected || expected > MAX_REGION_BYTES {
                    return Err("Compressed undo payload has an invalid length".into());
                }
                *self = Self::new(decode(bytes, expected)?);
            }
        }
        Ok(())
    }
    pub fn raw_mut(&mut self) -> &mut Vec<u8> {
        let Self::Raw {
            bytes,
            checked,
            prefetched,
        } = self
        else {
            unreachable!("Undo payload was prepared before applying")
        };
        *checked = false;
        *prefetched = false;
        Arc::make_mut(bytes)
    }
}

fn decode(bytes: &[u8], expected: usize) -> Result<Vec<u8>, String> {
    if expected > MAX_REGION_BYTES {
        return Err("Compressed undo payload is too large".into());
    }
    let mut decoder = flate2::Decompress::new(true);
    let mut decoded = Vec::with_capacity(expected + 1);
    let status = decoder
        .decompress_vec(bytes, &mut decoded, flate2::FlushDecompress::Finish)
        .map_err(|e| format!("Cannot decode undo pixels: {e}"))?;
    if status != flate2::Status::StreamEnd
        || decoded.len() != expected
        || decoder.total_in() != bytes.len() as u64
    {
        return Err("Compressed undo payload is incomplete or has extra data".into());
    }
    Ok(decoded)
}

pub(crate) fn compress(bytes: &[u8]) -> Option<Box<[u8]>> {
    if !(MIN_REGION_BYTES..=MAX_REGION_BYTES).contains(&bytes.len()) {
        return None;
    }
    let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::fast());
    encoder.write_all(bytes).ok()?;
    let compressed = encoder.finish().ok()?;
    // Retain raw noise instead of paying decode cost for negligible savings.
    (compressed.len() <= bytes.len() - bytes.len() / 8).then(|| compressed.into_boxed_slice())
}

pub(crate) struct Cooling {
    pub source: Source,
    pub worker: std::thread::JoinHandle<Output>,
}
#[derive(Clone)]
pub(crate) enum Source {
    Raw(Arc<Vec<u8>>),
    Packed { bytes: Arc<Vec<u8>>, raw_len: usize },
}
impl Source {
    pub fn bytes(&self) -> &Arc<Vec<u8>> {
        match self {
            Self::Raw(bytes) | Self::Packed { bytes, .. } => bytes,
        }
    }
    pub fn identity(&self) -> usize {
        Arc::as_ptr(self.bytes()) as usize
    }
    pub fn run(self) -> Output {
        match self {
            Self::Raw(bytes) => Output::Packed(compress(&bytes)),
            Self::Packed { bytes, raw_len } => Output::Unpacked(decode(&bytes, raw_len)),
        }
    }
}
pub(crate) enum Output {
    Packed(Option<Box<[u8]>>),
    Unpacked(Result<Vec<u8>, String>),
    Failed,
}
impl Output {
    pub fn prefetched_bytes(&self) -> usize {
        match self {
            Self::Unpacked(Ok(bytes)) => {
                bytes.capacity() + std::mem::size_of::<Vec<u8>>() + 2 * std::mem::size_of::<usize>()
            }
            _ => 0,
        }
    }
}
impl Cooling {
    pub fn start(source: Arc<Vec<u8>>) -> Option<Self> {
        Self::launch(Source::Raw(source))
    }
    pub fn prefetch(bytes: Arc<Vec<u8>>, raw_len: usize) -> Option<Self> {
        Self::launch(Source::Packed { bytes, raw_len })
    }
    fn launch(source: Source) -> Option<Self> {
        let input = source.clone();
        let worker = std::thread::Builder::new()
            .name("dudel-undo-cache".into())
            .spawn(move || input.run())
            .ok()?;
        Some(Self { source, worker })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn packed_regions_round_trip_exactly_without_retaining_a_raw_cache() {
        let bytes: Vec<_> = (0..MAX_REGION_BYTES)
            .map(|index| if index % 4 == 3 { 128 } else { 17 })
            .collect();
        let source = Arc::new(bytes.clone());
        let mut data = Payload::new(bytes.clone());
        let unrelated = Arc::new(bytes.clone());
        assert!(!data.accept(&unrelated, compress(&unrelated)));
        let Payload::Raw {
            bytes: own_source, ..
        } = &data
        else {
            unreachable!()
        };
        let own_source = own_source.clone();
        assert!(data.accept(&own_source, compress(&source)));
        assert!(data.capacity() < bytes.len() / 10);
        assert!(data.raw_identity().is_none());
        data.prepare(bytes.len()).unwrap();
        assert_eq!(data.raw_mut(), &bytes);
    }
    #[test]
    fn noise_is_not_recompressed_and_decode_rejects_wrong_lengths_extra_and_truncated_data() {
        let mut hash = 1u32;
        let noise: Vec<u8> = (0..MAX_REGION_BYTES)
            .map(|_| {
                hash ^= hash << 13;
                hash ^= hash >> 17;
                hash ^= hash << 5;
                hash as u8
            })
            .collect();
        assert!(compress(&noise).is_none());
        let source = Arc::new(noise);
        let mut data = Payload::Raw {
            bytes: source.clone(),
            checked: false,
            prefetched: false,
        };
        assert!(data.accept(&source, None));
        assert!(data.cold_candidate().is_none());
        let packed = compress(&vec![0; 8192]).unwrap();
        for bytes in (0..packed.len()).map(|end| packed[..end].to_vec()).chain([[
            packed.as_ref(),
            &[1, 2, 3],
        ]
        .concat()])
        {
            let mut data = Payload::packed(bytes.into_boxed_slice(), 8192);
            assert!(data.prepare(8192).is_err());
        }
        let mut data = Payload::packed(packed, 8192);
        assert!(data.prepare(4096).is_err());
        assert!(data.prepare(8192).is_ok());
        assert!(compress(&vec![0; MAX_REGION_BYTES + 1]).is_none());
    }
}
