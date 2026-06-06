use super::{MetricFormat, MetricLine};

#[cfg(target_os = "windows")]
use std::collections::BTreeSet;

#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;

#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x08000000;

const LS_SERVICE: &str = "exa.language_server_pb.LanguageServerService";
const CLOUD_CODE_URLS: &[&str] = &[
    "https://daily-cloudcode-pa.googleapis.com",
    "https://cloudcode-pa.googleapis.com",
];
const FETCH_MODELS_PATH: &str = "/v1internal:fetchAvailableModels";
const GOOGLE_OAUTH_URL: &str = "https://oauth2.googleapis.com/token";
const OAUTH_TOKEN_KEY: &str = "antigravityUnifiedStateSync.oauthToken";
const OAUTH_TOKEN_SENTINEL: &str = "oauthTokenInfoSentinelKey";
const AUTH_STATE_SENTINEL: &str = "authStateWithContextSentinelKey";

const GOOGLE_CLIENT_ID: Option<&str> = option_env!("USAGEDOCK_ANTIGRAVITY_GOOGLE_CLIENT_ID");
const GOOGLE_CLIENT_SECRET: Option<&str> = option_env!("USAGEDOCK_ANTIGRAVITY_GOOGLE_CLIENT_SECRET");

/// Models to skip — internal/placeholder entries that shouldn't display.
const MODEL_BLACKLIST: &[&str] = &[
    "MODEL_CHAT_20706",
    "MODEL_CHAT_23310",
    "MODEL_GOOGLE_GEMINI_2_5_FLASH",
    "MODEL_GOOGLE_GEMINI_2_5_FLASH_THINKING",
    "MODEL_GOOGLE_GEMINI_2_5_FLASH_LITE",
    "MODEL_GOOGLE_GEMINI_2_5_PRO",
    "MODEL_PLACEHOLDER_M19",
    "MODEL_PLACEHOLDER_M9",
    "MODEL_PLACEHOLDER_M12",
];

// ---------------------------------------------------------------------------
// Protobuf wire-format decoder
// ---------------------------------------------------------------------------

/// A decoded protobuf field value.
enum ProtoValue {
    Varint(u64),
    LengthDelimited(Vec<u8>),
}

/// Read a varint from `data` starting at `pos`. Returns (value, new_pos).
fn read_varint(data: &[u8], mut pos: usize) -> Option<(u64, usize)> {
    let mut value: u64 = 0;
    let mut shift = 0u32;
    while pos < data.len() {
        let b = data[pos];
        pos += 1;
        value |= ((b & 0x7f) as u64) << shift;
        if (b & 0x80) == 0 {
            return Some((value, pos));
        }
        shift += 7;
        if shift >= 64 {
            return None;
        }
    }
    None
}

/// Parse all top-level fields from a protobuf-encoded byte slice.
/// Only keeps the last occurrence of each field number.
fn read_fields(data: &[u8]) -> std::collections::HashMap<u32, ProtoValue> {
    let mut fields = std::collections::HashMap::new();
    let mut pos = 0;
    while pos < data.len() {
        let (tag, new_pos) = match read_varint(data, pos) {
            Some(v) => v,
            None => break,
        };
        pos = new_pos;
        let field_num = (tag >> 3) as u32;
        let wire_type = (tag & 0x07) as u8;

        match wire_type {
            0 => {
                // Varint
                let (val, new_pos) = match read_varint(data, pos) {
                    Some(v) => v,
                    None => break,
                };
                fields.insert(field_num, ProtoValue::Varint(val));
                pos = new_pos;
            }
            1 => {
                // 64-bit fixed
                if pos + 8 > data.len() {
                    break;
                }
                pos += 8;
            }
            2 => {
                // Length-delimited
                let (len, new_pos) = match read_varint(data, pos) {
                    Some(v) => v,
                    None => break,
                };
                pos = new_pos;
                let len = len as usize;
                if pos + len > data.len() {
                    break;
                }
                fields.insert(
                    field_num,
                    ProtoValue::LengthDelimited(data[pos..pos + len].to_vec()),
                );
                pos += len;
            }
            5 => {
                // 32-bit fixed
                if pos + 4 > data.len() {
                    break;
                }
                pos += 4;
            }
            _ => break,
        }
    }
    fields
}

/// Helper: extract a length-delimited field as bytes.
fn field_bytes(fields: &std::collections::HashMap<u32, ProtoValue>, num: u32) -> Option<&[u8]> {
    match fields.get(&num) {
        Some(ProtoValue::LengthDelimited(data)) => Some(data),
        _ => None,
    }
}

/// Helper: extract a length-delimited field as a UTF-8 string.
fn field_str(fields: &std::collections::HashMap<u32, ProtoValue>, num: u32) -> Option<&str> {
    field_bytes(fields, num).and_then(|b| std::str::from_utf8(b).ok())
}

/// Helper: extract a varint field.
fn field_varint(fields: &std::collections::HashMap<u32, ProtoValue>, num: u32) -> Option<u64> {
    match fields.get(&num) {
        Some(ProtoValue::Varint(v)) => Some(*v),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// DB path discovery
// ---------------------------------------------------------------------------

fn get_state_db_paths() -> Vec<String> {
    let data_dir = if cfg!(target_os = "windows") {
        std::env::var("APPDATA").ok()
    } else if cfg!(target_os = "linux") {
        dirs::config_dir().map(|p| p.to_string_lossy().to_string())
    } else {
        // macOS
        dirs::data_dir().map(|p| p.to_string_lossy().to_string())
    };

    match data_dir {
        Some(d) => {
            let base = std::path::PathBuf::from(d);
            vec![
                base.join("Antigravity")
                    .join("User")
                    .join("globalStorage")
                    .join("state.vscdb")
                    .to_string_lossy()
                    .to_string(),
                base.join("Antigravity IDE")
                    .join("User")
                    .join("globalStorage")
                    .join("state.vscdb")
                    .to_string_lossy()
                    .to_string(),
            ]
        }
        None => vec![],
    }
}

// ---------------------------------------------------------------------------
// SQLite credential reading
// ---------------------------------------------------------------------------

fn read_db_value(db_path: &str, key: &str) -> Option<String> {
    let conn = rusqlite::Connection::open_with_flags(
        db_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .ok()?;

    let mut stmt = conn
        .prepare("SELECT value FROM ItemTable WHERE key = ?1 LIMIT 1")
        .ok()?;
    stmt.query_row([key], |row| row.get(0)).ok()
}

struct OAuthTokens {
    access_token: Option<String>,
    refresh_token: Option<String>,
    expiry_seconds: Option<u64>,
}

/// Unwrap the double-base64 sentinel envelope around OAuth state.
///
/// Layout: b64(outer.f1 = wrapper{ f1=sentinel, f2=payload{ f1=b64(inner proto) } })
fn unwrap_oauth_sentinel(base64_text: &str) -> Option<Vec<u8>> {
    use base64::Engine;
    let trimmed = base64_text.trim();
    if trimmed.is_empty() {
        return None;
    }

    let outer_bytes = base64::engine::general_purpose::STANDARD
        .decode(trimmed)
        .ok()?;
    let outer = read_fields(&outer_bytes);
    let wrapper_bytes = field_bytes(&outer, 1)?;

    let wrapper = read_fields(wrapper_bytes);
    let sentinel = field_str(&wrapper, 1)?;
    if sentinel != OAUTH_TOKEN_SENTINEL && sentinel != AUTH_STATE_SENTINEL {
        return None;
    }
    let payload_bytes = field_bytes(&wrapper, 2)?;

    let payload = read_fields(payload_bytes);
    let inner_b64 = field_str(&payload, 1)?.trim();
    if inner_b64.is_empty() {
        return None;
    }

    base64::engine::general_purpose::STANDARD
        .decode(inner_b64)
        .ok()
}

fn load_oauth_tokens(db_path: &str) -> Option<OAuthTokens> {
    let raw = read_db_value(db_path, OAUTH_TOKEN_KEY)?;
    let inner = unwrap_oauth_sentinel(&raw)?;
    let fields = read_fields(&inner);

    let access_token = field_str(&fields, 1).map(|s| s.to_string());
    let refresh_token = field_str(&fields, 3).map(|s| s.to_string());
    let mut expiry_seconds: Option<u64> = None;

    if let Some(ts_bytes) = field_bytes(&fields, 4) {
        let ts = read_fields(ts_bytes);
        expiry_seconds = field_varint(&ts, 1);
    }

    if access_token.is_none() && refresh_token.is_none() {
        return None;
    }

    Some(OAuthTokens {
        access_token,
        refresh_token,
        expiry_seconds,
    })
}

// ---------------------------------------------------------------------------
// LS discovery
// ---------------------------------------------------------------------------

struct LsDiscovery {
    /// Listening ports for the LS process (excluding the extension server port).
    ls_ports: Vec<u16>,
    /// The --csrf_token value.
    csrf: String,
    /// The --extension_server_port (fallback).
    extension_port: u16,
}

fn extract_flag(command: &str, flag: &str) -> Option<String> {
    let parts: Vec<&str> = command.split_whitespace().collect();
    let flag_eq = format!("{}=", flag);

    for (i, part) in parts.iter().enumerate() {
        if *part == flag {
            if i + 1 < parts.len() {
                return Some(parts[i + 1].to_string());
            }
        } else if part.starts_with(&flag_eq) {
            return Some(part[flag_eq.len()..].to_string());
        }
    }

    None
}

#[cfg(target_os = "windows")]
fn get_powershell_executable_path() -> Option<std::path::PathBuf> {
    let mut candidates = Vec::new();

    for env_key in ["WINDIR", "SystemRoot"] {
        if let Some(root) = std::env::var_os(env_key) {
            let base = std::path::PathBuf::from(root);
            candidates.push(
                base.join("System32")
                    .join("WindowsPowerShell")
                    .join("v1.0")
                    .join("powershell.exe"),
            );
            candidates.push(
                base.join("Sysnative")
                    .join("WindowsPowerShell")
                    .join("v1.0")
                    .join("powershell.exe"),
            );
        }
    }

    for candidate in candidates {
        if candidate.exists() {
            return Some(candidate);
        }
    }

    None
}

#[cfg(target_os = "windows")]
fn run_hidden_powershell(script: &str) -> Option<String> {
    let powershell_path = get_powershell_executable_path()?;
    let output = std::process::Command::new(powershell_path)
        .creation_flags(CREATE_NO_WINDOW)
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            script,
        ])
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

#[cfg(target_os = "windows")]
fn parse_json_items(raw: &str) -> Vec<serde_json::Value> {
    let value = match serde_json::from_str::<serde_json::Value>(raw) {
        Ok(value) => value,
        Err(_) => return Vec::new(),
    };

    match value {
        serde_json::Value::Array(items) => items,
        serde_json::Value::Null => Vec::new(),
        other => vec![other],
    }
}

#[cfg(target_os = "windows")]
fn parse_ports(raw: &str) -> Vec<u16> {
    let mut ports = BTreeSet::new();
    for item in parse_json_items(raw) {
        if let Some(port) = item.as_u64().and_then(|v| u16::try_from(v).ok()) {
            ports.insert(port);
        }
    }
    ports.into_iter().collect()
}

fn discover_ls() -> Option<LsDiscovery> {
    #[cfg(target_os = "windows")]
    {
        discover_windows_ls()
    }

    #[cfg(target_os = "linux")]
    {
        discover_linux_ls()
    }

    #[cfg(target_os = "macos")]
    {
        None
    }
}

#[cfg(target_os = "windows")]
fn discover_windows_ls() -> Option<LsDiscovery> {
    let process_json = run_hidden_powershell(
        "& { $procs = @(Get-CimInstance Win32_Process | Where-Object { $_.CommandLine -like '*language_server*' -and $_.CommandLine -like '*antigravity*' } | Select-Object ProcessId, CommandLine); if ($procs.Count -eq 0) { '[]' } else { $procs | ConvertTo-Json -Compress } }",
    )?;

    let processes = parse_json_items(&process_json);

    for item in processes {
        let process_id = item
            .get("ProcessId")
            .and_then(|v| v.as_u64())
            .map(|v| v as u32);
        let command = item
            .get("CommandLine")
            .and_then(|v| v.as_str())
            .map(|v| v.to_string());

        let (process_id, command) = match (process_id, command) {
            (Some(pid), Some(cmd)) => (pid, cmd),
            _ => continue,
        };

        let csrf = match extract_flag(&command, "--csrf_token") {
            Some(c) => c,
            None => continue,
        };
        let ext_port_str = match extract_flag(&command, "--extension_server_port") {
            Some(p) => p,
            None => continue,
        };
        let extension_port: u16 = match ext_port_str.parse() {
            Ok(p) if p > 0 => p,
            _ => continue,
        };

        // Discover all listening ports for this process.
        let port_script = format!(
            "& {{ $ports = @(Get-NetTCPConnection -OwningProcess {} -State Listen -ErrorAction SilentlyContinue | Select-Object -ExpandProperty LocalPort); if ($ports.Count -eq 0) {{ '[]' }} else {{ $ports | ConvertTo-Json -Compress }} }}",
            process_id
        );
        let ports_json = run_hidden_powershell(&port_script).unwrap_or_else(|| "[]".into());
        let mut ls_ports: Vec<u16> = parse_ports(&ports_json)
            .into_iter()
            .filter(|&p| p != extension_port)
            .collect();

        // If no other ports found, the list stays empty and we'll fall back to extension_port.
        if ls_ports.is_empty() {
            // Try extension port as last resort (handled in find_working_port).
        }
        let _ = &mut ls_ports; // suppress unused warning

        return Some(LsDiscovery {
            ls_ports,
            csrf,
            extension_port,
        });
    }

    None
}

#[cfg(target_os = "linux")]
fn get_ps_executable_path() -> Option<std::path::PathBuf> {
    for candidate in ["/usr/bin/ps", "/bin/ps"] {
        let path = std::path::PathBuf::from(candidate);
        if path.exists() {
            return Some(path);
        }
    }
    None
}

#[cfg(target_os = "linux")]
fn discover_linux_ls() -> Option<LsDiscovery> {
    let ps_path = get_ps_executable_path()?;
    let output = std::process::Command::new(ps_path)
        .args(["aux"])
        .output()
        .ok()?;

    let stdout = String::from_utf8_lossy(&output.stdout);

    for line in stdout.lines() {
        if !line.contains("language_server") {
            continue;
        }
        let lower = line.to_lowercase();
        if !lower.contains("antigravity") {
            continue;
        }

        let csrf = match extract_flag(line, "--csrf_token") {
            Some(c) => c,
            None => continue,
        };
        let ext_port_str = match extract_flag(line, "--extension_server_port") {
            Some(p) => p,
            None => continue,
        };
        let extension_port: u16 = match ext_port_str.parse() {
            Ok(p) if p > 0 => p,
            _ => continue,
        };

        // On Linux, port discovery via lsof is possible but keep it simple for now.
        return Some(LsDiscovery {
            ls_ports: vec![],
            csrf,
            extension_port,
        });
    }

    None
}

// ---------------------------------------------------------------------------
// Local HTTP helpers for LS calls
// ---------------------------------------------------------------------------

fn build_local_client(accept_invalid_certs: bool) -> Result<reqwest::blocking::Client, String> {
    reqwest::blocking::Client::builder()
        .danger_accept_invalid_certs(accept_invalid_certs)
        .build()
        .map_err(|e| format!("HTTP client error: {}", e))
}

fn probe_port(
    client: &reqwest::blocking::Client,
    scheme: &str,
    port: u16,
    csrf: &str,
) -> bool {
    let url = format!(
        "{}://127.0.0.1:{}/{}/GetUnleashData",
        scheme, port, LS_SERVICE
    );

    let body = serde_json::json!({
        "context": {
            "properties": {
                "devMode": "false",
                "extensionVersion": "unknown",
                "ide": "antigravity",
                "ideVersion": "unknown",
                "os": std::env::consts::OS,
            }
        }
    });

    // Any response (even error status) means the port is alive.
    client
        .post(&url)
        .header("Content-Type", "application/json")
        .header("Connect-Protocol-Version", "1")
        .header("x-codeium-csrf-token", csrf)
        .json(&body)
        .timeout(std::time::Duration::from_secs(5))
        .send()
        .is_ok()
}

fn find_working_port(
    discovery: &LsDiscovery,
) -> Option<(u16, &'static str)> {
    let client = build_local_client(true).ok()?;

    // Try all discovered LS ports first (these are NOT the extension server port).
    for &port in &discovery.ls_ports {
        for scheme in ["http", "https"] {
            if probe_port(&client, scheme, port, &discovery.csrf) {
                return Some((port, scheme));
            }
        }
    }

    // Fall back to the extension server port.
    for scheme in ["http", "https"] {
        if probe_port(&client, scheme, discovery.extension_port, &discovery.csrf) {
            return Some((discovery.extension_port, scheme));
        }
    }

    None
}

fn call_ls(
    client: &reqwest::blocking::Client,
    port: u16,
    scheme: &str,
    csrf: &str,
    method: &str,
    body: &serde_json::Value,
) -> Option<serde_json::Value> {
    let url = format!(
        "{}://127.0.0.1:{}/{}/{}",
        scheme, port, LS_SERVICE, method
    );

    let resp = client
        .post(&url)
        .header("Content-Type", "application/json")
        .header("Connect-Protocol-Version", "1")
        .header("x-codeium-csrf-token", csrf)
        .json(body)
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .ok()?;

    let status = resp.status().as_u16();
    if status < 200 || status >= 300 {
        return None;
    }

    resp.json::<serde_json::Value>().ok()
}

// ---------------------------------------------------------------------------
// LS probe — GetUserStatus then GetCommandModelConfigs
// ---------------------------------------------------------------------------

struct LsProbeResult {
    plan: Option<String>,
    lines: Vec<MetricLine>,
}

fn probe_ls() -> Option<LsProbeResult> {
    let discovery = discover_ls()?;
    let (port, scheme) = find_working_port(&discovery)?;

    let client = build_local_client(true).ok()?;

    let metadata = serde_json::json!({
        "ideName": "antigravity",
        "extensionName": "antigravity",
        "ideVersion": "unknown",
        "locale": "en",
    });

    // Try GetUserStatus first
    let data = call_ls(
        &client,
        port,
        scheme,
        &discovery.csrf,
        "GetUserStatus",
        &serde_json::json!({ "metadata": metadata }),
    );

    let has_user_status = data
        .as_ref()
        .and_then(|d| d.get("userStatus"))
        .is_some();

    let data = if has_user_status {
        data
    } else {
        // Fall back to GetCommandModelConfigs
        call_ls(
            &client,
            port,
            scheme,
            &discovery.csrf,
            "GetCommandModelConfigs",
            &serde_json::json!({ "metadata": metadata }),
        )
    };

    let data = data?;

    // Parse model configs
    let configs = if has_user_status {
        data.pointer("/userStatus/cascadeModelConfigData/clientModelConfigs")
    } else {
        data.get("clientModelConfigs")
    };

    let configs = configs?.as_array()?;

    let model_configs: Vec<ModelConfig> = configs
        .iter()
        .filter_map(|c| {
            let label = c.get("label")?.as_str()?.trim().to_string();
            if label.is_empty() {
                return None;
            }
            let model_id = c
                .pointer("/modelOrAlias/model")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if MODEL_BLACKLIST.contains(&model_id) {
                return None;
            }
            let remaining_fraction = c
                .pointer("/quotaInfo/remainingFraction")
                .and_then(|v| v.as_f64())
                .unwrap_or(0.0);
            let reset_time = c
                .pointer("/quotaInfo/resetTime")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            Some(ModelConfig {
                label,
                remaining_fraction,
                reset_time,
            })
        })
        .collect();

    let lines = build_model_lines(&model_configs);
    if lines.is_empty() {
        return None;
    }

    // Extract plan name
    let plan = if has_user_status {
        let user_tier_name = data
            .pointer("/userStatus/userTier/name")
            .and_then(|v| v.as_str())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());

        if user_tier_name.is_some() {
            user_tier_name
        } else {
            data.pointer("/userStatus/planStatus/planInfo/planName")
                .and_then(|v| v.as_str())
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
        }
    } else {
        None
    };

    Some(LsProbeResult { plan, lines })
}

// ---------------------------------------------------------------------------
// Model line helpers
// ---------------------------------------------------------------------------

struct ModelConfig {
    label: String,
    remaining_fraction: f64,
    reset_time: Option<String>,
}

fn normalize_label(label: &str) -> String {
    // "Gemini 3 Pro (High)" -> "Gemini 3 Pro"
    let re = regex_lite::Regex::new(r"\s*\([^)]*\)\s*$").unwrap();
    re.replace(label, "").trim().to_string()
}

fn pool_label(normalized: &str) -> &'static str {
    let lower = normalized.to_lowercase();
    if lower.contains("gemini") && lower.contains("pro") {
        return "Gemini Pro";
    }
    if lower.contains("gemini") && lower.contains("flash") {
        return "Gemini Flash";
    }
    "Claude"
}

fn model_sort_key(label: &str) -> String {
    let lower = label.to_lowercase();
    if lower.contains("gemini") && lower.contains("pro") {
        return format!("0a_{}", label);
    }
    if lower.contains("gemini") {
        return format!("0b_{}", label);
    }
    if lower.contains("claude") && lower.contains("opus") {
        return format!("1a_{}", label);
    }
    if lower.contains("claude") {
        return format!("1b_{}", label);
    }
    format!("2_{}", label)
}

fn build_model_lines(configs: &[ModelConfig]) -> Vec<MetricLine> {
    let mut deduped: std::collections::HashMap<&'static str, (&'static str, f64, Option<String>)> =
        std::collections::HashMap::new();

    for config in configs {
        let label = config.label.trim();
        if label.is_empty() {
            continue;
        }
        let fraction = if config.remaining_fraction.is_finite() {
            config.remaining_fraction
        } else {
            0.0
        };
        let pool = pool_label(&normalize_label(label));

        let entry = deduped.entry(pool).or_insert((pool, fraction, config.reset_time.clone()));
        if fraction < entry.1 {
            *entry = (pool, fraction, config.reset_time.clone());
        }
    }

    let mut models: Vec<_> = deduped.into_values().collect();
    models.sort_by(|a, b| model_sort_key(a.0).cmp(&model_sort_key(b.0)));

    models
        .into_iter()
        .map(|(label, remaining_fraction, reset_time)| {
            let clamped = remaining_fraction.clamp(0.0, 1.0);
            let used = ((1.0 - clamped) * 100.0).round();
            MetricLine::Progress {
                label: label.to_string(),
                used,
                limit: 100.0,
                format: MetricFormat {
                    kind: "percent".into(),
                    suffix: None,
                },
                resets_at: reset_time,
            }
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Cloud Code API (token-based fallback when LS is not running)
// ---------------------------------------------------------------------------

fn refresh_access_token(
    client: &reqwest::blocking::Client,
    refresh_token: &str,
) -> Option<String> {
    let client_id = GOOGLE_CLIENT_ID.filter(|s| !s.trim().is_empty())?;
    let client_secret = GOOGLE_CLIENT_SECRET.filter(|s| !s.trim().is_empty())?;

    let body = format!(
        "client_id={}&client_secret={}&refresh_token={}&grant_type=refresh_token",
        urlencoding::encode(client_id),
        urlencoding::encode(client_secret),
        urlencoding::encode(refresh_token),
    );

    let resp = client
        .post(GOOGLE_OAUTH_URL)
        .header("Content-Type", "application/x-www-form-urlencoded")
        .body(body)
        .timeout(std::time::Duration::from_secs(15))
        .send()
        .ok()?;

    if !resp.status().is_success() {
        return None;
    }

    let data: serde_json::Value = resp.json().ok()?;
    data.get("access_token")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
}

/// Returns Ok(json) on success, Err(true) on auth failure, Err(false) on other errors.
fn probe_cloud_code(
    client: &reqwest::blocking::Client,
    token: &str,
) -> Result<serde_json::Value, bool> {
    for base_url in CLOUD_CODE_URLS {
        let url = format!("{}{}", base_url, FETCH_MODELS_PATH);
        match client
            .post(&url)
            .header("Authorization", format!("Bearer {}", token))
            .header("Content-Type", "application/json")
            .header("User-Agent", "antigravity")
            .body("{}")
            .timeout(std::time::Duration::from_secs(15))
            .send()
        {
            Ok(resp) => {
                let status = resp.status().as_u16();
                if status == 401 || status == 403 {
                    return Err(true); // auth failure
                }
                if status >= 200 && status < 300 {
                    if let Ok(json) = resp.json::<serde_json::Value>() {
                        return Ok(json);
                    }
                }
            }
            Err(_) => continue,
        }
    }
    Err(false)
}

fn parse_cloud_code_models(data: &serde_json::Value) -> Vec<ModelConfig> {
    let models = match data.get("models").and_then(|v| v.as_object()) {
        Some(m) => m,
        None => return vec![],
    };

    let mut configs = Vec::new();
    for (key, model) in models {
        if model.get("isInternal").and_then(|v| v.as_bool()).unwrap_or(false) {
            continue;
        }
        let model_id = model
            .get("model")
            .and_then(|v| v.as_str())
            .unwrap_or(key.as_str());
        if MODEL_BLACKLIST.contains(&model_id) {
            continue;
        }
        let display_name = model
            .get("displayName")
            .and_then(|v| v.as_str())
            .map(|s| s.trim())
            .filter(|s| !s.is_empty());
        let display_name = match display_name {
            Some(n) => n.to_string(),
            None => continue,
        };
        let remaining_fraction = model
            .pointer("/quotaInfo/remainingFraction")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);
        let reset_time = model
            .pointer("/quotaInfo/resetTime")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        configs.push(ModelConfig {
            label: display_name,
            remaining_fraction,
            reset_time,
        });
    }
    configs
}

// ---------------------------------------------------------------------------
// Main probe entry point
// ---------------------------------------------------------------------------

pub fn probe() -> Result<(Option<String>, Vec<MetricLine>), String> {
    // --- Strategy 1: LS probe (returns model data directly, no token needed) ---
    if let Some(result) = probe_ls() {
        return Ok((result.plan, result.lines));
    }

    // --- Strategy 2: Cloud Code API with tokens from the DB ---
    let db_paths = get_state_db_paths();
    let mut token_candidates: Vec<OAuthTokens> = Vec::new();

    for db_path in &db_paths {
        if std::path::Path::new(db_path).exists() {
            if let Some(tokens) = load_oauth_tokens(db_path) {
                token_candidates.push(tokens);
            }
        }
    }

    if token_candidates.is_empty() {
        return Err("Antigravity not installed or not signed in.".into());
    }

    let client = reqwest::blocking::Client::builder()
        .build()
        .map_err(|e| format!("HTTP client error: {}", e))?;

    // Collect valid access tokens
    let now_seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let mut tokens: Vec<String> = Vec::new();
    for candidate in &token_candidates {
        if let Some(ref at) = candidate.access_token {
            let expired = candidate
                .expiry_seconds
                .map(|exp| exp <= now_seconds)
                .unwrap_or(false);
            if !expired && !tokens.contains(at) {
                tokens.push(at.clone());
            }
        }
    }

    if tokens.is_empty() && token_candidates.iter().all(|c| c.refresh_token.is_none()) {
        return Err("Start Antigravity and try again.".into());
    }

    let mut cloud_data: Option<serde_json::Value> = None;
    let mut saw_auth_failure = false;

    for token in &tokens {
        match probe_cloud_code(&client, token) {
            Ok(data) => {
                cloud_data = Some(data);
                break;
            }
            Err(true) => {
                saw_auth_failure = true;
            }
            Err(false) => {}
        }
    }

    // Refresh token if needed
    if cloud_data.is_none() && (saw_auth_failure || tokens.is_empty()) {
        let mut tried_refresh: Vec<String> = Vec::new();
        for candidate in &token_candidates {
            if let Some(ref rt) = candidate.refresh_token {
                if tried_refresh.contains(rt) {
                    continue;
                }
                tried_refresh.push(rt.clone());

                if let Some(new_token) = refresh_access_token(&client, rt) {
                    match probe_cloud_code(&client, &new_token) {
                        Ok(data) => {
                            cloud_data = Some(data);
                            break;
                        }
                        Err(_) => {}
                    }
                }
            }
        }
    }

    if let Some(data) = cloud_data {
        let configs = parse_cloud_code_models(&data);
        let lines = build_model_lines(&configs);
        if !lines.is_empty() {
            return Ok((None, lines));
        }
    }

    Err("Start Antigravity and try again.".into())
}
