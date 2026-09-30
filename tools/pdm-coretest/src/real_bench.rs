//! Real-network benchmark against a GitHub Release asset and a large public
//! mirror, using the same engine, probe and `NetworkPool` the app uses.
//!
//! Run explicitly (each config downloads for `PDM_BENCH_SECS` seconds, default
//! 30, then cancels — no full file is downloaded):
//!
//! ```text
//! PDM_BENCH_SECS=30 cargo test --lib real_network_benchmark -- --ignored \
//!     --nocapture --test-threads=1
//! ```
//!
//! If GitHub or the mirror is unreachable the test prints
//! `real-world benchmark unavailable ...` and skips instead of failing, so a
//! sandboxed run never invents numbers.

use crate::caplog;
use crate::engine::chunk;
use crate::network::pool::NetworkPool;
use crate::perf_tests::{engine_cfg, next_id, run_budget};
use crate::probe;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

const UA: &str = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/125.0.0.0 Safari/537.36";

struct Target {
    label: &'static str,
    url: String,
    size: u64,
}

async fn github_largest_asset(client: &reqwest::Client, repo: &str) -> Option<(String, u64)> {
    let api = format!("https://api.github.com/repos/{repo}/releases/latest");
    let resp = client.get(&api).header("User-Agent", "pdm-bench").send().await.ok()?;
    let body: serde_json::Value = resp.json().await.ok()?;
    let assets = body.get("assets")?.as_array()?;
    assets
        .iter()
        .filter_map(|a| {
            let url = a.get("browser_download_url")?.as_str()?.to_string();
            let size = a.get("size")?.as_u64()?;
            Some((url, size))
        })
        .max_by_key(|(_, size)| *size)
}

async fn kali_iso(client: &reqwest::Client) -> Option<(String, u64)> {
    let listing = client
        .get("https://cdimage.kali.org/current/")
        .header("User-Agent", "pdm-bench")
        .send()
        .await
        .ok()?
        .text()
        .await
        .ok()?;
    // pick the first installer-amd64 ISO link
    let name = listing
        .split("href=\"")
        .skip(1)
        .filter_map(|s| s.split('"').next())
        .find(|n| n.ends_with("installer-amd64.iso"))?
        .to_string();
    let url = format!("https://cdimage.kali.org/current/{name}");
    let head = client
        .head(&url)
        .header("User-Agent", "pdm-bench")
        .send()
        .await
        .ok()?;
    let size = head.content_length().unwrap_or(0);
    Some((url, size))
}

async fn discover() -> Vec<Target> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .unwrap();
    let mut out = Vec::new();

    match github_largest_asset(&client, "NationalSecurityAgency/ghidra").await {
        Some((url, size)) => {
            println!("[bench] target github size={size} url={url}");
            out.push(Target {
                label: "github",
                url,
                size,
            });
        }
        None => println!("real-world benchmark unavailable: GitHub assets"),
    }

    match kali_iso(&client).await {
        Some((url, size)) => {
            println!("[bench] target kali size={size} url={url}");
            out.push(Target {
                label: "kali",
                url,
                size,
            });
        }
        None => println!("real-world benchmark unavailable: Kali mirror"),
    }

    out
}

#[allow(dead_code)]
struct Row {
    connections: String,
    protocol: String,
    ttfb_ms: u128,
    first_progress_ms: u128,
    seconds: f64,
    mbps: f64,
    retries: u64,
    stalls: u64,
    completed: bool,
}

fn parse_perf(cap: &caplog::Capture) -> (u64, u64) {
    let mut retries = 0;
    let mut stalls = 0;
    for (_, line) in cap.lines().iter().filter(|(_, l)| l.contains("[perf]")) {
        for token in line.split_whitespace() {
            if let Some(v) = token.strip_prefix("retries=") {
                retries = v.parse().unwrap_or(retries);
            }
            if let Some(v) = token.strip_prefix("stalls=") {
                stalls = v.parse().unwrap_or(stalls);
            }
        }
    }
    (retries, stalls)
}

async fn bench_one(
    cap: &caplog::Capture,
    target: &Target,
    auto: bool,
    connections: u32,
    budget: Duration,
) -> Option<Row> {
    let pool = Arc::new(NetworkPool::new(true));
    let headers = HashMap::new();
    let uas = vec![UA.to_string()];

    // The New Download window probes first; Start then reuses it (ProbeCache).
    let probe_started = Instant::now();
    let probed = match probe::probe(&target.url, &headers, None, &pool, &uas).await {
        Ok(p) => p,
        Err(e) => {
            println!(
                "real-world benchmark unavailable: {} probe failed: {e}",
                target.label
            );
            return None;
        }
    };
    let probe_ms = probe_started.elapsed().as_millis();
    let size = if probed.file_size > 0 {
        probed.file_size
    } else {
        target.size
    };
    if !probed.supports_range || size == 0 {
        println!(
            "real-world benchmark unavailable: {} no range support (size={size})",
            target.label
        );
        return None;
    }

    let (desired, auto_flag) = if auto {
        (
            Arc::new(AtomicU32::new(chunk::auto_initial_connections(size))),
            Arc::new(AtomicBool::new(true)),
        )
    } else {
        (
            Arc::new(AtomicU32::new(connections)),
            Arc::new(AtomicBool::new(false)),
        )
    };
    let cfg = engine_cfg(
        &target.url,
        size,
        if auto { 0 } else { connections },
        auto,
        Some(desired.clone()),
        Some(auto_flag),
        next_id(),
    );

    cap.reset();
    let started = Instant::now();
    let (result, downloaded, completed) = run_budget(cfg, pool, budget).await;
    let seconds = started.elapsed().as_secs_f64();

    let protocol = cap
        .line("[net] range_offset=0 protocol=")
        .and_then(|l| {
            l.split("protocol=")
                .nth(1)
                .map(|s| s.split_whitespace().next().unwrap_or("?").to_string())
        })
        .unwrap_or_else(|| "?".to_string());
    let (retries, stalls) = parse_perf(cap);
    let label = if auto {
        format!("auto(final {})", desired.load(Ordering::Relaxed))
    } else {
        format!("manual {connections}")
    };
    let ttfb = cap.at_ms("[startup] first-header").unwrap_or(0);
    let first_progress = cap.at_ms("[startup] first-progress").unwrap_or(0);

    println!(
        "REAL {} | {label:>14} | probe={probe_ms}ms protocol={protocol} ttfb={ttfb}ms first_progress={first_progress}ms {seconds:.1}s {:.1}MB/s retries={retries} stalls={stalls} completed={completed} ({result:?})",
        target.label,
        downloaded as f64 / seconds.max(0.001) / (1024.0 * 1024.0),
    );

    Some(Row {
        connections: label,
        protocol,
        ttfb_ms: ttfb,
        first_progress_ms: first_progress,
        seconds,
        mbps: downloaded as f64 / seconds.max(0.001) / (1024.0 * 1024.0),
        retries,
        stalls,
        completed,
    })
}

#[tokio::test]
#[ignore = "manual real-network benchmark"]
async fn real_network_benchmark() {
    let cap = caplog::init();
    let seconds: u64 = std::env::var("PDM_BENCH_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(30);
    let budget = Duration::from_secs(seconds);

    let targets = discover().await;
    if targets.is_empty() {
        println!("real-world benchmark unavailable: no reachable targets");
        return;
    }

    let configs: Vec<(bool, u32)> = vec![(true, 0), (false, 4), (false, 8), (false, 16), (false, 32)];
    println!("\n=== real benchmark: {seconds}s per config ===");
    for target in &targets {
        for (auto, connections) in &configs {
            let _ = bench_one(&cap, target, *auto, *connections, budget).await;
        }
    }
    println!("\nnote: proxy benchmark unavailable — no proxy is configured in this environment.");
}
