//! Messages on the `\\.\pipe\RunAdminGate` named pipe.
//!
//! Any interactive user can write to the pipe of a SYSTEM service, so decoding
//! is strict: fixed version byte, bounded lengths, valid UTF-8 and no
//! trailing bytes. The service learns *who* is asking from the pipe itself
//! (`ImpersonateNamedPipeClient`), never from message contents.

use crate::LogonMode;

pub const PIPE_NAME: &str = r"\\.\pipe\RunAdminGate";
pub const VERSION: u8 = 1;
/// Largest message either side accepts.
pub const MAX_MESSAGE: usize = 64 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchRequest {
    /// Program as the user named it; the service resolves and checks it.
    pub program: String,
    /// Raw argument string passed after the program.
    pub args: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Refusal {
    NotListed = 1,
    NotAbsolute = 2,
    UnsupportedType = 3,
    UnsafeArguments = 4,
    AdminModeUnavailable = 5,
    NotInteractive = 6,
    BadRequest = 7,
}

impl Refusal {
    fn from_u8(v: u8) -> Option<Refusal> {
        Some(match v {
            1 => Refusal::NotListed,
            2 => Refusal::NotAbsolute,
            3 => Refusal::UnsupportedType,
            4 => Refusal::UnsafeArguments,
            5 => Refusal::AdminModeUnavailable,
            6 => Refusal::NotInteractive,
            7 => Refusal::BadRequest,
            _ => return None,
        })
    }

    /// Text shown to the user.
    pub fn message(self) -> &'static str {
        match self {
            Refusal::NotListed => "This program is not on the RunAdminGate allow-list.",
            Refusal::NotAbsolute => "The program path must be a full path.",
            Refusal::UnsupportedType => "This file type can't be started through RunAdminGate.",
            Refusal::UnsafeArguments => {
                "The arguments contain characters that are not allowed for this file type."
            }
            Refusal::AdminModeUnavailable => "The administrator logon mode is not available yet.",
            Refusal::NotInteractive => {
                "Requests are only accepted from an interactive desktop session."
            }
            Refusal::BadRequest => "The request was malformed.",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Response {
    Started {
        pid: u32,
        mode: LogonMode,
    },
    Refused {
        reason: Refusal,
        detail: String,
    },
    /// A Win32 error while opening or starting the program.
    Failed {
        code: u32,
        detail: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecodeError {
    Truncated,
    TooLong,
    BadVersion,
    BadKind,
    BadUtf8,
    TrailingBytes,
}

struct Writer(Vec<u8>);

impl Writer {
    fn new(kind: u8) -> Writer {
        Writer(vec![VERSION, kind])
    }
    fn u8(&mut self, v: u8) {
        self.0.push(v);
    }
    fn u32(&mut self, v: u32) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn str(&mut self, s: &str) {
        self.u32(s.len() as u32);
        self.0.extend_from_slice(s.as_bytes());
    }
}

struct Reader<'a>(&'a [u8]);

impl<'a> Reader<'a> {
    fn open(buf: &'a [u8]) -> Result<(Reader<'a>, u8), DecodeError> {
        if buf.len() > MAX_MESSAGE {
            return Err(DecodeError::TooLong);
        }
        let mut r = Reader(buf);
        if r.u8()? != VERSION {
            return Err(DecodeError::BadVersion);
        }
        let kind = r.u8()?;
        Ok((r, kind))
    }
    fn take(&mut self, n: usize) -> Result<&'a [u8], DecodeError> {
        if self.0.len() < n {
            return Err(DecodeError::Truncated);
        }
        let (head, tail) = self.0.split_at(n);
        self.0 = tail;
        Ok(head)
    }
    fn u8(&mut self) -> Result<u8, DecodeError> {
        Ok(self.take(1)?[0])
    }
    fn u32(&mut self) -> Result<u32, DecodeError> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn str(&mut self) -> Result<String, DecodeError> {
        let len = self.u32()? as usize;
        if len > MAX_MESSAGE {
            return Err(DecodeError::TooLong);
        }
        let bytes = self.take(len)?;
        if bytes.contains(&0) {
            return Err(DecodeError::BadUtf8);
        }
        String::from_utf8(bytes.to_vec()).map_err(|_| DecodeError::BadUtf8)
    }
    fn finish<T>(self, value: T) -> Result<T, DecodeError> {
        if self.0.is_empty() {
            Ok(value)
        } else {
            Err(DecodeError::TrailingBytes)
        }
    }
}

const KIND_LAUNCH: u8 = 1;
const KIND_STARTED: u8 = 1;
const KIND_REFUSED: u8 = 2;
const KIND_FAILED: u8 = 3;

impl LaunchRequest {
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::new(KIND_LAUNCH);
        w.str(&self.program);
        w.str(&self.args);
        w.0
    }

    pub fn decode(buf: &[u8]) -> Result<LaunchRequest, DecodeError> {
        let (mut r, kind) = Reader::open(buf)?;
        if kind != KIND_LAUNCH {
            return Err(DecodeError::BadKind);
        }
        let program = r.str()?;
        let args = r.str()?;
        r.finish(LaunchRequest { program, args })
    }
}

impl Response {
    pub fn encode(&self) -> Vec<u8> {
        match self {
            Response::Started { pid, mode } => {
                let mut w = Writer::new(KIND_STARTED);
                w.u32(*pid);
                w.u8(match mode {
                    LogonMode::System => 1,
                    LogonMode::Admin => 2,
                });
                w.0
            }
            Response::Refused { reason, detail } => {
                let mut w = Writer::new(KIND_REFUSED);
                w.u8(*reason as u8);
                w.str(detail);
                w.0
            }
            Response::Failed { code, detail } => {
                let mut w = Writer::new(KIND_FAILED);
                w.u32(*code);
                w.str(detail);
                w.0
            }
        }
    }

    pub fn decode(buf: &[u8]) -> Result<Response, DecodeError> {
        let (mut r, kind) = Reader::open(buf)?;
        let value = match kind {
            KIND_STARTED => {
                let pid = r.u32()?;
                let mode = match r.u8()? {
                    1 => LogonMode::System,
                    2 => LogonMode::Admin,
                    _ => return Err(DecodeError::BadKind),
                };
                Response::Started { pid, mode }
            }
            KIND_REFUSED => {
                let reason = Refusal::from_u8(r.u8()?).ok_or(DecodeError::BadKind)?;
                Response::Refused {
                    reason,
                    detail: r.str()?,
                }
            }
            KIND_FAILED => Response::Failed {
                code: r.u32()?,
                detail: r.str()?,
            },
            _ => return Err(DecodeError::BadKind),
        };
        r.finish(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips() {
        let req = LaunchRequest {
            program: r"C:\Program Files\ü\app.exe".into(),
            args: r#"-x "y z""#.into(),
        };
        assert_eq!(LaunchRequest::decode(&req.encode()), Ok(req));
        for resp in [
            Response::Started {
                pid: 4242,
                mode: LogonMode::Admin,
            },
            Response::Refused {
                reason: Refusal::NotListed,
                detail: "C:\\x.exe".into(),
            },
            Response::Failed {
                code: 2,
                detail: "not found".into(),
            },
        ] {
            assert_eq!(Response::decode(&resp.encode()), Ok(resp));
        }
    }

    #[test]
    fn rejects_malformed_input() {
        let good = LaunchRequest {
            program: "a".into(),
            args: String::new(),
        }
        .encode();
        for cut in 0..good.len() {
            assert!(LaunchRequest::decode(&good[..cut]).is_err(), "prefix {cut}");
        }
        let mut trailing = good.clone();
        trailing.push(0);
        assert_eq!(
            LaunchRequest::decode(&trailing),
            Err(DecodeError::TrailingBytes)
        );
        let mut version = good.clone();
        version[0] = 9;
        assert_eq!(
            LaunchRequest::decode(&version),
            Err(DecodeError::BadVersion)
        );
        let huge = [VERSION, 1, 0xff, 0xff, 0xff, 0xff];
        assert_eq!(LaunchRequest::decode(&huge), Err(DecodeError::TooLong));
        let bad_utf8 = [VERSION, 1, 1, 0, 0, 0, 0xff, 0, 0, 0, 0];
        assert_eq!(LaunchRequest::decode(&bad_utf8), Err(DecodeError::BadUtf8));
        let nul = [VERSION, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0];
        assert_eq!(LaunchRequest::decode(&nul), Err(DecodeError::BadUtf8));
        assert_eq!(
            LaunchRequest::decode(&vec![0u8; MAX_MESSAGE + 1]),
            Err(DecodeError::TooLong)
        );
    }
}
