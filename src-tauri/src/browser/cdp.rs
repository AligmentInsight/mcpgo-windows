//! Minimal Chrome DevTools Protocol (CDP) discovery client.
//!
//! Chrome exposes its remote-debugging endpoint as a plain-HTTP JSON server
//! on `http://127.0.0.1:<port>` (the port given to
//! `--remote-debugging-port`). McpGo uses two read-only endpoints of that
//! server - the macOS original reaches them through the Playwright/CDP
//! daemon; this port queries them directly:
//!
//! | macOS original                     | Here                            |
//! |------------------------------------|---------------------------------|
//! | Playwright daemon polls CDP        | [`fetch_version`] /             |
//! |                                    | [`fetch_targets`]               |
//! | "is Chrome debuggable?"            | [`endpoint_alive`]              |
//! | known-debug-Chrome registry        | [`scan_debug_endpoints`]        |
//!
//! Why a hand-rolled client instead of an HTTP dependency: the endpoint is
//! loopback-only, plain HTTP (never TLS) and answers one-shot GETs of a few
//! kilobytes. A ~150-line client over `std::net::TcpStream` keeps the GUI
//! process free of an async HTTP stack; the service worker domain stays the
//! only place where heavier process plumbing exists.
//!
//! Wire notes: Chrome may answer with `Content-Length` or with chunked
//! transfer encoding - both are decoded ([`split_http_response`]). The
//! `Connection: close` request header makes Chrome close the socket after
//! the body, so "read to EOF" is a safe body delimiter.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

use serde::Deserialize;

use crate::error::{McpGoError, McpGoResult};

/// Host of every DevTools endpoint Chrome opens. The endpoint binds to the
/// loopback interface only - by design, nothing outside this machine may
/// reach it.
pub const HOST: &str = "127.0.0.1";

/// First port McpGo allocates or scans (`--remote-debugging-port` default
/// convention). Also the port the UI suggests in the launch form.
pub const PORT_RANGE_START: u16 = 9222;

/// Last port of the allocation/scan range. 100 candidates are plenty for a
/// developer workstation (each debug-enabled Chrome occupies exactly one).
pub const PORT_RANGE_END: u16 = 9321;

/// DevTools endpoint: build/protocol information (`/json/version`).
pub const VERSION_PATH: &str = "/json/version";
/// DevTools endpoint: the live list of debuggable targets (`/json/list`).
pub const LIST_PATH: &str = "/json/list";

/// Per-request socket timeout. Generous for loopback; a healthy endpoint
/// answers in a few milliseconds, a wedged one must not stall a supervisor
/// thread for long.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(3);

/// Response body cap: the targets list of even a tab-heavy Chrome stays far
/// below this; the cap protects the reader threads from a misbehaving
/// server that never closes the connection.
const MAX_BODY_BYTES: usize = 1024 * 1024;

/// User agent sent with every probe. Identifies McpGo in the endpoint's
/// access record.
const USER_AGENT: &str = "McpGo-Windows/0.1 (devtools discovery)";

// ----------------------------------------------------------------------------
// HTTP transport
// ----------------------------------------------------------------------------

/// Performs one `GET http://HOST:<port><path>` and returns the decoded body
/// bytes. Errors carry human-readable context (they surface in session
/// records and the log file).
pub fn http_get(port: u16, path: &str) -> Result<Vec<u8>, String> {
    let address = format!("{HOST}:{port}");
    let mut stream = TcpStream::connect(address.as_str())
        .map_err(|e| format!("connect {address}: {e}"))?;
    // A failed timeout configuration is not fatal on loopback sockets; the
    // worst case is the OS default.
    let _ = stream.set_read_timeout(Some(REQUEST_TIMEOUT));
    let _ = stream.set_write_timeout(Some(REQUEST_TIMEOUT));

    let request = format!(
        "GET {path} HTTP/1.1\r\n\
         Host: {HOST}:{port}\r\n\
         Accept: application/json\r\n\
         User-Agent: {USER_AGENT}\r\n\
         Connection: close\r\n\
         \r\n"
    );
    stream
        .write_all(request.as_bytes())
        .map_err(|e| format!("write request: {e}"))?;

    // Read to EOF (Connection: close) with the body cap as the only limit.
    let mut buffer: Vec<u8> = Vec::new();
    let mut chunk = [0u8; 8192];
    loop {
        match stream.read(&mut chunk) {
            Ok(0) => break,
            Ok(read) => {
                buffer.extend_from_slice(&chunk[..read]);
                if buffer.len() > MAX_BODY_BYTES {
                    return Err("response exceeds the 1 MiB body limit".into());
                }
            }
            Err(e) => return Err(format!("read response: {e}")),
        }
    }

    let (_head, body) = split_http_response(&buffer)?;
    Ok(body)
}

/// Splits a raw HTTP response buffer into its header block and its (de
/// chunked, ready-to-parse) body. Extracted from [`http_get`] so the parser
/// is unit-testable without a live endpoint.
fn split_http_response(buffer: &[u8]) -> Result<(String, Vec<u8>), String> {
    let separator: &[u8] = b"\r\n\r\n";
    let header_end = buffer
        .windows(separator.len())
        .position(|window| window == separator)
        .ok_or("malformed http response: no header block")?;

    let head = String::from_utf8_lossy(&buffer[..header_end]).into_owned();
    let raw_body = &buffer[header_end + separator.len()..];

    // Status line: "HTTP/1.1 200 OK" - anything else is an error worth
    // reporting verbatim.
    let status_line = head.lines().next().unwrap_or_default();
    if !status_line.contains(" 200 ") {
        return Err(format!("unexpected http status: {status_line}"));
    }

    // Chunked transfer encoding is decoded; a Content-Length body (or a
    // close-delimited body) is returned as-is - reading to EOF already
    // captured exactly the bytes of the response.
    let chunked = head
        .lines()
        .skip(1)
        .any(|line| match line.split_once(':') {
            Some((name, value)) => {
                name.trim().eq_ignore_ascii_case("transfer-encoding")
                    && value.to_ascii_lowercase().contains("chunked")
            }
            None => false,
        });
    let body = if chunked {
        decode_chunked(raw_body)?
    } else {
        raw_body.to_vec()
    };
    Ok((head, body))
}

/// Decodes a chunked transfer-encoding body (byte-oriented: chunk boundaries
/// may split UTF-8 sequences, which is valid on the wire and fatal for
/// string slicing). Tolerates a missing trailing CRLF after the last data
/// chunk at EOF.
fn decode_chunked(mut body: &[u8]) -> Result<Vec<u8>, String> {
    let mut decoded: Vec<u8> = Vec::new();
    loop {
        let line_end = body
            .windows(2)
            .position(|window| window == b"\r\n")
            .ok_or("chunked body: missing chunk size line")?;
        let size_text = String::from_utf8_lossy(&body[..line_end]);
        // Chunk extensions ("1a;name=value") are not part of the size.
        let size_hex = size_text.split(';').next().unwrap_or_default().trim();
        let size = usize::from_str_radix(size_hex, 16)
            .map_err(|_| format!("chunked body: bad chunk size '{size_text}'"))?;
        body = &body[line_end + 2..];

        if size == 0 {
            return Ok(decoded); // terminating chunk
        }
        if body.len() < size {
            return Err("chunked body: truncated chunk".into());
        }
        decoded.extend_from_slice(&body[..size]);
        body = &body[size..];

        // Skip the CRLF that follows every data chunk; at EOF (a server
        // that omitted the final trailers) the decoded body is complete.
        if body.len() >= 2 {
            body = &body[2..];
        } else {
            return Ok(decoded);
        }
        if decoded.len() > MAX_BODY_BYTES {
            return Err("chunked body exceeds the 1 MiB body limit".into());
        }
    }
}

// ----------------------------------------------------------------------------
// DevTools JSON payloads
// ----------------------------------------------------------------------------

/// `/json/version` payload. Only the `Browser` field (e.g.
/// `"Chrome/133.0.6943.98"`) is consumed - McpGo records it as the session's
/// chrome version. The field name is PascalCase on the wire, so the rename
/// is explicit rather than derived.
#[derive(Debug, Clone, Deserialize)]
pub struct DevToolsVersion {
    /// Full product/version string of the debugged Chrome.
    #[serde(rename = "Browser", default)]
    pub browser: String,
}

/// One debuggable target (tab, service worker, background page) as reported
/// by `/json/list`. This is the wire type shared with the frontend
/// (`src/types/browser.ts`): McpGo surfaces targets so MCP servers can
/// attach their own CDP WebSocket connections.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserTarget {
    /// Target id assigned by Chrome.
    pub target_id: String,
    /// Target kind ("page", "service_worker", "background_page", ...).
    pub kind: String,
    /// Page title (empty for non-document targets).
    pub title: String,
    /// Current URL of the target.
    pub url: String,
    /// CDP WebSocket URL of the target; absent for some target kinds.
    pub web_socket_debugger_url: Option<String>,
}

/// Raw `/json/list` record. Chrome's JSON names the kind field `type` (a
/// reserved word in many languages); it is mapped to `kind` on the Rust
/// side and serialized as `kind` for the frontend.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawTarget {
    #[serde(default)]
    target_id: String,
    #[serde(default, rename = "type")]
    kind: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    url: String,
    #[serde(default)]
    web_socket_debugger_url: Option<String>,
}

/// One live DevTools endpoint found by [`scan_debug_endpoints`].
#[derive(Debug, Clone)]
pub struct DiscoveredEndpoint {
    /// Loopback port the endpoint answers on.
    pub port: u16,
    /// Product/version string reported by `/json/version`.
    pub browser: String,
}

/// Fetches and parses `/json/version` of the endpoint on `port`.
pub fn fetch_version(port: u16) -> Result<DevToolsVersion, String> {
    let body = http_get(port, VERSION_PATH)?;
    let text = String::from_utf8_lossy(&body);
    serde_json::from_str(&text).map_err(|e| format!("parse {VERSION_PATH}: {e}"))
}

/// Fetches and parses `/json/list` of the endpoint on `port`, ordered
/// deterministically: document targets ("page") first, then by title and
/// target id (both case-insensitive) as stable tie-breakers.
pub fn fetch_targets(port: u16) -> Result<Vec<BrowserTarget>, String> {
    let body = http_get(port, LIST_PATH)?;
    let text = String::from_utf8_lossy(&body);
    let raw: Vec<RawTarget> =
        serde_json::from_str(&text).map_err(|e| format!("parse {LIST_PATH}: {e}"))?;
    let mut targets: Vec<BrowserTarget> = raw
        .into_iter()
        .map(|record| BrowserTarget {
            target_id: record.target_id,
            kind: record.kind,
            title: record.title,
            url: record.url,
            web_socket_debugger_url: record.web_socket_debugger_url,
        })
        .collect();
    targets.sort_by(|a, b| {
        let page_first = is_page(b).cmp(&is_page(a));
        page_first
            .then_with(|| a.title.to_lowercase().cmp(&b.title.to_lowercase()))
            .then_with(|| a.target_id.cmp(&b.target_id))
    });
    Ok(targets)
}

/// `true` when the target is a document target ("page" or the new
/// "tab" alias some builds report).
fn is_page(target: &BrowserTarget) -> bool {
    target.kind == "page" || target.kind == "tab"
}

/// `true` when a live DevTools endpoint answers on `port`.
pub fn endpoint_alive(port: u16) -> bool {
    fetch_version(port).is_ok()
}

/// Scans the whole McpGo port range for live DevTools endpoints. This is the
/// attach flow of the macOS original: Chromes that were started with
/// `--remote-debugging-port` (by McpGo, by a script, or by hand) become
/// discoverable sessions the UI can list and read targets from.
pub fn scan_debug_endpoints() -> Vec<DiscoveredEndpoint> {
    let mut found: Vec<DiscoveredEndpoint> = Vec::new();
    for port in PORT_RANGE_START..=PORT_RANGE_END {
        if let Ok(version) = fetch_version(port) {
            log::info!(
                "[browser] discovered DevTools endpoint on port {port} ({})",
                version.browser
            );
            found.push(DiscoveredEndpoint {
                port,
                browser: version.browser,
            });
        }
    }
    found
}

/// Reads one endpoint's version into an [`McpGoError`]-typed result for the
/// command layer (the session id is appended by the caller's context).
pub fn try_fetch_version(port: u16) -> McpGoResult<DevToolsVersion> {
    fetch_version(port)
        .map_err(|detail| McpGoError::operation("querying the DevTools endpoint", &detail))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn response(status: &str, headers: &[&str], body: &[u8]) -> Vec<u8> {
        let mut buffer = Vec::new();
        buffer.extend_from_slice(status.as_bytes());
        buffer.extend_from_slice(b"\r\n");
        for header in headers {
            buffer.extend_from_slice(header.as_bytes());
            buffer.extend_from_slice(b"\r\n");
        }
        buffer.extend_from_slice(b"\r\n");
        buffer.extend_from_slice(body);
        buffer
    }

    #[test]
    fn splits_a_content_length_response() {
        let buffer = response(
            "HTTP/1.1 200 OK",
            &["Content-Length: 2", "Content-Type: application/json"],
            b"{}",
        );
        let (head, body) = split_http_response(&buffer).expect("parse");
        assert!(head.starts_with("HTTP/1.1 200"));
        assert_eq!(body, b"{}");
    }

    #[test]
    fn decodes_a_chunked_response() {
        // Two chunks carrying a JSON payload, plus the terminating zero
        // chunk. The decoder must be byte-safe at any split point.
        let payload = "{\"browser\": \"Chrome/133\"}".as_bytes().to_vec();
        let split = 14;
        let mut body = Vec::new();
        body.extend_from_slice(format!("{:X}\r\n", split).as_bytes());
        body.extend_from_slice(&payload[..split]);
        body.extend_from_slice(b"\r\n");
        body.extend_from_slice(format!("{:X}\r\n", payload.len() - split).as_bytes());
        body.extend_from_slice(&payload[split..]);
        body.extend_from_slice(b"\r\n0\r\n\r\n");

        let buffer = response("HTTP/1.1 200 OK", &["Transfer-Encoding: chunked"], &body);
        let (_head, decoded) = split_http_response(&buffer).expect("parse");
        assert_eq!(decoded, payload);
        let parsed: DevToolsVersion = serde_json::from_slice(&decoded).expect("json");
        assert_eq!(parsed.browser, "Chrome/133");
    }

    #[test]
    fn rejects_error_status_lines() {
        let buffer = response("HTTP/1.1 404 Not Found", &[], b"nope");
        assert!(split_http_response(&buffer).is_err());
    }

    #[test]
    fn decodes_chunked_bodies_directly() {
        let body = b"4\r\nWiki\r\n5\r\npedia\r\n0\r\n\r\n";
        assert_eq!(decode_chunked(body).expect("decode"), b"Wikipedia");
        // Missing final CRLF after the terminating chunk is tolerated.
        assert_eq!(decode_chunked(b"2\r\nok\r\n0\r\n").expect("decode"), b"ok");
    }

    #[test]
    fn parses_a_targets_payload() {
        let raw = r#"[
            {
                "targetId": "abc",
                "type": "service_worker",
                "title": "",
                "url": "https://example.com/sw.js",
                "webSocketDebuggerUrl": "ws://127.0.0.1:9222/devtools/page/abc"
            },
            {
                "targetId": "def",
                "type": "page",
                "title": "Example Domain",
                "url": "https://example.com/"
            }
        ]"#;
        let targets: Vec<RawTarget> = serde_json::from_str(raw).expect("json");
        assert_eq!(targets.len(), 2);
        assert_eq!(targets[0].kind, "service_worker");
        // The page record has no webSocketDebuggerUrl: Option defaults to None.
        assert!(targets[1].web_socket_debugger_url.is_none());
    }

    #[test]
    fn orders_pages_first() {
        let page = BrowserTarget {
            target_id: "1".into(),
            kind: "page".into(),
            title: "Zed".into(),
            url: "https://zed.example".into(),
            web_socket_debugger_url: Some("ws://x".into()),
        };
        let worker = BrowserTarget {
            target_id: "2".into(),
            kind: "service_worker".into(),
            title: "alpha".into(),
            url: "https://a.example/sw.js".into(),
            web_socket_debugger_url: None,
        };
        let mut sorted = vec![worker, page];
        sorted.sort_by(|a, b| {
            let page_first = is_page(b).cmp(&is_page(a));
            page_first
                .then_with(|| a.title.to_lowercase().cmp(&b.title.to_lowercase()))
                .then_with(|| a.target_id.cmp(&b.target_id))
        });
        assert_eq!(sorted.first().map(|t| t.kind.as_str()), Some("page"));
    }
}
