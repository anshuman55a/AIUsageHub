use super::{MetricFormat, MetricLine};
use std::collections::HashMap;
use std::io::{Read, Seek, SeekFrom};

// ── Settings ──────────────────────────────────────────────────────

const DEFAULT_URL: &str = "http://localhost:11434";
const SETTINGS_FILE: &str = "ollama_settings.json";

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct OllamaSettings {
    #[serde(default = "default_url")]
    pub url: String,
    #[serde(default)]
    pub api_key: String,
}

fn default_url() -> String {
    DEFAULT_URL.to_string()
}

impl Default for OllamaSettings {
    fn default() -> Self {
        Self {
            url: DEFAULT_URL.to_string(),
            api_key: String::new(),
        }
    }
}

/// Returns the path to the settings file in the app's data directory.
fn settings_path() -> Option<std::path::PathBuf> {
    // Tauri stores app data under:
    //   Windows: %LOCALAPPDATA%/{bundle-identifier}/
    //   Linux:   ~/.local/share/{bundle-identifier}/
    //   macOS:   ~/Library/Application Support/{bundle-identifier}/
    let base = if cfg!(target_os = "windows") {
        std::env::var("LOCALAPPDATA")
            .ok()
            .map(std::path::PathBuf::from)
    } else if cfg!(target_os = "macos") {
        dirs::data_dir()
    } else {
        dirs::data_local_dir()
    };
    base.map(|d| d.join("com.usagedock.app").join(SETTINGS_FILE))
}

pub fn load_settings() -> OllamaSettings {
    let path = match settings_path() {
        Some(p) => p,
        None => return OllamaSettings::default(),
    };
    match std::fs::read_to_string(&path) {
        Ok(content) => serde_json::from_str(&content).unwrap_or_default(),
        Err(_) => OllamaSettings::default(),
    }
}

pub fn save_settings(settings: &OllamaSettings) -> Result<(), String> {
    let path = settings_path().ok_or("Cannot determine app data directory")?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create settings directory: {}", e))?;
    }
    let json = serde_json::to_string_pretty(settings)
        .map_err(|e| format!("Failed to serialize settings: {}", e))?;
    std::fs::write(&path, json).map_err(|e| format!("Failed to write settings: {}", e))
}

// ── HTTP helpers ──────────────────────────────────────────────────

fn build_client() -> reqwest::blocking::Client {
    reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .unwrap_or_else(|_| reqwest::blocking::Client::new())
}

/// Returns true if the URL is safe to send an API key to (localhost or ollama.com).
fn is_trusted_ollama_url(url: &str) -> bool {
    let lower = url.to_lowercase();
    // Strip scheme
    let host_part = lower
        .strip_prefix("http://")
        .or_else(|| lower.strip_prefix("https://"))
        .unwrap_or(&lower);
    // Extract host (before first / or :)
    let host = host_part
        .split('/')
        .next()
        .unwrap_or("")
        .split(':')
        .next()
        .unwrap_or("");
    matches!(
        host,
        "localhost" | "127.0.0.1" | "::1" | "0.0.0.0"
    ) || host.ends_with(".ollama.com")
      || host == "ollama.com"
}

fn ollama_get(
    client: &reqwest::blocking::Client,
    url: &str,
    api_key: &str,
) -> Result<(serde_json::Value, HashMap<String, String>), String> {
    let mut req = client.get(url).header("Accept", "application/json");
    if !api_key.is_empty() && is_trusted_ollama_url(url) {
        req = req.header("Authorization", format!("Bearer {}", api_key));
    }
    let resp = req.send().map_err(|e| format!("Network error: {}", e))?;
    let status = resp.status().as_u16();
    if status == 401 || status == 403 {
        return Err(format!(
            "Ollama auth failed (HTTP {}). Check your API key.",
            status
        ));
    }
    if status < 200 || status >= 300 {
        return Err(format!("Ollama request failed (HTTP {})", status));
    }
    let headers = extract_rate_limit_headers(&resp);
    let body: serde_json::Value = resp
        .json()
        .map_err(|e| format!("Invalid response: {}", e))?;
    Ok((body, headers))
}

fn ollama_post(
    client: &reqwest::blocking::Client,
    url: &str,
    api_key: &str,
) -> Option<serde_json::Value> {
    let mut req = client
        .post(url)
        .header("Accept", "application/json")
        .header("Content-Type", "application/json")
        .body("{}");
    if !api_key.is_empty() && is_trusted_ollama_url(url) {
        req = req.header("Authorization", format!("Bearer {}", api_key));
    }
    let resp = req.send().ok()?;
    if !resp.status().is_success() {
        return None;
    }
    resp.json().ok()
}

// ── Rate-limit header parsing ─────────────────────────────────────

struct RateLimitBucket {
    label: String,
    limit: i64,
    remaining: i64,
    reset_at: Option<String>,
}

fn extract_rate_limit_headers(
    resp: &reqwest::blocking::Response,
) -> HashMap<String, String> {
    let mut map = HashMap::new();
    let keys = [
        "x-ratelimit-limit-requests",
        "ratelimit-limit",
        "x-ratelimit-limit",
        "x-ratelimit-remaining-requests",
        "ratelimit-remaining",
        "x-ratelimit-remaining",
        "x-ratelimit-reset-requests",
        "ratelimit-reset",
        "x-ratelimit-reset",
        "x-ratelimit-limit-tokens",
        "x-ratelimit-remaining-tokens",
        "x-ratelimit-reset-tokens",
    ];
    for key in keys {
        if let Some(val) = resp.headers().get(key) {
            if let Ok(s) = val.to_str() {
                map.insert(key.to_string(), s.to_string());
            }
        }
    }
    map
}

fn parse_rate_limits(headers: &HashMap<String, String>) -> Vec<RateLimitBucket> {
    let mut buckets = Vec::new();

    let try_int = |names: &[&str]| -> Option<i64> {
        for name in names {
            if let Some(val) = headers.get(*name) {
                if let Ok(n) = val.parse::<i64>() {
                    return Some(n);
                }
            }
        }
        None
    };

    let try_str = |names: &[&str]| -> Option<String> {
        for name in names {
            if let Some(val) = headers.get(*name) {
                let trimmed = val.trim();
                if !trimmed.is_empty() {
                    return Some(trimmed.to_string());
                }
            }
        }
        None
    };

    // Request rate limits
    let req_limit = try_int(&[
        "x-ratelimit-limit-requests",
        "ratelimit-limit",
        "x-ratelimit-limit",
    ]);
    let req_remaining = try_int(&[
        "x-ratelimit-remaining-requests",
        "ratelimit-remaining",
        "x-ratelimit-remaining",
    ]);
    if let (Some(limit), Some(remaining)) = (req_limit, req_remaining) {
        if limit > 0 {
            buckets.push(RateLimitBucket {
                label: "Requests".into(),
                limit,
                remaining,
                reset_at: try_str(&[
                    "x-ratelimit-reset-requests",
                    "ratelimit-reset",
                    "x-ratelimit-reset",
                ]),
            });
        }
    }

    // Token rate limits
    let tok_limit = try_int(&["x-ratelimit-limit-tokens"]);
    let tok_remaining = try_int(&["x-ratelimit-remaining-tokens"]);
    if let (Some(limit), Some(remaining)) = (tok_limit, tok_remaining) {
        if limit > 0 {
            buckets.push(RateLimitBucket {
                label: "Tokens".into(),
                limit,
                remaining,
                reset_at: try_str(&["x-ratelimit-reset-tokens"]),
            });
        }
    }

    buckets
}

fn reset_to_iso(reset: &str) -> Option<String> {
    // Already ISO-ish
    if reset.len() >= 10 && reset.chars().nth(4) == Some('-') {
        return Some(reset.to_string());
    }
    // Unix timestamp
    if reset.chars().all(|c| c.is_ascii_digit()) && reset.len() >= 8 {
        if let Ok(ts) = reset.parse::<i64>() {
            return Some(unix_sec_to_iso(ts));
        }
    }
    // Duration like "1h30m5s" or "2d3h"
    parse_duration_to_iso(reset)
}

fn unix_sec_to_iso(secs: i64) -> String {
    let d = std::time::UNIX_EPOCH + std::time::Duration::from_secs(secs as u64);
    let total_secs = d
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let days = total_secs / 86400;
    let time_secs = total_secs % 86400;
    let hours = time_secs / 3600;
    let minutes = (time_secs % 3600) / 60;
    let seconds = time_secs % 60;
    let (year, month, day) = days_to_ymd(days as i64);
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        year, month, day, hours, minutes, seconds
    )
}

fn parse_duration_to_iso(s: &str) -> Option<String> {
    let re = regex_lite::Regex::new(r"(?:(\d+)d)?(?:(\d+)h)?(?:(\d+)m)?(?:([\d.]+)s)?").ok()?;
    let caps = re.captures(s)?;
    let d: u64 = caps.get(1).and_then(|m| m.as_str().parse().ok()).unwrap_or(0);
    let h: u64 = caps.get(2).and_then(|m| m.as_str().parse().ok()).unwrap_or(0);
    let m: u64 = caps.get(3).and_then(|m| m.as_str().parse().ok()).unwrap_or(0);
    let s_val: f64 = caps.get(4).and_then(|m| m.as_str().parse().ok()).unwrap_or(0.0);

    if d == 0 && h == 0 && m == 0 && s_val == 0.0 {
        return None;
    }

    let ms = ((d * 24 + h) * 60 + m) * 60000 + (s_val * 1000.0) as u64;
    if ms == 0 {
        return None;
    }

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64;
    let future_secs = (now + ms) / 1000;
    Some(unix_sec_to_iso(future_secs as i64))
}

fn days_to_ymd(days: i64) -> (i64, i64, i64) {
    let z = days + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = (z - era * 146097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y, m as i64, d as i64)
}

// ── API key validation ────────────────────────────────────────────

fn is_valid_api_key(key: &str) -> bool {
    let len = key.len();
    len >= 8
        && len <= 256
        && key
            .chars()
            .all(|c| c.is_alphanumeric() || "-_.:+=/".contains(c))
}

// ── Cloud usage via /api/me ───────────────────────────────────────

struct CloudUsage {
    plan: Option<String>,
    account_name: Option<String>,
    session_used: Option<f64>,
    session_reset: Option<String>,
    weekly_used: Option<f64>,
    weekly_reset: Option<String>,
}

fn extract_float(obj: &serde_json::Value, keys: &[&str]) -> Option<f64> {
    for key in keys {
        // Try exact key
        if let Some(val) = obj.get(key) {
            if let Some(n) = val.as_f64() {
                return Some(n);
            }
            if let Some(s) = val.as_str() {
                if let Ok(n) = s.parse::<f64>() {
                    return Some(n);
                }
            }
        }
    }
    None
}

fn extract_usage_from_payload(val: &serde_json::Value) -> (Option<f64>, Option<String>) {
    if let Some(n) = val.as_f64() {
        return (Some(n), None);
    }
    if let Some(s) = val.as_str() {
        let cleaned = s.trim_end_matches('%');
        if let Ok(n) = cleaned.parse::<f64>() {
            return (Some(n), None);
        }
    }
    if let Some(obj) = val.as_object() {
        let used = extract_float(val, &["used", "usage", "value", "percent", "pct", "used_percent"]);
        let mut reset: Option<String> = None;
        for key in ["reset_at", "resets_at", "reset_time", "reset"] {
            if let Some(v) = obj.get(key) {
                if let Some(s) = v.as_str() {
                    let trimmed = s.trim();
                    if !trimmed.is_empty() {
                        reset = Some(trimmed.to_string());
                        break;
                    }
                }
            }
        }
        if reset.is_none() {
            let seconds = extract_float(
                val,
                &[
                    "reset_in",
                    "reset_in_seconds",
                    "resets_in",
                    "seconds_to_reset",
                ],
            );
            if let Some(s) = seconds {
                if s > 0.0 {
                    let now_ms = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap()
                        .as_millis() as u64;
                    let future_secs = (now_ms + (s * 1000.0) as u64) / 1000;
                    reset = Some(unix_sec_to_iso(future_secs as i64));
                }
            }
        }
        return (used, reset);
    }
    (None, None)
}

fn fetch_cloud_usage(
    client: &reqwest::blocking::Client,
    base: &str,
    api_key: &str,
) -> CloudUsage {
    let mut result = CloudUsage {
        plan: None,
        account_name: None,
        session_used: None,
        session_reset: None,
        weekly_used: None,
        weekly_reset: None,
    };

    // Try local /api/me first
    let local_me = ollama_post(client, &format!("{}/api/me", base), api_key);

    // Try cloud endpoint if API key is set
    let mut cloud_me: Option<serde_json::Value> = None;
    if is_valid_api_key(api_key) {
        let cloud_client = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(8))
            .build()
            .unwrap_or_else(|_| reqwest::blocking::Client::new());
        if let Ok(resp) = cloud_client
            .post("https://ollama.com/api/me")
            .header("Content-Type", "application/json")
            .header("Authorization", format!("Bearer {}", api_key))
            .header("Accept", "application/json")
            .body("{}")
            .send()
        {
            if resp.status().is_success() {
                cloud_me = resp.json().ok();
            }
        }
    }

    let me = cloud_me.or(local_me);
    let me = match me {
        Some(ref v) if v.is_object() => v,
        _ => return result,
    };

    // Extract account info — handle both PascalCase (cloud) and lowercase (local)
    for key in ["Plan", "plan"] {
        if let Some(v) = me.get(key).and_then(|v| v.as_str()) {
            if !v.trim().is_empty() {
                result.plan = Some(v.trim().to_string());
                break;
            }
        }
    }
    for key in ["Name", "name"] {
        if let Some(v) = me.get(key).and_then(|v| v.as_str()) {
            if !v.trim().is_empty() {
                result.account_name = Some(v.trim().to_string());
                break;
            }
        }
    }

    // Look for session/weekly usage in multiple possible locations
    let sources: Vec<&serde_json::Value> = ["usage", "cloud_usage", "quota", "Usage", "CloudUsage", "Quota"]
        .iter()
        .filter_map(|k| me.get(k))
        .chain(std::iter::once(me))
        .collect();

    let session_keys = [
        "session_usage",
        "usage_5h",
        "five_hour_usage",
        "SessionUsage",
        "FiveHourUsage",
    ];
    let weekly_keys = [
        "weekly_usage",
        "usage_1d",
        "daily_usage",
        "WeeklyUsage",
        "DailyUsage",
    ];

    for src in &sources {
        if let Some(obj) = src.as_object() {
            if result.session_used.is_none() {
                for key in &session_keys {
                    if let Some(val) = obj.get(*key) {
                        let (used, reset) = extract_usage_from_payload(val);
                        if used.is_some() {
                            result.session_used = used;
                            result.session_reset = reset;
                            break;
                        }
                    }
                }
            }
            if result.weekly_used.is_none() {
                for key in &weekly_keys {
                    if let Some(val) = obj.get(*key) {
                        let (used, reset) = extract_usage_from_payload(val);
                        if used.is_some() {
                            result.weekly_used = used;
                            result.weekly_reset = reset;
                            break;
                        }
                    }
                }
            }
        }
    }

    result
}

// ── Desktop DB stats ──────────────────────────────────────────────

struct DesktopStats {
    messages_today: i64,
    sessions_today: i64,
    total_messages: i64,
    total_chats: i64,
    cached_plan: Option<String>,
}

fn get_ollama_db_path() -> Option<String> {
    if cfg!(target_os = "windows") {
        let local = std::env::var("LOCALAPPDATA").ok()?;
        Some(
            std::path::PathBuf::from(local)
                .join("Ollama")
                .join("db.sqlite")
                .to_string_lossy()
                .to_string(),
        )
    } else if cfg!(target_os = "macos") {
        let home = dirs::home_dir()?;
        Some(
            home.join("Library")
                .join("Application Support")
                .join("Ollama")
                .join("db.sqlite")
                .to_string_lossy()
                .to_string(),
        )
    } else {
        // Linux
        let xdg = std::env::var("XDG_DATA_HOME")
            .ok()
            .map(std::path::PathBuf::from)
            .or_else(|| dirs::home_dir().map(|h| h.join(".local").join("share")));
        let primary = xdg
            .as_ref()
            .map(|d| d.join("Ollama").join("db.sqlite").to_string_lossy().to_string());
        let fallback = dirs::home_dir().map(|h| {
            h.join(".config")
                .join("Ollama")
                .join("db.sqlite")
                .to_string_lossy()
                .to_string()
        });

        if let Some(ref p) = primary {
            if std::path::Path::new(p).exists() {
                return primary;
            }
        }
        fallback
    }
}

/// Only accepts compile-time static SQL to prevent accidental injection.
fn query_count(conn: &rusqlite::Connection, sql: &'static str) -> i64 {
    conn.query_row(sql, [], |row| row.get::<_, i64>(0))
        .unwrap_or(0)
}

fn fetch_desktop_stats() -> Option<DesktopStats> {
    let db_path = get_ollama_db_path()?;
    if !std::path::Path::new(&db_path).exists() {
        return None;
    }

    let conn = rusqlite::Connection::open_with_flags(
        &db_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .ok()?;

    let stats = DesktopStats {
        messages_today: query_count(
            &conn,
            "SELECT COUNT(*) FROM messages WHERE date(created_at)=date('now','localtime')",
        ),
        sessions_today: query_count(
            &conn,
            "SELECT COUNT(*) FROM chats WHERE date(created_at)=date('now','localtime')",
        ),
        total_messages: query_count(&conn, "SELECT COUNT(*) FROM messages"),
        total_chats: query_count(&conn, "SELECT COUNT(*) FROM chats"),
        cached_plan: conn
            .query_row("SELECT plan FROM users LIMIT 1", [], |row| {
                row.get::<_, String>(0)
            })
            .ok()
            .filter(|s| !s.trim().is_empty()),
    };

    Some(stats)
}

// ── Server log parsing ────────────────────────────────────────────

struct ServerLogStats {
    requests_today: i64,
    requests_5h: i64,
    requests_24h: i64,
    chat_requests_today: i64,
    generate_requests_today: i64,
}

fn get_ollama_log_dir() -> String {
    if cfg!(target_os = "windows") {
        let local = std::env::var("LOCALAPPDATA").unwrap_or_else(|_| {
            dirs::home_dir()
                .map(|h| h.join("AppData").join("Local").to_string_lossy().to_string())
                .unwrap_or_default()
        });
        std::path::PathBuf::from(local)
            .join("Ollama")
            .to_string_lossy()
            .to_string()
    } else if cfg!(target_os = "macos") {
        dirs::home_dir()
            .map(|h| {
                h.join(".ollama")
                    .join("logs")
                    .to_string_lossy()
                    .to_string()
            })
            .unwrap_or_default()
    } else {
        dirs::home_dir()
            .map(|h| {
                h.join(".ollama")
                    .join("logs")
                    .to_string_lossy()
                    .to_string()
            })
            .unwrap_or_default()
    }
}

fn fetch_server_log_stats() -> Option<ServerLogStats> {
    let log_dir = get_ollama_log_dir();
    let dir = std::path::Path::new(&log_dir);
    if !dir.exists() {
        return None;
    }

    let log_re = regex_lite::Regex::new(r"(?i)^server-?\d*\.log$").ok()?;
    let gin_re = regex_lite::Regex::new(
        r#"^\[GIN\]\s+(\d{4}/\d{2}/\d{2})\s+-\s+(\d{2}:\d{2}:\d{2})\s+\|\s+(\d+)\s+\|[^|]*\|\s+[^|]*\|\s+\w+\s+"([^"]+)""#,
    )
    .ok()?;

    let inference_paths: std::collections::HashSet<&str> = [
        "/api/chat",
        "/api/generate",
        "/v1/chat/completions",
        "/v1/completions",
        "/v1/responses",
        "/v1/messages",
    ]
    .iter()
    .copied()
    .collect();

    let entries = std::fs::read_dir(dir).ok()?;
    let log_files: Vec<std::path::PathBuf> = entries
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.file_name()
                .to_str()
                .map(|s| log_re.is_match(s))
                .unwrap_or(false)
        })
        .map(|e| e.path())
        .collect();

    if log_files.is_empty() {
        return None;
    }

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let today_str = {
        let total_days = now / 86400;
        let (y, m, d) = days_to_ymd(total_days as i64);
        format!("{:04}/{:02}/{:02}", y, m, d)
    };
    let five_hours_ago = now.saturating_sub(5 * 3600);
    let twenty_four_hours_ago = now.saturating_sub(24 * 3600);

    let mut stats = ServerLogStats {
        requests_today: 0,
        requests_5h: 0,
        requests_24h: 0,
        chat_requests_today: 0,
        generate_requests_today: 0,
    };

    const MAX_LOG_BYTES: u64 = 2 * 1024 * 1024;

    for file in &log_files {
        let content = match read_log_tail(file, MAX_LOG_BYTES) {
            Some(c) => c,
            None => continue,
        };

        for line in content.lines() {
            let caps = match gin_re.captures(line) {
                Some(c) => c,
                None => continue,
            };
            let date_str = &caps[1];
            let time_str = &caps[2];
            let url_path = &caps[4];

            if !inference_paths.contains(url_path) {
                continue;
            }

            // Parse timestamp to epoch seconds
            let ts_secs = match parse_gin_timestamp(date_str, time_str) {
                Some(s) => s,
                None => continue,
            };

            if ts_secs >= twenty_four_hours_ago {
                stats.requests_24h += 1;
            }
            if ts_secs >= five_hours_ago {
                stats.requests_5h += 1;
            }
            if date_str == today_str {
                stats.requests_today += 1;
                if url_path == "/api/chat" || url_path == "/v1/chat/completions" {
                    stats.chat_requests_today += 1;
                } else if url_path == "/api/generate" || url_path == "/v1/completions" {
                    stats.generate_requests_today += 1;
                }
            }
        }
    }

    if stats.requests_today == 0 && stats.requests_24h == 0 {
        return None;
    }
    Some(stats)
}

fn read_log_tail(path: &std::path::Path, max_bytes: u64) -> Option<String> {
    let mut file = std::fs::File::open(path).ok()?;
    let metadata = file.metadata().ok()?;
    let size = metadata.len();
    if size <= max_bytes {
        let mut content = String::new();
        file.read_to_string(&mut content).ok()?;
        Some(content)
    } else {
        file.seek(SeekFrom::End(-(max_bytes as i64))).ok()?;
        let mut buf = vec![0u8; max_bytes as usize];
        file.read_exact(&mut buf).ok()?;
        Some(String::from_utf8_lossy(&buf).to_string())
    }
}

fn parse_gin_timestamp(date_str: &str, time_str: &str) -> Option<u64> {
    // date_str: "2026/06/01", time_str: "09:57:34"
    let parts: Vec<&str> = date_str.split('/').collect();
    if parts.len() != 3 {
        return None;
    }
    let year: i64 = parts[0].parse().ok()?;
    let month: i64 = parts[1].parse().ok()?;
    let day: i64 = parts[2].parse().ok()?;

    let time_parts: Vec<&str> = time_str.split(':').collect();
    if time_parts.len() != 3 {
        return None;
    }
    let hour: u64 = time_parts[0].parse().ok()?;
    let minute: u64 = time_parts[1].parse().ok()?;
    let second: u64 = time_parts[2].parse().ok()?;

    // Convert to days since epoch using the inverse of days_to_ymd
    let days = ymd_to_days(year, month, day)?;
    Some(days as u64 * 86400 + hour * 3600 + minute * 60 + second)
}

fn ymd_to_days(year: i64, month: i64, day: i64) -> Option<i64> {
    // Inverse of the Howard Hinnant algorithm
    let y = if month <= 2 { year - 1 } else { year };
    let m = if month <= 2 { month + 9 } else { month - 3 };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = (y - era * 400) as u64;
    let doy = (153 * m as u64 + 2) / 5 + day as u64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Some(era * 146097 + doe as i64 - 719468)
}

// ── Settings page scraper ─────────────────────────────────────────

struct SettingsPageUsage {
    session_used: Option<f64>,
    session_reset: Option<String>,
    weekly_used: Option<f64>,
    weekly_reset: Option<String>,
}

fn fetch_settings_page_usage(api_key: &str) -> Option<SettingsPageUsage> {
    if !is_valid_api_key(api_key) {
        return None;
    }

    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(8))
        .build()
        .ok()?;

    let resp = client
        .get("https://ollama.com/settings")
        .header(
            "Accept",
            "text/html,application/xhtml+xml",
        )
        .header("Authorization", format!("Bearer {}", api_key))
        .send()
        .ok()?;

    if resp.status().as_u16() != 200 {
        return None;
    }

    let html = resp.text().ok()?;
    if html.len() > 512 * 1024 {
        return None;
    }

    let mut result = SettingsPageUsage {
        session_used: None,
        session_reset: None,
        weekly_used: None,
        weekly_reset: None,
    };

    // Parse usage percentages
    let usage_re = regex_lite::Regex::new(
        r"(?i)(Session usage|Weekly usage)\s*</span>\s*<span[^>]*>\s*([0-9]+(?:\.[0-9]+)?)%\s*used\s*</span>",
    )
    .ok()?;

    for caps in usage_re.captures_iter(&html) {
        let label = caps[1].to_lowercase();
        let value: f64 = caps[2].parse().ok()?;
        if label == "session usage" {
            result.session_used = Some(value);
        } else if label == "weekly usage" {
            result.weekly_used = Some(value);
        }
    }

    // Parse reset times
    let html_lower = html.to_lowercase();
    for label in ["session usage", "weekly usage"] {
        if let Some(idx) = html_lower.find(label) {
            let end = (idx + 2000).min(html.len());
            let slice = &html[idx..end];
            let dt_re = regex_lite::Regex::new(r#"data-time="([^"]{1,64})""#).ok()?;
            if let Some(caps) = dt_re.captures(slice) {
                let timestamp = &caps[1];
                // Basic validation — looks like a date
                if timestamp.len() >= 10 && timestamp.contains('-') {
                    if label == "session usage" {
                        result.session_reset = Some(timestamp.to_string());
                    } else {
                        result.weekly_reset = Some(timestamp.to_string());
                    }
                }
            }
        }
    }

    if result.session_used.is_some() || result.weekly_used.is_some() {
        Some(result)
    } else {
        None
    }
}

// ── Format helpers ────────────────────────────────────────────────

fn format_bytes(bytes: f64) -> String {
    if bytes >= 1_073_741_824.0 {
        format!("{:.1} GB", bytes / 1_073_741_824.0)
    } else if bytes >= 1_048_576.0 {
        format!("{:.0} MB", bytes / 1_048_576.0)
    } else {
        format!("{} KB", (bytes / 1024.0).round() as i64)
    }
}

// ── Probe ─────────────────────────────────────────────────────────

pub fn probe() -> Result<(Option<String>, Vec<MetricLine>), String> {
    let settings = load_settings();
    let base = settings.url.trim_end_matches('/');
    let api_key = settings.api_key.trim();
    let client = build_client();

    // 1. Version — liveness check
    let mut version = String::new();
    let mut all_rate_headers: HashMap<String, String> = HashMap::new();

    match ollama_get(&client, &format!("{}/api/version", base), api_key) {
        Ok((body, headers)) => {
            version = body
                .get("version")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            all_rate_headers = headers;
        }
        Err(_) => {
            return Err(format!(
                "Ollama not running at {}. Start Ollama and try again.",
                base
            ));
        }
    }

    // 2. Running models (/api/ps)
    let mut running_models: Vec<serde_json::Value> = Vec::new();
    if let Ok((body, headers)) = ollama_get(&client, &format!("{}/api/ps", base), api_key) {
        running_models = body
            .get("models")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        if all_rate_headers.is_empty() {
            all_rate_headers = headers;
        }
    }

    // 3. Available models (/api/tags)
    let mut available_count: usize = 0;
    if let Ok((body, headers)) = ollama_get(&client, &format!("{}/api/tags", base), api_key) {
        available_count = body
            .get("models")
            .and_then(|v| v.as_array())
            .map(|a| a.len())
            .unwrap_or(0);
        if all_rate_headers.is_empty() {
            all_rate_headers = headers;
        }
    }

    // 4. Cloud usage (/api/me)
    let mut cloud = fetch_cloud_usage(&client, base, api_key);

    // 5. Desktop DB stats
    let desktop = fetch_desktop_stats();

    // 6. Server log stats
    let log_stats = fetch_server_log_stats();

    // 7. Settings page scraper — fills in cloud usage when /api/me doesn't have it
    if cloud.session_used.is_none() && cloud.weekly_used.is_none() {
        if let Some(settings_usage) = fetch_settings_page_usage(api_key) {
            cloud.session_used = settings_usage.session_used;
            cloud.session_reset = settings_usage.session_reset;
            cloud.weekly_used = settings_usage.weekly_used;
            cloud.weekly_reset = settings_usage.weekly_reset;
        }
    }

    // ── Build metric lines ────────────────────────────────────────
    let mut lines: Vec<MetricLine> = Vec::new();
    let rate_limits = parse_rate_limits(&all_rate_headers);
    let is_cloud = !rate_limits.is_empty();
    let plan = cloud
        .plan
        .clone()
        .or_else(|| desktop.as_ref().and_then(|d| d.cached_plan.clone()));

    // Status badge
    let version_label = if version.is_empty() {
        "Running".to_string()
    } else {
        format!("Running (v{})", version)
    };
    lines.push(MetricLine::Badge {
        label: "Server".into(),
        text: version_label,
        color: Some("#4ade80".into()),
    });

    // Account info
    if let Some(ref name) = cloud.account_name {
        lines.push(MetricLine::Text {
            label: "Account".into(),
            value: name.clone(),
        });
    }

    // Cloud usage bars
    if let Some(session) = cloud.session_used {
        lines.push(MetricLine::Progress {
            label: "Session usage".into(),
            used: session.clamp(0.0, 100.0),
            limit: 100.0,
            format: MetricFormat {
                kind: "percent".into(),
                suffix: None,
            },
            resets_at: cloud.session_reset.clone(),
        });
    }

    if let Some(weekly) = cloud.weekly_used {
        lines.push(MetricLine::Progress {
            label: "Weekly usage".into(),
            used: weekly.clamp(0.0, 100.0),
            limit: 100.0,
            format: MetricFormat {
                kind: "percent".into(),
                suffix: None,
            },
            resets_at: cloud.weekly_reset.clone(),
        });
    }

    // Free plan note
    if cloud.account_name.is_some()
        && cloud.session_used.is_none()
        && cloud.weekly_used.is_none()
        && !is_cloud
    {
        lines.push(MetricLine::Badge {
            label: "Usage".into(),
            text: "No limits on free plan".into(),
            color: Some("#22c55e".into()),
        });
    }

    // Rate-limit usage bars (cloud-hosted services)
    for bucket in &rate_limits {
        let used = bucket.limit - bucket.remaining;
        let pct = ((used as f64 / bucket.limit as f64) * 100.0)
            .round()
            .clamp(0.0, 100.0);
        lines.push(MetricLine::Progress {
            label: bucket.label.clone(),
            used: pct,
            limit: 100.0,
            format: MetricFormat {
                kind: "percent".into(),
                suffix: None,
            },
            resets_at: bucket
                .reset_at
                .as_ref()
                .and_then(|s| reset_to_iso(s)),
        });
    }

    // Model count
    let loaded_count = running_models.len();
    let count_text = if available_count > 0 {
        format!("{} loaded · {} available", loaded_count, available_count)
    } else if loaded_count > 0 {
        format!("{} loaded", loaded_count)
    } else {
        "No models loaded".to_string()
    };
    lines.push(MetricLine::Text {
        label: "Models".into(),
        value: count_text,
    });

    // Per-loaded-model details
    for m in &running_models {
        let name = m
            .get("name")
            .or_else(|| m.get("model"))
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");

        let vram = m.get("size_vram").and_then(|v| v.as_f64());
        let size = m.get("size").and_then(|v| v.as_f64());
        let param_size = m
            .get("details")
            .and_then(|d| d.get("parameter_size"))
            .and_then(|v| v.as_str());
        let quant = m
            .get("details")
            .and_then(|d| d.get("quantization_level"))
            .and_then(|v| v.as_str());

        let mut parts: Vec<String> = Vec::new();
        if let Some(v) = vram {
            if v > 0.0 {
                parts.push(format!("{} VRAM", format_bytes(v)));
            }
        } else if let Some(s) = size {
            if s > 0.0 {
                parts.push(format_bytes(s));
            }
        }
        if let Some(p) = param_size {
            parts.push(p.to_string());
        }
        if let Some(q) = quant {
            parts.push(q.to_string());
        }

        lines.push(MetricLine::Text {
            label: name.to_string(),
            value: if parts.is_empty() {
                String::new()
            } else {
                parts.join(" · ")
            },
        });
    }

    // Desktop DB stats
    if let Some(ref desktop) = desktop {
        if desktop.messages_today > 0 {
            lines.push(MetricLine::Text {
                label: "Today".into(),
                value: format!(
                    "{} msgs · {} sessions",
                    desktop.messages_today, desktop.sessions_today
                ),
            });
        }
        if desktop.total_messages > 0 {
            lines.push(MetricLine::Text {
                label: "All time".into(),
                value: format!(
                    "{} msgs · {} chats",
                    desktop.total_messages, desktop.total_chats
                ),
            });
        }
    }

    // Server log request counts
    if let Some(ref log) = log_stats {
        let mut log_parts: Vec<String> = Vec::new();
        if log.requests_today > 0 {
            log_parts.push(format!("{} today", log.requests_today));
        }
        if log.requests_5h > 0 {
            log_parts.push(format!("{} last 5h", log.requests_5h));
        }
        if log.requests_24h > 0 && log.requests_24h != log.requests_today {
            log_parts.push(format!("{} last 24h", log.requests_24h));
        }
        if !log_parts.is_empty() {
            lines.push(MetricLine::Text {
                label: "Requests".into(),
                value: log_parts.join(" · "),
            });
        }
        // Chat vs generate breakdown
        if log.chat_requests_today > 0 || log.generate_requests_today > 0 {
            let mut breakdown: Vec<String> = Vec::new();
            if log.chat_requests_today > 0 {
                breakdown.push(format!("{} chat", log.chat_requests_today));
            }
            if log.generate_requests_today > 0 {
                breakdown.push(format!("{} generate", log.generate_requests_today));
            }
            lines.push(MetricLine::Text {
                label: "Breakdown".into(),
                value: breakdown.join(" · "),
            });
        }
    }

    // Purely local hint when there's nothing at all
    if !is_cloud
        && cloud.account_name.is_none()
        && loaded_count == 0
        && available_count == 0
        && desktop
            .as_ref()
            .map(|d| d.total_messages == 0)
            .unwrap_or(true)
        && log_stats.is_none()
    {
        lines.push(MetricLine::Badge {
            label: "Hint".into(),
            text: "Local Ollama has no usage limits".into(),
            color: Some("#a3a3a3".into()),
        });
    }

    Ok((plan, lines))
}
