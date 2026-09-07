//! Fake `sing-box` for hermetic probe tests (std only, no dependencies).
//!
//! Understands just enough of the real CLI surface for `probe.rs`:
//! - `fake-singbox version` prints a parseable version and exits 0.
//! - `fake-singbox run -c <config>` reads the generated batch JSON,
//!   binds every `inbounds[].listen_port` on 127.0.0.1, and answers
//!   `204` to any HTTP request without forwarding anything.
//!
//! The probe only needs local TCP handshakes (readiness wait) plus one
//! plain-HTTP exchange per candidate, so no TLS, SOCKS, or routing is
//! implemented. Anything unexpected fails loudly so tests notice.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::atomic::{AtomicUsize, Ordering};

/// Every 7th accepted connection (including readiness checks) gets a 500,
/// mirroring real feeds where a fraction of configs is dead. The rate is
/// stable across runs; exact placement is intentionally racy like production.
static ACCEPTED: AtomicUsize = AtomicUsize::new(0);

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("version") {
        println!("sing-box version 1.13.13");
        return;
    }
    if args.get(1).map(String::as_str) != Some("run") || args.get(2).map(String::as_str) != Some("-c")
    {
        eprintln!("fake-singbox: expected `run -c <config>`, got {args:?}");
        std::process::exit(2);
    }
    let path = args.get(3).cloned().unwrap_or_default();
    let text = std::fs::read_to_string(&path).unwrap_or_default();

    let mut ports = Vec::new();
    let mut rest = text.as_str();
    loop {
        let Some(idx) = rest.find("\"listen_port\"") else {
            break;
        };
        rest = &rest[idx + 13..];
        let Some(digit_at) = rest.find(|c: char| c.is_ascii_digit()) else {
            break;
        };
        let run: String = rest[digit_at..]
            .chars()
            .take_while(|c| c.is_ascii_digit())
            .collect();
        if run.is_empty() {
            break;
        }
        if let Ok(port) = run.parse::<u16>() {
            ports.push(port);
        }
        rest = &rest[digit_at + run.len()..];
    }

    if ports.is_empty() {
        eprintln!("fake-singbox: no listen ports in {path}");
        std::process::exit(1);
    }

    let mut listeners = Vec::with_capacity(ports.len());
    for port in &ports {
        // Retry briefly: the harness reserves ports and releases them just
        // before spawning us, so a lingering TIME_WAIT rebinding can fail
        // spuriously on boxes with small dynamic ranges. The probe waits up
        // to its startup timeout, so a short retry here costs nothing.
        let mut bound = None;
        for _ in 0..40 {
            match TcpListener::bind(("127.0.0.1", *port)) {
                Ok(listener) => {
                    bound = Some(listener);
                    break;
                }
                Err(_) => std::thread::sleep(std::time::Duration::from_millis(50)),
            }
        }
        match bound {
            Some(listener) => listeners.push(listener),
            None => {
                eprintln!("fake-singbox: bind {port} failed after retries");
                std::process::exit(1);
            }
        }
    }

    let mut acceptors = Vec::with_capacity(listeners.len());
    for listener in listeners {
        acceptors.push(std::thread::spawn(move || {
            for stream in listener.incoming() {
                let mut stream = match stream {
                    Ok(stream) => stream,
                    Err(_) => continue,
                };
                std::thread::spawn(move || {
                    let flaky = ACCEPTED.fetch_add(1, Ordering::SeqCst) % 7 == 6;
                    let _ = stream
                        .set_read_timeout(Some(std::time::Duration::from_secs(5)));
                    let mut buf = [0u8; 8192];
                    let mut total = 0usize;
                    loop {
                        if total >= buf.len() {
                            break;
                        }
                        match stream.read(&mut buf[total..]) {
                            Ok(0) => break,
                            Ok(n) => {
                                total += n;
                                if total >= 4
                                    && buf[..total]
                                        .windows(4)
                                        .any(|window| window == b"\r\n\r\n")
                                {
                                    break;
                                }
                            }
                            Err(_) => break,
                        }
                    }
                    if flaky {
                        let _ = stream.write_all(
                            b"HTTP/1.1 500 Flaky Sink\r\ncontent-length: 0\r\nconnection: close\r\n\r\n",
                        );
                    } else {
                        let _ = stream.write_all(
                            b"HTTP/1.1 204 No Content\r\ncontent-length: 0\r\nconnection: close\r\n\r\n",
                        );
                    }
                    let _ = stream.flush();
                });
            }
        }));
    }
    for handle in acceptors {
        let _ = handle.join();
    }
}
