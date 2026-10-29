//! The index service's wire format: small, fixed, versioned binary frames.
//!
//! Every message is one frame: a 12-byte header (magic `OXIS`, version,
//! kind, payload length, all little-endian) and the payload. Requests have
//! fixed sizes of at most [`MAX_REQUEST`] bytes; responses at most
//! [`MAX_RESPONSE`]. Decoding is bounds-checked throughout: a frame that is
//! too long, has an unknown kind, the wrong size, a drive that isn't a
//! letter, a name that isn't valid UTF-8 or contains a path separator, or
//! bytes left over, is rejected as a whole.

use std::io::{self, Read, Write};

pub const MAGIC: [u8; 4] = *b"OXIS";
/// Bump on any change to the format; both sides refuse other versions.
pub const VERSION: u16 = 1;
pub const HEADER_LEN: usize = 12;
/// The longest request payload (`ReadJournal`: 17 bytes).
pub const MAX_REQUEST: usize = 32;
/// The longest response payload.
pub const MAX_RESPONSE: usize = 4 << 20;
/// The longest name in bytes (NTFS names have at most 255 UTF-16 units).
pub const MAX_NAME: usize = 1024;
/// Bytes of a record without its name (both record types).
const RECORD_FIXED: usize = 34;

const REQ_LIST_VOLUMES: u16 = 1;
const REQ_ENUMERATE: u16 = 2;
const REQ_QUERY_JOURNAL: u16 = 3;
const REQ_READ_JOURNAL: u16 = 4;
const RESP_VOLUMES: u16 = 0x81;
const RESP_ENUM_CHUNK: u16 = 0x82;
const RESP_ENUM_DONE: u16 = 0x83;
const RESP_JOURNAL: u16 = 0x84;
const RESP_JOURNAL_CHUNK: u16 = 0x85;
const RESP_ERROR: u16 = 0xFF;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Request {
    /// The drive letters the service reads (fixed NTFS volumes).
    ListVolumes,
    /// Every file record of the volume: answered by `EnumChunk`s and an `EnumDone`.
    Enumerate {
        drive: u8,
    },
    QueryJournal {
        drive: u8,
    },
    /// Change journal records from USN `start` on (one chunk).
    ReadJournal {
        drive: u8,
        journal: u64,
        start: i64,
    },
}

impl Request {
    /// The drive the request is about (an uppercase ASCII letter).
    pub fn drive(&self) -> Option<u8> {
        match *self {
            Request::ListVolumes => None,
            Request::Enumerate { drive }
            | Request::QueryJournal { drive }
            | Request::ReadJournal { drive, .. } => Some(drive),
        }
    }
}

/// One file or folder from the master file table: only what the index keeps.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnumRecord {
    pub frn: u64,
    pub parent: u64,
    pub attrs: u32,
    pub size: u64,
    /// Last write time, seconds since 1970 (0 = unknown).
    pub mtime: u32,
    pub name: String,
}

/// One change journal record (no contents, only what changed where).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JournalRecord {
    pub frn: u64,
    pub parent: u64,
    pub usn: i64,
    pub reason: u32,
    pub attrs: u32,
    pub name: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct JournalInfo {
    pub journal: u64,
    /// The oldest USN still in the journal.
    pub first: i64,
    /// The USN the next change will get.
    pub next: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorCode {
    /// Malformed request (the service hangs up after sending this).
    BadRequest = 1,
    /// Other protocol version (the service hangs up after sending this).
    Version = 2,
    /// Not a fixed NTFS drive the service reads.
    NotServed = 3,
    /// The volume or its master file table couldn't be read.
    Unreadable = 4,
    /// The journal was recreated or the USN is no longer in it.
    JournalGone = 5,
}

impl ErrorCode {
    fn from_u16(v: u16) -> Option<Self> {
        Some(match v {
            1 => ErrorCode::BadRequest,
            2 => ErrorCode::Version,
            3 => ErrorCode::NotServed,
            4 => ErrorCode::Unreadable,
            5 => ErrorCode::JournalGone,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Response {
    /// Drive letters, ascending.
    Volumes(Vec<u8>),
    EnumChunk(Vec<EnumRecord>),
    EnumDone {
        total: u64,
    },
    Journal(JournalInfo),
    JournalChunk {
        /// Where to continue reading.
        next: i64,
        records: Vec<JournalRecord>,
    },
    Error(ErrorCode),
}

#[derive(Debug)]
pub enum FrameError {
    Io(io::Error),
    BadMagic,
    Version(u16),
    TooLong(u32),
}

#[derive(Debug, PartialEq, Eq)]
pub struct Frame {
    pub kind: u16,
    pub payload: Vec<u8>,
}

/// Read one frame whose payload may be at most `max` bytes. Nothing beyond
/// the header is read when the header is bad.
pub fn read_frame(r: &mut impl Read, max: usize) -> Result<Frame, FrameError> {
    let mut h = [0u8; HEADER_LEN];
    r.read_exact(&mut h).map_err(FrameError::Io)?;
    if h[0..4] != MAGIC {
        return Err(FrameError::BadMagic);
    }
    let version = u16::from_le_bytes([h[4], h[5]]);
    if version != VERSION {
        return Err(FrameError::Version(version));
    }
    let kind = u16::from_le_bytes([h[6], h[7]]);
    let len = u32::from_le_bytes([h[8], h[9], h[10], h[11]]);
    if len as usize > max {
        return Err(FrameError::TooLong(len));
    }
    let mut payload = vec![0u8; len as usize];
    r.read_exact(&mut payload).map_err(FrameError::Io)?;
    Ok(Frame { kind, payload })
}

pub fn write_frame(w: &mut impl Write, kind: u16, payload: &[u8]) -> io::Result<()> {
    let len = u32::try_from(payload.len())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "frame too long"))?;
    let mut buf = Vec::with_capacity(HEADER_LEN + payload.len());
    buf.extend_from_slice(&MAGIC);
    buf.extend_from_slice(&VERSION.to_le_bytes());
    buf.extend_from_slice(&kind.to_le_bytes());
    buf.extend_from_slice(&len.to_le_bytes());
    buf.extend_from_slice(payload);
    w.write_all(&buf)?;
    w.flush()
}

/// A bounds-checked reader over a payload.
struct Cur<'a> {
    b: &'a [u8],
    at: usize,
}

impl<'a> Cur<'a> {
    fn new(b: &'a [u8]) -> Self {
        Cur { b, at: 0 }
    }

    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let end = self.at.checked_add(n)?;
        let s = self.b.get(self.at..end)?;
        self.at = end;
        Some(s)
    }

    fn u8(&mut self) -> Option<u8> {
        Some(self.take(1)?[0])
    }

    fn u16(&mut self) -> Option<u16> {
        Some(u16::from_le_bytes(self.take(2)?.try_into().ok()?))
    }

    fn u32(&mut self) -> Option<u32> {
        Some(u32::from_le_bytes(self.take(4)?.try_into().ok()?))
    }

    fn u64(&mut self) -> Option<u64> {
        Some(u64::from_le_bytes(self.take(8)?.try_into().ok()?))
    }

    fn i64(&mut self) -> Option<i64> {
        Some(self.u64()? as i64)
    }

    fn drive(&mut self) -> Option<u8> {
        self.u8().filter(u8::is_ascii_uppercase)
    }

    fn name(&mut self) -> Option<String> {
        let len = self.u16()? as usize;
        if len > MAX_NAME {
            return None;
        }
        let s = std::str::from_utf8(self.take(len)?).ok()?;
        valid_name(s).then(|| s.to_owned())
    }

    /// A record count that the rest of the payload can actually hold.
    fn count(&mut self) -> Option<usize> {
        let n = self.u32()? as usize;
        (n <= (self.b.len() - self.at) / RECORD_FIXED).then_some(n)
    }

    fn end(self) -> Option<()> {
        (self.at == self.b.len()).then_some(())
    }
}

/// A file name as NTFS stores it: not empty, not too long, no separators or NUL.
pub fn valid_name(s: &str) -> bool {
    !s.is_empty() && s.len() <= MAX_NAME && !s.contains(['\0', '/', '\\'])
}

pub fn encode_request(req: &Request) -> (u16, Vec<u8>) {
    match *req {
        Request::ListVolumes => (REQ_LIST_VOLUMES, Vec::new()),
        Request::Enumerate { drive } => (REQ_ENUMERATE, vec![drive]),
        Request::QueryJournal { drive } => (REQ_QUERY_JOURNAL, vec![drive]),
        Request::ReadJournal {
            drive,
            journal,
            start,
        } => {
            let mut p = vec![drive];
            p.extend_from_slice(&journal.to_le_bytes());
            p.extend_from_slice(&start.to_le_bytes());
            (REQ_READ_JOURNAL, p)
        }
    }
}

pub fn decode_request(kind: u16, payload: &[u8]) -> Option<Request> {
    let mut c = Cur::new(payload);
    let req = match kind {
        REQ_LIST_VOLUMES => Request::ListVolumes,
        REQ_ENUMERATE => Request::Enumerate { drive: c.drive()? },
        REQ_QUERY_JOURNAL => Request::QueryJournal { drive: c.drive()? },
        REQ_READ_JOURNAL => Request::ReadJournal {
            drive: c.drive()?,
            journal: c.u64()?,
            start: c.i64()?,
        },
        _ => return None,
    };
    c.end()?;
    Some(req)
}

fn put_name(p: &mut Vec<u8>, name: &str) {
    p.extend_from_slice(&(name.len() as u16).to_le_bytes());
    p.extend_from_slice(name.as_bytes());
}

fn enum_record_len(r: &EnumRecord) -> usize {
    RECORD_FIXED + r.name.len()
}

fn put_enum_record(p: &mut Vec<u8>, r: &EnumRecord) {
    p.extend_from_slice(&r.frn.to_le_bytes());
    p.extend_from_slice(&r.parent.to_le_bytes());
    p.extend_from_slice(&r.attrs.to_le_bytes());
    p.extend_from_slice(&r.size.to_le_bytes());
    p.extend_from_slice(&r.mtime.to_le_bytes());
    put_name(p, &r.name);
}

fn put_journal_record(p: &mut Vec<u8>, r: &JournalRecord) {
    p.extend_from_slice(&r.frn.to_le_bytes());
    p.extend_from_slice(&r.parent.to_le_bytes());
    p.extend_from_slice(&r.usn.to_le_bytes());
    p.extend_from_slice(&r.reason.to_le_bytes());
    p.extend_from_slice(&r.attrs.to_le_bytes());
    put_name(p, &r.name);
}

/// Encode a response. Records with names that wouldn't decode are left out;
/// enumeration chunks must be split with [`enum_chunks`] first.
pub fn encode_response(resp: &Response) -> (u16, Vec<u8>) {
    let mut p = Vec::new();
    let kind = match resp {
        Response::Volumes(v) => {
            p.push(v.len() as u8);
            p.extend_from_slice(v);
            RESP_VOLUMES
        }
        Response::EnumChunk(records) => {
            let ok: Vec<_> = records.iter().filter(|r| valid_name(&r.name)).collect();
            p.extend_from_slice(&(ok.len() as u32).to_le_bytes());
            for r in ok {
                put_enum_record(&mut p, r);
            }
            RESP_ENUM_CHUNK
        }
        Response::EnumDone { total } => {
            p.extend_from_slice(&total.to_le_bytes());
            RESP_ENUM_DONE
        }
        Response::Journal(j) => {
            p.extend_from_slice(&j.journal.to_le_bytes());
            p.extend_from_slice(&j.first.to_le_bytes());
            p.extend_from_slice(&j.next.to_le_bytes());
            RESP_JOURNAL
        }
        Response::JournalChunk { next, records } => {
            p.extend_from_slice(&next.to_le_bytes());
            let ok: Vec<_> = records.iter().filter(|r| valid_name(&r.name)).collect();
            p.extend_from_slice(&(ok.len() as u32).to_le_bytes());
            for r in ok {
                put_journal_record(&mut p, r);
            }
            RESP_JOURNAL_CHUNK
        }
        Response::Error(code) => {
            p.extend_from_slice(&(*code as u16).to_le_bytes());
            RESP_ERROR
        }
    };
    (kind, p)
}

/// Split enumeration records into chunks that each fit in one response
/// (records with invalid names are dropped).
pub fn enum_chunks(records: Vec<EnumRecord>) -> Vec<Vec<EnumRecord>> {
    let mut chunks = Vec::new();
    let mut cur = Vec::new();
    let mut len = 4;
    for r in records.into_iter().filter(|r| valid_name(&r.name)) {
        let n = enum_record_len(&r);
        if len + n > MAX_RESPONSE && !cur.is_empty() {
            chunks.push(std::mem::take(&mut cur));
            len = 4;
        }
        len += n;
        cur.push(r);
    }
    if !cur.is_empty() {
        chunks.push(cur);
    }
    chunks
}

pub fn decode_response(kind: u16, payload: &[u8]) -> Option<Response> {
    let mut c = Cur::new(payload);
    let resp = match kind {
        RESP_VOLUMES => {
            let n = c.u8()? as usize;
            if n > 26 {
                return None;
            }
            let v = c.take(n)?.to_vec();
            // Letters, ascending (so no duplicates).
            let ok = v.iter().all(u8::is_ascii_uppercase) && v.windows(2).all(|w| w[0] < w[1]);
            if !ok {
                return None;
            }
            Response::Volumes(v)
        }
        RESP_ENUM_CHUNK => {
            let n = c.count()?;
            let mut records = Vec::with_capacity(n);
            for _ in 0..n {
                records.push(EnumRecord {
                    frn: c.u64()?,
                    parent: c.u64()?,
                    attrs: c.u32()?,
                    size: c.u64()?,
                    mtime: c.u32()?,
                    name: c.name()?,
                });
            }
            Response::EnumChunk(records)
        }
        RESP_ENUM_DONE => Response::EnumDone { total: c.u64()? },
        RESP_JOURNAL => Response::Journal(JournalInfo {
            journal: c.u64()?,
            first: c.i64()?,
            next: c.i64()?,
        }),
        RESP_JOURNAL_CHUNK => {
            let next = c.i64()?;
            let n = c.count()?;
            let mut records = Vec::with_capacity(n);
            for _ in 0..n {
                records.push(JournalRecord {
                    frn: c.u64()?,
                    parent: c.u64()?,
                    usn: c.i64()?,
                    reason: c.u32()?,
                    attrs: c.u32()?,
                    name: c.name()?,
                });
            }
            Response::JournalChunk { next, records }
        }
        RESP_ERROR => Response::Error(ErrorCode::from_u16(c.u16()?)?),
        _ => return None,
    };
    c.end()?;
    Some(resp)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn er(frn: u64, name: &str) -> EnumRecord {
        EnumRecord {
            frn,
            parent: 5,
            attrs: 0x10,
            size: frn * 3,
            mtime: 1_700_000_000,
            name: name.into(),
        }
    }

    fn jr(usn: i64, name: &str) -> JournalRecord {
        JournalRecord {
            frn: 40,
            parent: 5,
            usn,
            reason: 0x100,
            attrs: 0x20,
            name: name.into(),
        }
    }

    fn samples() -> Vec<Response> {
        vec![
            Response::Volumes(b"CDZ".to_vec()),
            Response::Volumes(Vec::new()),
            Response::EnumChunk(vec![er(1, "a.txt"), er(2, "Résumé 新しい.docx")]),
            Response::EnumChunk(Vec::new()),
            Response::EnumDone { total: 12345 },
            Response::Journal(JournalInfo {
                journal: 0xDEAD_BEEF,
                first: 100,
                next: -1,
            }),
            Response::JournalChunk {
                next: 4096,
                records: vec![jr(1, "x"), jr(2, "y.log")],
            },
            Response::Error(ErrorCode::JournalGone),
        ]
    }

    fn requests() -> Vec<Request> {
        vec![
            Request::ListVolumes,
            Request::Enumerate { drive: b'C' },
            Request::QueryJournal { drive: b'Z' },
            Request::ReadJournal {
                drive: b'D',
                journal: u64::MAX,
                start: i64::MIN,
            },
        ]
    }

    #[test]
    fn requests_and_responses_roundtrip() {
        for req in requests() {
            let (kind, p) = encode_request(&req);
            assert!(p.len() <= MAX_REQUEST);
            assert_eq!(decode_request(kind, &p), Some(req));
        }
        for resp in samples() {
            let (kind, p) = encode_response(&resp);
            assert_eq!(decode_response(kind, &p), Some(resp));
        }
    }

    #[test]
    fn frames_roundtrip_and_reject_bad_headers() {
        let mut buf = Vec::new();
        write_frame(&mut buf, 3, b"C").unwrap();
        write_frame(&mut buf, 1, b"").unwrap();
        let mut r = &buf[..];
        let f = read_frame(&mut r, MAX_REQUEST).unwrap();
        assert_eq!((f.kind, f.payload.as_slice()), (3, &b"C"[..]));
        assert_eq!(read_frame(&mut r, MAX_REQUEST).unwrap().kind, 1);
        assert!(matches!(read_frame(&mut r, 8), Err(FrameError::Io(_))));

        let mut bad = buf.clone();
        bad[0] = b'X';
        assert!(matches!(
            read_frame(&mut &bad[..], 64),
            Err(FrameError::BadMagic)
        ));
        let mut bad = buf.clone();
        bad[4] = 9;
        assert!(matches!(
            read_frame(&mut &bad[..], 64),
            Err(FrameError::Version(9))
        ));
        // A huge length is refused before anything is allocated or read.
        let mut bad = buf.clone();
        bad[8..12].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(matches!(
            read_frame(&mut &bad[..], MAX_RESPONSE),
            Err(FrameError::TooLong(u32::MAX))
        ));
        // Truncated payload.
        assert!(matches!(
            read_frame(&mut &buf[..HEADER_LEN], 64),
            Err(FrameError::Io(_))
        ));
    }

    #[test]
    fn rejects_malformed_requests() {
        assert_eq!(decode_request(REQ_ENUMERATE, b""), None);
        assert_eq!(decode_request(REQ_ENUMERATE, b"CD"), None, "extra bytes");
        assert_eq!(decode_request(REQ_ENUMERATE, b"c"), None, "lowercase");
        for d in [b'\\', b'.', b'0', b':', 0, 0xFF, b'[', b'@'] {
            assert_eq!(decode_request(REQ_QUERY_JOURNAL, &[d]), None);
        }
        assert_eq!(decode_request(REQ_LIST_VOLUMES, b"x"), None);
        assert_eq!(decode_request(REQ_READ_JOURNAL, &[b'C'; 16]), None);
        assert_eq!(decode_request(REQ_READ_JOURNAL, &[b'C'; 18]), None);
        assert_eq!(decode_request(0, b""), None);
        assert_eq!(decode_request(RESP_VOLUMES, b""), None);
        assert_eq!(decode_request(u16::MAX, b"C"), None);
    }

    #[test]
    fn rejects_malformed_responses() {
        // Bad letters, too many, unsorted or duplicated.
        assert_eq!(decode_response(RESP_VOLUMES, &[1, b'c']), None);
        assert_eq!(decode_response(RESP_VOLUMES, &[2, b'D', b'C']), None);
        assert_eq!(decode_response(RESP_VOLUMES, &[2, b'C', b'C']), None);
        assert_eq!(decode_response(RESP_VOLUMES, &[27]), None);
        assert_eq!(decode_response(RESP_VOLUMES, &[2, b'C']), None);
        // Unknown error codes and kinds.
        assert_eq!(decode_response(RESP_ERROR, &[9, 0]), None);
        assert_eq!(decode_response(0x86, &[]), None);
        assert_eq!(decode_response(REQ_ENUMERATE, b"C"), None);

        // A count larger than the payload can hold.
        let mut p = u32::MAX.to_le_bytes().to_vec();
        p.extend([0u8; 40]);
        assert_eq!(decode_response(RESP_ENUM_CHUNK, &p), None);

        let (kind, good) = encode_response(&Response::EnumChunk(vec![er(1, "ab")]));
        // Trailing bytes.
        let mut p = good.clone();
        p.push(0);
        assert_eq!(decode_response(kind, &p), None);
        // A name length past the end, a name with a separator, bad UTF-8, empty.
        let name_at = 4 + RECORD_FIXED - 2;
        let mut p = good.clone();
        p[name_at] = 3;
        assert_eq!(decode_response(kind, &p), None);
        for bad in [b"a\\", b"a/", b"a\0", &[0xC3, 0x28]] {
            let mut p = good.clone();
            p[name_at + 2..].copy_from_slice(bad);
            assert_eq!(decode_response(kind, &p), None, "{bad:?}");
        }
        let mut p = good.clone();
        p.truncate(name_at);
        p.extend([0, 0]);
        assert_eq!(decode_response(kind, &p), None, "empty name");
        let mut p = good;
        p[name_at..name_at + 2].copy_from_slice(&((MAX_NAME + 1) as u16).to_le_bytes());
        p.resize(name_at + 2 + MAX_NAME + 1, b'a');
        assert_eq!(decode_response(kind, &p), None, "name too long");
    }

    /// Fuzz-style: truncations and random byte flips of valid messages must
    /// never panic, and decoders never return more than the payload holds.
    #[test]
    fn decoding_garbage_never_panics() {
        let mut seed = 0x9E37_79B9_7F4A_7C15u64;
        let mut rand = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        let mut inputs: Vec<(u16, Vec<u8>)> = samples().iter().map(encode_response).collect();
        inputs.extend(requests().iter().map(encode_request));
        for (kind, p) in &inputs {
            for cut in 0..p.len() {
                let _ = decode_response(*kind, &p[..cut]);
                let _ = decode_request(*kind, &p[..cut]);
            }
            for _ in 0..2000 {
                let mut q = p.clone();
                if !q.is_empty() {
                    let i = rand() as usize % q.len();
                    q[i] ^= (rand() % 255 + 1) as u8;
                }
                if rand() % 4 == 0 {
                    q.push(rand() as u8);
                }
                let k = if rand() % 8 == 0 {
                    rand() as u16
                } else {
                    *kind
                };
                if let Some(Response::EnumChunk(r)) = decode_response(k, &q) {
                    assert!(r.len() * RECORD_FIXED <= q.len());
                }
                let _ = decode_request(k, &q);
                let mut framed = Vec::new();
                write_frame(&mut framed, k, &q).unwrap();
                if rand() % 2 == 0 {
                    let i = rand() as usize % framed.len();
                    framed[i] = rand() as u8;
                }
                let _ = read_frame(&mut &framed[..], MAX_REQUEST);
            }
        }
        for _ in 0..20_000 {
            let len = (rand() % 64) as usize;
            let junk: Vec<u8> = (0..len).map(|_| rand() as u8).collect();
            let k = (rand() % 6) as u16 | if rand() % 2 == 0 { 0x80 } else { 0 };
            let _ = decode_response(k, &junk);
            let _ = decode_request(k, &junk);
        }
    }

    #[test]
    fn splits_big_enumerations_and_drops_bad_names() {
        let long = "x".repeat(1000);
        let records: Vec<_> = (0..10_000).map(|i| er(i, &long)).collect();
        let chunks = enum_chunks(records);
        assert!(chunks.len() >= 2);
        assert_eq!(chunks.iter().map(Vec::len).sum::<usize>(), 10_000);
        for c in chunks {
            let (kind, p) = encode_response(&Response::EnumChunk(c));
            assert!(p.len() <= MAX_RESPONSE);
            assert!(decode_response(kind, &p).is_some());
        }
        let chunks = enum_chunks(vec![er(1, "ok"), er(2, "a\\b"), er(3, "")]);
        assert_eq!(chunks, vec![vec![er(1, "ok")]]);
        let (kind, p) = encode_response(&Response::JournalChunk {
            next: 1,
            records: vec![jr(1, "a/b"), jr(2, "fine")],
        });
        assert_eq!(
            decode_response(kind, &p),
            Some(Response::JournalChunk {
                next: 1,
                records: vec![jr(2, "fine")]
            })
        );
    }
}
