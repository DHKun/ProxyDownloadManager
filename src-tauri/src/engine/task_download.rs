use crate::engine::file_io::write_at;
use crate::engine::part_progress::PartProgressTracker;
use crate::headers::prepare_request;
use crate::network::limiter::MultiLimiter;
use crate::retry::{is_fatal_client_status, is_retryable_status};
use crate::types::Task;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

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

fn note_write(
    bytes_written: &AtomicU64,
    parts: Option<&PartProgressTracker>,
    file_offset: u64,
    len: u64,
) {
    if len == 0 {
        return;
    }
    bytes_written.fetch_add(len, Ordering::Relaxed);
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

    const BUF_SIZE: usize = 1024 * 1024; // 1MB
    let mut buf = Vec::with_capacity(BUF_SIZE);

    // Slow chunk detection: if >30s elapsed and <10% done, abort
    let start_time = std::time::Instant::now();

    loop {
        // Check cancel (responsive Stop even during streaming)
        if cancel.load(Ordering::Relaxed) {
            // Flush buffered data before returning to avoid data loss
            if !buf.is_empty() {
                if let Err(e) = write_at(file, &buf, base_offset + written) {
                    return TaskResult::Fatal(format!("write_at error on cancel: {}", e));
                }
                let n = buf.len() as u64;
                note_write(bytes_written, parts.as_deref(), base_offset + written, n);
                written += n;
                buf.clear();
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

        // Abort slow chunks so other workers can steal remaining work.
        // Flush first: bytes sitting in `buf` are real progress and must not
        // be dropped when the remainder is re-queued.
        let elapsed = start_time.elapsed();
        let pending = written + buf.len() as u64;
        if elapsed > std::time::Duration::from_secs(30)
            && chunk_size > 0
            && pending < chunk_size / 10
        {
            if !buf.is_empty() {
                if let Err(e) = write_at(file, &buf, base_offset + written) {
                    return TaskResult::Fatal(format!("write_at error: {}", e));
                }
                let n = buf.len() as u64;
                note_write(bytes_written, parts.as_deref(), base_offset + written, n);
                written += n;
                buf.clear();
            }
            log::debug!(
                "[ProxyDM] slow chunk offset={} written={}/{} after {}s, re-queuing",
                base_offset,
                written,
                chunk_size,
                elapsed.as_secs()
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

        let chunk_result =
            tokio::time::timeout(std::time::Duration::from_secs(10), stream.next()).await;
        let chunk = match chunk_result {
            Ok(Some(Ok(c))) => c,
            Ok(Some(Err(e))) => {
                if !buf.is_empty() {
                    if let Err(err) = write_at(file, &buf, base_offset + written) {
                        return TaskResult::Fatal(format!("write_at error: {}", err));
                    }
                    let n = buf.len() as u64;
                    note_write(bytes_written, parts.as_deref(), base_offset + written, n);
                    written += n;
                    buf.clear();
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
                return TaskResult::Fatal(format!("Stream error: {}", e));
            }
            Ok(None) => {
                if !buf.is_empty() {
                    if let Err(e) = write_at(file, &buf, base_offset + written) {
                        return TaskResult::Fatal(format!("write_at error: {}", e));
                    }
                    let n = buf.len() as u64;
                    note_write(bytes_written, parts.as_deref(), base_offset + written, n);
                    written += n;
                }
                break;
            }
            Err(_elapsed) => {
                if cancel.load(Ordering::Relaxed) {
                    // Flush buffered data before returning
                    if !buf.is_empty() {
                        if let Err(e) = write_at(file, &buf, base_offset + written) {
                            return TaskResult::Fatal(format!("write_at error on cancel: {}", e));
                        }
                        let n = buf.len() as u64;
                        note_write(bytes_written, parts.as_deref(), base_offset + written, n);
                        written += n;
                        buf.clear();
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
                continue;
            }
        };
        limiter.wait_n(chunk.len() as u64).await;

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
            if let Err(e) = write_at(file, &buf, base_offset + written) {
                return TaskResult::Fatal(format!("write_at error: {}", e));
            }
            let n = buf.len() as u64;
            note_write(bytes_written, parts.as_deref(), base_offset + written, n);
            written += n;
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

        if buf.len() >= BUF_SIZE {
            if let Err(e) = write_at(file, &buf, base_offset + written) {
                return TaskResult::Fatal(format!("write_at error: {}", e));
            }
            let n = buf.len() as u64;
            note_write(bytes_written, parts.as_deref(), base_offset + written, n);
            written += n;
            buf.clear();
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
}
