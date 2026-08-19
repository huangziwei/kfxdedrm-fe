//! PalmDOC `encryption_type` for `.azw`, `.azw3`, `.azw4`, `.mobi`, `.prc`.
//!
//! `[crate::scan]` gates a MOBI-family book on this. A Topaz database fails
//! the `[BOOKMOBI]` check and reads as `None`.
//!
//! [`Encryption::is_drm`] covers types 1 and 2, the pair the engine decodes.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

/// PalmDB `type`+`creator` at offset 60 for a Mobipocket-family database.
const BOOKMOBI: &[u8; 8] = b"BOOKMOBI";

/// Offset of the `type`+`creator` pair in the PalmDB header.
const TYPE_CREATOR_OFF: usize = 60;
/// Record-info list: 8 bytes per record, the first four its file offset.
const RECORD_LIST_OFF: usize = 78;
/// Header bytes through the first record-list entry's 4-byte offset.
pub const HEADER_PREFIX: usize = RECORD_LIST_OFF + 4;

/// `encryption_type` within record 0, and the bytes of record 0 to read.
const ENCRYPTION_OFF: usize = 12;
const REC0_PREFIX: usize = ENCRYPTION_OFF + 2;

/// What record 0's `encryption_type` field says.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Encryption {
    /// Type 0.
    None,
    /// Type 1.
    Legacy,
    /// Type 2.
    Mobipocket,
    /// Any other value. The engine reports "Cannot decode unknown Mobipocket
    /// encryption type" for these.
    Unknown(u16),
}

impl Encryption {
    /// True for [`Encryption::Legacy`] and [`Encryption::Mobipocket`], the two
    /// the engine decodes.
    pub fn is_drm(self) -> bool {
        matches!(self, Encryption::Legacy | Encryption::Mobipocket)
    }
}

/// Big-endian `u16` at `off`, or `None` if the slice is too short.
fn be16(bytes: &[u8], off: usize) -> Option<u16> {
    let b = bytes.get(off..off + 2)?;
    Some(u16::from_be_bytes([b[0], b[1]]))
}

/// Big-endian `u32` at `off`, or `None` if the slice is too short.
fn be32(bytes: &[u8], off: usize) -> Option<u32> {
    let b = bytes.get(off..off + 4)?;
    Some(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
}

/// Offset of record 0, from at least [`HEADER_PREFIX`] bytes. `None` for a
/// short slice or a type/creator pair that is not [`BOOKMOBI`].
pub fn record0_offset(header: &[u8]) -> Option<u64> {
    if header.get(TYPE_CREATOR_OFF..TYPE_CREATOR_OFF + 8)? != BOOKMOBI {
        return None;
    }
    Some(be32(header, RECORD_LIST_OFF)? as u64)
}

/// `encryption_type`, from at least [`REC0_PREFIX`] bytes of record 0.
pub fn encryption_of_record0(rec0: &[u8]) -> Option<Encryption> {
    Some(match be16(rec0, ENCRYPTION_OFF)? {
        0 => Encryption::None,
        1 => Encryption::Legacy,
        2 => Encryption::Mobipocket,
        other => Encryption::Unknown(other),
    })
}

/// [`record0_offset`] then [`encryption_of_record0`] over one buffer holding
/// the header and record 0.
pub fn encryption(bytes: &[u8]) -> Option<Encryption> {
    let rec0 = record0_offset(bytes)? as usize;
    encryption_of_record0(bytes.get(rec0..)?)
}

/// [`encryption`] over `path`, as two reads of [`HEADER_PREFIX`] and
/// [`REC0_PREFIX`] bytes. `None` on an I/O error, a truncated file, or a
/// non-[`BOOKMOBI`] database.
pub fn file_encryption(path: &Path) -> Option<Encryption> {
    let mut f = File::open(path).ok()?;

    let mut header = [0u8; HEADER_PREFIX];
    f.read_exact(&mut header).ok()?;
    let rec0 = record0_offset(&header)?;

    f.seek(SeekFrom::Start(rec0)).ok()?;
    let mut prefix = [0u8; REC0_PREFIX];
    f.read_exact(&mut prefix).ok()?;
    encryption_of_record0(&prefix)
}

/// [`Encryption::is_drm`] of [`file_encryption`]. False when that is `None`.
pub fn is_encrypted(path: &Path) -> bool {
    file_encryption(path).is_some_and(Encryption::is_drm)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A minimal PalmDB: the 78-byte header, one 8-byte record-list entry,
    /// then a record 0 carrying `enc`.
    fn palmdb_with(type_creator: &[u8; 8], enc: u16) -> Vec<u8> {
        let rec0 = (RECORD_LIST_OFF + 8) as u32;
        let mut v = vec![0u8; RECORD_LIST_OFF + 8];
        v[TYPE_CREATOR_OFF..TYPE_CREATOR_OFF + 8].copy_from_slice(type_creator);
        v[76..78].copy_from_slice(&1u16.to_be_bytes()); // one record
        v[RECORD_LIST_OFF..RECORD_LIST_OFF + 4].copy_from_slice(&rec0.to_be_bytes());
        // Record 0: compression, unused, text length, record count, record
        // size, then `encryption_type`.
        v.extend_from_slice(&[0u8; ENCRYPTION_OFF]);
        v.extend_from_slice(&enc.to_be_bytes());
        v
    }

    /// [`palmdb_with`] as a Mobipocket book — the common case for a fixture.
    fn palmdb(enc: u16) -> Vec<u8> {
        palmdb_with(BOOKMOBI, enc)
    }

    #[test]
    fn reads_each_defined_encryption_type() {
        assert_eq!(encryption(&palmdb(0)), Some(Encryption::None));
        assert_eq!(encryption(&palmdb(1)), Some(Encryption::Legacy));
        assert_eq!(encryption(&palmdb(2)), Some(Encryption::Mobipocket));
        // Types 1 and 2 are the pair the engine decodes.
        assert!(!Encryption::None.is_drm());
        assert!(Encryption::Legacy.is_drm());
        assert!(Encryption::Mobipocket.is_drm());
    }

    #[test]
    fn an_undefined_type_is_not_offered() {
        // The engine reports "Cannot decode unknown Mobipocket encryption
        // type" for anything outside 0, 1 and 2.
        let e = encryption(&palmdb(9)).unwrap();
        assert_eq!(e, Encryption::Unknown(9));
        assert!(!e.is_drm());
        assert!(!Encryption::Unknown(0xFFFF).is_drm());
    }

    #[test]
    fn rejects_databases_that_are_not_mobipocket() {
        // Topaz: a PalmDB, but not one the engine's MOBI path can open.
        assert_eq!(encryption(&palmdb_with(b"TPZ3TPZ3", 2)), None);
        assert_eq!(record0_offset(&palmdb_with(b"TEXtREAd", 2)), None);
    }

    #[test]
    fn short_and_truncated_files_classify_as_nothing() {
        assert_eq!(encryption(&[]), None);
        // Header present and well-formed, but record 0 is cut off before the
        // field — a partially copied file, which must not read as DRM-free.
        let mut short = palmdb(2);
        short.truncate(RECORD_LIST_OFF + 8 + 4);
        assert_eq!(encryption(&short), None);
        // The header itself truncated inside the type/creator pair.
        let mut stub = palmdb(2);
        stub.truncate(TYPE_CREATOR_OFF + 4);
        assert_eq!(record0_offset(&stub), None);
    }

    #[test]
    fn record0_offset_is_read_from_the_record_list() {
        // The offset is a real pointer, not an assumption about layout: move
        // record 0 and the field must follow it.
        let mut v = palmdb(2);
        let moved = v.len() as u32 + 64;
        v[RECORD_LIST_OFF..RECORD_LIST_OFF + 4].copy_from_slice(&moved.to_be_bytes());
        assert_eq!(record0_offset(&v), Some(moved as u64));
        // `moved` points past the end of `v`.
        assert_eq!(encryption(&v), None);
        v.resize(moved as usize, 0);
        v.extend_from_slice(&[0u8; ENCRYPTION_OFF]);
        v.extend_from_slice(&2u16.to_be_bytes());
        assert_eq!(encryption(&v), Some(Encryption::Mobipocket));
    }
}
