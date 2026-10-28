//! Complete diagnostic frame history with fixed RAM staging, independent of FPS.
use serde::{Serialize, de::DeserializeOwned, ser::SerializeSeq};
use std::{
    io::{BufRead, BufReader, BufWriter, Read, Write},
    marker::PhantomData,
    path::Path,
};

const BUFFER_BYTES: usize = 64 * 1024;
// FrameSample contains only fixed numeric fields and a small phase name.
const MAX_RECORD_BYTES: usize = 4096;

pub(crate) struct FrameLog<T> {
    writer: BufWriter<tempfile::NamedTempFile>,
    count: usize,
    marker: PhantomData<T>,
}

fn append(writer: &mut impl Write, sample: &impl Serialize) -> Result<(), String> {
    serde_json::to_writer(&mut *writer, sample).map_err(|e| e.to_string())?;
    writer.write_all(b"\n").map_err(|e| e.to_string())
}

impl<T: Serialize + DeserializeOwned> FrameLog<T> {
    pub(crate) fn new() -> Result<Self, String> {
        Self::with_capacity(BUFFER_BYTES)
    }
    fn with_capacity(bytes: usize) -> Result<Self, String> {
        Ok(Self {
            writer: BufWriter::with_capacity(
                bytes,
                tempfile::NamedTempFile::new().map_err(|e| e.to_string())?,
            ),
            count: 0,
            marker: PhantomData,
        })
    }
    pub(crate) fn push(&mut self, sample: &T) -> Result<(), String> {
        append(&mut self.writer, sample)?;
        self.count += 1;
        Ok(())
    }
    pub(crate) fn len(&self) -> usize {
        self.count
    }
    pub(crate) fn capacity_bytes(&self) -> usize {
        self.writer.capacity()
    }
    pub(crate) fn used_bytes(&self) -> usize {
        self.writer.buffer().len()
    }
    pub(crate) fn write_report(
        &mut self,
        path: &Path,
        metadata: &serde_json::Value,
    ) -> Result<(), String> {
        self.writer.flush().map_err(|e| e.to_string())?;
        #[derive(Serialize)]
        #[serde(bound(serialize = "T: Serialize + DeserializeOwned"))]
        struct Report<'a, T> {
            #[serde(flatten)]
            metadata: &'a serde_json::Value,
            frames: &'a FrameLog<T>,
        }
        crate::mask_transform_benchmark::write_report(
            path,
            &Report {
                metadata,
                frames: self,
            },
        )
    }
}

impl<T: Serialize + DeserializeOwned> Serialize for FrameLog<T> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::Error;
        if !self.writer.buffer().is_empty() {
            return Err(S::Error::custom("Diagnostic frame log must be flushed"));
        }
        let file = self.writer.get_ref().reopen().map_err(S::Error::custom)?;
        let mut reader = BufReader::new(file);
        let mut line = String::new();
        let mut sequence = serializer.serialize_seq(Some(self.count))?;
        let mut count = 0;
        loop {
            line.clear();
            if reader
                .by_ref()
                .take((MAX_RECORD_BYTES + 1) as u64)
                .read_line(&mut line)
                .map_err(S::Error::custom)?
                == 0
            {
                break;
            }
            if line.len() > MAX_RECORD_BYTES {
                return Err(S::Error::custom("Oversized diagnostic frame record"));
            }
            let value: T = serde_json::from_str(&line).map_err(S::Error::custom)?;
            sequence.serialize_element(&value)?;
            count += 1;
        }
        if count != self.count {
            return Err(S::Error::custom("Diagnostic frame log is incomplete"));
        }
        sequence.end()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_frame_survives_many_buffer_flushes_with_fixed_staging_and_temp_cleanup() {
        let mut log = FrameLog::<[u64; 3]>::with_capacity(32).unwrap();
        let temporary = log.writer.get_ref().path().to_owned();
        for index in 0..25_000 {
            log.push(&[index, index % 30, index * 7]).unwrap();
            assert_eq!(log.capacity_bytes(), 32);
            assert!(log.used_bytes() <= 32);
        }
        assert_eq!(log.len(), 25_000);
        let dir = tempfile::tempdir().unwrap();
        let output = dir.path().join("complete.json");
        log.write_report(&output, &serde_json::json!({"complete":true}))
            .unwrap();
        let report: serde_json::Value =
            serde_json::from_reader(std::fs::File::open(output).unwrap()).unwrap();
        let frames = report["frames"].as_array().unwrap();
        assert_eq!(frames.len(), 25_000);
        for (index, frame) in frames.iter().enumerate() {
            assert_eq!(*frame, serde_json::json!([index, index % 30, index * 7]));
        }
        drop(log);
        assert!(!temporary.exists());
    }

    #[test]
    fn damaged_frame_logs_cannot_publish_and_existing_reports_are_preserved() {
        let dir = tempfile::tempdir().unwrap();
        for damage in [
            b"1\n".to_vec(),
            b"invalid\n".to_vec(),
            b"1\n2\n3\n".to_vec(),
            vec![b'1'; MAX_RECORD_BYTES + 1],
        ] {
            let mut log = FrameLog::<u64>::new().unwrap();
            log.push(&1).unwrap();
            log.push(&2).unwrap();
            log.writer.flush().unwrap();
            std::fs::write(log.writer.get_ref().path(), damage).unwrap();
            let output = dir.path().join("must-not-publish.json");
            assert!(log.write_report(&output, &serde_json::json!({})).is_err());
            assert!(!output.exists());
        }
        let output = dir.path().join("existing.json");
        std::fs::write(&output, b"ORIGINAL").unwrap();
        assert!(
            FrameLog::<u64>::new()
                .unwrap()
                .write_report(&output, &serde_json::json!({}))
                .is_err()
        );
        assert_eq!(std::fs::read(output).unwrap(), b"ORIGINAL");
    }

    #[test]
    fn frame_write_errors_are_returned_instead_of_becoming_successful_samples() {
        struct FailedDisk;
        impl Write for FailedDisk {
            fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
                Err(std::io::Error::other("injected diagnostic disk failure"))
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Err(std::io::Error::other("injected diagnostic disk failure"))
            }
        }
        let error = append(&mut FailedDisk, &[1, 2, 3]).unwrap_err();
        assert!(error.contains("injected diagnostic disk failure"));
    }
}
