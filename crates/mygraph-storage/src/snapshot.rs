//! Full-catalog snapshot files (checkpoints).
//!
//! Layout: `MAGIC (8) | format version u16 | body length u64 | body | crc32(body) u32`.
//! The body starts with the WAL sequence number (LSN) the snapshot includes.
//! Version 2 adds the property-index definitions of each schema, version 3
//! the constraint definitions; older versions are still read.

use std::fs::{self, File};
use std::io::{BufReader, BufWriter, Cursor, Read, Seek, SeekFrom, Write};
use std::path::Path;

use mygraph_core::{Catalog, Change, Graph, GraphDatabase, MyGraphError, Result, Schema};

use crate::codec::{Decoder, Encoder};

const MAGIC: &[u8; 8] = b"MYGRSNAP";
pub const FORMAT_VERSION: u16 = 3;
const HEADER_LEN: u64 = 8 + 2 + 8;
/// The encoder hands its buffer to the writer once it holds this many bytes,
/// so a checkpoint never holds more than this much of the snapshot in memory.
const CHUNK: usize = 1 << 20;

/// Writes the body to `out` in chunks while hashing and counting it.
struct BodySink<'w, W: Write> {
    out: &'w mut W,
    enc: Encoder,
    crc: crc32fast::Hasher,
    len: u64,
}

impl<W: Write> BodySink<'_, W> {
    fn maybe_flush(&mut self) -> Result<()> {
        if self.enc.buf.len() >= CHUNK {
            self.flush()?;
        }
        Ok(())
    }

    fn flush(&mut self) -> Result<()> {
        self.crc.update(&self.enc.buf);
        self.len += self.enc.buf.len() as u64;
        self.out.write_all(&self.enc.buf)?;
        self.enc.buf.clear();
        Ok(())
    }
}

/// Streams the snapshot of `catalog` into `out`. The body length goes in the
/// header, so it is written as a placeholder and filled in at the end.
pub fn encode_to<W: Write + Seek>(catalog: &Catalog, lsn: u64, out: &mut W) -> Result<()> {
    let base = out.stream_position()?;
    out.write_all(MAGIC)?;
    out.write_all(&FORMAT_VERSION.to_le_bytes())?;
    out.write_all(&0u64.to_le_bytes())?;

    let mut s = BodySink { out, enc: Encoder::default(), crc: crc32fast::Hasher::new(), len: 0 };
    s.enc.buf.reserve(CHUNK + 4096);
    let b = &mut s.enc;
    b.u64(lsn);
    b.u32(catalog.next_database_id());
    let dbs: Vec<&GraphDatabase> = catalog.databases().collect();
    b.u32(dbs.len() as u32);
    for db in dbs {
        let b = &mut s.enc;
        b.u32(db.id);
        b.str(&db.name)?;
        b.u32(db.next_schema_id);
        b.u64(db.ids.next_node_id);
        b.u64(db.ids.next_relationship_id);
        let schemas: Vec<&Schema> = db.schemas().collect();
        b.u32(schemas.len() as u32);
        for sc in schemas {
            let g = &sc.graph;
            let b = &mut s.enc;
            b.u32(sc.id);
            b.str(&sc.name)?;
            b.u64(g.node_count() as u64);
            for n in g.nodes() {
                let b = &mut s.enc;
                b.u64(n.id);
                b.strings(&g.node_label_names(n))?;
                b.stored_properties(&n.properties)?;
                b.u64(n.created_at);
                b.u64(n.updated_at);
                s.maybe_flush()?;
            }
            s.enc.u64(g.relationship_count() as u64);
            for r in g.relationships() {
                let b = &mut s.enc;
                b.u64(r.id);
                b.u64(r.start_node);
                b.u64(r.end_node);
                b.str(g.rel_type_name(r.rel_type))?;
                b.stored_properties(&r.properties)?;
                s.maybe_flush()?;
            }
            let indexes = g.indexes();
            let b = &mut s.enc;
            b.u32(indexes.len() as u32);
            for i in indexes {
                b.str(&i.name)?;
                b.str(&i.label)?;
                b.str(&i.key)?;
            }
            let constraints = g.constraints();
            b.u32(constraints.len() as u32);
            for c in constraints {
                b.constraint(&c.name, &c.label, &c.key, c.kind, c.owns_index)?;
            }
        }
    }
    s.flush()?;
    let (len, crc) = (s.len, s.crc.finalize());
    out.write_all(&crc.to_le_bytes())?;
    let end = out.stream_position()?;
    out.seek(SeekFrom::Start(base + 10))?;
    out.write_all(&len.to_le_bytes())?;
    out.seek(SeekFrom::Start(end))?;
    Ok(())
}

pub fn encode(catalog: &Catalog, lsn: u64) -> Result<Vec<u8>> {
    let mut out = Cursor::new(Vec::new());
    encode_to(catalog, lsn, &mut out)?;
    Ok(out.into_inner())
}

/// Hashes what passes through it, to check the body CRC while streaming.
struct CrcReader<R> {
    inner: R,
    crc: crc32fast::Hasher,
}

impl<R: Read> Read for CrcReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = self.inner.read(buf)?;
        self.crc.update(&buf[..n]);
        Ok(n)
    }
}

fn corrupt(m: &str) -> MyGraphError {
    MyGraphError::Corruption(format!("snapshot: {m}"))
}

/// Decodes a snapshot of `total` bytes from `src`.
fn decode_from<R: Read>(mut src: R, total: u64) -> Result<(Catalog, u64)> {
    let mut header = [0u8; HEADER_LEN as usize];
    if total < HEADER_LEN + 4 || src.read_exact(&mut header).is_err() || &header[..8] != MAGIC {
        return Err(corrupt("bad header"));
    }
    let version = u16::from_le_bytes([header[8], header[9]]);
    if !(1..=FORMAT_VERSION).contains(&version) {
        return Err(MyGraphError::Storage(format!(
            "snapshot format version {version} is not supported (expected {FORMAT_VERSION})"
        )));
    }
    let len = u64::from_le_bytes(header[10..18].try_into().unwrap());
    if total.checked_sub(HEADER_LEN + 4) != Some(len) {
        return Err(corrupt("length mismatch"));
    }

    let mut d = Decoder::from_reader(CrcReader { inner: src, crc: crc32fast::Hasher::new() }, len);
    let body = decode_body(&mut d, version);
    // Check the CRC over the whole body even when decoding stopped early, so
    // a damaged file is reported as such and not as whatever error the
    // damaged bytes happened to cause.
    let drained = if body.is_err() { d.skip_rest().is_ok() } else { true };
    let CrcReader { inner: mut src, crc } = d.into_inner();
    let mut tail = [0u8; 4];
    let crc_ok = drained && src.read_exact(&mut tail).is_ok() && crc.finalize() == u32::from_le_bytes(tail);
    if !crc_ok {
        return Err(corrupt("checksum mismatch"));
    }
    body
}

fn decode_body<R: Read>(d: &mut Decoder<R>, version: u16) -> Result<(Catalog, u64)> {
    let lsn = d.u64()?;
    let mut catalog = Catalog::empty();
    let next_db_id = d.u32()?;
    for _ in 0..d.u32()? {
        let mut db = GraphDatabase::empty(d.u32()?, &d.str()?);
        db.next_schema_id = d.u32()?;
        db.ids.next_node_id = d.u64()?;
        db.ids.next_relationship_id = d.u64()?;
        for _ in 0..d.u32()? {
            let id = d.u32()?;
            let name = d.str()?;
            let mut graph = Graph::new(id);
            for _ in 0..d.u64()? {
                graph.apply(&Change::CreateNode {
                    id: d.u64()?,
                    labels: d.strings()?,
                    properties: d.properties()?,
                    created_at: d.u64()?,
                    updated_at: d.u64()?,
                })?;
            }
            for _ in 0..d.u64()? {
                graph.apply(&Change::CreateRelationship {
                    id: d.u64()?,
                    start: d.u64()?,
                    end: d.u64()?,
                    rel_type: d.str()?,
                    properties: d.properties()?,
                })?;
            }
            if version >= 2 {
                for _ in 0..d.u32()? {
                    graph.apply(&Change::CreateIndex { name: d.str()?, label: d.str()?, key: d.str()? })?;
                }
            }
            if version >= 3 {
                for _ in 0..d.u32()? {
                    graph.apply(&d.constraint()?)?;
                }
            }
            db.insert_schema(Schema { id, name, graph });
        }
        catalog.insert_database(db);
    }
    catalog.set_next_database_id(next_db_id);
    if !d.is_empty() {
        return Err(corrupt("trailing data"));
    }
    Ok((catalog, lsn))
}

pub fn decode(bytes: &[u8]) -> Result<(Catalog, u64)> {
    decode_from(bytes, bytes.len() as u64)
}

/// Atomically replaces the snapshot at `path` (write temp file, fsync, rename, fsync dir).
pub fn write(path: &Path, catalog: &Catalog, lsn: u64) -> Result<()> {
    let tmp = path.with_extension("tmp");
    {
        let mut f = BufWriter::with_capacity(CHUNK, File::create(&tmp)?);
        encode_to(catalog, lsn, &mut f)?;
        let f = f.into_inner().map_err(|e| MyGraphError::from(e.into_error()))?;
        f.sync_all()?;
    }
    fs::rename(&tmp, path)?;
    if let Some(dir) = path.parent() {
        sync_dir(dir)?;
    }
    Ok(())
}

pub fn read(path: &Path) -> Result<Option<(Catalog, u64)>> {
    match File::open(path) {
        Ok(f) => {
            let total = f.metadata()?.len();
            decode_from(BufReader::with_capacity(CHUNK, f), total).map(Some)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.into()),
    }
}

pub(crate) fn sync_dir(dir: &Path) -> Result<()> {
    #[cfg(unix)]
    File::open(dir)?.sync_all()?;
    #[cfg(not(unix))]
    let _ = dir;
    Ok(())
}
