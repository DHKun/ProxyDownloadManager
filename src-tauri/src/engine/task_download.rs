use crate::engine::file_io::write_at;
use crate::engine::part_progress::PartProgressTracker;
use crate::headers::prepare_request;
use crate::network::limiter::MultiLimiter;
use crate::retry::{is_fatal_client_status, is_retryable_status};
use crate::types::Task;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// How long a chunk may sit with no body bytes before it is stalled.
/// Limiter waits happen outside this timer.
pub const BODY_IDLE: Duration = Duration::from_secs(30);
const WRITE_BUFFER: usize = 256 * 1024;
const PROGRESS_FLUSH: Duration = Duration::from_millis(250);

/// Outcome of a single chunk download attempt.
#[derive(Debug, PartialEq)]
pub enum TaskResult {
    /// Chunk fully downloaded.
    Complete,
    /// Partial progress: remaining bytes should be re-queued.
    Partial { remaining: Task },
    /// User cancelled.
    Cancelled,
    /// Server ignored the Range header — retrying is pointless; concurrent
    /// download cannot proceed at all.
    RangeNotSupported,
    /// HTTP 206 whose Content-Range does not match the bytes we asked for.
    /// Nothing was written.
    InvalidRangeResponse,
    /// Unrecoverable error — don't retry this chunk.
    Fatal(String),
    /// Client error that must not be retried (401/403/404…).
    FatalNoRetry(String),
}

fn flush_progress(
    file: &std::fs::File,
    buf: &mut Vec<u8>,
    bytes_written: &AtomicU64,
    parts: Option<&PartProgressTracker>,
    at: u64,
) -> Result<u64, String> {
    if buf.is_empty() {
        return Ok(0);
    }
    write_at(file, buf, at).map_err(|e| e.to_string())?;
    let n = buf.len() as u64;
    note_write(bytes_written, parts, at, n);
    buf.clear();
    Ok(n)
}

fn note_write(
    bytes_written: &AtomicU64,
    parts: Option<&PartProgressTracker>,
    file_offset: u64,
    len: u64,
) {
    if len == 0 {
        return;
    }
    let prev = bytes_written.fetch_add(len, Ordering::Relaxed);
    if prev == 0 {
        log::debug!("[startup] first-progress-byte");
    }
    if let Some(p) = parts {
        p.record_write(file_offset, len);
    }
}

/// `Content-Range: bytes start-end/total` (`total` may be `*`).
pub fn parse_content_range(header: &str) -> Option<(u64, u64, Option<u64>)> {
    let rest = header.trim().strip_prefix("bytes ")?;
    let (range, total) = rest.split_once('/')?;
    let (start, end) = range.split_once('-')?;
    let start: u64 = start.trim().parse().ok()?;
    let end: u64 = end.trim().parse().ok()?;
    let total = match total.trim() {
        "*" => None,
        s => Some(s.parse().ok()?),
    };
    Some((start, end, total))
}

/// A 206 is usable only when the returned interval sits inside the request
/// and, when we already know the object size, the total matches.
pub fn validate_content_range(
    requested_start: u64,
    requested_end: u64,
    actual_start: u64,
    actual_end: u64,
    actual_total: Option<u64>,
    expected_total: u64,
) -> bool {
    if actual_start != requested_start || actual_end < actual_start || actual_end > requested_end {
        return false;
    }
    if expected_total > 0 {
        if let Some(total) = actual_total {
            if total != expected_total {
                return false;
            }
        }
    }
    true
}

/// Why waiting for response headers failed. The body is not covered:
/// reqwest's request timeout would abort a multi-gigabyte transfer.
#[derive(Debug)]
pub enum HeaderWait {
    Cancelled,
    Failed(String),
}

impl std::fmt::Display for HeaderWait {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Cancelled => f.write_str("cancelled"),
            Self::Failed(msg) => f.write_str(msg),
        }
    }
}

/// Send a request and wait only for the response headers.
/// `cancel` is polled while the headers are outstanding so pause does not
/// sit behind a 30s header timeout.
pub async fn send_headers(
    req: reqwest::RequestBuilder,
    cancel: Option<&AtomicBool>,
) -> Result<reqwest::Response, HeaderWait> {
    let fut = req.send();
    tokio::pin!(fut);
    let timeout = tokio::time::sleep(std::time::Duration::from_secs(30));
    tokio::pin!(timeout);
    tokio::select! {
        biased;
        _ = until_cancelled(cancel) => Err(HeaderWait::Cancelled),
        result = &mut fut => match result {
            Ok(resp) => Ok(resp),
            Err(e) => {
                let mut msg = e.to_string();
                let mut src = std::error::Error::source(&e);
                while let Some(s) = src {
                    msg.push_str(&format!(": {s}"));
                    src = s.source();
                }
                Err(HeaderWait::Failed(msg))
            }
        },
        _ = &mut timeout => Err(HeaderWait::Failed(
            "timed out waiting for response headers".into(),
        )),
    }
}

async fn until_cancelled(cancel: Option<&AtomicBool>) {
    let Some(flag) = cancel else {
        std::future::pending::<()>().await;
        return;
    };
    loop {
        if flag.load(Ordering::Relaxed) {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
}

pub async fn download_task(
    url: &str,
    client: &reqwest::Client,
    file: &std::fs::File,
    task: &Task,
    cancel: &AtomicBool,
    limiter: &MultiLimiter,
    user_agent: &str,
    bytes_written: &AtomicU64,
    parts: Option<Arc<PartProgressTracker>>,
    headers: &HashMap<String, String>,
    expected_total: u64,
    idle: Duration,
) -> TaskResult {
    let mut written: u64 = 0;
    let range_end = if task.length == 0 {
        String::new()
    } else {
        format!("{}", task.offset + task.length - 1)
    };
    let range_header = format!("bytes={}-{}", task.offset, range_end);
    let req = prepare_request(client.get(url), headers, user_agent, Some(&range_header));
    log::debug!(
        "[ProxyDM] concurrent_task offset={} range={}",
        task.offset,
        range_header
    );
    let resp = match send_headers(req, Some(cancel)).await {
        Ok(r) => r,
        Err(HeaderWait::Cancelled) => {
            return TaskResult::Partial {
                remaining: task.clone(),
            };
        }
        Err(HeaderWait::Failed(msg)) => {
            log::error!(
                "[ProxyDM] concurrent_task REQUEST ERROR offset={}: {}",
                task.offset,
                msg
            );
            return TaskResult::Fatal(msg);
        }
    };

    if cancel.load(Ordering::Relaxed) {
        // Nothing written yet — hand the whole task back so the queue drain
        // at cancel time still covers it. Returning Cancelled here would drop
        // the popped task from the resume snapshot entirely.
        return TaskResult::Partial {
            remaining: task.clone(),
        };
    }

    let status = resp.status();
    if status != reqwest::StatusCode::OK && status != reqwest::StatusCode::PARTIAL_CONTENT {
        log::info!(
            "[ProxyDM] concurrent_task offset={} HTTP {} range={}",
            task.offset,
            status,
            range_header
        );
    } else if task.offset == 0 {
        log::info!(
            "[ProxyDM] concurrent_task offset=0 HTTP {} range={}",
            status,
            range_header
        );
    } else {
        log::debug!(
            "[ProxyDM] concurrent_task offset={} HTTP {}",
            task.offset,
            status
        );
    }

    // 200 on a Range request means the server ignored the range. The first
    // chunk may still be a full-object 200 whose Content-Length matches the
    // task; anything else must not be written at this offset.
    if status == reqwest::StatusCode::OK {
        let content_len = resp
            .headers()
            .get(reqwest::header::CONTENT_LENGTH)
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.parse::<u64>().ok());
        let ignored = if task.offset > 0 {
            true
        } else {
            match content_len {
                Some(n) if task.length > 0 && n > task.length => true,
                _ => false,
            }
        };
        if ignored {
            log::warn!(
                "[ProxyDM] server ignored Range header (HTTP 200), offset={}",
                task.offset
            );
            return TaskResult::RangeNotSupported;
        }
    }
    let mut body_limit = task.length;
    if status == reqwest::StatusCode::PARTIAL_CONTENT {
        let header = resp
            .headers()
            .get(reqwest::header::CONTENT_RANGE)
            .and_then(|v| v.to_str().ok())
            .map(str::to_string);
        let Some(header) = header else {
            log::warn!("[ProxyDM] 206 missing Content-Range offset={}", task.offset);
            return TaskResult::InvalidRangeResponse;
        };
        let Some((start, end, total)) = parse_content_range(&header) else {
            log::warn!(
                "[ProxyDM] 206 bad Content-Range {:?} offset={}",
                header,
                task.offset
            );
            return TaskResult::InvalidRangeResponse;
        };
        let requested_end = if task.length == 0 {
            u64::MAX
        } else {
            task.offset + task.length - 1
        };
        if !validate_content_range(
            task.offset,
            requested_end,
            start,
            end,
            total,
            expected_total,
        ) {
            log::warn!(
                "[ProxyDM] Content-Range {start}-{end}/{total:?} != requested {}-{} total={expected_total}",
                task.offset, requested_end
            );
            return TaskResult::InvalidRangeResponse;
        }
        body_limit = end - start + 1;
    }
    if status != reqwest::StatusCode::OK && status != reqwest::StatusCode::PARTIAL_CONTENT {
        let code = status.as_u16();
        let msg = format!("HTTP {}", code);
        if is_fatal_client_status(code) {
            return TaskResult::FatalNoRetry(msg);
        }
        if is_retryable_status(code) {
            return TaskResult::Fatal(msg);
        }
        return TaskResult::Fatal(msg);
    }

    let stream = resp.bytes_stream();
    use futures_util::StreamExt;
    let mut stream = std::pin::pin!(stream);
    let base_offset = task.offset;
    let chunk_size = task.length;

    let mut buf = Vec::with_capacity(WRITE_BUFFER);
    let mut last_flush = Instant::now();
    let mut last_byte = Instant::now();

    loop {
        if cancel.load(Ordering::Relaxed) {
            match flush_progress(
                file,
                &mut buf,
                bytes_written,
                parts.as_deref(),
                base_offset + written,
            ) {
                Ok(n) => written += n,
                Err(e) => return TaskResult::Fatal(format!("write_at error on cancel: {e}")),
            }
            let remaining = chunk_size.saturating_sub(written);
            if remaining > 0 {
                return TaskResult::Partial {
                    remaining: Task {
                        offset: base_offset + written,
                        length: remaining,
                    },
                };
            }
            return TaskResult::Cancelled;
        }

        // Flush while the next read is still blocked, so a small chunk is
        // visible within PROGRESS_FLUSH instead of sitting until the next packet.
        let idle_left = idle.saturating_sub(last_byte.elapsed());
        let flush_left = if buf.is_empty() {
            None
        } else {
            Some(PROGRESS_FLUSH.saturating_sub(last_flush.elapsed()))
        };
        let chunk_result = if let Some(flush_left) = flush_left {
            tokio::select! {
                biased;
                _ = tokio::time::sleep(flush_left) => {
                    match flush_progress(file, &mut buf, bytes_written, parts.as_deref(), base_offset + written) {
                        Ok(n) => written += n,
                        Err(e) => return TaskResult::Fatal(format!("write_at error: {e}")),
                    }
                    last_flush = Instant::now();
                    continue;
                }
                result = tokio::time::timeout(idle_left, stream.next()) => result,
            }
        } else {
            tokio::time::timeout(idle_left, stream.next()).await
        };
        let chunk = match chunk_result {
            Ok(Some(Ok(c))) => c,
            Ok(Some(Err(e))) => {
                match flush_progress(
                    file,
                    &mut buf,
                    bytes_written,
                    parts.as_deref(),
                    base_offset + written,
                ) {
                    Ok(n) => written += n,
                    Err(err) => return TaskResult::Fatal(format!("write_at error: {err}")),
                }
                let remaining = chunk_size.saturating_sub(written);
                if remaining == 0 {
                    return TaskResult::Complete;
                }
                if written > 0 {
                    return TaskResult::Partial {
                        remaining: Task {
                            offset: base_offset + written,
                            length: remaining,
                        },
                    };
                }
                return TaskResult::Fatal(format!("Stream error: {e}"));
            }
            Ok(None) => {
                match flush_progress(
                    file,
                    &mut buf,
                    bytes_written,
                    parts.as_deref(),
                    base_offset + written,
                ) {
                    Ok(n) => written += n,
                    Err(e) => return TaskResult::Fatal(format!("write_at error: {e}")),
                }
                break;
            }
            Err(_) => {
                match flush_progress(
                    file,
                    &mut buf,
                    bytes_written,
                    parts.as_deref(),
                    base_offset + written,
                ) {
                    Ok(n) => written += n,
                    Err(e) => return TaskResult::Fatal(format!("write_at error: {e}")),
                }
                if cancel.load(Ordering::Relaxed) {
                    let remaining = chunk_size.saturating_sub(written);
                    if remaining > 0 {
                        return TaskResult::Partial {
                            remaining: Task {
                                offset: base_offset + written,
                                length: remaining,
                            },
                        };
                    }
                    return TaskResult::Cancelled;
                }
                log::debug!(
                    "[ProxyDM] body idle offset={} written={} for {}s",
                    base_offset,
                    written,
                    idle.as_secs()
                );
                let remaining = chunk_size.saturating_sub(written);
                if remaining == 0 {
                    return TaskResult::Complete;
                }
                return TaskResult::Partial {
                    remaining: Task {
                        offset: base_offset + written,
                        length: remaining,
                    },
                };
            }
        };
        if chunk.is_empty() {
            continue;
        }
        limiter.wait_n(chunk.len() as u64).await;
        // Idle starts after throttling. A user limit must not look like a stall.
        last_byte = Instant::now();

        buf.extend_from_slice(&chunk);

        // Bound the write to this task's region: a server that ignores Range
        // on the offset-0 task streams the WHOLE file — everything past
        // chunk_size belongs to other tasks and would only inflate counters.
        // A short Content-Range caps even earlier; the unread tail is re-queued.
        let cap = if chunk_size == 0 {
            body_limit
        } else if body_limit == 0 {
            chunk_size
        } else {
            chunk_size.min(body_limit)
        };
        if cap > 0 && written + buf.len() as u64 >= cap {
            buf.truncate((cap - written) as usize);
            match flush_progress(
                file,
                &mut buf,
                bytes_written,
                parts.as_deref(),
                base_offset + written,
            ) {
                Ok(n) => written += n,
                Err(e) => return TaskResult::Fatal(format!("write_at error: {e}")),
            }
            if chunk_size > written {
                return TaskResult::Partial {
                    remaining: Task {
                        offset: base_offset + written,
                        length: chunk_size - written,
                    },
                };
            }
            return TaskResult::Complete;
        }

        if buf.len() >= WRITE_BUFFER {
            match flush_progress(
                file,
                &mut buf,
                bytes_written,
                parts.as_deref(),
                base_offset + written,
            ) {
                Ok(n) => written += n,
                Err(e) => return TaskResult::Fatal(format!("write_at error: {e}")),
            }
            last_flush = Instant::now();
        }
    }

    if chunk_size > written {
        return TaskResult::Partial {
            remaining: Task {
                offset: base_offset + written,
                length: chunk_size - written,
            },
        };
    }
    TaskResult::Complete
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_result_complete_is_not_partial() {
        assert_eq!(TaskResult::Complete, TaskResult::Complete);
        assert_ne!(TaskResult::Complete, TaskResult::Cancelled);
    }

    #[test]
    fn task_result_partial_has_remaining() {
        let remaining = Task {
            offset: 3000,
            length: 2000,
        };
        let r = TaskResult::Partial {
            remaining: remaining.clone(),
        };
        if let TaskResult::Partial { remaining } = r {
            assert_eq!(remaining.offset, 3000);
            assert_eq!(remaining.length, 2000);
        } else {
            panic!("expected Partial");
        }
    }

    #[test]
    fn content_range_accepts_exact_match() {
        assert!(validate_content_range(0, 99, 0, 99, Some(100), 100));
        assert!(validate_content_range(50, 99, 50, 80, Some(100), 100));
    }

    #[test]
    fn content_range_rejects_bad_start_end_and_total() {
        assert!(!validate_content_range(10, 20, 0, 20, Some(100), 100));
        assert!(!validate_content_range(0, 10, 0, 11, Some(100), 100));
        assert!(!validate_content_range(0, 10, 5, 4, Some(100), 100));
        assert!(!validate_content_range(0, 10, 0, 10, Some(99), 100));
    }

    #[test]
    fn content_range_missing_total_ok_when_size_unknown() {
        assert!(validate_content_range(0, 10, 0, 10, None, 0));
        assert!(validate_content_range(0, 10, 0, 10, None, 100));
    }

    #[test]
    fn parse_content_range_star_total() {
        assert_eq!(parse_content_range("bytes 0-9/*"), Some((0, 9, None)));
        assert_eq!(
            parse_content_range("bytes 8-15/100"),
            Some((8, 15, Some(100)))
        );
        assert_eq!(parse_content_range("not-a-range"), None);
    }

    #[test]
    fn task_result_fatal_contains_message() {
        let r = TaskResult::Fatal("HTTP 403".to_string());
        if let TaskResult::Fatal(msg) = r {
            assert_eq!(msg, "HTTP 403");
        } else {
            panic!("expected Fatal");
        }
    }

    #[test]
    fn body_idle_is_thirty_seconds() {
        assert_eq!(BODY_IDLE, Duration::from_secs(30));
    }

    fn requested_range(req: &str) -> Option<(u64, u64)> {
        for line in req.lines() {
            let line = line.trim();
            if line.len() < 6 || !line[..6].eq_ignore_ascii_case("range:") {
                continue;
            }
            let spec = line[6..].trim().strip_prefix("bytes=")?;
            let (start, end) = spec.split_once('-')?;
            return Some((start.trim().parse().ok()?, end.trim().parse().ok()?));
        }
        None
    }

    async fn read_http(stream: &mut tokio::net::TcpStream) -> Option<String> {
        use tokio::io::AsyncReadExt;
        let mut total = Vec::new();
        let mut buf = [0u8; 2048];
        loop {
            let n = stream.read(&mut buf).await.unwrap_or(0);
            if n == 0 {
                return None;
            }
            total.extend_from_slice(&buf[..n]);
            if total.windows(4).any(|w| w == b"\r\n\r\n") || total.len() > 8192 {
                break;
            }
        }
        Some(String::from_utf8_lossy(&total).into_owned())
    }

    enum Pace {
        /// One byte at a time, faster than the idle window.
        Trickle,
        /// A few bytes, then silence while the socket stays open.
        Stall,
        /// A first slice, a pause, then the rest. The pause is long enough
        /// for the client to read the slice and enter the limiter.
        Split { first: usize, gap: Duration },
    }

    async fn pace_server(total: u64, pace: Pace) -> String {
        use tokio::io::AsyncWriteExt;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            let Ok((mut stream, _)) = listener.accept().await else {
                return;
            };
            let Some(req) = read_http(&mut stream).await else {
                return;
            };
            let Some((start, end)) = requested_range(&req) else {
                return;
            };
            let len = (end - start + 1) as usize;
            let hdr = format!(
                "HTTP/1.1 206 Partial Content\r\nContent-Range: bytes {start}-{end}/{total}\r\nContent-Length: {len}\r\nConnection: close\r\n\r\n"
            );
            if stream.write_all(hdr.as_bytes()).await.is_err() {
                return;
            }
            let body = vec![7u8; len];
            match pace {
                Pace::Trickle => {
                    for byte in &body {
                        if stream.write_all(&[*byte]).await.is_err() {
                            return;
                        }
                        let _ = stream.flush().await;
                        tokio::time::sleep(Duration::from_millis(40)).await;
                    }
                }
                Pace::Stall => {
                    let n = 128.min(body.len());
                    let _ = stream.write_all(&body[..n]).await;
                    let _ = stream.flush().await;
                    tokio::time::sleep(Duration::from_secs(3)).await;
                    return;
                }
                Pace::Split { first, gap } => {
                    let first = first.min(body.len());
                    let _ = stream.write_all(&body[..first]).await;
                    let _ = stream.flush().await;
                    tokio::time::sleep(gap).await;
                    let _ = stream.write_all(&body[first..]).await;
                }
            }
            let _ = stream.shutdown().await;
        });
        format!("http://127.0.0.1:{port}/file.bin")
    }

    fn temp_bin(prefix: &str) -> std::path::PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT: AtomicU64 = AtomicU64::new(1);
        std::env::temp_dir().join(format!(
            "{prefix}_{}_{}.bin",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ))
    }

    async fn run_one(url: &str, len: u64, idle: Duration, bps: u64) -> (TaskResult, u64, Duration) {
        let client = reqwest::Client::builder().build().unwrap();
        let path = temp_bin("pdm_idle");
        let file = std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .read(true)
            .open(&path)
            .unwrap();
        let bytes = Arc::new(AtomicU64::new(0));
        let limiter = MultiLimiter::new(bps, 0);
        let cancel = AtomicBool::new(false);
        let started = Instant::now();
        let result = download_task(
            url,
            &client,
            &file,
            &Task {
                offset: 0,
                length: len,
            },
            &cancel,
            &limiter,
            "pdm-test",
            &bytes,
            None,
            &HashMap::new(),
            len,
            idle,
        )
        .await;
        let n = bytes.load(Ordering::Relaxed);
        drop(file);
        let _ = std::fs::remove_file(&path);
        (result, n, started.elapsed())
    }

    #[tokio::test]
    async fn slow_but_continuous_body_is_not_a_stall() {
        let total = 25u64;
        let url = pace_server(total, Pace::Trickle).await;
        let (result, n, elapsed) = run_one(&url, total, Duration::from_millis(300), 0).await;
        assert_eq!(result, TaskResult::Complete, "{elapsed:?}");
        assert_eq!(n, total);
    }

    #[tokio::test]
    async fn silent_body_is_a_stall() {
        let total = 4096u64;
        let url = pace_server(total, Pace::Stall).await;
        let (result, n, elapsed) = run_one(&url, total, Duration::from_millis(400), 0).await;
        assert!(
            matches!(result, TaskResult::Partial { .. }),
            "{result:?} after {elapsed:?}"
        );
        assert!(n > 0, "stall flushed no bytes");
        assert!(
            elapsed < Duration::from_secs(2),
            "idle waited {elapsed:?} instead of ~400ms"
        );
    }

    #[tokio::test]
    async fn limiter_wait_is_not_a_body_stall() {
        let total = 16 * 1024u64;
        let url = pace_server(
            total,
            Pace::Split {
                first: 8 * 1024,
                gap: Duration::from_millis(400),
            },
        )
        .await;
        // 8 KiB at 8 KiB/s waits about a second, longer than the 250ms idle.
        let (result, n, elapsed) = run_one(&url, total, Duration::from_millis(250), 8 * 1024).await;
        assert_eq!(
            result,
            TaskResult::Complete,
            "throttled read looked like a stall after {elapsed:?}, bytes={n}"
        );
        assert_eq!(n, total);
        assert!(
            elapsed > Duration::from_millis(600),
            "limiter did not wait: {elapsed:?}"
        );
    }

    #[tokio::test]
    async fn progress_is_visible_before_a_one_megabyte_buffer() {
        let total = 64 * 1024u64;
        let url = pace_server(
            total,
            Pace::Split {
                first: 4096,
                gap: Duration::from_millis(800),
            },
        )
        .await;
        let client = reqwest::Client::builder().build().unwrap();
        let path = temp_bin("pdm_flush");
        let file = std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .read(true)
            .open(&path)
            .unwrap();
        let bytes = Arc::new(AtomicU64::new(0));
        let watch = bytes.clone();
        let url2 = url.clone();
        let handle = tokio::spawn(async move {
            let limiter = MultiLimiter::new(0, 0);
            let cancel = AtomicBool::new(false);
            download_task(
                &url2,
                &client,
                &file,
                &Task {
                    offset: 0,
                    length: total,
                },
                &cancel,
                &limiter,
                "pdm-test",
                &bytes,
                None,
                &HashMap::new(),
                total,
                Duration::from_secs(5),
            )
            .await
        });
        let started = Instant::now();
        let mut seen = 0u64;
        while started.elapsed() < Duration::from_millis(600) {
            seen = watch.load(Ordering::Relaxed);
            if seen > 0 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert!(seen > 0, "no bytes flushed within 600ms");
        assert!(
            seen < total,
            "full file was buffered before any progress: {seen}"
        );
        let result = handle.await.unwrap();
        assert_eq!(result, TaskResult::Complete);
        let _ = std::fs::remove_file(&path);
    }
}
