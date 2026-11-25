//! A tiny HTTP/1.1 client with Basic and Digest authentication.
//! Used for camera control, recording search, device info and the alarm feed.

use md5::{Digest, Md5};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

#[derive(Clone, Debug)]
pub struct Target {
    pub host: String,
    pub port: u16,
    pub user: String,
    pub pass: String,
}

fn md5_hex(s: &str) -> String {
    let mut h = Md5::new();
    h.update(s.as_bytes());
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

/// A plain or encrypted (HTTPS) connection.
enum Conn {
    Plain(TcpStream),
    Tls(Box<rustls::StreamOwned<rustls::ClientConnection, TcpStream>>),
}

impl Conn {
    fn tcp(&self) -> &TcpStream {
        match self {
            Conn::Plain(s) => s,
            Conn::Tls(s) => s.get_ref(),
        }
    }
}

impl Read for Conn {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self {
            Conn::Plain(s) => s.read(buf),
            Conn::Tls(s) => s.read(buf),
        }
    }
}

impl Write for Conn {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        match self {
            Conn::Plain(s) => s.write(buf),
            Conn::Tls(s) => s.write(buf),
        }
    }
    fn flush(&mut self) -> std::io::Result<()> {
        match self {
            Conn::Plain(s) => s.flush(),
            Conn::Tls(s) => s.flush(),
        }
    }
}

/// Recorders and cameras use self-made certificates, so (like the browser's
/// "continue anyway") we accept any certificate but still check the signatures.
#[derive(Debug)]
struct AnyCertificate(std::sync::Arc<rustls::crypto::CryptoProvider>);

impl rustls::client::danger::ServerCertVerifier for AnyCertificate {
    fn verify_server_cert(
        &self,
        _end_entity: &rustls::pki_types::CertificateDer<'_>,
        _intermediates: &[rustls::pki_types::CertificateDer<'_>],
        _server_name: &rustls::pki_types::ServerName<'_>,
        _ocsp: &[u8],
        _now: rustls::pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }
    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &rustls::pki_types::CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(message, cert, dss, &self.0.signature_verification_algorithms)
    }
    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &rustls::pki_types::CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(message, cert, dss, &self.0.signature_verification_algorithms)
    }
    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        self.0.signature_verification_algorithms.supported_schemes()
    }
}

fn tls_config() -> std::sync::Arc<rustls::ClientConfig> {
    static CONFIG: std::sync::OnceLock<std::sync::Arc<rustls::ClientConfig>> = std::sync::OnceLock::new();
    CONFIG
        .get_or_init(|| {
            let provider = std::sync::Arc::new(rustls::crypto::ring::default_provider());
            let cfg = rustls::ClientConfig::builder_with_provider(provider.clone())
                .with_safe_default_protocol_versions()
                .expect("TLS versions")
                .dangerous()
                .with_custom_certificate_verifier(std::sync::Arc::new(AnyCertificate(provider)))
                .with_no_client_auth();
            std::sync::Arc::new(cfg)
        })
        .clone()
}

/// Devices that sent us to their HTTPS page: host -> HTTPS port.
fn secure_ports() -> &'static std::sync::Mutex<std::collections::HashMap<String, u16>> {
    static M: std::sync::OnceLock<std::sync::Mutex<std::collections::HashMap<String, u16>>> = std::sync::OnceLock::new();
    M.get_or_init(Default::default)
}

fn open_conn(t: &Target, timeout: Duration) -> Result<Conn, String> {
    let secure = secure_ports().lock().unwrap().get(&t.host).copied();
    match secure {
        None => Ok(Conn::Plain(connect(t, timeout)?)),
        Some(port) => {
            let tt = Target { port, ..t.clone() };
            let tcp = connect(&tt, timeout)?;
            let name = rustls::pki_types::ServerName::try_from(t.host.clone()).map_err(|e| format!("bad host name: {e}"))?;
            let conn = rustls::ClientConnection::new(tls_config(), name).map_err(|e| format!("secure connection: {e}"))?;
            Ok(Conn::Tls(Box::new(rustls::StreamOwned::new(conn, tcp))))
        }
    }
}

fn connect(t: &Target, timeout: Duration) -> Result<TcpStream, String> {
    // Through the device's chosen network connection, when it has one (homes with two routers).
    let s = crate::netpath::connect(&t.host, t.port, timeout)?;
    s.set_read_timeout(Some(timeout)).ok();
    s.set_write_timeout(Some(timeout)).ok();
    Ok(s)
}

struct Head {
    status: u16,
    headers: Vec<(String, String)>,
}

impl Head {
    fn get(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

fn send(t: &Target, path: &str, auth: Option<&str>, timeout: Duration) -> Result<(BufReader<Conn>, Head), String> {
    let mut s = open_conn(t, timeout)?;
    let mut req = format!(
        "GET {path} HTTP/1.1\r\nHost: {}:{}\r\nUser-Agent: MyCCTV/0.1\r\nAccept: */*\r\nConnection: close\r\n",
        t.host, t.port
    );
    if let Some(a) = auth {
        req.push_str("Authorization: ");
        req.push_str(a);
        req.push_str("\r\n");
    }
    req.push_str("\r\n");
    s.write_all(req.as_bytes()).map_err(|e| format!("send failed: {e}"))?;
    let mut r = BufReader::new(s);
    let mut line = String::new();
    r.read_line(&mut line).map_err(|e| format!("no answer: {e}"))?;
    let status = line
        .split_whitespace()
        .nth(1)
        .and_then(|c| c.parse::<u16>().ok())
        .ok_or_else(|| "not an HTTP answer".to_string())?;
    let mut headers = Vec::new();
    loop {
        line.clear();
        r.read_line(&mut line).map_err(|e| format!("bad answer: {e}"))?;
        let l = line.trim_end();
        if l.is_empty() {
            break;
        }
        if let Some((k, v)) = l.split_once(':') {
            headers.push((k.trim().to_string(), v.trim().to_string()));
        }
    }
    Ok((r, Head { status, headers }))
}

fn param(h: &str, key: &str) -> Option<String> {
    // key="value" or key=value
    let lower = h.to_ascii_lowercase();
    let mut search_from = 0;
    while let Some(pos) = lower[search_from..].find(&format!("{key}=")) {
        let p = search_from + pos;
        let ok_start = p == 0 || matches!(lower.as_bytes()[p - 1], b' ' | b',');
        let rest = &h[p + key.len() + 1..];
        if ok_start {
            if let Some(stripped) = rest.strip_prefix('"') {
                return stripped.find('"').map(|e| stripped[..e].to_string());
            }
            let end = rest.find(',').unwrap_or(rest.len());
            return Some(rest[..end].trim().to_string());
        }
        search_from = p + 1;
    }
    None
}

fn auth_header(t: &Target, challenge: &str, path: &str) -> String {
    if challenge.trim_start().to_ascii_lowercase().starts_with("basic") {
        return format!("Basic {}", base64(&format!("{}:{}", t.user, t.pass)));
    }
    let realm = param(challenge, "realm").unwrap_or_default();
    let nonce = param(challenge, "nonce").unwrap_or_default();
    let opaque = param(challenge, "opaque");
    let qop = param(challenge, "qop");
    let ha1 = md5_hex(&format!("{}:{}:{}", t.user, realm, t.pass));
    let ha2 = md5_hex(&format!("GET:{path}"));
    let cnonce = format!("{:08x}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.subsec_nanos()).unwrap_or(7));
    let mut h = format!("Digest username=\"{}\", realm=\"{}\", nonce=\"{}\", uri=\"{}\"", t.user, realm, nonce, path);
    if qop.as_deref().map(|q| q.split(',').any(|x| x.trim() == "auth")).unwrap_or(false) {
        let resp = md5_hex(&format!("{ha1}:{nonce}:00000001:{cnonce}:auth:{ha2}"));
        h.push_str(&format!(", qop=auth, nc=00000001, cnonce=\"{cnonce}\", response=\"{resp}\""));
    } else {
        let resp = md5_hex(&format!("{ha1}:{nonce}:{ha2}"));
        h.push_str(&format!(", response=\"{resp}\""));
    }
    if let Some(o) = opaque {
        h.push_str(&format!(", opaque=\"{o}\""));
    }
    h
}

fn base64(s: &str) -> String {
    const T: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let b = s.as_bytes();
    let mut out = String::new();
    for c in b.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        out.push(T[(n >> 18) as usize & 63] as char);
        out.push(T[(n >> 12) as usize & 63] as char);
        out.push(if c.len() > 1 { T[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if c.len() > 2 { T[n as usize & 63] as char } else { '=' });
    }
    out
}

/// Opens a request, answering an authentication challenge if one comes back.
fn open(t: &Target, path: &str, timeout: Duration) -> Result<(BufReader<Conn>, Head), String> {
    // Whether this request goes out encrypted (decided before sending, so two requests started at
    // the same moment both follow the device's redirect to HTTPS).
    let sent_secure = secure_ports().lock().unwrap().contains_key(&t.host);
    let (r, head) = send(t, path, None, timeout)?;
    // A device that only allows its secure page sends us there: switch to HTTPS and ask again.
    if matches!(head.status, 301 | 302 | 303 | 307 | 308) {
        let to = head.get("Location").unwrap_or("").to_string();
        if to.to_ascii_lowercase().starts_with("https://") && !sent_secure {
            let hostport = to[8..].split('/').next().unwrap_or("");
            let port = hostport.rsplit_once(':').and_then(|(_, p)| p.parse::<u16>().ok()).unwrap_or(443);
            secure_ports().lock().unwrap().insert(t.host.clone(), port);
            drop(r);
            return open(t, path, timeout);
        }
    }
    if head.status != 401 {
        return Ok((r, head));
    }
    let challenge = head.get("WWW-Authenticate").unwrap_or("Digest").to_string();
    drop(r);
    let auth = auth_header(t, &challenge, path);
    let (r, head) = send(t, path, Some(&auth), timeout)?;
    if head.status == 401 {
        return Err("user name or password is wrong".into());
    }
    Ok((r, head))
}

/// GET a page and return (status, body text).
pub fn get(t: &Target, path: &str, timeout: Duration) -> Result<(u16, String), String> {
    let (mut r, head) = open(t, path, timeout)?;
    let mut body = Vec::new();
    let chunked = head.get("Transfer-Encoding").map(|v| v.eq_ignore_ascii_case("chunked")).unwrap_or(false);
    if chunked {
        loop {
            let mut size_line = String::new();
            if r.read_line(&mut size_line).unwrap_or(0) == 0 {
                break;
            }
            let size = usize::from_str_radix(size_line.trim().split(';').next().unwrap_or("0"), 16).unwrap_or(0);
            if size == 0 {
                break;
            }
            let mut chunk = vec![0u8; size];
            if r.read_exact(&mut chunk).is_err() {
                break;
            }
            body.extend_from_slice(&chunk);
            let mut crlf = String::new();
            let _ = r.read_line(&mut crlf);
        }
    } else if let Some(len) = head.get("Content-Length").and_then(|v| v.parse::<usize>().ok()) {
        body.resize(len, 0);
        r.read_exact(&mut body).map_err(|e| format!("short answer: {e}"))?;
    } else {
        let _ = r.read_to_end(&mut body);
    }
    Ok((head.status, String::from_utf8_lossy(&body).into_owned()))
}

/// Removes HTTP chunk framing from a body as it is read.
struct Chunked<R: BufRead> {
    r: R,
    left: usize,
    done: bool,
}

impl<R: BufRead> Read for Chunked<R> {
    fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        if self.done {
            return Ok(0);
        }
        if self.left == 0 {
            let mut line = String::new();
            if self.r.read_line(&mut line)? == 0 {
                self.done = true;
                return Ok(0);
            }
            if line.trim().is_empty() {
                line.clear();
                self.r.read_line(&mut line)?;
            }
            self.left = usize::from_str_radix(line.trim().split(';').next().unwrap_or("0"), 16).unwrap_or(0);
            if self.left == 0 {
                self.done = true;
                return Ok(0);
            }
        }
        let n = out.len().min(self.left);
        let got = self.r.read(&mut out[..n])?;
        self.left -= got;
        Ok(got)
    }
}

/// Opens a (possibly large) download and returns its body as a stream, read as it arrives.
pub fn open_stream(t: &Target, path: &str, timeout: Duration) -> Result<Box<dyn Read + Send>, String> {
    let (r, head) = open(t, path, timeout)?;
    if head.status != 200 {
        return Err(format!("device answered {}", head.status));
    }
    r.get_ref().tcp().set_read_timeout(Some(Duration::from_secs(30))).ok();
    let chunked = head.get("Transfer-Encoding").map(|v| v.eq_ignore_ascii_case("chunked")).unwrap_or(false);
    if chunked {
        Ok(Box::new(Chunked { r, left: 0, done: false }))
    } else if let Some(len) = head.get("Content-Length").and_then(|v| v.parse::<u64>().ok()) {
        Ok(Box::new(r.take(len)))
    } else {
        Ok(Box::new(r))
    }
}

/// Keeps a long-running request open and hands every text line to `on_line`
/// until `stop` is set or the device closes the connection.
pub fn stream_lines(t: &Target, path: &str, stop: &AtomicBool, mut on_line: impl FnMut(&str)) -> Result<(), String> {
    let (mut r, head) = open(t, path, Duration::from_secs(5))?;
    if head.status != 200 {
        return Err(format!("device answered {}", head.status));
    }
    r.get_ref().tcp().set_read_timeout(Some(Duration::from_millis(800))).ok();
    let mut buf: Vec<u8> = Vec::new();
    while !stop.load(Ordering::Relaxed) {
        match r.read_until(b'\n', &mut buf) {
            Ok(0) => return Err("device closed the alarm feed".into()),
            Ok(_) => {
                if buf.ends_with(b"\n") {
                    let line = String::from_utf8_lossy(&buf).trim().to_string();
                    buf.clear();
                    if !line.is_empty() {
                        on_line(&line);
                    }
                }
            }
            Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => continue,
            Err(e) => return Err(format!("alarm feed broke: {e}")),
        }
    }
    Ok(())
}

/// Parses `key=value` lines (the device's usual answer format).
pub fn kv_lines(body: &str) -> Vec<(String, String)> {
    body.lines()
        .filter_map(|l| l.split_once('=').map(|(k, v)| (k.trim().to_string(), v.trim().to_string())))
        .collect()
}

pub fn url_escape(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b':' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}
