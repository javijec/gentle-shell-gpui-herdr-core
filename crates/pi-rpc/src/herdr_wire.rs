//! Line-framed, base64-encoded Pi JSONL over a Herdr PTY.
//!
//! A PTY is not a byte-transparent pipe. In particular, newline rewriting,
//! input echo, terminal control codes and limited canonical-line length make
//! direct Pi JSONL unsuitable. Each direction uses distinct tagged frames,
//! and each frame carries at most 96 unencoded bytes. No raw Pi JSON reaches
//! the PTY; callers reassemble JSONL only after decoding these frames.

use base64::{Engine as _, engine::general_purpose::STANDARD};

pub const PREFIX: &str = "GS4A1|";
pub const MAX_CHUNK: usize = 96;
const MAX_LINE: usize = 4096;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub kind: char,
    pub payload: Vec<u8>,
}

pub fn valid_token(token: &str) -> bool {
    !token.is_empty()
        && token.len() <= 48
        && token.bytes().all(|b| b.is_ascii_alphanumeric())
}

/// One frame per input chunk. An empty payload still produces a frame (READY).
pub fn encode(token: &str, kind: char, bytes: &[u8]) -> Vec<Vec<u8>> {
    assert!(valid_token(token), "invalid bridge token");
    assert!(matches!(kind, 'I' | 'O' | 'E' | 'R' | 'X'));
    let chunks: Vec<&[u8]> = if bytes.is_empty() {
        vec![&[]]
    } else {
        bytes.chunks(MAX_CHUNK).collect()
    };
    chunks
        .into_iter()
        .map(|chunk| {
            format!("{PREFIX}{token}|{kind}|{}\n", STANDARD.encode(chunk)).into_bytes()
        })
        .collect()
}

/// Keeps incomplete CRLF lines between framed stream events. Input echoed by
/// the PTY is ignored by listeners which only accept expected frame kinds.
pub struct Decoder {
    token: String,
    pending: Vec<u8>,
    discarding: bool,
}

impl Decoder {
    pub fn new(token: String) -> Self {
        assert!(valid_token(&token), "invalid bridge token");
        Self {
            token,
            pending: Vec::new(),
            discarding: false,
        }
    }

    pub fn push(&mut self, bytes: &[u8]) -> Vec<Frame> {
        let mut result = Vec::new();
        for &byte in bytes {
            if byte == b'\n' {
                if !self.discarding {
                    if let Some(frame) = self.parse_line() {
                        result.push(frame);
                    }
                }
                self.pending.clear();
                self.discarding = false;
            } else if !self.discarding {
                if self.pending.len() < MAX_LINE {
                    self.pending.push(byte);
                } else {
                    self.pending.clear();
                    self.discarding = true;
                }
            }
        }
        result
    }

    fn parse_line(&self) -> Option<Frame> {
        let line = self.pending.strip_suffix(b"\r").unwrap_or(&self.pending);
        // Some PTYs add leading ANSI control sequences. Only the exact
        // per-process prefix is accepted; noise and echoed IN frames are not.
        let prefix = format!("{PREFIX}{}|", self.token);
        let start = line.windows(prefix.len()).position(|w| w == prefix.as_bytes())?;
        let tail = std::str::from_utf8(&line[start + prefix.len()..]).ok()?;
        let (kind, data) = tail.split_once('|')?;
        let ch = match kind {
            "I" => 'I',
            "O" => 'O',
            "E" => 'E',
            "R" => 'R',
            "X" => 'X',
            _ => return None,
        };
        let payload = STANDARD.decode(data).ok()?;
        (payload.len() <= MAX_CHUNK).then_some(Frame { kind: ch, payload })
    }
}

#[cfg(test)]
mod tests {
    use super::{Decoder, MAX_CHUNK, encode, valid_token};

    #[test]
    fn unicode_binary_and_newlines_round_trip() {
        let bytes = "hola 👋\r\n\0".repeat(40).into_bytes();
        let frames = encode("abc123", 'O', &bytes);
        assert!(frames.len() > 1);
        let mut decoder = Decoder::new("abc123".into());
        let mut actual = Vec::new();
        for byte in frames.concat().chunks(17) {
            for frame in decoder.push(byte) {
                assert_eq!(frame.kind, 'O');
                assert!(frame.payload.len() <= MAX_CHUNK);
                actual.extend(frame.payload);
            }
        }
        assert_eq!(actual, bytes);
    }

    #[test]
    fn ignores_other_processes_echoed_inputs_and_noise() {
        let mut decoder = Decoder::new("own".into());
        let mut data = b"\x1b[31mgarbage\r\n".to_vec();
        data.extend(encode("other", 'O', b"not ours"));
        data.extend(encode("own", 'I', b"echoed"));
        data.extend(encode("own", 'R', b""));
        let frames = decoder.push(&data);
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0].kind, 'I');
        assert_eq!(frames[1].kind, 'R');
    }

    #[test]
    fn rejects_overlong_or_malformed_lines() {
        let mut decoder = Decoder::new("abc".into());
        assert!(decoder.push(&vec![b'x'; 5000]).is_empty());
        assert!(decoder.push(b"\n").is_empty());
        assert!(decoder.push(b"GS4A1|abc|O|**bad**\r\n").is_empty());
        assert!(decoder.push(b"GS4A1|abc|Z|YQ==\n").is_empty());
        assert!(!valid_token("bad|token"));
        assert!(!valid_token(""));
    }
}
