//! Full-catalog snapshot files (checkpoints).
//!
//! Layout: `MAGIC (8) | format version u16 | body length u64 | body | crc32(body) u32`.
//! The body starts with the WAL sequence number (LSN) the snapshot includes.

use std::fs::{self, File};
use std::io::Write;
use std::path::Path;

use mygraph_core::{Catalog, Change, Graph, GraphDatabase, MyGraphError, Result, Schema};

use crate::codec::{Decoder, Encoder};

const MAGIC: &[u8; 8] = b"MYGRSNAP";
pub const FORMAT_VERSION: u16 = 1;

pub fn encode(catalog: &Catalog, lsn: u64) -> Result<Vec<u8>> {
    let mut b = Encoder::default();
    b.u64(lsn);
    b.u32(catalog.next_database_id());
    let dbs: Vec<&GraphDatabase> = catalog.databases().collect();
    b.u32(dbs.len() as u32);
    for db in dbs {
        b.u32(db.id);
        b.str(&db.name)?;
        b.u32(db.next_schema_id);
        b.u64(db.ids.next_node_id);
        b.u64(db.ids.next_relationship_id);
        let schemas: Vec<&Schema> = db.schemas().collect();
        b.u32(schemas.len() as u32);
        for s in schemas {
            b.u32(s.id);
            b.str(&s.name)?;
            let g = &s.graph;
            b.u64(g.node_count() as u64);
            for n in g.nodes() {
                b.u64(n.id);
                b.strings(&g.node_label_names(n))?;
                b.properties(&n.properties)?;
                b.u64(n.created_at);
                b.u64(n.updated_at);
            }
            b.u64(g.relationship_count() as u64);
            for r in g.relationships() {
                b.u64(r.id);
                b.u64(r.start_node);
                b.u64(r.end_node);
                b.str(g.rel_type_name(r.rel_type))?;
                b.properties(&r.properties)?;
            }
        }
    }

    let mut out = Encoder::default();
    out.buf.extend_from_slice(MAGIC);
    out.u16(FORMAT_VERSION);
    out.u64(b.buf.len() as u64);
    out.buf.extend_from_slice(&b.buf);
    out.u32(crc32fast::hash(&b.buf));
    Ok(out.buf)
}

pub fn decode(bytes: &[u8]) -> Result<(Catalog, u64)> {
    let corrupt = |m: &str| MyGraphError::Corruption(format!("snapshot: {m}"));
    if bytes.len() < 8 + 2 + 8 + 4 || &bytes[..8] != MAGIC {
        return Err(corrupt("bad header"));
    }
    let mut h = Decoder::new(&bytes[8..18]);
    let version = h.u16()?;
    if version != FORMAT_VERSION {
        return Err(MyGraphError::Storage(format!(
            "snapshot format version {version} is not supported (expected {FORMAT_VERSION})"
        )));
    }
    let len = h.u64()? as usize;
    if bytes.len() != 18 + len + 4 {
        return Err(corrupt("length mismatch"));
    }
    let body = &bytes[18..18 + len];
    let crc = u32::from_le_bytes(bytes[18 + len..].try_into().unwrap());
    if crc32fast::hash(body) != crc {
        return Err(corrupt("checksum mismatch"));
    }

    let mut d = Decoder::new(body);
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

/// Atomically replaces the snapshot at `path` (write temp file, fsync, rename, fsync dir).
pub fn write(path: &Path, catalog: &Catalog, lsn: u64) -> Result<()> {
    let bytes = encode(catalog, lsn)?;
    let tmp = path.with_extension("tmp");
    {
        let mut f = File::create(&tmp)?;
        f.write_all(&bytes)?;
        f.sync_all()?;
    }
    fs::rename(&tmp, path)?;
    if let Some(dir) = path.parent() {
        sync_dir(dir)?;
    }
    Ok(())
}

pub fn read(path: &Path) -> Result<Option<(Catalog, u64)>> {
    match fs::read(path) {
        Ok(bytes) => decode(&bytes).map(Some),
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
