//! Single-instance detection for the desktop mode: ask the ratiod that already owns the UI port to show its window.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::time::Duration;

const TIMEOUT: Duration = Duration::from_secs(2);

/// Sends `POST /api/desktop/show` to `addr`. True when a desktop ratiod answered `204`.
pub fn notify_running(addr: SocketAddr) -> bool {
    probe(addr).unwrap_or(false)
}

fn probe(addr: SocketAddr) -> std::io::Result<bool> {
    let mut stream = TcpStream::connect_timeout(&addr, TIMEOUT)?;
    stream.set_read_timeout(Some(TIMEOUT))?;
    stream.set_write_timeout(Some(TIMEOUT))?;
    let request =
        format!("POST /api/desktop/show HTTP/1.1\r\nHost: {addr}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
    stream.write_all(request.as_bytes())?;
    let mut response = Vec::new();
    let mut chunk = [0u8; 256];
    // The status line is all we need; stop once it is complete.
    while !response.contains(&b'\n') && response.len() < 1024 {
        match stream.read(&mut chunk)? {
            0 => break,
            n => response.extend_from_slice(&chunk[..n]),
        }
    }
    let text = String::from_utf8_lossy(&response);
    let status = text.lines().next().unwrap_or("").split_whitespace().nth(1);
    Ok(status == Some("204"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    /// A one-shot fake server answering `reply` (raw bytes) to the first connection.
    fn fake_server(reply: &'static [u8]) -> (SocketAddr, std::thread::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            let mut buf = [0u8; 1024];
            let n = socket.read(&mut buf).unwrap();
            socket.write_all(reply).unwrap();
            String::from_utf8_lossy(&buf[..n]).into_owned()
        });
        (addr, handle)
    }

    #[test]
    fn true_on_204() {
        let (addr, server) = fake_server(b"HTTP/1.1 204 No Content\r\nconnection: close\r\n\r\n");
        assert!(notify_running(addr));
        let request = server.join().unwrap();
        assert!(request.starts_with("POST /api/desktop/show HTTP/1.1\r\n"), "{request}");
    }

    #[test]
    fn false_on_404() {
        let (addr, server) = fake_server(b"HTTP/1.1 404 Not Found\r\ncontent-length: 0\r\n\r\n");
        assert!(!notify_running(addr));
        server.join().unwrap();
    }

    #[test]
    fn false_when_nothing_listens() {
        let addr = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap();
        assert!(!notify_running(addr));
    }
}
