/// On-disk record format (see docs/FORMAT.md for definitive spec).
///
/// Layout (little-endian):
///   0..4   magic     [u8;4] = b"RWL1"
///   4..6   version   u16 = 1
///   6..14  lsn       u64
///   14..18 payload_len u32
///   18..22 crc32c    u32
///   22     flags     u8
///   23+    payload   [payload_len]u8
///
/// CRC32C input (in order): LSN (8 LE) + payload_len (4 LE) + flags (1) + stored_payload.
use crc::{CRC_32_ISCSI, Crc};

use crate::error::Corruption;
use crate::lsn::Lsn;

pub const RECORD_MAGIC: [u8; 4] = *b"RWL1";
pub const FORMAT_VERSION: u16 = 1;
pub const HEADER_SIZE: usize = 23;
pub const MAX_RECORD_SIZE: u32 = 64 * 1024;

pub const FLAG_COMPRESSED: u8 = 0x01;

const CRC32C: Crc<u32> = Crc::<u32>::new(&CRC_32_ISCSI);

/// A decoded record header after all format validation has passed.
pub struct RecordHeader {
    pub lsn: Lsn,
    pub payload_len: u32,
    pub crc32c: u32,
    pub flags: u8,
}

impl RecordHeader {
    /// Encode into a fixed-size byte buffer.
    pub fn encode(&self, dst: &mut [u8; HEADER_SIZE]) {
        dst[0..4].copy_from_slice(&RECORD_MAGIC);
        dst[4..6].copy_from_slice(&FORMAT_VERSION.to_le_bytes());
        dst[6..14].copy_from_slice(&self.lsn.get().to_le_bytes());
        dst[14..18].copy_from_slice(&self.payload_len.to_le_bytes());
        dst[18..22].copy_from_slice(&self.crc32c.to_le_bytes());
        dst[22] = self.flags;
    }

    /// Build header from fields, computing CRC over the stored payload.
    pub fn new(lsn: Lsn, stored_payload: &[u8], flags: u8) -> Self {
        let crc32c = crc32c_of(lsn, stored_payload.len() as u32, flags, stored_payload);
        Self {
            lsn,
            payload_len: stored_payload.len() as u32,
            crc32c,
            flags,
        }
    }

    /// Decode header from bytes. Returns None if insufficient bytes, Err if format error.
    pub fn decode(bytes: &[u8]) -> Option<Result<Self, Corruption>> {
        if bytes.len() < HEADER_SIZE {
            return None;
        }

        if bytes[0..4] != RECORD_MAGIC {
            return Some(Err(Corruption::InvalidMagic { offset: 0 }));
        }

        let version = u16::from_le_bytes([bytes[4], bytes[5]]);
        if version != FORMAT_VERSION {
            return Some(Err(Corruption::UnsupportedVersion { offset: 4, version }));
        }

        let raw = u64::from_le_bytes([
            bytes[6], bytes[7], bytes[8], bytes[9], bytes[10], bytes[11], bytes[12], bytes[13],
        ]);
        let lsn = Lsn::new(raw);

        let plen = u32::from_le_bytes([bytes[14], bytes[15], bytes[16], bytes[17]]);
        if plen > MAX_RECORD_SIZE {
            return Some(Err(Corruption::InvalidLength {
                offset: 14,
                length: plen,
            }));
        }

        let crc32c = u32::from_le_bytes([bytes[18], bytes[19], bytes[20], bytes[21]]);

        let flags = bytes[22];
        if flags & !FLAG_COMPRESSED != 0 {
            return Some(Err(Corruption::InvalidFlags { offset: 22, flags }));
        }

        Some(Ok(Self {
            lsn,
            payload_len: plen,
            crc32c,
            flags,
        }))
    }
}

pub(crate) fn crc32c_of(lsn: Lsn, plen: u32, flags: u8, payload: &[u8]) -> u32 {
    let mut d = CRC32C.digest();
    d.update(&lsn.get().to_le_bytes());
    d.update(&plen.to_le_bytes());
    d.update(&[flags]);
    d.update(payload);
    d.finalize()
}

/// Verify CRC32C of a complete record (header + payload) against stored checksum.
pub fn verify_crc(hdr: &RecordHeader, stored_payload: &[u8]) -> Result<(), Corruption> {
    let actual = crc32c_of(hdr.lsn, hdr.payload_len, hdr.flags, stored_payload);
    if actual != hdr.crc32c {
        return Err(Corruption::ChecksumMismatch {
            offset: 18,
            lsn: hdr.lsn,
        });
    }
    Ok(())
}

// ── Compression helpers ─────────────────────────────────────────────────

pub fn encode_payload(raw: &[u8], compress: bool, min_bytes: usize) -> (Vec<u8>, u8) {
    if compress && raw.len() > min_bytes {
        let c = lz4_flex::compress_prepend_size(raw);
        (c, FLAG_COMPRESSED)
    } else {
        (raw.to_vec(), 0x00)
    }
}

pub fn decode_payload(data: &[u8], flags: u8, max_size: usize) -> Result<Vec<u8>, String> {
    if flags & FLAG_COMPRESSED == 0 {
        if data.len() > max_size {
            return Err(format!("payload size {} > max {}", data.len(), max_size));
        }

        return Ok(data.to_vec());
    }

    if data.len() < 4 {
        return Err("compressed payload too short".to_string());
    }

    let mut buf = [0u8; 4];
    buf.copy_from_slice(&data[..4]);

    let declared = u32::from_le_bytes(buf) as usize;

    if declared > max_size {
        return Err(format!("decompressed size {} > max {}", declared, max_size));
    }

    let decoded =
        lz4_flex::decompress_size_prepended(data).map_err(|e| format!("LZ4 error: {e}"))?;

    if decoded.len() != declared {
        return Err(format!(
            "decompressed size mismatch: declared {}, actual {}",
            declared,
            decoded.len()
        ));
    }

    if decoded.len() > max_size {
        return Err(format!(
            "decompressed size {} > max {}",
            decoded.len(),
            max_size
        ));
    }

    Ok(decoded)
}
