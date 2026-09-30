//! A small, dependency-free HTTP/1.1 benchmark server.
//!
//! Features needed by the performance tests:
//! - HTTP Range (`bytes=start-end`) with 206 + `Content-Range`
//! - HTTP keep-alive (multiple requests per TCP connection) so connection
//!   reuse can be counted
//! - configurable per-connection or global bandwidth
//! - configurable latency before response headers
//! - optional "server overload" mode: past N concurrent connections the
//!   effective bandwidth collapses, so an adaptive controller can observe a
//!   real throughput regression and back off
//!
//! It is intentionally not a general-purpose HTTP server.

use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

#[derive(Clone)]
pub struct ServerConfig {
    pub size: u64,
    pub supports_range: bool,
    /// Delay before writing response headers.
    pub latency: Duration,
    /// Per-connection cap, bytes/s. 0 = unlimited.
    pub per_conn_bps: u64,
    /// Aggregate cap across all connections, bytes/s. 0 = unlimited.
    pub global_bps: u64,
    /// Keep the TCP connection open for further requests.
    pub keep_alive: bool,
    /// Body write chunk size.
    pub chunk: usize,
    /// Once more than this many connections are open, bandwidth is scaled by
    /// `collapse_num / collapse_den`. 0 disables the overload simulation.
    pub collapse_at_connections: usize,
    pub collapse_num: u64,
    pub collapse_den: u64,
    /// Misbehaving-server mode: reply to a Range request with a 206 whose body
    /// is the whole file (a `Content-Range` covering everything). Used to prove
    /// probe never drains an oversized 206.
    pub force_full_206: bool,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            size: 64 * 1024 * 1024,
            supports_range: true,
            latency: Duration::ZERO,
            per_conn_bps: 0,
            global_bps: 0,
            keep_alive: true,
            chunk: 64 * 1024,
            collapse_at_connections: 0,
            collapse_num: 1,
            collapse_den: 1,
            force_full_206: false,
        }
    }
}

#[derive(Default)]
pub struct ServerStats {
    pub tcp_connections: AtomicUsize,
    pub requests: AtomicUsize,
    pub bytes_sent: AtomicU64,
    per_conn: Mutex<Vec<usize>>,
    active: AtomicUsize,
}

impl ServerStats {
    pub fn tcp_connections(&self) -> usize {
        self.tcp_connections.load(Ordering::Relaxed)
    }
    pub fn requests(&self) -> usize {
        self.requests.load(Ordering::Relaxed)
    }
    pub fn bytes_sent(&self) -> u64 {
        self.bytes_sent.load(Ordering::Relaxed)
    }
    pub fn active_connections(&self) -> usize {
        self.active.load(Ordering::Relaxed)
    }
    /// Largest number of requests served over a single TCP connection.
    pub fn max_requests_on_one_connection(&self) -> usize {
        self.per_conn
            .lock()
            .map(|v| v.iter().copied().max().unwrap_or(0))
            .unwrap_or(0)
    }
    fn note_request(&self, conn_index: usize) {
        if let Ok(mut v) = self.per_conn.lock() {
            while v.len() <= conn_index {
                v.push(0);
            }
            v[conn_index] += 1;
        }
    }
}

pub struct TestServer {
    pub addr: std::net::SocketAddr,
    stats: Arc<ServerStats>,
    shutdown: Arc<AtomicBool>,
}

impl TestServer {
    pub async fn start(cfg: ServerConfig) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let stats = Arc::new(ServerStats::default());
        let shutdown = Arc::new(AtomicBool::new(false));

        let cfg = Arc::new(cfg);
        let gstats = stats.clone();
        let gshutdown = shutdown.clone();
        let global_pace: Arc<Mutex<Instant>> = Arc::new(Mutex::new(Instant::now()));
        tokio::spawn(async move {
            let mut conn_index = 0usize;
            loop {
                if gshutdown.load(Ordering::Relaxed) {
                    return;
                }
                let accepted =
                    tokio::time::timeout(Duration::from_millis(200), listener.accept()).await;
                let Ok(Ok((stream, _))) = accepted else {
                    continue;
                };
                let my_index = conn_index;
                conn_index += 1;
                gstats.tcp_connections.fetch_add(1, Ordering::Relaxed);
                gstats.active.fetch_add(1, Ordering::Relaxed);
                let cfg = cfg.clone();
                let stats = gstats.clone();
                let global_pace = global_pace.clone();
                tokio::spawn(async move {
                    serve_connection(stream, cfg, stats.clone(), my_index, global_pace).await;
                    stats.active.fetch_sub(1, Ordering::Relaxed);
                });
            }
        });

        Self {
            addr,
            stats,
            shutdown,
        }
    }

    pub fn url(&self, path: &str) -> String {
        format!("http://{}/{}", self.addr, path.trim_start_matches('/'))
    }

    pub fn stats(&self) -> &Arc<ServerStats> {
        &self.stats
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Relaxed);
    }
}

struct RequestHead {
    method: String,
    range: Option<(u64, Option<u64>)>,
}

fn parse_request(raw: &str) -> Option<RequestHead> {
    let mut lines = raw.split("\r\n");
    let request_line = lines.next()?;
    let method = request_line.split_whitespace().next()?.to_string();
    let mut range = None;
    for line in lines {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        if name.eq_ignore_ascii_case("range") {
            let spec = value.trim().strip_prefix("bytes=")?;
            let spec = spec.split(',').next()?.trim();
            let (start, end) = spec.split_once('-')?;
            let start: u64 = start.trim().parse().ok()?;
            let end = if end.trim().is_empty() {
                None
            } else {
                Some(end.trim().parse().ok()?)
            };
            range = Some((start, end));
        }
    }
    Some(RequestHead { method, range })
}

async fn read_head(stream: &mut TcpStream) -> Option<String> {
    let mut total: Vec<u8> = Vec::new();
    let mut buf = [0u8; 4096];
    loop {
        let n = stream.read(&mut buf).await.ok()?;
        if n == 0 {
            return None;
        }
        total.extend_from_slice(&buf[..n]);
        if let Some(pos) = total.windows(4).position(|w| w == b"\r\n\r\n") {
            total.truncate(pos + 4);
            return Some(String::from_utf8_lossy(&total).into_owned());
        }
        if total.len() > 64 * 1024 {
            return None;
        }
    }
}

async fn serve_connection(
    mut stream: TcpStream,
    cfg: Arc<ServerConfig>,
    stats: Arc<ServerStats>,
    conn_index: usize,
    global_pace: Arc<Mutex<Instant>>,
) {
    let started = Instant::now();
    let mut conn_bytes: u64 = 0;
    loop {
        let Some(raw) = read_head(&mut stream).await else {
            return;
        };
        let Some(req) = parse_request(&raw) else {
            return;
        };
        stats.requests.fetch_add(1, Ordering::Relaxed);
        stats.note_request(conn_index);

        if !cfg.latency.is_zero() {
            tokio::time::sleep(cfg.latency).await;
        }

        // Serve a range when asked and supported.
        let served_range = if cfg.supports_range {
            req.range
        } else {
            None
        };
        let head_only = req.method.eq_ignore_ascii_case("HEAD");

        let (status, body_start, body_len, total) = if cfg.force_full_206
            && req.range.is_some()
        {
            // Broken CDN: 206 but the whole object in the body.
            (206u16, 0, cfg.size, Some(cfg.size))
        } else {
            match served_range {
                Some((start, end)) => {
                    if start >= cfg.size {
                        let _ = stream
                            .write_all(
                                b"HTTP/1.1 416 Range Not Satisfiable\r\nContent-Length: 0\r\n\r\n",
                            )
                            .await;
                        return;
                    }
                    let end = end.unwrap_or(cfg.size - 1).min(cfg.size - 1);
                    (206u16, start, end - start + 1, Some(cfg.size))
                }
                None => (200u16, 0, cfg.size, None),
            }
        };

        let mut head = format!("HTTP/1.1 {status} {}\r\n", reason(status));
        match total {
            Some(total) => head.push_str(&format!(
                "Content-Range: bytes {}-{}/{}\r\n",
                body_start,
                body_start + body_len - 1,
                total
            )),
            None => {}
        }
        head.push_str(&format!(
            "Content-Length: {body_len}\r\nContent-Type: application/octet-stream\r\nContent-Disposition: attachment; filename=bench.bin\r\n",
        ));
        if !cfg.keep_alive {
            head.push_str("Connection: close\r\n");
        }
        head.push_str("\r\n");
        if stream.write_all(head.as_bytes()).await.is_err() {
            return;
        }
        if head_only {
            continue;
        }

        let per_conn_bps = effective_per_conn_bps(&cfg, &stats);
        let global_bps = effective_global_bps(&cfg, &stats);
        let mut offset = body_start;
        let mut remaining = body_len;
        while remaining > 0 {
            let n = remaining.min(cfg.chunk.max(1) as u64) as usize;
            let body = body_bytes(offset, n);
            if stream.write_all(&body).await.is_err() {
                return;
            }
            let n = n as u64;
            stats.bytes_sent.fetch_add(n, Ordering::Relaxed);
            conn_bytes += n;
            offset += n;
            remaining -= n;
            pace(per_conn_bps, global_bps, n, conn_bytes, started, &global_pace).await;
        }
        let _ = stream.flush().await;

        if !cfg.keep_alive {
            return;
        }
    }
}

fn reason(status: u16) -> &'static str {
    match status {
        206 => "Partial Content",
        416 => "Range Not Satisfiable",
        _ => "OK",
    }
}

/// Collapse factor applied when more than `collapse_at_connections` are open.
fn collapse_ratio(cfg: &ServerConfig, stats: &ServerStats) -> (u64, u64) {
    if cfg.collapse_at_connections > 0 && stats.active_connections() > cfg.collapse_at_connections {
        (cfg.collapse_num, cfg.collapse_den.max(1))
    } else {
        (1, 1)
    }
}

/// Per-connection cap. `0` means this connection is not rate limited.
fn effective_per_conn_bps(cfg: &ServerConfig, stats: &ServerStats) -> u64 {
    let (num, den) = collapse_ratio(cfg, stats);
    cfg.per_conn_bps * num / den
}

/// Aggregate cap across all connections. `0` means no aggregate cap — this is
/// what lets extra connections actually raise aggregate throughput.
fn effective_global_bps(cfg: &ServerConfig, stats: &ServerStats) -> u64 {
    let (num, den) = collapse_ratio(cfg, stats);
    cfg.global_bps * num / den
}

/// Deterministic, allocation-light body content. Not meaningful data, but
/// stable per offset so callers can sanity-check what was written.
fn body_bytes(offset: u64, len: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(len);
    for i in 0..len {
        out.push((((offset + i as u64) % 251) as u8).wrapping_add(1));
    }
    out
}

async fn pace(
    per_conn_bps: u64,
    global_bps: u64,
    n: u64,
    conn_bytes: u64,
    started: Instant,
    global: &Mutex<Instant>,
) {
    let mut wait = Duration::ZERO;
    // Per-connection pacing: this connection alone may not exceed its cap.
    if per_conn_bps > 0 {
        let target = started + Duration::from_secs_f64(conn_bytes as f64 / per_conn_bps as f64);
        wait = wait.max(target.saturating_duration_since(Instant::now()));
    }
    // Aggregate pacing, only when configured: serialises all connections
    // through one timeline so N connections cannot exceed global_bps.
    if global_bps > 0 {
        let global_wait = {
            let mut next = match global.lock() {
                Ok(g) => g,
                Err(_) => return,
            };
            let now = Instant::now();
            let base = if *next > now { *next } else { now };
            *next = base + Duration::from_secs_f64(n as f64 / global_bps as f64);
            base.saturating_duration_since(now)
        };
        wait = wait.max(global_wait);
    }
    if !wait.is_zero() {
        tokio::time::sleep(wait).await;
    }
}
