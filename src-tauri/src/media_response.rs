//! Bounded custom-protocol reads. Tauri's responder accepts a complete byte body,
//! so range clients receive at most 4 MiB per response (RFC 9110 §15.3.7).
use std::io::{self, Read, Seek, SeekFrom};

pub const RANGE_BYTES: u64 = 4 * 1024 * 1024;
pub const FULL_BODY_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Debug, PartialEq, Eq)]
pub struct ReadPlan {
    pub status: u16,
    pub start: u64,
    pub content_length: u64,
    pub content_range: Option<String>,
    pub send_body: bool,
}
impl ReadPlan {
    fn empty(status: u16) -> Self {
        Self {
            status,
            start: 0,
            content_length: 0,
            content_range: None,
            send_body: false,
        }
    }
}

/// HEAD ignores Range. Unknown range units / unvalidated If-Range fall back to
/// a complete response. Multipart requests return one satisfiable range, which
/// is an explicitly permitted subset, without claiming a multipart body.
pub fn plan(method: &str, range: Option<&str>, if_range: bool, len: u64) -> ReadPlan {
    if !matches!(method, "GET" | "HEAD") {
        return ReadPlan::empty(405);
    }
    if method == "HEAD" {
        return ReadPlan {
            content_length: len,
            ..ReadPlan::empty(200)
        };
    }
    if len > 0 && !if_range {
        if let Some(ranges) = range.and_then(|r| r.strip_prefix("bytes=")) {
            let rejected = || ReadPlan {
                content_range: Some(format!("bytes */{len}")),
                ..ReadPlan::empty(416)
            };
            if ranges.split(',').count() > 16 {
                return rejected();
            }
            let mut selected = None;
            for range in ranges.split(',') {
                let Some((first, last)) = range.trim().split_once('-') else {
                    return rejected();
                };
                let decimal = |s: &str| {
                    if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) {
                        None
                    } else {
                        s.parse::<u64>().ok()
                    }
                };
                let candidate = if first.is_empty() {
                    let Some(suffix) = decimal(last) else {
                        return rejected();
                    };
                    (suffix > 0).then_some((len.saturating_sub(suffix), len - 1))
                } else {
                    let Some(start) = decimal(first) else {
                        return rejected();
                    };
                    let end = if last.is_empty() {
                        len - 1
                    } else if let Some(end) = decimal(last) {
                        if end < start {
                            return rejected();
                        }
                        end.min(len - 1)
                    } else {
                        return rejected();
                    };
                    (start < len).then_some((start, end))
                };
                if selected.is_none() {
                    selected = candidate;
                }
            }
            return selected.map_or_else(rejected, |(start, end)| {
                let end = end.min(start.saturating_add(RANGE_BYTES - 1));
                ReadPlan {
                    status: 206,
                    start,
                    content_length: end - start + 1,
                    content_range: Some(format!("bytes {start}-{end}/{len}")),
                    send_body: true,
                }
            });
        }
    }
    // A full response cannot be streamed through UriSchemeResponder. Large
    // non-range clients get a visible playback fallback, never a truncated 200
    // or a fabricated 206. Range-capable players have no file-size limit here.
    if len > FULL_BODY_BYTES {
        return ReadPlan::empty(413);
    }
    ReadPlan {
        status: 200,
        start: 0,
        content_length: len,
        content_range: None,
        send_body: true,
    }
}

pub fn read_body(file: &mut (impl Read + Seek), plan: &ReadPlan) -> io::Result<Vec<u8>> {
    if !plan.send_body {
        return Ok(vec![]);
    }
    if plan.content_length > FULL_BODY_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "media body exceeds limit",
        ));
    }
    file.seek(SeekFrom::Start(plan.start))?;
    let mut body = vec![0; plan.content_length as usize];
    // A concurrently truncated source must fail, rather than return bytes with
    // a false Content-Length / Content-Range.
    file.read_exact(&mut body)?;
    Ok(body)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn byte_ranges_match_actual_bytes_and_headers() {
        for (range, expected) in [
            ("bytes=0-1", b"01".as_slice()),
            ("bytes=7-", b"789".as_slice()),
            ("bytes=-3", b"789".as_slice()),
            ("bytes=-30", b"0123456789".as_slice()),
            ("bytes=8-99", b"89".as_slice()),
            ("bytes=99-100, 4-5", b"45".as_slice()),
        ] {
            let p = plan("GET", Some(range), false, 10);
            assert_eq!(p.status, 206, "{range}");
            let body = read_body(&mut Cursor::new(b"0123456789"), &p).unwrap();
            assert_eq!(body, expected, "{range}");
            assert_eq!(p.content_length, body.len() as u64);
            assert_eq!(
                p.content_range,
                Some(format!(
                    "bytes {}-{}/10",
                    p.start,
                    p.start + p.content_length - 1
                ))
            );
        }
        for range in [
            "bytes=10-",
            "bytes=3-1",
            "bytes=-0",
            "bytes=x-2",
            "bytes=",
            "bytes=1-2-3",
            "bytes=1-2,",
            "bytes=18446744073709551616-",
        ] {
            let p = plan("GET", Some(range), false, 10);
            assert_eq!(p.status, 416, "{range}");
            assert_eq!(p.content_range.as_deref(), Some("bytes */10"));
        }
    }

    #[test]
    fn methods_full_responses_empty_files_and_memory_bounds() {
        let head = plan("HEAD", Some("bytes=2-3"), false, u64::MAX);
        assert_eq!(head.status, 200);
        assert_eq!(head.content_length, u64::MAX);
        assert!(!head.send_body);
        assert_eq!(plan("POST", None, false, 10).status, 405);
        for range in [None, Some("items=0-1")] {
            let p = plan("GET", range, false, 10);
            assert_eq!(p.status, 200);
            assert_eq!(p.content_length, 10);
            assert!(p.content_range.is_none());
        }
        assert_eq!(plan("GET", Some("bytes=-1"), false, 0).status, 200);
        assert_eq!(plan("GET", Some("bytes=2-3"), true, 10).status, 200);
        assert_eq!(plan("GET", None, false, FULL_BODY_BYTES + 1).status, 413);
        let p = plan("GET", Some("bytes=0-"), false, u64::MAX);
        assert_eq!(p.content_length, RANGE_BYTES);
        let p = plan("GET", Some("bytes=18446744073709551614-"), false, u64::MAX);
        assert_eq!(p.content_length, 1);
        let p = plan("GET", Some("bytes=-8388608"), false, 100_000_000);
        assert_eq!(p.start, 100_000_000 - 8388608);
        assert_eq!(p.content_length, RANGE_BYTES);
        let p = plan("GET", Some("bytes=0-9"), false, 10);
        assert_eq!(
            read_body(&mut Cursor::new(b"short"), &p)
                .unwrap_err()
                .kind(),
            io::ErrorKind::UnexpectedEof
        );
    }
}
