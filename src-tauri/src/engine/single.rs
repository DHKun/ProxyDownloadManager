use crate::engine::file_io::{self, length_shortfall};
use crate::engine::part_progress::encode_progress_data;
use crate::engine::task_download::{parse_content_range, validate_content_range};
use crate::network::limiter::MultiLimiter;
use crate::network::pool::NetworkPool;
use crate::types::{EngineConfig, Event, EventKind, PdmError, PdmResult};
use std::io::{Seek, SeekFrom, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::mpsc;

pub struct SingleDownloader {
    pool: Arc<NetworkPool>,
    event_tx: mpsc::UnboundedSender<Event>,
}

impl SingleDownloader {
    pub fn new(pool: Arc<NetworkPool>, event_tx: mpsc::UnboundedSender<Event>) -> Self {
        Self { pool, event_tx }
    }

    pub async fn download(
        &self,
        cfg: &EngineConfig,
        limiter: Arc<MultiLimiter>,
        cancel: Arc<AtomicBool>,
        on_resume: &crate::engine::OnResumeState,
    ) -> PdmResult<()> {
        log::info!("[ProxyDM] single id={} url={}", cfg.id, cfg.url);
        let resume_from = if cfg.is_resume && cfg.downloaded > 0 {
            cfg.downloaded
        } else {
            0
        };

        let mut req = self
            .pool
            .get_client(if cfg.proxy_url.is_empty() {
                None
            } else {
                Some(&cfg.proxy_url)
            })
            .map_err(|e| PdmError::ClientBuild(e.to_string()))?
            .get(&cfg.url);
        let range = if resume_from > 0 {
            Some(format!("bytes={resume_from}-"))
        } else {
            None
        };
        req = crate::headers::prepare_request(req, &cfg.headers, &cfg.user_agent, range.as_deref());
        let resp = match crate::engine::task_download::send_headers(req, Some(&cancel)).await {
            Ok(resp) => resp,
            Err(crate::engine::task_download::HeaderWait::Cancelled) => {
                return Err(PdmError::Cancelled);
            }
            Err(e) => return Err(PdmError::Network(e.to_string())),
        };
        log::info!("[ProxyDM] single id={} HTTP {}", cfg.id, resp.status());

        if cancel.load(Ordering::Relaxed) {
            return Err(PdmError::Cancelled);
        }

        let status = resp.status();
        if !status.is_success() && status != reqwest::StatusCode::PARTIAL_CONTENT {
            if status == reqwest::StatusCode::TOO_MANY_REQUESTS
                || status == reqwest::StatusCode::SERVICE_UNAVAILABLE
            {
                let retry_after = resp
                    .headers()
                    .get("retry-after")
                    .and_then(|v| v.to_str().ok())
                    .and_then(|s| s.parse::<u64>().ok())
                    .unwrap_or(5);
                return Err(PdmError::Other(format!(
                    "Rate limited, retry after {}s",
                    retry_after
                )));
            }
            return Err(PdmError::Http(status.as_u16()));
        }

        // 206 continues at resume_from. Anything else (200, or a 206 whose
        // range doesn't start where we left off) restarts from byte 0.
        let mut start_at = 0u64;
        let mut restart = false;
        if resume_from > 0 && status == reqwest::StatusCode::PARTIAL_CONTENT {
            let header = resp
                .headers()
                .get(reqwest::header::CONTENT_RANGE)
                .and_then(|v| v.to_str().ok())
                .and_then(parse_content_range);
            if let Some((start, end, total)) = header {
                if validate_content_range(resume_from, u64::MAX, start, end, total, cfg.total_size)
                {
                    start_at = resume_from;
                } else {
                    log::warn!(
                        "[ProxyDM] single id={} resume range rejected, restarting",
                        cfg.id
                    );
                    restart = true;
                }
            } else {
                log::warn!(
                    "[ProxyDM] single id={} 206 without Content-Range, restarting",
                    cfg.id
                );
                restart = true;
            }
        } else if resume_from > 0 && status != reqwest::StatusCode::OK {
            restart = true;
        } else if resume_from > 0 {
            log::info!(
                "[ProxyDM] single id={} server has no Range, restarting from 0",
                cfg.id
            );
        }

        let content_len = resp
            .headers()
            .get(reqwest::header::CONTENT_LENGTH)
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.parse::<u64>().ok());
        let expected = if cfg.total_size > 0 {
            cfg.total_size
        } else {
            content_len.map(|n| start_at + n).unwrap_or(0)
        };

        file_io::migrate_legacy_temp(cfg.id, &cfg.save_path);
        let pdm_path = file_io::temp_path(cfg.id);
        if let Some(parent) = std::path::Path::new(&pdm_path).parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| PdmError::Io(e.to_string()))?;
        }
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .read(true)
            .open(&pdm_path)
            .map_err(|e| PdmError::Io(e.to_string()))?;
        if restart {
            return Err(PdmError::Incomplete(format!(
                "resume rejected at offset {resume_from}"
            )));
        }
        if start_at == 0 {
            file.set_len(0).map_err(|e| PdmError::Io(e.to_string()))?;
        } else {
            let len = file.metadata().map(|m| m.len()).unwrap_or(0);
            if len < start_at {
                return Err(PdmError::Incomplete(format!(
                    "temp file is {len} bytes, resume offset is {start_at}"
                )));
            }
            file.seek(SeekFrom::Start(start_at))
                .map_err(|e| PdmError::Io(e.to_string()))?;
        }

        let stream = resp.bytes_stream();
        use futures_util::StreamExt;
        let mut stream = std::pin::pin!(stream);
        let mut total = start_at;
        const WRITE_BUFFER: usize = 256 * 1024;
        let mut buf = Vec::with_capacity(WRITE_BUFFER);
        let mut last_flush = std::time::Instant::now();
        let mut last_byte = std::time::Instant::now();

        let save_progress = |written: u64, id: u64, cfg: &EngineConfig| {
            let size = if expected > 0 {
                expected
            } else {
                cfg.total_size
            };
            let remaining = size.saturating_sub(written);
            if written > 0 && (size == 0 || remaining > 0) {
                let saved = crate::types::DownloadState {
                    url: cfg.url.clone(),
                    id,
                    file_name: cfg.file_name.clone(),
                    save_path: cfg.save_path.clone(),
                    total_size: size,
                    downloaded: written,
                    tasks: vec![crate::types::Task {
                        offset: written,
                        length: remaining,
                    }],
                    proxy_name: cfg.proxy_name.clone(),
                    workers: 1,
                };
                on_resume(id, &saved);
            }
        };

        loop {
            if cancel.load(Ordering::Relaxed) {
                if !buf.is_empty() {
                    let _ = file.write_all(&buf);
                    total += buf.len() as u64;
                    buf.clear();
                }
                let _ = file.flush();
                save_progress(total, cfg.id, cfg);
                return Err(PdmError::Cancelled);
            }

            let idle_left =
                crate::engine::task_download::BODY_IDLE.saturating_sub(last_byte.elapsed());
            let flush_left = if buf.is_empty() {
                None
            } else {
                Some(std::time::Duration::from_millis(250).saturating_sub(last_flush.elapsed()))
            };
            let chunk_result = if let Some(flush_left) = flush_left {
                tokio::select! {
                    biased;
                    _ = tokio::time::sleep(flush_left) => {
                        file.write_all(&buf)
                            .map_err(|e| PdmError::Io(e.to_string()))?;
                        total += buf.len() as u64;
                        buf.clear();
                        last_flush = std::time::Instant::now();
                        let _ = self.event_tx.send(Event {
                            kind: EventKind::DownloadProgress,
                            download_id: cfg.id,
                            data: Some(encode_progress_data(total, &[total], true)),
                        });
                        continue;
                    }
                    result = tokio::time::timeout(idle_left, stream.next()) => result,
                }
            } else {
                tokio::time::timeout(idle_left, stream.next()).await
            };
            let chunk = match chunk_result {
                Ok(Some(c)) => c,
                Ok(None) => break,
                Err(_) => {
                    if cancel.load(Ordering::Relaxed) {
                        if !buf.is_empty() {
                            let _ = file.write_all(&buf);
                            total += buf.len() as u64;
                            buf.clear();
                        }
                        let _ = file.flush();
                        save_progress(total, cfg.id, cfg);
                        return Err(PdmError::Cancelled);
                    }
                    return Err(PdmError::Network("body idle timeout".into()));
                }
            };
            let chunk = chunk.map_err(|e| PdmError::Network(e.to_string()))?;
            if chunk.is_empty() {
                continue;
            }
            limiter.wait_n(chunk.len() as u64).await;
            last_byte = std::time::Instant::now();
            buf.extend_from_slice(&chunk);

            if buf.len() >= WRITE_BUFFER {
                file.write_all(&buf)
                    .map_err(|e| PdmError::Io(e.to_string()))?;
                total += buf.len() as u64;
                buf.clear();
                last_flush = std::time::Instant::now();

                let _ = self.event_tx.send(Event {
                    kind: EventKind::DownloadProgress,
                    download_id: cfg.id,
                    data: Some(encode_progress_data(total, &[total], true)),
                });
            }
        }

        if !buf.is_empty() {
            file.write_all(&buf)
                .map_err(|e| PdmError::Io(e.to_string()))?;
            total += buf.len() as u64;
        }
        file.flush().map_err(|e| PdmError::Io(e.to_string()))?;
        file.sync_all().map_err(|e| PdmError::Io(e.to_string()))?;
        drop(file);

        if let Some(missing) = length_shortfall(total, expected) {
            log::error!(
                "[ProxyDM] single id={} incomplete, missing {missing} bytes",
                cfg.id
            );
            save_progress(total, cfg.id, cfg);
            return Err(PdmError::Incomplete(format!("{total}/{expected} bytes")));
        }

        file_io::finalize_file(cfg.id, &cfg.save_path)
            .await
            .map_err(PdmError::Io)?;

        log::info!("[ProxyDM] single id={} done total={} bytes", cfg.id, total);

        let _ = self.event_tx.send(Event {
            kind: EventKind::DownloadProgress,
            download_id: cfg.id,
            data: Some(encode_progress_data(total, &[total], true)),
        });

        let _ = self.event_tx.send(Event {
            kind: EventKind::DownloadCompleted,
            download_id: cfg.id,
            data: None,
        });

        Ok(())
    }
}
