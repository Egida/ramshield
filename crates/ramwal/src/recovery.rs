/// Recovery state machine — authoritative scanner.
use std::io::{ErrorKind, Read, Seek};
use std::path::PathBuf;

use crate::error::{Corruption, Error, TailState};
use crate::lsn::{Lsn, Position};
use crate::record::{HEADER_SIZE, MAX_RECORD_SIZE, RecordHeader, decode_payload, verify_crc};
use crate::segment::{QUARANTINE_DIR, fsync_dir, list_segs, open_segment_read, open_segment_write};

/// Per-segment metadata produced by the scan.
///
/// Deliberately crate-private: retention needs it, consumers do not.
pub(crate) struct SegmentReport {
    pub segment: u64,
    pub last_lsn: Option<Lsn>,
}

/// Outcome of a full recovery scan — what happened, not where to write.
pub struct RecoveryReport {
    pub first_lsn: Option<Lsn>,
    pub last_lsn: Option<Lsn>,
    pub records: Vec<RecoveredRecord>,
    pub segments: usize,

    /// Valid byte offset within the final active segment. This is the
    /// offset a new record may be appended at, not a grand total.
    pub append_offset: u64,

    pub tail: TailState,
    pub repaired: bool,
    pub truncated_bytes: u64,

    pub(crate) segment_reports: Vec<SegmentReport>,
}

/// Recovery result plus the authoritative append boundary.
///
/// The writer is only permitted to append at this position.
pub struct Recovery {
    pub report: RecoveryReport,

    /// Exact physical location at which a new record may be appended.
    pub(crate) append_position: Position,
}

/// One recovered record with decoded payload.
pub struct RecoveredRecord {
    pub lsn: Lsn,
    pub payload: Vec<u8>,
}

/// Scan a single segment. Returns (last_valid_offset, needs_truncate, tail_state).
///
/// Reports only; whether an incomplete tail is legal is decided by the
/// directory-level scanner, not here.
pub(crate) fn scan_segment<R: Read + Seek>(
    file: &mut R,
    segment_id: u64,
    prev_lsn: &mut Option<Lsn>,
    out: &mut Vec<RecoveredRecord>,
) -> Result<(u64, bool, TailState), Error> {
    let mut payload_buf = vec![0u8; MAX_RECORD_SIZE as usize];
    let mut last_valid: u64 = 0;
    let mut tail = TailState::Clean;
    let mut needs_truncate = false;

    loop {
        let mut peek = [0u8; 1];
        let n = file.read(&mut peek).map_err(Error::from_io)?;
        if n == 0 {
            break;
        }

        let mut hdr_buf = [0u8; HEADER_SIZE];
        hdr_buf[0] = peek[0];
        // Only EOF is evidence of an incomplete final write. Recovery may
        // mutate the filesystem only on that evidence — an arbitrary I/O
        // failure is not proof that the record never reached the disk.
        match file.read_exact(&mut hdr_buf[1..]) {
            Ok(()) => {}

            Err(e) if e.kind() == ErrorKind::UnexpectedEof => {
                tail = TailState::PartialHeader { offset: last_valid };
                needs_truncate = true;
                break;
            }

            Err(e) => return Err(Error::from_io(e)),
        }

        let hdr = match RecordHeader::decode(&hdr_buf) {
            Some(Ok(h)) => h,
            Some(Err(c)) => {
                return Err(Error::Corruption {
                    segment: segment_id,
                    offset: last_valid,
                    reason: c,
                });
            }
            None => {
                tail = TailState::PartialHeader { offset: last_valid };
                needs_truncate = true;
                break;
            }
        };

        let plen = hdr.payload_len as usize;

        let mut available = 0usize;
        while available < plen {
            let n = file
                .read(&mut payload_buf[available..plen])
                .map_err(Error::from_io)?;

            if n == 0 {
                break;
            }

            available += n;
        }

        if available != plen {
            tail = TailState::PartialPayload {
                offset: last_valid,
                expected: hdr.payload_len,
                available: available as u32,
            };
            needs_truncate = true;
            break;
        }

        let payload = &payload_buf[..plen];

        // A complete record whose checksum fails is corruption. There is no
        // position-based exception: completeness is not in doubt here.
        if let Err(c) = verify_crc(&hdr, payload) {
            return Err(Error::Corruption {
                segment: segment_id,
                offset: last_valid,
                reason: c,
            });
        }

        if let Some(prev) = *prev_lsn
            && hdr.lsn <= prev
        {
            return Err(Error::Corruption {
                segment: segment_id,
                offset: last_valid,
                reason: Corruption::LsnViolation {
                    previous: prev,
                    current: hdr.lsn,
                },
            });
        }
        *prev_lsn = Some(hdr.lsn);

        let decoded =
            decode_payload(payload, hdr.flags, MAX_RECORD_SIZE as usize).map_err(|reason| {
                Error::Corruption {
                    segment: segment_id,
                    offset: last_valid,
                    reason: Corruption::PayloadDecode {
                        offset: last_valid,
                        lsn: hdr.lsn,
                        reason,
                    },
                }
            })?;

        last_valid = file.stream_position().map_err(Error::from_io)?;
        out.push(RecoveredRecord {
            lsn: hdr.lsn,
            payload: decoded,
        });
    }

    Ok((last_valid, needs_truncate, tail))
}

/// Full recovery: scan all segments, verify integrity, repair final tail.
///
/// Returns a `Recovery` that also carries the authoritative append boundary,
/// so the writer never guesses its starting position.
pub fn recover(dir: &str) -> Result<Recovery, Error> {
    let report = recover_dir(dir)?;
    let segment = report.segment_reports.last().map_or(1, |s| s.segment);
    let offset = report.append_offset;
    Ok(Recovery {
        report,
        append_position: Position::new(segment, offset),
    })
}

/// Scan-and-repair recovery: the mutating entry point.
pub fn recover_dir(dir: &str) -> Result<RecoveryReport, Error> {
    scan_dir(dir, true)
}

/// Verify-only: identical scan, but never mutates. Truncation and
/// quarantine are reported, not applied.
pub fn verify_dir(dir: &str) -> Result<RecoveryReport, Error> {
    scan_dir(dir, false)
}

pub fn scan_dir(dir: &str, repair: bool) -> Result<RecoveryReport, Error> {
    let segs = list_segs(dir).map_err(Error::from_io)?;
    let mut all_records: Vec<RecoveredRecord> = Vec::new();
    let mut prev_lsn: Option<Lsn> = None;
    let mut append_offset: u64 = 0;
    let mut repaired = false;
    let mut final_tail = TailState::Clean;
    let mut truncated_bytes: u64 = 0;
    let mut segment_reports: Vec<SegmentReport> = Vec::new();

    for (i, seg) in segs.iter().enumerate() {
        let is_newest = i == segs.len() - 1;
        let segment_id = seg
            .file_name()
            .and_then(|n| n.to_str())
            .and_then(crate::segment::parse_seg_idx)
            .unwrap_or(0);
        let orig_len = std::fs::metadata(seg).map_err(Error::from_io)?.len();
        let before = all_records.len();
        let mut reader = open_segment_read(seg).map_err(Error::from_io)?;
        let (seg_valid, needs_truncate, tail) =
            scan_segment(&mut reader, segment_id, &mut prev_lsn, &mut all_records)?;
        drop(reader);

        // Only the final segment may end in an incomplete record. A historical
        // segment with a torn tail means damage, and must not be skipped.
        if needs_truncate && !is_newest {
            return Err(Error::Corruption {
                segment: segment_id,
                offset: seg_valid,
                reason: Corruption::NonFinalSegmentTail { offset: seg_valid },
            });
        }

        if needs_truncate {
            if repair {
                if seg_valid > 0 {
                    repair_segment_tail(seg, seg_valid)?;
                } else {
                    let q = PathBuf::from(dir).join(QUARANTINE_DIR);
                    std::fs::create_dir_all(&q).map_err(Error::from_io)?;
                    let dest = q.join(seg.file_name().unwrap_or_default());
                    std::fs::rename(seg, &dest).map_err(Error::from_io)?;
                    fsync_dir(dir)?;
                }
            }
            truncated_bytes += orig_len.saturating_sub(seg_valid);
            repaired = repair;
            append_offset = seg_valid;
            final_tail = tail;
        } else if is_newest {
            append_offset = seg_valid;
            final_tail = tail;
        }

        let slice = &all_records[before..];
        segment_reports.push(SegmentReport {
            segment: segment_id,
            last_lsn: slice.last().map(|r| r.lsn),
        });
    }

    Ok(RecoveryReport {
        first_lsn: all_records.first().map(|r| r.lsn),
        last_lsn: prev_lsn,
        records: all_records,
        segments: segs.len(),
        append_offset,
        tail: final_tail,
        repaired,
        truncated_bytes,
        segment_reports,
    })
}

/// Truncate a segment's torn tail back to `valid_bytes` and sync.
/// This is the repair action: it makes the on-disk tail exactly match
/// the verified prefix so a subsequent scan reads Clean.
pub fn repair_segment_tail(path: &PathBuf, valid_bytes: u64) -> Result<(), Error> {
    let f = open_segment_write(path).map_err(Error::from_io)?;
    f.set_len(valid_bytes).map_err(Error::from_io)?;
    f.sync_all().map_err(Error::from_io)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::record::{HEADER_SIZE, RecordHeader, encode_payload};
    use crate::testkit::FailingReader;
    use std::io::Write;

    fn tmp_path(tag: &str) -> PathBuf {
        let ns = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        std::env::temp_dir().join(format!("ramwal-unit-{tag}-{ns}"))
    }

    /// A non-EOF I/O failure while reading a header continuation must surface
    /// as an I/O error, never as a repairable partial header.
    #[test]
    fn io_error_in_header_is_not_a_tail() {
        let path = tmp_path("ioerr");
        {
            let mut f = std::fs::File::create(&path).unwrap();
            let lsn = Lsn::new(1);
            let (stored, flags) = encode_payload(b"first", false, 64);
            let rh = RecordHeader::new(lsn, &stored, flags);
            let mut hdr = [0u8; HEADER_SIZE];
            rh.encode(&mut hdr);
            f.write_all(&hdr).unwrap();
            f.write_all(&stored).unwrap();
            f.write_all(&hdr[..1]).unwrap(); // one byte of the next header
            f.sync_all().unwrap();
        }

        // Allow the first record, then fail while reading the next header.
        let f = std::fs::File::open(&path).unwrap();
        let ok = (HEADER_SIZE + 5) as u64;
        let mut r = FailingReader::new(f, ok, std::io::ErrorKind::PermissionDenied);

        let mut prev = None;
        let mut out = Vec::new();
        let res = scan_segment(&mut r, 1, &mut prev, &mut out);

        assert!(
            matches!(res, Err(Error::Io(_))),
            "I/O failure must not be classified as a torn tail, got {res:?}"
        );
        assert_eq!(out.len(), 1, "the complete first record must still be read");

        // The file was never repaired: scan_segment does not mutate.
        let len_after = std::fs::metadata(&path).unwrap().len();
        assert_eq!(len_after, ok + 1, "scan must not truncate");
        let _ = std::fs::remove_file(&path);
    }

    /// The same truncation, reached through EOF, *is* a repairable tail.
    #[test]
    fn eof_in_header_is_a_tail() {
        let path = tmp_path("eoftail");
        {
            let mut f = std::fs::File::create(&path).unwrap();
            let (stored, flags) = encode_payload(b"first", false, 64);
            let rh = RecordHeader::new(Lsn::new(1), &stored, flags);
            let mut hdr = [0u8; HEADER_SIZE];
            rh.encode(&mut hdr);
            f.write_all(&hdr).unwrap();
            f.write_all(&stored).unwrap();
            f.write_all(&hdr[..1]).unwrap();
            f.sync_all().unwrap();
        }

        let mut f = std::fs::File::open(&path).unwrap();
        let mut prev = None;
        let mut out = Vec::new();
        let (valid, truncate, tail) = scan_segment(&mut f, 1, &mut prev, &mut out).unwrap();

        assert!(truncate, "EOF must be repairable");
        assert_eq!(valid, (HEADER_SIZE + 5) as u64);
        assert!(matches!(tail, TailState::PartialHeader { .. }));
        let _ = std::fs::remove_file(&path);
    }
}
