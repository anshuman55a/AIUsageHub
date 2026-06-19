import { useState, useEffect, useCallback, useRef, type CSSProperties } from "react";
import { invoke } from "@tauri-apps/api/core";
import { RefreshCw, AlertCircle, ChevronDown, Settings, X } from "lucide-react";
import { PROVIDER_ICONS } from "./ProviderIcons";
import "./App.css";

const AUTO_REFRESH_ENABLED_KEY = "usagedock:autoRefreshEnabled";
const AUTO_REFRESH_MINUTES_KEY = "usagedock:autoRefreshMinutes";
const AUTO_REFRESH_OPTIONS = [5, 10, 15, 30, 60] as const;

// Types matching the Rust backend
interface MetricFormat {
  kind: "percent" | "dollars" | "count";
  suffix?: string;
}

interface ProgressLine {
  type: "progress";
  label: string;
  used: number;
  limit: number;
  format: MetricFormat;
  resetsAt?: string | null;
}

interface TextLine {
  type: "text";
  label: string;
  value: string;
}

interface BadgeLine {
  type: "badge";
  label: string;
  text: string;
  color?: string;
}

type MetricLine = ProgressLine | TextLine | BadgeLine;

interface ProviderResult {
  id: string;
  name: string;
  icon: string;
  brandColor: string;
  plan?: string | null;
  lines: MetricLine[];
  error?: string | null;
}

interface AppUpdateInfo {
  version: string;
  currentVersion: string;
  notes?: string | null;
  publishedAt?: string | null;
}

// Provider brand colors
const PROVIDER_STYLES: Record<string, { bg: string }> = {
  cursor: { bg: "#000000" },
  claude: { bg: "#D97757" },
  copilot: { bg: "#000000" },
  codex: { bg: "#000000" },
  windsurf: { bg: "#00B4D8" },
  antigravity: { bg: "#4285F4" },
  ollama: { bg: "#000000" },
};

function BoltIcon({ className }: { className?: string }) {
  return (
    <svg className={className} viewBox="0 0 24 24" fill="none" aria-hidden="true">
      <path
        d="M13.5 1.5 5 13h5l-1.5 9.5L19 10h-5.25L13.5 1.5Z"
        fill="currentColor"
        stroke="currentColor"
        strokeLinejoin="round"
        strokeWidth="1"
      />
    </svg>
  );
}

function formatValue(used: number, limit: number, format: MetricFormat): string {
  switch (format.kind) {
    case "percent":
      return `${Math.round(used)}%`;
    case "dollars":
      return `$${used.toFixed(2)} / $${limit.toFixed(2)}`;
    case "count":
      return `${Math.round(used)} / ${Math.round(limit)} ${format.suffix || ""}`;
    default:
      return `${used} / ${limit}`;
  }
}

function getProgressColor(pct: number): string {
  if (pct < 50) return "#22c55e";
  if (pct < 75) return "#f59e0b";
  if (pct < 90) return "#f97316";
  return "#ef4444";
}

function timeUntilReset(isoStr: string): string {
  const trimmed = isoStr.trim();
  const numericReset = /^\d+$/.test(trimmed) ? Number(trimmed) : null;
  const resetMs = numericReset !== null
    ? (trimmed.length <= 10 ? numericReset * 1000 : numericReset)
    : new Date(trimmed).getTime();
  const now = Date.now();
  if (Number.isNaN(resetMs)) return "reset time unavailable";
  const diffMs = resetMs - now;
  if (diffMs <= 0) return "resetting...";
  const days = Math.floor(diffMs / 86400000);
  const hours = Math.floor((diffMs % 86400000) / 3600000);
  if (days > 0) return `Resets in ${days}d ${hours}h`;
  const mins = Math.floor((diffMs % 3600000) / 60000);
  if (hours > 0) return `Resets in ${hours}h ${mins}m`;
  return `Resets in ${mins}m`;
}

function getErrorMessage(error: unknown, fallback: string): string {
  if (typeof error === "string" && error.trim()) return error;
  if (error instanceof Error && error.message.trim()) return error.message;
  return fallback;
}

function summarizeUpdateNotes(notes?: string | null): string | null {
  if (!notes) return null;
  const firstLine = notes
    .split(/\r?\n/)
    .map((line) => line.trim())
    .find(Boolean);

  if (!firstLine) return null;
  return firstLine.length > 92 ? `${firstLine.slice(0, 89)}...` : firstLine;
}

function getSharedResetLabel(lines: MetricLine[]): string | null {
  const resetValues = lines
    .filter((line): line is ProgressLine => line.type === "progress" && Boolean(line.resetsAt))
    .map((line) => line.resetsAt as string);

  if (resetValues.length === 0) {
    return null;
  }

  const uniqueValues = [...new Set(resetValues)];
  if (uniqueValues.length !== 1) {
    return null;
  }

  return timeUntilReset(uniqueValues[0]);
}

// Progress Bar Component
function ProgressMetric({ line, hideReset }: { line: ProgressLine; hideReset?: boolean }) {
  const pct = line.limit > 0 ? Math.min((line.used / line.limit) * 100, 100) : 0;
  const color = getProgressColor(pct);

  return (
    <div className="progress-metric">
      <div className="progress-header">
        <span className="progress-label">{line.label}</span>
        <span className="progress-value">{formatValue(line.used, line.limit, line.format)}</span>
      </div>
      <div className="progress-track">
        <div
          className="progress-fill"
          style={{
            width: `${pct}%`,
            background: `linear-gradient(90deg, ${color}cc, ${color})`,
          }}
        />
      </div>
      {!hideReset && line.resetsAt && (
        <span className="progress-subtitle">{timeUntilReset(line.resetsAt)}</span>
      )}
    </div>
  );
}

// Badge Component
function BadgeMetric({ line }: { line: BadgeLine }) {
  const color = line.color || "#7da88c";
  return (
    <div className="badge-line">
      <span className="badge-label">{line.label}</span>
      <span className="badge-chip" style={{ color, borderColor: `${color}40` }}>
        {line.text}
      </span>
    </div>
  );
}

// Text Component
function TextMetric({ line }: { line: TextLine }) {
  return (
    <div className="text-metric">
      <span className="text-label">{line.label}</span>
      <span className="text-value">{line.value}</span>
    </div>
  );
}

// Provider Card Component
function ProviderCard({
  provider,
  onRefresh,
  isRefreshing,
}: {
  provider: ProviderResult;
  onRefresh: () => void;
  isRefreshing: boolean;
}) {
  const style = PROVIDER_STYLES[provider.id] || { bg: "#666" };
  const IconComponent = PROVIDER_ICONS[provider.id];
  const accent = provider.brandColor || style.bg;
  const progressLineCount = provider.lines.filter((line) => line.type === "progress").length;
  const sharedResetLabel = provider.error ? null : getSharedResetLabel(provider.lines);
  const providerCaption = provider.error
    ? "Connection needs attention"
    : isRefreshing
      ? "Refreshing usage..."
      : sharedResetLabel
        ? sharedResetLabel
        : provider.lines.length === 0
          ? "Waiting for usage signals"
          : null;
  const providerCardStyle = {
    "--provider-accent": accent,
    "--provider-accent-soft": `${accent}20`,
  } as CSSProperties;

  return (
    <div
      className={`provider-card ${provider.error ? "provider-card-error" : ""} ${progressLineCount > 1 ? "provider-card-dense-metrics" : ""}`}
      style={providerCardStyle}
    >
      <div className="provider-card-header">
        <div className="provider-info">
          <div className="provider-icon" style={{ background: style.bg }}>
            {IconComponent ? <IconComponent /> : "?"}
          </div>
          <div className="provider-copy">
            <div className="provider-name-row">
              <div className="provider-name">{provider.name}</div>
              {provider.plan && <div className="provider-plan">{provider.plan}</div>}
            </div>
            {providerCaption && <div className="provider-caption">{providerCaption}</div>}
          </div>
        </div>
        <button
          className={`btn-icon provider-refresh ${isRefreshing ? "spinning" : ""}`}
          onClick={onRefresh}
          title="Refresh"
          aria-label={`Refresh ${provider.name}`}
        >
          <RefreshCw />
        </button>
      </div>

      {isRefreshing && (
        <div className="provider-loading">
          <div className="skeleton" style={{ width: "100%", marginBottom: 6 }} />
          <div className="skeleton" style={{ width: "60%" }} />
        </div>
      )}

      {!isRefreshing && provider.error && (
        <div className="provider-error">
          <div className="error-msg">
            <AlertCircle />
            <span>{provider.error}</span>
          </div>
        </div>
      )}

      {!isRefreshing && !provider.error && provider.lines.length > 0 && (
        <div className="metric-lines">
          {provider.lines.map((line, i) => {
            switch (line.type) {
              case "progress":
                return <ProgressMetric key={i} line={line} hideReset={!!sharedResetLabel} />;
              case "text":
                return <TextMetric key={i} line={line} />;
              case "badge":
                return <BadgeMetric key={i} line={line} />;
              default:
                return null;
            }
          })}
        </div>
      )}
    </div>
  );
}

// Main App
function App() {
  const [providers, setProviders] = useState<ProviderResult[]>([]);
  const [refreshing, setRefreshing] = useState<Set<string>>(new Set());
  const [lastRefresh, setLastRefresh] = useState<Date | null>(null);
  const [isLoading, setIsLoading] = useState(true);
  const [showUnavailable, setShowUnavailable] = useState(false);
  const [updaterEnabled, setUpdaterEnabled] = useState(false);
  const [updateInfo, setUpdateInfo] = useState<AppUpdateInfo | null>(null);
  const [isCheckingUpdate, setIsCheckingUpdate] = useState(false);
  const [isInstallingUpdate, setIsInstallingUpdate] = useState(false);
  const [updateError, setUpdateError] = useState<string | null>(null);
  const [autoRefreshEnabled, setAutoRefreshEnabled] = useState(() => {
    if (typeof window === "undefined") return true;
    const stored = window.localStorage.getItem(AUTO_REFRESH_ENABLED_KEY);
    return stored === null ? true : stored === "true";
  });
  const [autoRefreshMinutes, setAutoRefreshMinutes] = useState<number>(() => {
    if (typeof window === "undefined") return 15;
    const stored = Number(window.localStorage.getItem(AUTO_REFRESH_MINUTES_KEY));
    return AUTO_REFRESH_OPTIONS.includes(stored as (typeof AUTO_REFRESH_OPTIONS)[number]) ? stored : 15;
  });
  const [isIntervalMenuOpen, setIsIntervalMenuOpen] = useState(false);
  const [isSettingsOpen, setIsSettingsOpen] = useState(false);
  const [isSettingsClosing, setIsSettingsClosing] = useState(false);
  const intervalMenuRef = useRef<HTMLDivElement>(null);
  const settingsPanelRef = useRef<HTMLDivElement>(null);
  const settingsBtnRef = useRef<HTMLButtonElement>(null);
  const settingsCloseTimeoutRef = useRef<number | null>(null);
  const [ollamaUrl, setOllamaUrl] = useState("http://localhost:11434");
  const [ollamaApiKey, setOllamaApiKey] = useState("");
  const [ollamaSaved, setOllamaSaved] = useState(false);
  const availableProviders = providers.filter((provider) => !provider.error && provider.lines.length > 0);
  const unavailableProviders = providers.filter((provider) => provider.error || provider.lines.length === 0);
  const unavailableCaption = availableProviders.length > 0
    ? `${unavailableProviders.length} hidden until needed`
    : `${unavailableProviders.length} provider${unavailableProviders.length === 1 ? "" : "s"} need attention`;
  const connectedProviders = providers.filter((provider) => !provider.error && provider.lines.length > 0).length;
  const statusText = isLoading
    ? "Refreshing local usage"
    : providers.length === 0
      ? "Waiting for connected tools"
      : `${connectedProviders} of ${providers.length} providers reporting`;

  const refreshAll = useCallback(async () => {
    setIsLoading(true);
    try {
      const results = await invoke<ProviderResult[]>("probe_all");
      setProviders(results);
      setLastRefresh(new Date());
    } catch (e) {
      console.error("Failed to probe providers:", e);
    } finally {
      setIsLoading(false);
    }
  }, []);

  const refreshSingle = useCallback(async (id: string) => {
    setRefreshing((prev) => new Set(prev).add(id));
    try {
      const result = await invoke<ProviderResult>("probe", { id });
      setProviders((prev) =>
        prev.map((p) => (p.id === id ? result : p))
      );
    } catch (e) {
      console.error(`Failed to probe ${id}:`, e);
    } finally {
      setRefreshing((prev) => {
        const next = new Set(prev);
        next.delete(id);
        return next;
      });
    }
  }, []);

  const checkForUpdates = useCallback(async (showErrors = true) => {
    setIsCheckingUpdate(true);
    setUpdateError(null);

    try {
      const enabled = await invoke<boolean>("updater_enabled_command");
      setUpdaterEnabled(enabled);

      if (!enabled) {
        setUpdateInfo(null);
        return;
      }

      const availableUpdate = await invoke<AppUpdateInfo | null>("check_for_app_update");
      setUpdateInfo(availableUpdate);
    } catch (error) {
      console.error("Failed to check for updates:", error);
      setUpdateInfo(null);
      if (showErrors) {
        setUpdateError(getErrorMessage(error, "Could not check for updates right now."));
      }
    } finally {
      setIsCheckingUpdate(false);
    }
  }, []);

  const installUpdate = useCallback(async () => {
    setIsInstallingUpdate(true);
    setUpdateError(null);

    try {
      await invoke("install_app_update");
    } catch (error) {
      console.error("Failed to install update:", error);
      setUpdateError(getErrorMessage(error, "Could not install the latest release."));
    } finally {
      setIsInstallingUpdate(false);
    }
  }, []);

  const closeSettings = useCallback(() => {
    setIsIntervalMenuOpen(false);
    setIsSettingsClosing(true);
    setIsSettingsOpen(false);

    if (settingsCloseTimeoutRef.current !== null) {
      window.clearTimeout(settingsCloseTimeoutRef.current);
    }

    settingsCloseTimeoutRef.current = window.setTimeout(() => {
      setIsSettingsClosing(false);
      settingsCloseTimeoutRef.current = null;
    }, 220);
  }, []);

  const openSettings = useCallback(() => {
    if (settingsCloseTimeoutRef.current !== null) {
      window.clearTimeout(settingsCloseTimeoutRef.current);
      settingsCloseTimeoutRef.current = null;
    }
    setIsSettingsClosing(false);
    setIsSettingsOpen(true);
  }, []);

  useEffect(() => {
    refreshAll();
  }, [refreshAll]);

  useEffect(() => {
    checkForUpdates(false);
  }, [checkForUpdates]);

  useEffect(() => {
    invoke<{ url: string; apiKey: string }>("get_ollama_settings")
      .then((s) => {
        setOllamaUrl(s.url || "http://localhost:11434");
        setOllamaApiKey(s.apiKey || "");
      })
      .catch(() => {});
  }, []);

  useEffect(() => {
    window.localStorage.setItem(AUTO_REFRESH_ENABLED_KEY, String(autoRefreshEnabled));
  }, [autoRefreshEnabled]);

  useEffect(() => {
    window.localStorage.setItem(AUTO_REFRESH_MINUTES_KEY, String(autoRefreshMinutes));
  }, [autoRefreshMinutes]);

  useEffect(() => {
    if (!autoRefreshEnabled) {
      return;
    }

    const interval = window.setInterval(refreshAll, autoRefreshMinutes * 60 * 1000);
    return () => window.clearInterval(interval);
  }, [autoRefreshEnabled, autoRefreshMinutes, refreshAll]);

  useEffect(() => {
    if (availableProviders.length === 0 && unavailableProviders.length > 0) {
      setShowUnavailable(true);
    }
  }, [availableProviders.length, unavailableProviders.length]);

  useEffect(() => {
    if (!isIntervalMenuOpen) {
      return;
    }

    function handlePointerDown(event: MouseEvent) {
      if (!intervalMenuRef.current?.contains(event.target as Node)) {
        setIsIntervalMenuOpen(false);
      }
    }

    function handleEscape(event: KeyboardEvent) {
      if (event.key === "Escape") {
        setIsIntervalMenuOpen(false);
      }
    }

    window.addEventListener("mousedown", handlePointerDown);
    window.addEventListener("keydown", handleEscape);
    return () => {
      window.removeEventListener("mousedown", handlePointerDown);
      window.removeEventListener("keydown", handleEscape);
    };
  }, [isIntervalMenuOpen]);

  useEffect(() => {
    if (!isSettingsOpen && !isSettingsClosing) {
      return;
    }

    function handlePointerDown(event: MouseEvent) {
      if (
        settingsPanelRef.current &&
        !settingsPanelRef.current.contains(event.target as Node) &&
        settingsBtnRef.current &&
        !settingsBtnRef.current.contains(event.target as Node)
      ) {
        closeSettings();
      }
    }

    function handleEscape(event: KeyboardEvent) {
      if (event.key === "Escape") {
        closeSettings();
      }
    }

    window.addEventListener("mousedown", handlePointerDown);
    window.addEventListener("keydown", handleEscape);
    return () => {
      window.removeEventListener("mousedown", handlePointerDown);
      window.removeEventListener("keydown", handleEscape);
    };
  }, [closeSettings, isSettingsClosing, isSettingsOpen]);

  useEffect(() => () => {
    if (settingsCloseTimeoutRef.current !== null) {
      window.clearTimeout(settingsCloseTimeoutRef.current);
    }
  }, []);

  useEffect(() => {
    function handleRefreshShortcut(event: KeyboardEvent) {
      if (event.defaultPrevented || event.repeat) {
        return;
      }

      if (event.ctrlKey || event.metaKey || event.altKey) {
        return;
      }

      const target = event.target as HTMLElement | null;
      const tagName = target?.tagName;
      if (
        target?.isContentEditable ||
        tagName === "INPUT" ||
        tagName === "TEXTAREA" ||
        tagName === "SELECT"
      ) {
        return;
      }

      if (event.key.toLowerCase() === "r") {
        event.preventDefault();
        refreshAll();
      }
    }

    window.addEventListener("keydown", handleRefreshShortcut);
    return () => window.removeEventListener("keydown", handleRefreshShortcut);
  }, [refreshAll]);

  const autoRefreshSummary = autoRefreshEnabled
    ? `Auto refresh every ${autoRefreshMinutes} min`
    : "Auto refresh off";
  const updateSummary = summarizeUpdateNotes(updateInfo?.notes);

  return (
    <div className="app-shell">
      <div className="header">
        <div className="header-mark">
          <BoltIcon />
        </div>
        <span className="header-product">UsageDock</span>
        <div className={`header-status ${isLoading ? "header-status-live" : ""}`}>
          <span className="header-status-dot" />
          <span>{statusText}</span>
        </div>
        <div className="header-actions">
          <button
            ref={settingsBtnRef}
            className={`btn-icon settings-btn ${isSettingsOpen ? "settings-btn-active" : ""}`}
            onClick={() => {
              if (isSettingsOpen) {
                closeSettings();
              } else {
                openSettings();
              }
            }}
            title="Settings"
            aria-label="Toggle settings"
          >
            {isSettingsOpen ? <X /> : <Settings />}
          </button>
          <button
            className={`btn-icon refresh-all ${isLoading ? "spinning" : ""}`}
            onClick={refreshAll}
            title="Refresh all (R)"
            aria-label="Refresh all providers"
          >
            <RefreshCw />
          </button>
        </div>
      </div>

      {(isSettingsOpen || isSettingsClosing) && (
        <div
          className={`settings-panel ${isSettingsClosing ? "settings-panel-closing" : "settings-panel-opening"}`}
          ref={settingsPanelRef}
        >
          <div className="settings-row">
            <label className="toggle-field">
              <span className="toggle-label">Auto refresh</span>
              <button
                type="button"
                role="switch"
                aria-checked={autoRefreshEnabled}
                className={`toggle-switch ${autoRefreshEnabled ? "toggle-switch-on" : ""}`}
                onClick={() => setAutoRefreshEnabled((prev) => !prev)}
              >
                <span className="toggle-thumb" />
              </button>
            </label>

            <div
              ref={intervalMenuRef}
              className={`select-field ${!autoRefreshEnabled ? "select-field-disabled" : ""}`}
            >
              <span className="select-label">Interval</span>
              <button
                type="button"
                className="footer-select interval-trigger"
                onClick={() => autoRefreshEnabled && setIsIntervalMenuOpen((prev) => !prev)}
                disabled={!autoRefreshEnabled}
                aria-haspopup="listbox"
                aria-expanded={isIntervalMenuOpen}
              >
                {autoRefreshMinutes} min
                <ChevronDown className={`interval-trigger-icon ${isIntervalMenuOpen ? "interval-trigger-icon-open" : ""}`} />
              </button>
              {autoRefreshEnabled && isIntervalMenuOpen && (
                <div className="interval-menu" role="listbox" aria-label="Auto refresh interval">
                  {AUTO_REFRESH_OPTIONS.map((minutes) => (
                    <button
                      key={minutes}
                      type="button"
                      role="option"
                      aria-selected={minutes === autoRefreshMinutes}
                      className={`interval-option ${minutes === autoRefreshMinutes ? "interval-option-active" : ""}`}
                      onClick={() => {
                        setAutoRefreshMinutes(minutes);
                        setIsIntervalMenuOpen(false);
                      }}
                    >
                      <span>{minutes} min</span>
                      {minutes === autoRefreshMinutes && <span className="interval-option-check">Current</span>}
                    </button>
                  ))}
                </div>
              )}
            </div>
          </div>

          {updaterEnabled && (
            <div className="settings-row settings-row-update">
              <span className="settings-update-label">
                {isCheckingUpdate ? "Checking for updates…" : "App updates"}
              </span>
              <button
                type="button"
                className="settings-update-btn"
                onClick={() => checkForUpdates(true)}
                disabled={isCheckingUpdate || isInstallingUpdate}
              >
                {isCheckingUpdate ? "Checking…" : "Check now"}
              </button>
            </div>
          )}

          <div className="settings-section-label">Ollama</div>
          <div className="settings-row settings-row-ollama">
            <label className="settings-input-field">
              <span className="settings-input-label">URL</span>
              <input
                type="text"
                className="settings-input"
                value={ollamaUrl}
                onChange={(e) => { setOllamaUrl(e.target.value); setOllamaSaved(false); }}
                placeholder="http://localhost:11434"
                spellCheck={false}
              />
            </label>
            <label className="settings-input-field">
              <span className="settings-input-label">API Key</span>
              <input
                type="password"
                className="settings-input"
                value={ollamaApiKey}
                onChange={(e) => { setOllamaApiKey(e.target.value); setOllamaSaved(false); }}
                placeholder="Optional"
                spellCheck={false}
                autoComplete="off"
              />
            </label>
            <button
              type="button"
              className={`settings-update-btn ${ollamaSaved ? "settings-saved-btn" : ""}`}
              onClick={async () => {
                try {
                  await invoke("save_ollama_settings", { url: ollamaUrl, apiKey: ollamaApiKey });
                  setOllamaSaved(true);
                } catch (e) {
                  console.error("Failed to save Ollama settings:", e);
                }
              }}
            >
              {ollamaSaved ? "Saved ✓" : "Save"}
            </button>
          </div>
        </div>
      )}

      <div className="provider-list">
        {updaterEnabled && (updateInfo || isInstallingUpdate || updateError) && (
          <section
            className={`update-banner ${
              updateError ? "update-banner-error" : updateInfo ? "update-banner-ready" : ""
            }`}
          >
            <div className="update-banner-copy">
              <span className="update-banner-label">
                {updateError
                  ? "Updater needs attention"
                  : isInstallingUpdate
                    ? "Installing update"
                    : `Version ${updateInfo?.version} is ready`}
              </span>
              <span className="update-banner-text">
                {updateError
                  ? updateError
                  : isInstallingUpdate
                    ? "UsageDock will close when the installer takes over."
                    : updateSummary || "A newer UsageDock release is available for this device."}
              </span>
            </div>

            {updateInfo && !isInstallingUpdate && !updateError && (
              <button
                type="button"
                className="update-banner-button"
                onClick={installUpdate}
              >
                Install
              </button>
            )}
          </section>
        )}

        {availableProviders.map((provider) => (
          <ProviderCard
            key={provider.id}
            provider={provider}
            onRefresh={() => refreshSingle(provider.id)}
            isRefreshing={refreshing.has(provider.id)}
          />
        ))}

        {!isLoading && unavailableProviders.length > 0 && (
          <section className={`provider-collapse ${showUnavailable ? "provider-collapse-open" : ""}`}>
            <button
              className="provider-collapse-toggle"
              type="button"
              onClick={() => setShowUnavailable((prev) => !prev)}
              aria-expanded={showUnavailable}
              aria-controls="unavailable-providers"
            >
              <div className="provider-collapse-copy">
                <span className="provider-collapse-title">Unavailable providers</span>
                <span className="provider-collapse-caption">{unavailableCaption}</span>
              </div>
              <div className="provider-collapse-action">
                <span className="provider-collapse-count">{unavailableProviders.length}</span>
                <ChevronDown className="provider-collapse-icon" />
              </div>
            </button>

            {showUnavailable && (
              <div id="unavailable-providers" className="provider-collapse-body">
                {unavailableProviders.map((provider) => (
                  <ProviderCard
                    key={provider.id}
                    provider={provider}
                    onRefresh={() => refreshSingle(provider.id)}
                    isRefreshing={refreshing.has(provider.id)}
                  />
                ))}
              </div>
            )}
          </section>
        )}

        {!isLoading && providers.length === 0 && (
          <div className="empty-state">
            <BoltIcon />
            <p>No providers configured.<br />Sign into Cursor, Claude, Copilot, Codex, Antigravity, or start Ollama to get started.</p>
          </div>
        )}
      </div>

      <div className="footer">
        <span className="footer-text">
          {lastRefresh
            ? `Updated ${lastRefresh.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" })}`
            : "Loading provider activity"}
        </span>
        <span className="footer-text">{autoRefreshSummary}</span>
      </div>
    </div>
  );
}

export default App;
