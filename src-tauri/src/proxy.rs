// Local HTTP proxy for streams and their status pages. The webview enforces CORS, unlike the
// Electron window that ran with webSecurity off: the visualizer needs CORS headers on the audio,
// and Icecast's status-json.xsl rarely sends them. The proxy adds them, and the random token in
// the path keeps other local programs from using it.

use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hasher};
use std::io::{self, BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::thread;
use std::time::Duration;

pub fn start() -> io::Result<String> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let port = listener.local_addr()?.port();
    let prefix = format!("/{:016x}", RandomState::new().build_hasher().finish());
    let base = format!("http://127.0.0.1:{port}{prefix}");
    thread::spawn(move || {
        for client in listener.incoming().flatten() {
            let prefix = prefix.clone();
            thread::spawn(move || {
                let _ = serve(client, &prefix);
            });
        }
    });
    Ok(base)
}

fn serve(mut client: TcpStream, prefix: &str) -> io::Result<()> {
    let mut reader = BufReader::new(client.try_clone()?);
    let mut request = String::new();
    reader.read_line(&mut request)?;
    loop {
        let mut header = String::new();
        if reader.read_line(&mut header)? <= 2 {
            break;
        }
    }

    let mut parts = request.split_whitespace();
    let method = parts.next().unwrap_or("");
    let url = parts
        .next()
        .and_then(|target| target.strip_prefix(prefix))
        .and_then(|query| query.strip_prefix("?url="))
        .and_then(decode)
        .filter(|url| url.starts_with("http://") || url.starts_with("https://"));
    let Some(url) = url else {
        return client.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
    };

    let agent = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(10))
        .timeout_read(Duration::from_secs(30))
        .user_agent(concat!("Qust/", env!("CARGO_PKG_VERSION")))
        .build();
    match agent.get(&url).call() {
        Ok(response) => {
            let content_type = response.header("Content-Type").unwrap_or("application/octet-stream").to_string();
            write!(
                client,
                "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nAccess-Control-Allow-Origin: *\r\nCache-Control: no-cache\r\nConnection: close\r\n\r\n"
            )?;
            if method != "HEAD" {
                io::copy(&mut response.into_reader(), &mut client)?;
            }
            Ok(())
        }
        Err(_) => client.write_all(
            b"HTTP/1.1 502 Bad Gateway\r\nAccess-Control-Allow-Origin: *\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        ),
    }
}

fn decode(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            out.push(u8::from_str_radix(text.get(i + 1..i + 3)?, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}
