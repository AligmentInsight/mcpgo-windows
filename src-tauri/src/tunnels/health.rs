//! Loopback health probe for the tunnel-client runtimes.
//!
//! Every McpGo-supervised `tunnel-client` opens its operator surface on
//! `127.0.0.1:<health port>` serving `/healthz`, `/readyz`, `/metrics` and
//! `/ui` (the documented health endpoints of the client). McpGo probes
//! `/healthz` for liveness:
//!
//! * the start confirmation waits for it before a tunnel counts as
//!   `Running` (the listener binds before the first control-plane poll);
//! * the supervisor keeps probing it while running, so a wedged runtime is
//!   detected even though its process is still alive.
//!
//! Why a hand-rolled client (the same reasoning as `browser/cdp.rs`): the
//! endpoint is loopback-only plain HTTP answering one-shot GETs of a few
//! bytes, so a ~100-line client over `std::net::TcpStream` keeps the GUI
//! process free of an HTTP dependency. Only the status line is consumed -
//! the Go `net/http` server buffers small responses and answers with
//! `Content-Length`, so there is no chunked decoding to do.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::time::Duration;

/// Host of every health listener (loopback only, by design).
pub const HOST: &str = "127.0.0.1";

/// Liveness endpoint of the tunnel-client runtime.
pub const HEALTHZ_PATH: &str = "/healthz";

/// First health port McpGo allocates. Far away from the Chrome DevTools
/// range (9222..=9321) and above the ephemeral-port floor Windows uses by
/// default.
pub const PORT_RANGE_START: u16 = 18080;

/// Last health port McpGo allocates (100 candidates - one per supervised
/// tunnel, plenty for a developer workstation).
pub const PORT_RANGE_END: u16 = 18179;

/// Per-request socket timeout: generous for loopback; a healthy listener
/// answers in a few milliseconds.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(3);

/// Response cap protecting the probe from a misbehaving server (the health
/// body is never parsed anyway).
const MAX_RESPONSE_BYTES: usize = 64 * 1024;

/// User agent sent with every probe (identifies McpGo in access records).
const USER_AGENT: &str = "McpGo-Windows/0.1 (tunnel health probe)";

/// Performs one `GET http://HOST:<port>/healthz` and checks the status
/// line. Only the outcome matters - liveness is "answered 200".
fn probe_healthz(port: u16) -> Result<(), String> {
    let address = format!("{HOST}:{port}");
    let mut stream = TcpStream::connect(address.as_str())
        .map_err(|e| format!("connect {address}: {e}"))?;
    let _ = stream.set_read_timeout(Some(REQUEST_TIMEOUT));
    let _ = stream.set_write_timeout(Some(REQUEST_TIMEOUT));

    let request = format!(
        "GET {HEALTHZ_PATH} HTTP/1.1\r\n\
         Host: {HOST}:{port}\r\n\
         User-Agent: {USER_AGENT}\r\n\
         Connection: close\r\n\
         \r\n"
    );
    stream
        .write_all(request.as_bytes())
        .map_err(|e| format!("write request: {e}"))?;

    // Read to EOF (Connection: close) with the response cap as the only
    // limit; the status line is the first line of the buffer.
    let mut buffer: Vec<u8> = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        match stream.read(&mut chunk) {
            Ok(0) => break,
            Ok(read) => {
                buffer.extend_from_slice(&chunk[..read]);
                if buffer.len() > MAX_RESPONSE_BYTES {
                    return Err("health response exceeds the 64 KiB limit".into());
                }
            }
            Err(e) => return Err(format!("read response: {e}")),
        }
    }

    let head = String::from_utf8_lossy(&buffer);
    let status_line = head.lines().next().unwrap_or_default();
    if status_line.contains(" 200 ") {
        Ok(())
    } else {
        Err(format!("unexpected http status: {status_line}"))
    }
}

/// `true` when a tunnel-client health listener answers on `port`.
pub fn health_alive(port: u16) -> bool {
    probe_healthz(port).is_ok()
}

/// Finds a free health port in the McpGo range: candidates already serving
/// a health listener are skipped (two runtimes can never share one port),
/// then a bind probe rules out foreign listeners. The released-bind race
/// between this probe and the runtime's own bind is closed by the start
/// confirmation: a stolen port means `/healthz` never answers, the start
/// times out and the tunnel records a `Failed` state.
pub fn allocate_health_port() -> Option<u16> {
    for port in PORT_RANGE_START..=PORT_RANGE_END {
        if health_alive(port) {
            continue;
        }
        if TcpListener::bind((HOST, port)).is_ok() {
            return Some(port);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_health_port_range_is_exclusive() {
        // 100 ports, starting where the doc comment says, and never
        // overlapping the DevTools range of the browser domain.
        assert_eq!(PORT_RANGE_END - PORT_RANGE_START + 1, 100);
        assert!(PORT_RANGE_START > 9321);
    }

    #[test]
    fn dead_ports_are_not_alive() {
        // Nothing serves /healthz in the sandbox; a closed port must read
        // as not alive (the connect itself fails).
        assert!(!health_alive(18179));
    }
}
