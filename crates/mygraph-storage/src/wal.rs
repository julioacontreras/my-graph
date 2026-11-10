//! Write-ahead log.
//!
//! File layout: `MAGIC (8) | format version u16`, then records:
//! `payload length u32 | crc32(payload) u32 | payload`, where
//! `payload = lsn u64 | op count u32 | ops...`.
//!
//! One record is one committed transaction. A torn or corrupted tail (crash
//! during append) is detected by length/CRC checks and truncated on open.

use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

use mygraph_core::{MyGraphError, Op, Result};

use crate::codec::{Decoder, Encoder};

const MAGIC: &[u8; 8] = b"MYGRWAL\0";
pub const FORMAT_VERSION: u16 = 1;
const HEADER_LEN: u64 = 10;
const RECORD_HEADER_LEN: usize = 8;

#[derive(Debug, Clone, PartialEq)]
pub struct WalRecord {
    pub lsn: u64,
    pub ops: Vec<Op>,
}

pub struct Wal {
    file: File,
    size: u64,
    sync: bool,
    truncated_bytes: u64,
}

fn header() -> Vec<u8> {
    let mut h = MAGIC.to_vec();
    h.extend_from_slice(&FORMAT_VERSION.to_le_bytes());
    h
}

impl Wal {
    /// Opens (or creates) the WAL and returns the valid records it contains.
    pub fn open(path: &Path, sync: bool) -> Result<(Wal, Vec<WalRecord>)> {
        let mut file = OpenOptions::new().read(true).write(true).create(true).truncate(false).open(path)?;
        let mut data = Vec::new();
        file.read_to_end(&mut data)?;

        if data.is_empty() {
            file.write_all(&header())?;
            file.sync_all()?;
            return Ok((Wal { file, size: HEADER_LEN, sync, truncated_bytes: 0 }, Vec::new()));
        }
        if data.len() < HEADER_LEN as usize || &data[..8] != MAGIC {
            return Err(MyGraphError::Corruption("WAL: bad header".into()));
        }
        let version = u16::from_le_bytes([data[8], data[9]]);
        if version != FORMAT_VERSION {
            return Err(MyGraphError::Storage(format!(
                "WAL format version {version} is not supported (expected {FORMAT_VERSION})"
            )));
        }

        let mut records = Vec::new();
        let mut pos = HEADER_LEN as usize;
        while let Some((rec, next)) = parse_record(&data, pos) {
            records.push(rec);
            pos = next;
        }
        let truncated_bytes = (data.len() - pos) as u64;
        if truncated_bytes > 0 {
            file.set_len(pos as u64)?;
            file.sync_all()?;
        }
        Ok((Wal { file, size: pos as u64, sync, truncated_bytes }, records))
    }

    /// Appends a record and, if configured, fsyncs before returning.
    pub fn append(&mut self, rec: &WalRecord) -> Result<()> {
        let mut payload = Encoder::default();
        payload.u64(rec.lsn);
        payload.u32(rec.ops.len() as u32);
        for op in &rec.ops {
            payload.op(op)?;
        }
        let len = u32::try_from(payload.buf.len())
            .map_err(|_| MyGraphError::Storage("transaction too large for a WAL record".into()))?;
        let mut frame = Vec::with_capacity(RECORD_HEADER_LEN + payload.buf.len());
        frame.extend_from_slice(&len.to_le_bytes());
        frame.extend_from_slice(&crc32fast::hash(&payload.buf).to_le_bytes());
        frame.extend_from_slice(&payload.buf);

        let result = (|| -> Result<()> {
            self.file.seek(SeekFrom::Start(self.size))?;
            self.file.write_all(&frame)?;
            if self.sync {
                self.file.sync_data()?;
            }
            Ok(())
        })();
        match result {
            Ok(()) => {
                self.size += frame.len() as u64;
                Ok(())
            }
            Err(e) => {
                // Drop any partial write so the next append starts clean.
                let _ = self.file.set_len(self.size);
                Err(e)
            }
        }
    }

    /// Discards all records (after a checkpoint has persisted them).
    pub fn reset(&mut self) -> Result<()> {
        self.file.set_len(HEADER_LEN)?;
        self.file.sync_all()?;
        self.size = HEADER_LEN;
        Ok(())
    }

    pub fn size(&self) -> u64 {
        self.size
    }

    /// Bytes of torn/corrupted tail discarded when the WAL was opened.
    pub fn truncated_bytes(&self) -> u64 {
        self.truncated_bytes
    }
}

fn parse_record(data: &[u8], pos: usize) -> Option<(WalRecord, usize)> {
    let header = data.get(pos..pos + RECORD_HEADER_LEN)?;
    let len = u32::from_le_bytes(header[..4].try_into().unwrap()) as usize;
    let crc = u32::from_le_bytes(header[4..].try_into().unwrap());
    let start = pos + RECORD_HEADER_LEN;
    let payload = data.get(start..start.checked_add(len)?)?;
    if crc32fast::hash(payload) != crc {
        return None;
    }
    let mut d = Decoder::new(payload);
    let lsn = d.u64().ok()?;
    let n = d.u32().ok()?;
    let ops = (0..n).map(|_| d.op()).collect::<Result<Vec<_>>>().ok()?;
    if !d.is_empty() {
        return None;
    }
    Some((WalRecord { lsn, ops }, start + len))
}
