use serde::{Deserialize, Serialize};

/// Represents a single usage metric line
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum MetricLine {
    #[serde(rename = "progress")]
    Progress {
        label: String,
        used: f64,
        limit: f64,
        format: MetricFormat,
        #[serde(rename = "resetsAt")]
        #[serde(skip_serializing_if = "Option::is_none")]
        resets_at: Option<String>,
    },
    #[serde(rename = "text")]
    Text { label: String, value: String },
    #[serde(rename = "badge")]
    Badge {
        label: String,
        text: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        color: Option<String>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MetricFormat {
    pub kind: String, // "percent", "dollars", "count"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suffix: Option<String>,
}

/// Result from probing a provider
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderResult {
    pub id: String,
    pub name: String,
    pub icon: String,
    pub brand_color: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plan: Option<String>,
    pub lines: Vec<MetricLine>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Provider metadata for the frontend
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderMeta {
    pub id: String,
    pub name: String,
    pub icon: String,
    pub brand_color: String,
}

pub mod antigravity;
pub mod claude;
pub mod codex;
pub mod copilot;
pub mod cursor;
pub mod windsurf;

/// Get metadata for all supported providers
pub fn list_providers() -> Vec<ProviderMeta> {
    vec![
        ProviderMeta {
            id: "cursor".into(),
            name: "Cursor".into(),
            icon: "cursor".into(),
            brand_color: "#000000".into(),
        },
        ProviderMeta {
            id: "claude".into(),
            name: "Claude".into(),
            icon: "claude".into(),
            brand_color: "#D97757".into(),
        },
        ProviderMeta {
            id: "copilot".into(),
            name: "Copilot".into(),
            icon: "copilot".into(),
            brand_color: "#000000".into(),
        },
        ProviderMeta {
            id: "codex".into(),
            name: "Codex".into(),
            icon: "codex".into(),
            brand_color: "#000000".into(),
        },
        ProviderMeta {
            id: "windsurf".into(),
            name: "Windsurf".into(),
            icon: "windsurf".into(),
            brand_color: "#00B4D8".into(),
        },
        ProviderMeta {
            id: "antigravity".into(),
            name: "Antigravity".into(),
            icon: "antigravity".into(),
            brand_color: "#4285F4".into(),
        },
    ]
}

/// Probe a specific provider and return its usage data
pub fn probe_provider(id: &str) -> ProviderResult {
    let meta = list_providers();
    let provider_meta = meta.iter().find(|p| p.id == id);

    let (name, icon, brand_color) = match provider_meta {
        Some(m) => (m.name.clone(), m.icon.clone(), m.brand_color.clone()),
        None => {
            return ProviderResult {
                id: id.to_string(),
                name: id.to_string(),
                icon: String::new(),
                brand_color: "#666".into(),
                plan: None,
                lines: vec![],
                error: Some(format!("Unknown provider: {}", id)),
            }
        }
    };

    let result = match id {
        "cursor" => cursor::probe(),
        "claude" => claude::probe(),
        "copilot" => copilot::probe(),
        "codex" => codex::probe(),
        "windsurf" => windsurf::probe(),
        "antigravity" => antigravity::probe(),
        _ => Err(format!("Unknown provider: {}", id)),
    };

    match result {
        Ok((plan, lines)) => ProviderResult {
            id: id.to_string(),
            name,
            icon,
            brand_color,
            plan,
            lines,
            error: None,
        },
        Err(e) => ProviderResult {
            id: id.to_string(),
            name,
            icon,
            brand_color,
            plan: None,
            lines: vec![],
            error: Some(e),
        },
    }
}
