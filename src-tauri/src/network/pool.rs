use crate::types::{PdmError, PdmResult};
use reqwest::Proxy;
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::Duration;

/// Idle sockets are recycled after this long so a host that is no longer
/// active does not hold a socket forever. This is an *idle* pool policy, not a
/// limit on connections a running download may open.
const POOL_IDLE_TIMEOUT: Duration = Duration::from_secs(90);
/// Idle sockets kept per host. This does not cap active connections: reqwest
/// opens as many as the workers ask for and only the *idle* surplus is dropped.
const POOL_MAX_IDLE_PER_HOST: usize = 128;
const TCP_KEEPALIVE: Duration = Duration::from_secs(60);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);

/// One `reqwest::Client` per network path (Direct, or one proxy URL), shared by
/// probe, every download worker and every retry of that path. Cloning a client
/// is cheap and shares the same hyper connection pool.
pub struct NetworkPool {
    clients: Mutex<HashMap<String, reqwest::Client>>,
    danger_accept_invalid_certs: bool,
    client_creations: AtomicUsize,
}

impl NetworkPool {
    pub fn new(danger_accept_invalid_certs: bool) -> Self {
        Self {
            clients: Mutex::new(HashMap::new()),
            danger_accept_invalid_certs,
            client_creations: AtomicUsize::new(0),
        }
    }

    pub fn get_client(&self, proxy_url: Option<&str>) -> PdmResult<reqwest::Client> {
        let key = proxy_url.unwrap_or("direct").to_string();
        let mut map = self
            .clients
            .lock()
            .map_err(|e| PdmError::Other(e.to_string()))?;
        if let Some(client) = map.get(&key) {
            return Ok(client.clone());
        }
        log::info!(
            "[ProxyDM] pool creating new client for proxy={}",
            crate::headers::redact_log(&key)
        );
        let mut builder = reqwest::Client::builder()
            .pool_max_idle_per_host(POOL_MAX_IDLE_PER_HOST)
            .pool_idle_timeout(Some(POOL_IDLE_TIMEOUT))
            .tcp_keepalive(Some(TCP_KEEPALIVE))
            .connect_timeout(CONNECT_TIMEOUT)
            .https_only(false)
            .danger_accept_invalid_certs(self.danger_accept_invalid_certs);

        if let Some(proxy_str) = proxy_url {
            if let Ok(proxy) = Proxy::all(proxy_str) {
                log::info!(
                    "[ProxyDM] pool applying proxy: {}",
                    crate::headers::redact_log(proxy_str)
                );
                builder = builder.proxy(proxy);
            }
        }

        let client = builder
            .build()
            .map_err(|e| PdmError::ClientBuild(e.to_string()))?;
        map.insert(key, client.clone());
        self.client_creations.fetch_add(1, Ordering::Relaxed);
        Ok(client)
    }

    /// Number of `reqwest::Client` instances this pool has ever built. Tests use
    /// it to prove probe, download and retry share one client per network path.
    pub fn client_creation_count(&self) -> usize {
        self.client_creations.load(Ordering::Relaxed)
    }

    /// Number of cached clients (one per distinct network path).
    pub fn cached_client_count(&self) -> usize {
        self.clients.lock().map(|m| m.len()).unwrap_or(0)
    }

    /// Clear cached clients so next get_client() rebuilds with current settings
    /// (only settings that change how a Client is constructed, e.g. TLS).
    pub fn clear(&self) {
        let mut map = self.clients.lock().unwrap();
        let count = map.len();
        map.clear();
        log::info!("[ProxyDM] pool cleared {} cached client(s)", count);
    }
}
