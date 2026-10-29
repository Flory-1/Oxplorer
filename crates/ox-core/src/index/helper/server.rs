//! The service side of the protocol: answers requests on one connection.
//! What is read from the volumes is behind [`Volumes`], so the request
//! handling is tested here with a fake.

use std::io::{self, Read, Write};

use super::proto::{
    decode_request, encode_response, enum_chunks, read_frame, write_frame, EnumRecord, ErrorCode,
    FrameError, JournalInfo, JournalRecord, Request, Response, MAX_REQUEST,
};

/// The volumes the service reads. Only ever called with a drive letter from
/// [`Volumes::served`].
pub trait Volumes {
    /// The drive letters that may be read (fixed NTFS volumes), ascending.
    fn served(&self) -> Vec<u8>;
    /// Every file record of the volume, handed to `sink` in pieces. Returns
    /// how many records were handed over; stops when `sink` fails.
    fn enumerate(
        &self,
        drive: u8,
        sink: &mut dyn FnMut(Vec<EnumRecord>) -> io::Result<()>,
    ) -> Result<u64, ErrorCode>;
    fn query_journal(&self, drive: u8) -> Result<JournalInfo, ErrorCode>;
    /// Journal records from `start` on, and where to continue. `JournalGone`
    /// when `journal` isn't the current journal or `start` was overwritten.
    fn read_journal(
        &self,
        drive: u8,
        journal: u64,
        start: i64,
    ) -> Result<(i64, Vec<JournalRecord>), ErrorCode>;
}

fn send(conn: &mut impl Write, resp: &Response) -> io::Result<()> {
    let (kind, payload) = encode_response(resp);
    write_frame(conn, kind, &payload)
}

/// Answer requests until the client hangs up or sends something malformed
/// (then the connection is closed after an error reply).
pub fn serve(conn: &mut (impl Read + Write), vols: &dyn Volumes) -> io::Result<()> {
    loop {
        let frame = match read_frame(conn, MAX_REQUEST) {
            Ok(f) => f,
            // Hung up, or not speaking the protocol at all.
            Err(FrameError::Io(_) | FrameError::BadMagic) => return Ok(()),
            Err(FrameError::Version(_)) => {
                return send(conn, &Response::Error(ErrorCode::Version));
            }
            Err(FrameError::TooLong(_)) => {
                return send(conn, &Response::Error(ErrorCode::BadRequest));
            }
        };
        let Some(req) = decode_request(frame.kind, &frame.payload) else {
            return send(conn, &Response::Error(ErrorCode::BadRequest));
        };
        if let Some(drive) = req.drive() {
            if !vols.served().contains(&drive) {
                send(conn, &Response::Error(ErrorCode::NotServed))?;
                continue;
            }
        }
        match req {
            Request::ListVolumes => send(conn, &Response::Volumes(vols.served()))?,
            Request::Enumerate { drive } => {
                let mut failed = None;
                let res = vols.enumerate(drive, &mut |records| {
                    for chunk in enum_chunks(records) {
                        if let Err(e) = send(conn, &Response::EnumChunk(chunk)) {
                            let kind = e.kind();
                            failed = Some(e);
                            return Err(kind.into());
                        }
                    }
                    Ok(())
                });
                if let Some(e) = failed {
                    return Err(e);
                }
                match res {
                    Ok(total) => send(conn, &Response::EnumDone { total })?,
                    Err(code) => send(conn, &Response::Error(code))?,
                }
            }
            Request::QueryJournal { drive } => match vols.query_journal(drive) {
                Ok(j) => send(conn, &Response::Journal(j))?,
                Err(code) => send(conn, &Response::Error(code))?,
            },
            Request::ReadJournal {
                drive,
                journal,
                start,
            } => match vols.read_journal(drive, journal, start) {
                Ok((next, records)) => send(conn, &Response::JournalChunk { next, records })?,
                Err(code) => send(conn, &Response::Error(code))?,
            },
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::super::proto::{decode_response, encode_request, MAX_RESPONSE};
    use super::*;

    /// A fake volume C: with a few records and a short journal.
    pub(crate) struct Fake {
        pub records: Vec<EnumRecord>,
        pub journal: Vec<JournalRecord>,
    }

    pub(crate) const JOURNAL_ID: u64 = 77;

    impl Fake {
        pub fn new() -> Self {
            let rec = |frn, parent, attrs, name: &str| EnumRecord {
                frn,
                parent,
                attrs,
                size: frn * 100,
                mtime: 1_700_000_000 + frn as u32,
                name: name.into(),
            };
            let j = |usn, frn, reason, name: &str| JournalRecord {
                frn,
                parent: 5,
                usn,
                reason,
                attrs: 0x20,
                name: name.into(),
            };
            Fake {
                records: vec![
                    rec(5, 5, 0x10, "."),
                    rec(30, 5, 0x10, "Users"),
                    rec(31, 30, 0, "notes.txt"),
                ],
                journal: vec![
                    j(100, 31, 0x100, "notes.txt"),
                    j(200, 32, 0x200, "old.txt"),
                    j(300, 33, 0x100, "new.txt"),
                ],
            }
        }
    }

    impl Volumes for Fake {
        fn served(&self) -> Vec<u8> {
            vec![b'C']
        }

        fn enumerate(
            &self,
            _drive: u8,
            sink: &mut dyn FnMut(Vec<EnumRecord>) -> io::Result<()>,
        ) -> Result<u64, ErrorCode> {
            for r in &self.records {
                sink(vec![r.clone()]).map_err(|_| ErrorCode::Unreadable)?;
            }
            Ok(self.records.len() as u64)
        }

        fn query_journal(&self, _drive: u8) -> Result<JournalInfo, ErrorCode> {
            Ok(JournalInfo {
                journal: JOURNAL_ID,
                first: 100,
                next: 400,
            })
        }

        fn read_journal(
            &self,
            _drive: u8,
            journal: u64,
            start: i64,
        ) -> Result<(i64, Vec<JournalRecord>), ErrorCode> {
            if journal != JOURNAL_ID || start < 100 {
                return Err(ErrorCode::JournalGone);
            }
            // Two records per read, like a small buffer.
            let records: Vec<_> = self
                .journal
                .iter()
                .filter(|r| r.usn >= start)
                .take(2)
                .cloned()
                .collect();
            let next = records.last().map_or(start, |r| r.usn + 1);
            Ok((next, records))
        }
    }

    /// Run `serve` on `input` and decode everything it answered.
    fn exchange(input: &[u8]) -> Vec<Response> {
        struct Conn<'a> {
            input: &'a [u8],
            output: Vec<u8>,
        }
        impl Read for Conn<'_> {
            fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
                self.input.read(buf)
            }
        }
        impl Write for Conn<'_> {
            fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
                self.output.write(buf)
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        let mut conn = Conn {
            input,
            output: Vec::new(),
        };
        serve(&mut conn, &Fake::new()).unwrap();
        let mut out = &conn.output[..];
        let mut responses = Vec::new();
        while !out.is_empty() {
            let f = read_frame(&mut out, MAX_RESPONSE).unwrap();
            responses.push(decode_response(f.kind, &f.payload).expect("valid response"));
        }
        responses
    }

    fn framed(reqs: &[Request]) -> Vec<u8> {
        let mut buf = Vec::new();
        for r in reqs {
            let (kind, p) = encode_request(r);
            write_frame(&mut buf, kind, &p).unwrap();
        }
        buf
    }

    #[test]
    fn answers_requests_in_order() {
        let input = framed(&[
            Request::ListVolumes,
            Request::QueryJournal { drive: b'C' },
            Request::Enumerate { drive: b'C' },
            Request::ReadJournal {
                drive: b'C',
                journal: JOURNAL_ID,
                start: 150,
            },
            Request::ReadJournal {
                drive: b'C',
                journal: 1,
                start: 150,
            },
        ]);
        let out = exchange(&input);
        assert_eq!(out[0], Response::Volumes(vec![b'C']));
        assert!(matches!(out[1], Response::Journal(j) if j.journal == JOURNAL_ID));
        assert!(matches!(&out[2], Response::EnumChunk(r) if r.len() == 1));
        assert_eq!(out[5], Response::EnumDone { total: 3 });
        assert!(
            matches!(&out[6], Response::JournalChunk { next: 301, records } if records.len() == 2)
        );
        assert_eq!(out[7], Response::Error(ErrorCode::JournalGone));
        assert_eq!(out.len(), 8);
    }

    #[test]
    fn only_serves_listed_drives() {
        let input = framed(&[
            Request::Enumerate { drive: b'D' },
            Request::QueryJournal { drive: b'Z' },
            Request::ListVolumes,
        ]);
        let out = exchange(&input);
        assert_eq!(
            out,
            vec![
                Response::Error(ErrorCode::NotServed),
                Response::Error(ErrorCode::NotServed),
                Response::Volumes(vec![b'C']),
            ]
        );
    }

    #[test]
    fn hangs_up_on_malformed_requests() {
        let follow = framed(&[Request::ListVolumes]);
        let with = |bad: Vec<u8>| {
            let mut input = bad;
            input.extend(&follow);
            exchange(&input)
        };
        // Unknown kind, wrong size, bad drive: an error, then nothing more.
        let mut bad = Vec::new();
        write_frame(&mut bad, 99, b"").unwrap();
        assert_eq!(with(bad), vec![Response::Error(ErrorCode::BadRequest)]);
        let mut bad = Vec::new();
        write_frame(&mut bad, 2, b"C:\\Windows").unwrap();
        assert_eq!(with(bad), vec![Response::Error(ErrorCode::BadRequest)]);
        let mut bad = Vec::new();
        write_frame(&mut bad, 2, b"\\").unwrap();
        assert_eq!(with(bad), vec![Response::Error(ErrorCode::BadRequest)]);
        // Too long: refused without reading it.
        let mut bad = Vec::new();
        write_frame(&mut bad, 2, &[b'C'; MAX_REQUEST + 1]).unwrap();
        assert_eq!(with(bad), vec![Response::Error(ErrorCode::BadRequest)]);
        // Another version.
        let mut bad = follow.clone();
        bad[4] = 2;
        assert_eq!(with(bad), vec![Response::Error(ErrorCode::Version)]);
        // Not the protocol at all, or cut off: just hang up.
        assert!(with(b"GET / HTTP/1.1\r\n\r\n".to_vec()).is_empty());
        assert!(exchange(&follow[..5]).is_empty());
        assert!(exchange(b"").is_empty());
    }

    #[test]
    fn random_input_never_panics() {
        let mut seed = 0x2545_F491_4F6C_DD1Du64;
        let mut rand = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        let base = framed(&[
            Request::ReadJournal {
                drive: b'C',
                journal: JOURNAL_ID,
                start: 100,
            },
            Request::Enumerate { drive: b'C' },
        ]);
        for _ in 0..5000 {
            let mut input = base.clone();
            for _ in 0..(rand() % 4) {
                let i = rand() as usize % input.len();
                input[i] = rand() as u8;
            }
            input.truncate(rand() as usize % (input.len() + 1));
            let _ = exchange(&input);
        }
    }
}
