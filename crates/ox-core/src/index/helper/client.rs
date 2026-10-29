//! The app side of the protocol, over any connection (the named pipe on
//! Windows, an in-memory pipe in the tests). Answers are checked as strictly
//! as requests are on the service side.

use std::io::{self, Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};

use super::proto::{
    decode_response, encode_request, read_frame, write_frame, EnumRecord, ErrorCode, FrameError,
    JournalInfo, JournalRecord, Request, Response, MAX_RESPONSE,
};

/// More records than any volume holds: a confused service.
const MAX_RECORDS: u64 = 500_000_000;

#[derive(Debug)]
pub enum ClientError {
    Io(io::Error),
    /// The service answered with an error.
    Service(ErrorCode),
    /// The answer didn't follow the protocol.
    Protocol,
    Cancelled,
}

impl From<io::Error> for ClientError {
    fn from(e: io::Error) -> Self {
        ClientError::Io(e)
    }
}

pub struct Client<S> {
    conn: S,
}

fn letter(drive: char) -> Result<u8, ClientError> {
    let d = drive.to_ascii_uppercase();
    if d.is_ascii_uppercase() {
        Ok(d as u8)
    } else {
        Err(ClientError::Protocol)
    }
}

impl<S: Read + Write> Client<S> {
    pub fn new(conn: S) -> Self {
        Client { conn }
    }

    fn send(&mut self, req: Request) -> Result<(), ClientError> {
        let (kind, payload) = encode_request(&req);
        write_frame(&mut self.conn, kind, &payload)?;
        Ok(())
    }

    fn receive(&mut self) -> Result<Response, ClientError> {
        let f = match read_frame(&mut self.conn, MAX_RESPONSE) {
            Ok(f) => f,
            Err(FrameError::Io(e)) => return Err(ClientError::Io(e)),
            Err(_) => return Err(ClientError::Protocol),
        };
        match decode_response(f.kind, &f.payload) {
            Some(Response::Error(code)) => Err(ClientError::Service(code)),
            Some(r) => Ok(r),
            None => Err(ClientError::Protocol),
        }
    }

    fn call(&mut self, req: Request) -> Result<Response, ClientError> {
        self.send(req)?;
        self.receive()
    }

    /// The drive letters the service reads.
    pub fn volumes(&mut self) -> Result<Vec<char>, ClientError> {
        match self.call(Request::ListVolumes)? {
            Response::Volumes(v) => Ok(v.into_iter().map(char::from).collect()),
            _ => Err(ClientError::Protocol),
        }
    }

    /// Every file record of `drive`, handed to `each`. Setting `cancel`
    /// stops (the connection can't be used afterwards).
    pub fn enumerate(
        &mut self,
        drive: char,
        cancel: &AtomicBool,
        mut each: impl FnMut(EnumRecord),
    ) -> Result<u64, ClientError> {
        self.send(Request::Enumerate {
            drive: letter(drive)?,
        })?;
        let mut count = 0u64;
        loop {
            if cancel.load(Ordering::Relaxed) {
                return Err(ClientError::Cancelled);
            }
            match self.receive()? {
                Response::EnumChunk(records) => {
                    count += records.len() as u64;
                    if count > MAX_RECORDS {
                        return Err(ClientError::Protocol);
                    }
                    records.into_iter().for_each(&mut each);
                }
                Response::EnumDone { total } if total == count => return Ok(count),
                _ => return Err(ClientError::Protocol),
            }
        }
    }

    pub fn query_journal(&mut self, drive: char) -> Result<JournalInfo, ClientError> {
        match self.call(Request::QueryJournal {
            drive: letter(drive)?,
        })? {
            Response::Journal(j) => Ok(j),
            _ => Err(ClientError::Protocol),
        }
    }

    /// Journal records from `start` on, and where to continue.
    pub fn read_journal(
        &mut self,
        drive: char,
        journal: u64,
        start: i64,
    ) -> Result<(i64, Vec<JournalRecord>), ClientError> {
        let req = Request::ReadJournal {
            drive: letter(drive)?,
            journal,
            start,
        };
        match self.call(req)? {
            // The journal only moves forward.
            Response::JournalChunk { next, records } if next >= start => Ok((next, records)),
            _ => Err(ClientError::Protocol),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::{PipeReader, PipeWriter};

    use super::super::server::serve;
    use super::super::server::tests::{Fake, JOURNAL_ID};
    use super::*;

    /// One end of an in-memory connection.
    struct Duplex(PipeReader, PipeWriter);

    impl Read for Duplex {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            self.0.read(buf)
        }
    }

    impl Write for Duplex {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.1.write(buf)
        }
        fn flush(&mut self) -> io::Result<()> {
            self.1.flush()
        }
    }

    fn connected() -> Client<Duplex> {
        let (r1, w1) = std::io::pipe().unwrap();
        let (r2, w2) = std::io::pipe().unwrap();
        std::thread::spawn(move || {
            let mut conn = Duplex(r1, w2);
            let _ = serve(&mut conn, &Fake::new());
        });
        Client::new(Duplex(r2, w1))
    }

    #[test]
    fn talks_to_the_service() {
        let mut c = connected();
        assert_eq!(c.volumes().unwrap(), vec!['C']);
        let mut names = Vec::new();
        let n = c
            .enumerate('c', &AtomicBool::new(false), |r| names.push(r.name))
            .unwrap();
        assert_eq!(n, 3);
        assert_eq!(names, vec![".", "Users", "notes.txt"]);
        let j = c.query_journal('C').unwrap();
        assert_eq!((j.journal, j.first, j.next), (JOURNAL_ID, 100, 400));
        let (next, records) = c.read_journal('C', JOURNAL_ID, 100).unwrap();
        assert_eq!((next, records.len()), (201, 2));
        let (next, records) = c.read_journal('C', JOURNAL_ID, next).unwrap();
        assert_eq!((next, records.len()), (301, 1));
        assert!(matches!(
            c.read_journal('C', JOURNAL_ID + 1, 100),
            Err(ClientError::Service(ErrorCode::JournalGone))
        ));
        assert!(matches!(
            c.enumerate('D', &AtomicBool::new(false), |_| {}),
            Err(ClientError::Service(ErrorCode::NotServed))
        ));
        // Still usable after an error answer.
        assert_eq!(c.volumes().unwrap(), vec!['C']);
        assert!(matches!(c.query_journal('1'), Err(ClientError::Protocol)));
    }

    #[test]
    fn rejects_answers_that_dont_fit() {
        // A "service" that answers every request with the same canned frame.
        fn canned(resp: Response) -> Client<Duplex> {
            let (r1, w1) = std::io::pipe().unwrap();
            let (r2, mut w2) = std::io::pipe().unwrap();
            std::thread::spawn(move || {
                let mut r1 = r1;
                let (kind, p) = super::super::proto::encode_response(&resp);
                while read_frame(&mut r1, 64).is_ok() {
                    if write_frame(&mut w2, kind, &p).is_err() {
                        break;
                    }
                }
            });
            Client::new(Duplex(r2, w1))
        }
        let mut c = canned(Response::EnumDone { total: 5 });
        assert!(matches!(c.volumes(), Err(ClientError::Protocol)));
        assert!(matches!(
            c.enumerate('C', &AtomicBool::new(false), |_| {}),
            Err(ClientError::Protocol)
        ));
        let mut c = canned(Response::JournalChunk {
            next: 10,
            records: Vec::new(),
        });
        assert!(matches!(
            c.read_journal('C', 1, 50),
            Err(ClientError::Protocol)
        ));
        assert!(c.read_journal('C', 1, 5).is_ok());
    }
}
