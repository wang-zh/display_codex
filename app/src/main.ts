import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { openUrl } from "@tauri-apps/plugin-opener";

type QuotaKind = "five_hour" | "weekly";
type QuotaSource = "live" | "cache";
type QuotaStatus =
  | "idle"
  | "refreshing"
  | "ok"
  | "stale"
  | "login_required"
  | "parse_error"
  | "network_error";

type QuotaEntry = {
  kind: QuotaKind;
  remainingPercent: number;
  resetLabel: string;
};

type QuotaState = {
  entries: QuotaEntry[];
  lastRefreshAt: string | null;
  nextRefreshAt: string | null;
  source: QuotaSource;
  status: QuotaStatus;
  errorSummary?: string | null;
};

type ProxyDiagnostics = {
  enabled: boolean;
  source: string;
  server: string | null;
  localProbe: boolean | null;
};

type EndpointDiagnostic = {
  label: string;
  url: string;
  reachable: boolean;
  statusCode: number | null;
  error: string | null;
};

type ConnectionDiagnostics = {
  proxy: ProxyDiagnostics;
  endpoints: EndpointDiagnostic[];
  summary: string;
};

const DEFAULT_REFRESH_MINUTES = 5;
const REFRESH_INTERVAL_KEY = "codex-quota-refresh-minutes";
const ANALYTICS_URL = "https://chatgpt.com/codex/cloud/settings/analytics#usage";

const cardsEl = document.querySelector<HTMLDivElement>("#quota-cards");
const refreshButton = document.querySelector<HTMLButtonElement>("#refresh-button");
const settingsButton = document.querySelector<HTMLButtonElement>("#settings-button");
const closeSettingsButton = document.querySelector<HTMLButtonElement>("#close-settings-button");
const statusEl = document.querySelector<HTMLDivElement>("#status-banner");
const lastRefreshEl = document.querySelector<HTMLSpanElement>("#last-refresh");
const nextRefreshEl = document.querySelector<HTMLSpanElement>("#next-refresh");
const settingsPanel = document.querySelector<HTMLElement>("#settings-panel");
const refreshIntervalSelect = document.querySelector<HTMLSelectElement>(
  "#refresh-interval-select",
);
const cookieHeaderInput =
  document.querySelector<HTMLTextAreaElement>("#cookie-header-input");
const applyCookieHeaderButton = document.querySelector<HTMLButtonElement>(
  "#apply-cookie-header-button",
);
const clearCookieHeaderButton = document.querySelector<HTMLButtonElement>(
  "#clear-cookie-header-button",
);
const openChatgptButton =
  document.querySelector<HTMLButtonElement>("#open-chatgpt-button");
const checkConnectionButton = document.querySelector<HTMLButtonElement>(
  "#check-connection-button",
);
const credentialHintEl =
  document.querySelector<HTMLParagraphElement>("#credential-hint");
const diagnosticNetworkSummaryEl = document.querySelector<HTMLSpanElement>(
  "#diagnostic-network-summary",
);
const diagnosticProxyEl = document.querySelector<HTMLSpanElement>("#diagnostic-proxy");
const diagnosticEndpointsEl = document.querySelector<HTMLSpanElement>(
  "#diagnostic-endpoints",
);
const diagnosticSourceEl =
  document.querySelector<HTMLSpanElement>("#diagnostic-source");
const diagnosticStatusEl =
  document.querySelector<HTMLSpanElement>("#diagnostic-status");
const diagnosticErrorEl =
  document.querySelector<HTMLSpanElement>("#diagnostic-error");
const diagnosticLogPathEl =
  document.querySelector<HTMLSpanElement>("#diagnostic-log-path");

let autoRefreshTimer: number | undefined;
let isRefreshing = false;
let refreshMinutes = loadRefreshMinutes();

function labelFor(kind: QuotaKind): string {
  return kind === "five_hour" ? "5 小时使用限额" : "每周使用限额";
}

function statusText(state: QuotaState): string {
  if (state.status === "ok") return "数据已更新";
  if (state.status === "idle") return "等待刷新";
  if (state.status === "login_required") return "需要登录态";
  if (state.status === "parse_error") return "解析失败";
  if (state.status === "network_error") return "网络失败";
  if (state.status === "stale") return "使用上次成功数据";
  return "正在刷新";
}

function renderQuotaCard(entry: QuotaEntry): string {
  const percent = Math.max(0, Math.min(100, entry.remainingPercent));
  return `
    <section class="quota-card">
      <div class="quota-card__header">
        <span>${labelFor(entry.kind)}</span>
        <strong>${percent}%</strong>
      </div>
      <div class="progress" aria-hidden="true">
        <div class="progress__bar" style="width: ${percent}%"></div>
      </div>
      <div class="quota-card__meta">
        <span>重置时间</span>
        <span>${entry.resetLabel}</span>
      </div>
    </section>
  `;
}

function renderState(state: QuotaState) {
  if (!cardsEl || !statusEl || !lastRefreshEl || !nextRefreshEl) return;

  if (state.entries.length === 0) {
    cardsEl.innerHTML = `
      <section class="empty-state">
        <strong>暂无额度数据</strong>
        <span>${state.errorSummary ?? "等待第一次刷新"}</span>
      </section>
    `;
  } else {
    cardsEl.innerHTML = state.entries.map(renderQuotaCard).join("");
  }

  statusEl.dataset.status = state.status;
  statusEl.hidden = state.status === "ok" && state.source === "live";
  statusEl.innerHTML = `
    <span>${statusText(state)}</span>
    <span>${state.errorSummary ?? "当前显示最近一次可用数据"}</span>
  `;

  lastRefreshEl.textContent = state.lastRefreshAt ?? "尚未成功刷新";
  nextRefreshEl.textContent = state.nextRefreshAt ?? `${refreshMinutes} 分钟后`;
  if (diagnosticSourceEl) diagnosticSourceEl.textContent = state.source;
  if (diagnosticStatusEl) diagnosticStatusEl.textContent = state.status;
  if (diagnosticErrorEl) diagnosticErrorEl.textContent = state.errorSummary ?? "无";
  if (credentialHintEl) {
    credentialHintEl.dataset.status =
      state.status === "login_required" ? "attention" : "normal";
  }
}

async function loadState() {
  const state = await invoke<QuotaState>("get_quota_state");
  renderState(state);
  if (state.source === "cache" || state.status === "idle") {
    await refreshQuota();
  }
}

async function loadLogPath() {
  if (!diagnosticLogPathEl) return;
  const logPath = await invoke<string | null>("get_log_file_path");
  diagnosticLogPathEl.textContent = logPath ?? "尚未初始化";
}

function labelForEndpoint(label: string): string {
  if (label === "session") return "session";
  if (label === "analytics") return "analytics";
  if (label === "usage") return "usage";
  return label;
}

function renderConnectionDiagnostics(diagnostics: ConnectionDiagnostics) {
  if (diagnosticNetworkSummaryEl) {
    diagnosticNetworkSummaryEl.textContent = diagnostics.summary;
  }

  if (diagnosticProxyEl) {
    const server = diagnostics.proxy.server ?? "无";
    const localProbe =
      diagnostics.proxy.localProbe === null
        ? ""
        : diagnostics.proxy.localProbe
          ? "，本地端口可连接"
          : "，本地端口不可连接";
    diagnosticProxyEl.textContent = `${diagnostics.proxy.source}：${server}${localProbe}`;
  }

  if (diagnosticEndpointsEl) {
    diagnosticEndpointsEl.textContent = diagnostics.endpoints
      .map((endpoint) => {
        const status = endpoint.statusCode === null ? "" : ` HTTP ${endpoint.statusCode}`;
        return `${labelForEndpoint(endpoint.label)}=${endpoint.reachable ? "可达" : "失败"}${status}`;
      })
      .join("；");
  }
}

async function loadConnectionDiagnostics() {
  checkConnectionButton?.setAttribute("disabled", "true");
  if (diagnosticNetworkSummaryEl) diagnosticNetworkSummaryEl.textContent = "检测中...";
  try {
    const diagnostics = await invoke<ConnectionDiagnostics>("get_connection_diagnostics");
    renderConnectionDiagnostics(diagnostics);
  } catch (error) {
    if (diagnosticNetworkSummaryEl) {
      diagnosticNetworkSummaryEl.textContent =
        error instanceof Error ? error.message : String(error);
    }
  } finally {
    checkConnectionButton?.removeAttribute("disabled");
  }
}

async function refreshQuota() {
  if (isRefreshing) return;
  isRefreshing = true;
  refreshButton?.setAttribute("disabled", "true");
  refreshButton?.classList.add("is-refreshing");

  try {
    const state = await invoke<QuotaState>("refresh_quota");
    renderState(state);
    if (state.status === "network_error" || state.status === "login_required") {
      await loadConnectionDiagnostics();
    }
  } catch (error) {
    renderState({
      entries: [],
      lastRefreshAt: null,
      nextRefreshAt: "5 分钟后",
      source: "cache",
      status: "network_error",
      errorSummary: error instanceof Error ? error.message : String(error),
    });
  } finally {
    isRefreshing = false;
    refreshButton?.removeAttribute("disabled");
    refreshButton?.classList.remove("is-refreshing");
    scheduleAutoRefresh();
  }
}

async function setCookieHeaderOverride(value: string) {
  await invoke<QuotaState>("set_cookie_header_override", { value });
}

function scheduleAutoRefresh() {
  if (autoRefreshTimer !== undefined) {
    window.clearTimeout(autoRefreshTimer);
  }
  autoRefreshTimer = window.setTimeout(refreshQuota, refreshMinutes * 60 * 1000);
}

function loadRefreshMinutes(): number {
  const raw = window.localStorage.getItem(REFRESH_INTERVAL_KEY);
  const value = raw ? Number.parseInt(raw, 10) : DEFAULT_REFRESH_MINUTES;
  return [1, 5, 10, 15, 30].includes(value) ? value : DEFAULT_REFRESH_MINUTES;
}

function setSettingsVisible(visible: boolean) {
  if (settingsPanel) settingsPanel.hidden = !visible;
}

window.addEventListener("DOMContentLoaded", async () => {
  if (refreshIntervalSelect) {
    refreshIntervalSelect.value = String(refreshMinutes);
    refreshIntervalSelect.addEventListener("change", () => {
      refreshMinutes = Number.parseInt(refreshIntervalSelect.value, 10);
      window.localStorage.setItem(REFRESH_INTERVAL_KEY, String(refreshMinutes));
      scheduleAutoRefresh();
      if (nextRefreshEl) nextRefreshEl.textContent = `${refreshMinutes} 分钟后`;
    });
  }
  refreshButton?.addEventListener("click", refreshQuota);
  settingsButton?.addEventListener("click", () => setSettingsVisible(true));
  closeSettingsButton?.addEventListener("click", () => setSettingsVisible(false));
  openChatgptButton?.addEventListener("click", () => openUrl(ANALYTICS_URL));
  checkConnectionButton?.addEventListener("click", loadConnectionDiagnostics);
  applyCookieHeaderButton?.addEventListener("click", async () => {
    await setCookieHeaderOverride(cookieHeaderInput?.value ?? "");
    await refreshQuota();
  });
  clearCookieHeaderButton?.addEventListener("click", async () => {
    if (cookieHeaderInput) cookieHeaderInput.value = "";
    await setCookieHeaderOverride("");
    await refreshQuota();
  });
  await listen("quota-refresh-requested", refreshQuota);
  await listen("quota-settings-requested", () => setSettingsVisible(true));
  await loadLogPath();
  await loadConnectionDiagnostics();
  await loadState();
  scheduleAutoRefresh();
});
