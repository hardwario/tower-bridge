//! Minimal blocking HTTP/1.1 client for talking to the local API server
//! (used by the MCP stdio bridge and tests). Plain HTTP only.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct HttpResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl HttpResponse {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

/// Largest response body and header line accepted: the answer comes from
/// whatever listens on the port, maybe not the app.
const MAX_BODY: usize = 64 * 1024 * 1024;
const MAX_LINE: u64 = 16 * 1024;
/// Header (or trailer) lines accepted in a response.
const MAX_HEADERS: usize = 100;

fn read_line(reader: &mut impl BufRead, line: &mut String) -> std::io::Result<usize> {
    reader.take(MAX_LINE).read_line(line)
}

fn invalid(msg: &str) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidData, msg.to_string())
}

/// Send one request (`Connection: close`) and read the whole response.
pub fn request(
    addr: SocketAddr,
    method: &str,
    path: &str,
    headers: &[(&str, &str)],
    body: &[u8],
    timeout: Duration,
) -> std::io::Result<HttpResponse> {
    // A line break in a header value would start a header of its own: the
    // MCP stdio bridge copies the client's method and version into headers.
    let unsafe_text = |t: &str| t.bytes().any(|b| matches!(b, b'\r' | b'\n' | 0));
    if unsafe_text(method)
        || unsafe_text(path)
        || headers
            .iter()
            .any(|(k, v)| unsafe_text(k) || unsafe_text(v))
    {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "line break in the request line or a header",
        ));
    }
    let mut stream = TcpStream::connect_timeout(&addr, timeout)?;
    stream.set_read_timeout(Some(timeout))?;
    stream.set_write_timeout(Some(timeout))?;

    let mut head = format!(
        "{method} {path} HTTP/1.1\r\nConnection: close\r\nContent-Length: {}\r\n",
        body.len()
    );
    if !headers.iter().any(|(k, _)| k.eq_ignore_ascii_case("host")) {
        head.push_str(&format!("Host: {addr}\r\n"));
    }
    for (k, v) in headers {
        head.push_str(&format!("{k}: {v}\r\n"));
    }
    head.push_str("\r\n");
    stream.write_all(head.as_bytes())?;
    stream.write_all(body)?;
    stream.flush()?;

    let mut reader = BufReader::new(stream);
    let mut status_line = String::new();
    read_line(&mut reader, &mut status_line)?;
    let status = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse::<u16>().ok())
        .ok_or_else(|| invalid("bad status line"))?;

    let mut headers = Vec::new();
    // Every line counts, with a colon or without.
    let mut lines = 0;
    loop {
        let mut line = String::new();
        if read_line(&mut reader, &mut line)? == 0 {
            break;
        }
        let line = line.trim_end();
        if line.is_empty() {
            break;
        }
        lines += 1;
        if lines > MAX_HEADERS {
            return Err(invalid("too many header lines"));
        }
        if let Some((k, v)) = line.split_once(':') {
            headers.push((k.trim().to_string(), v.trim().to_string()));
        }
    }
    let resp = HttpResponse {
        status,
        headers,
        body: Vec::new(),
    };
    let chunked = resp
        .header("transfer-encoding")
        .is_some_and(|v| v.eq_ignore_ascii_case("chunked"));
    let body = if chunked {
        read_chunked(&mut reader)?
    } else if let Some(len) = resp
        .header("content-length")
        .and_then(|v| v.parse::<usize>().ok())
    {
        if len > MAX_BODY {
            return Err(invalid("response too large"));
        }
        let mut b = vec![0u8; len];
        reader.read_exact(&mut b)?;
        b
    } else {
        let mut b = Vec::new();
        (&mut reader)
            .take(MAX_BODY as u64 + 1)
            .read_to_end(&mut b)?;
        if b.len() > MAX_BODY {
            return Err(invalid("response too large"));
        }
        b
    };
    Ok(HttpResponse { body, ..resp })
}

fn read_chunked(reader: &mut impl BufRead) -> std::io::Result<Vec<u8>> {
    let mut out = Vec::new();
    loop {
        let mut size_line = String::new();
        read_line(reader, &mut size_line)?;
        let size_hex = size_line.trim().split(';').next().unwrap_or("");
        let size = usize::from_str_radix(size_hex, 16).map_err(|_| invalid("bad chunk size"))?;
        if size == 0 {
            // Trailer section ends with an empty line.
            let mut line = String::new();
            let mut lines = 0;
            while read_line(reader, &mut line)? > 0 && !line.trim().is_empty() {
                lines += 1;
                if lines > MAX_HEADERS {
                    return Err(invalid("too many trailer lines"));
                }
                line.clear();
            }
            return Ok(out);
        }
        let start = out.len();
        let end = start
            .checked_add(size)
            .filter(|end| *end <= MAX_BODY)
            .ok_or_else(|| invalid("response too large"))?;
        out.resize(end, 0);
        reader.read_exact(&mut out[start..])?;
        let mut crlf = [0u8; 2];
        reader.read_exact(&mut crlf)?;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_chunked_body() -> std::io::Result<()> {
        let data = b"4\r\nWiki\r\n6;ext=1\r\npedia \r\n0\r\nX-Trailer: 1\r\n\r\n";
        let mut r = BufReader::new(&data[..]);
        assert_eq!(read_chunked(&mut r)?, b"Wikipedia ");
        Ok(())
    }

    #[test]
    fn refuses_line_breaks_in_headers() {
        let addr: SocketAddr = ([127, 0, 0, 1], 9).into();
        let t = Duration::from_millis(100);
        for (method, header) in [
            ("GET\r\nX: 1", "a"),
            ("GET", "tools/list\r\nX-Injected: yes"),
        ] {
            let e = request(addr, method, "/", &[("Mcp-Method", header)], b"", t);
            assert_eq!(
                e.map(|_| ()).map_err(|e| e.kind()),
                Err(std::io::ErrorKind::InvalidInput)
            );
        }
    }

    #[test]
    fn endless_trailers_are_refused() {
        let mut data = b"0\r\n".to_vec();
        for _ in 0..200 {
            data.extend_from_slice(b"X: 1\r\n");
        }
        let mut r = BufReader::new(&data[..]);
        assert!(read_chunked(&mut r).is_err());
    }
}
