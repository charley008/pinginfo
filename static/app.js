const HISTORY_LIMIT = 1000;
const CHART_WINDOW_OPTIONS = [10, 30, 60, 360, 1440];
const DEFAULT_CHART_WINDOW_MINUTES = 10;
const SETTINGS_VERSION = 2;

const settings = loadSettings();

const state = {
  targets: [],
  statuses: new Map(),
  results: new Map(),
  chartResults: new Map(),
  failureEvents: new Map(),
  selectedId: null,
  editingId: null,
  filter: "",
  chartWindowEnd: null,
  chartWindowMinutes: settings.chartWindowMinutes,
};

const els = {
  targetsBody: document.querySelector("#targetsBody"),
  resultsBody: document.querySelector("#resultsBody"),
  chart: document.querySelector("#latencyChart"),
  chartInfo: document.querySelector("#chartInfo"),
  chartTooltip: document.querySelector("#chartTooltip"),
  chartLatest: document.querySelector("#chartLatestBtn"),
  chartRangeButtons: [...document.querySelectorAll(".chart-range-btn")],
  failureSummary: document.querySelector("#failureSummary"),
  summary: document.querySelector("#summary"),
  selectedLabel: document.querySelector("#selectedLabel"),
  connection: document.querySelector("#connection"),
  appVersion: document.querySelector("#appVersion"),
  defaultsHint: document.querySelector("#defaultsHint"),
  healthyCount: document.querySelector("#healthyCount"),
  warningCount: document.querySelector("#warningCount"),
  downCount: document.querySelector("#downCount"),
  disabledCount: document.querySelector("#disabledCount"),
  filter: document.querySelector("#filterInput"),
  contentGrid: document.querySelector(".content-grid"),
  splitter: document.querySelector("#splitter"),
  detailsRow: document.querySelector(".details-row"),
  detailsSplitter: document.querySelector("#detailsSplitter"),
  targetDialog: document.querySelector("#targetDialog"),
  targetDialogTitle: document.querySelector("#targetDialogTitle"),
  targetForm: document.querySelector("#targetForm"),
  settingsDialog: document.querySelector("#settingsDialog"),
  settingsForm: document.querySelector("#settingsForm"),
  listDialog: document.querySelector("#listDialog"),
  listForm: document.querySelector("#listForm"),
  targetsText: document.querySelector("#targetsText"),
  settings: document.querySelector("#settingsBtn"),
  list: document.querySelector("#listBtn"),
  add: document.querySelector("#addTargetBtn"),
  refresh: document.querySelector("#refreshBtn"),
  edit: document.querySelector("#editSelectedBtn"),
  start: document.querySelector("#startSelectedBtn"),
  stop: document.querySelector("#stopSelectedBtn"),
  clearData: document.querySelector("#clearDataSelectedBtn"),
  delete: document.querySelector("#deleteSelectedBtn"),
};

let chartPoints = [];
let chartReloadTimer = null;
let chartDrag = null;

function loadSettings() {
  const saved = JSON.parse(localStorage.getItem("pinginfo.settings") || "{}");
  const version = Number(saved.version || 0);
  const savedWindow = Number(saved.chartWindowMinutes || DEFAULT_CHART_WINDOW_MINUTES);
  return {
    version: SETTINGS_VERSION,
    defaultIntervalMs: Number(saved.defaultIntervalMs || 1000),
    defaultTimeoutMs: Number(saved.defaultTimeoutMs || 1000),
    chartWindowMinutes:
      version >= SETTINGS_VERSION && CHART_WINDOW_OPTIONS.includes(savedWindow)
        ? savedWindow
        : DEFAULT_CHART_WINDOW_MINUTES,
  };
}

function saveSettings() {
  settings.version = SETTINGS_VERSION;
  settings.chartWindowMinutes = state.chartWindowMinutes;
  localStorage.setItem("pinginfo.settings", JSON.stringify(settings));
  renderSettingsHint();
}

function renderSettingsHint() {
  els.defaultsHint.innerHTML = `间隔 ${settings.defaultIntervalMs} ms<br>超时 ${settings.defaultTimeoutMs} ms<br>图表 ${formatWindowMinutes(state.chartWindowMinutes)}`;
}

function fmtMs(value) {
  if (value === null || value === undefined) return "-";
  return Number(value).toFixed(3);
}

function fmtTime(value) {
  if (!value) return "-";
  return new Date(value).toLocaleString();
}

function fmtShortTime(value) {
  if (!value) return "-";
  return new Date(value).toLocaleTimeString([], {
    hour12: false,
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
  });
}

function formatWindowMinutes(minutes) {
  if (minutes < 60) return `${minutes} 分钟`;
  if (minutes < 1440) return `${Math.round(minutes / 60)} 小时`;
  return `${Math.round(minutes / 1440)} 天`;
}

function formatAxisTime(value, includeDate = false) {
  return new Date(value).toLocaleString([], {
    hour12: false,
    month: includeDate ? "2-digit" : undefined,
    day: includeDate ? "2-digit" : undefined,
    hour: "2-digit",
    minute: "2-digit",
  });
}

function formatDurationSeconds(seconds) {
  const total = Math.max(0, Number(seconds || 0));
  if (total < 60) return `${total} 秒`;
  const minutes = Math.floor(total / 60);
  const remainSeconds = total % 60;
  if (minutes < 60) return remainSeconds > 0 ? `${minutes} 分 ${remainSeconds} 秒` : `${minutes} 分钟`;
  const hours = Math.floor(minutes / 60);
  const remainMinutes = minutes % 60;
  return remainMinutes > 0 ? `${hours} 小时 ${remainMinutes} 分` : `${hours} 小时`;
}

function stateClass(status, target) {
  if (!target.enabled) return "state-disabled";
  return `state-${status?.state || "healthy"}`;
}

function statusText(status, target) {
  if (!target.enabled) return "disabled";
  return status?.state || "healthy";
}

async function api(path, options = {}) {
  const response = await fetch(path, {
    headers: { "content-type": "application/json" },
    ...options,
  });
  if (!response.ok) {
    const payload = await response.json().catch(() => ({}));
    throw new Error(payload.error || response.statusText);
  }
  if (response.status === 204) return null;
  return response.json();
}

async function loadAll() {
  const [targets, statuses, meta] = await Promise.all([
    api("/api/targets"),
    api("/api/status"),
    api("/api/meta"),
  ]);
  state.targets = targets;
  state.statuses = new Map(statuses.map((item) => [item.target_id, item]));
  els.appVersion.textContent = meta?.version ? `版本 ${meta.version}` : "版本 --";
  if (!state.selectedId && targets.length > 0) state.selectedId = targets[0].id;
  syncChartRangeButtons();
  renderTargets();
  await loadSelectedResults();
}

function renderTargets() {
  els.targetsBody.innerHTML = "";
  const visibleTargets = filteredTargets();

  visibleTargets.forEach((target, index) => {
    const status = state.statuses.get(target.id);
    const tr = document.createElement("tr");
    if (target.id === state.selectedId) tr.classList.add("selected");
    tr.innerHTML = `
      <td>${index + 1}</td>
      <td><span class="host-cell"><span class="state-dot ${stateClass(status, target)}"></span>${escapeHtml(target.name)}</span></td>
      <td>${escapeHtml(target.host)}${target.port ? `:${target.port}` : ""}</td>
      <td>${target.probe_type.toUpperCase()}</td>
      <td>${status?.success_count ?? 0}</td>
      <td>${status?.failure_count ?? 0}</td>
      <td>${fmtMs(status?.last_latency_ms)}</td>
      <td>${fmtMs(status?.avg_latency_ms)}</td>
      <td>${fmtMs(status?.loss_rate)}</td>
      <td>${status?.consecutive_failures ?? 0}</td>
      <td>${fmtMs(status?.min_latency_ms)}</td>
      <td>${fmtMs(status?.max_latency_ms)}</td>
      <td>${fmtTime(status?.last_failure_at)}</td>
    `;
    tr.addEventListener("click", async () => {
      state.selectedId = target.id;
      renderTargets();
      await loadSelectedResults();
    });
    els.targetsBody.appendChild(tr);
  });

  const counts = { healthy: 0, warning: 0, down: 0, disabled: 0 };
  state.targets.forEach((target) => {
    counts[statusText(state.statuses.get(target.id), target)] += 1;
  });

  els.healthyCount.textContent = counts.healthy;
  els.warningCount.textContent = counts.warning;
  els.downCount.textContent = counts.down;
  els.disabledCount.textContent = counts.disabled;
  els.summary.textContent = `${visibleTargets.length}/${state.targets.length} 个目标`;
}

async function loadSelectedResults() {
  if (!state.selectedId) {
    renderResults([]);
    renderLatencyChart([], null, null, []);
    return;
  }

  const results = await api(`/api/targets/${state.selectedId}/results?limit=${HISTORY_LIMIT}`);
  state.results.set(state.selectedId, results);

  const target = state.targets.find((item) => item.id === state.selectedId);
  els.selectedLabel.textContent = target
    ? `${target.name} · ${target.host}${target.port ? `:${target.port}` : ""}`
    : "未选择目标";

  renderResults(results);
  state.chartWindowEnd = null;
  await loadSelectedChartResults();
}

async function loadSelectedChartResults() {
  if (!state.selectedId) {
    renderLatencyChart([], null, null, []);
    return;
  }

  const params = new URLSearchParams({ minutes: String(state.chartWindowMinutes) });
  if (state.chartWindowEnd) params.set("before", state.chartWindowEnd.toISOString());

  const [results, failureEvents] = await Promise.all([
    api(`/api/targets/${state.selectedId}/results?${params.toString()}`),
    api(`/api/targets/${state.selectedId}/failure-events?${params.toString()}`),
  ]);

  state.chartResults.set(state.selectedId, results);
  state.failureEvents.set(state.selectedId, failureEvents);

  const endAt = state.chartWindowEnd || new Date();
  const startAt = new Date(endAt.getTime() - state.chartWindowMinutes * 60 * 1000);
  renderLatencyChart(results, startAt, endAt, failureEvents);
}

function filteredTargets() {
  const term = state.filter.trim().toLowerCase();
  if (!term) return state.targets;
  return state.targets.filter((target) => {
    const status = state.statuses.get(target.id);
    return [
      target.name,
      target.host,
      target.port ? String(target.port) : "",
      target.probe_type,
      statusText(status, target),
    ].some((value) => String(value).toLowerCase().includes(term));
  });
}

function renderResults(results) {
  els.resultsBody.innerHTML = "";
  results.forEach((result) => {
    const tr = document.createElement("tr");
    tr.innerHTML = `
      <td><span class="host-cell"><span class="state-dot ${result.success ? "state-healthy" : "state-down"}"></span>${fmtTime(result.started_at)}</span></td>
      <td>${result.resolved_ip || "-"}</td>
      <td>${fmtMs(result.latency_ms)}</td>
      <td>${result.ttl ?? "-"}</td>
      <td>${result.success ? "成功" : "失败"}</td>
      <td>${escapeHtml(result.error_message || "")}</td>
    `;
    els.resultsBody.appendChild(tr);
  });
}

function renderLatencyChart(results, startAt, endAt, failureEvents = []) {
  const canvas = els.chart;
  const rect = canvas.getBoundingClientRect();
  const dpr = window.devicePixelRatio || 1;
  canvas.width = Math.max(1, Math.floor(rect.width * dpr));
  canvas.height = Math.max(1, Math.floor(rect.height * dpr));

  const ctx = canvas.getContext("2d");
  ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
  ctx.clearRect(0, 0, rect.width, rect.height);
  chartPoints = [];

  const pad = { left: 56, right: 16, top: 18, bottom: 36 };
  const width = Math.max(1, rect.width - pad.left - pad.right);
  const height = Math.max(1, rect.height - pad.top - pad.bottom);
  const successful = results.filter((item) => item.success && item.latency_ms !== null);
  const maxLatency = Math.max(10, ...successful.map((item) => Number(item.latency_ms || 0)));
  const yMax = niceMax(maxLatency);

  drawChartFrame(ctx, rect, pad, width, height, yMax, startAt, endAt);

  if (results.length === 0) {
    ctx.fillStyle = "#697386";
    ctx.fillText("当前时间窗口内没有数据", pad.left + 12, pad.top + 24);
    updateChartInfo([], startAt, endAt);
    renderFailureSummary(failureEvents);
    return;
  }

  const startMs = startAt.getTime();
  const spanMs = Math.max(1, endAt.getTime() - startMs);
  const points = results.map((result) => {
    const timeMs = new Date(result.finished_at).getTime();
    const ratio = Math.min(1, Math.max(0, (timeMs - startMs) / spanMs));
    const x = pad.left + ratio * width;
    const failed = !result.success;
    const y = failed
      ? pad.top + 10
      : pad.top + height - (Number(result.latency_ms || 0) / yMax) * height;
    return { x, y, result, failed };
  });

  ctx.lineWidth = 1.6;
  ctx.strokeStyle = "#226ce0";
  ctx.beginPath();
  let lineStarted = false;
  points.forEach((point) => {
    if (point.failed || point.result.latency_ms === null) {
      lineStarted = false;
      return;
    }
    if (!lineStarted) {
      ctx.moveTo(point.x, point.y);
      lineStarted = true;
    } else {
      ctx.lineTo(point.x, point.y);
    }
  });
  ctx.stroke();

  points.forEach((point) => {
    if (point.failed) {
      ctx.strokeStyle = "#e34848";
      ctx.lineWidth = 1;
      ctx.beginPath();
      ctx.moveTo(point.x, pad.top + 6);
      ctx.lineTo(point.x, pad.top + height);
      ctx.stroke();

      ctx.beginPath();
      ctx.fillStyle = "#e34848";
      ctx.arc(point.x, pad.top + 8, 3.8, 0, Math.PI * 2);
      ctx.fill();
    } else {
      ctx.beginPath();
      ctx.fillStyle = "#226ce0";
      ctx.arc(point.x, point.y, 2.2, 0, Math.PI * 2);
      ctx.fill();
    }
    chartPoints.push(point);
  });

  updateChartInfo(results, startAt, endAt);
  renderFailureSummary(failureEvents);
}

function updateChartInfo(results, startAt, endAt) {
  const failures = results.filter((item) => !item.success).length;
  const successes = results.length - failures;
  const suffix = state.chartWindowEnd ? "历史窗口" : "最新";
  const includeDate = state.chartWindowMinutes >= 1440;
  const rangeText = `${formatAxisTime(startAt, includeDate)} - ${formatAxisTime(endAt, includeDate)}`;
  const coverage = chartCoverageText(results);
  els.chartInfo.textContent = `${results.length} 条 · 成功 ${successes} · 失败 ${failures} · 覆盖 ${coverage} · ${rangeText} · ${suffix}`;
  els.chartLatest.hidden = !state.chartWindowEnd;
}

function chartCoverageText(results) {
  if (results.length <= 1) return results.length === 1 ? "单点" : "0 分钟";
  const first = new Date(results[0].finished_at).getTime();
  const last = new Date(results[results.length - 1].finished_at).getTime();
  const minutes = Math.max(1, Math.round((last - first) / 60000));
  return formatWindowMinutes(minutes);
}

function renderFailureSummary(events) {
  if (!els.failureSummary) return;

  if (!events || events.length === 0) {
    const label = state.chartWindowMinutes >= 1440 ? "24 小时异常事件" : "失败定位";
    els.failureSummary.innerHTML = `<strong>${label}</strong> 当前窗口内没有丢包。`;
    return;
  }

  if (state.chartWindowMinutes >= 1440) {
    const totalFailures = events.reduce((sum, event) => sum + Number(event.failure_count || 0), 0);
    const longest = events.reduce((max, event) => Math.max(max, Number(event.duration_seconds || 0)), 0);
    const items = [...events]
      .reverse()
      .map((event) => `
        <div class="failure-event-item">
          <div><strong>${escapeHtml(fmtTime(event.started_at))}</strong></div>
          <div>${escapeHtml(fmtTime(event.ended_at))}</div>
          <div>丢包 ${event.failure_count} 次</div>
          <div>持续 ${formatDurationSeconds(event.duration_seconds)}</div>
        </div>
      `)
      .join("");

    els.failureSummary.innerHTML = `
      <strong>24 小时异常事件</strong>
      <div class="failure-event-stats">
        <span>异常 ${events.length} 段</span>
        <span>累计丢包 ${totalFailures} 次</span>
        <span>最长 ${formatDurationSeconds(longest)}</span>
      </div>
      <div class="failure-event-list">${items}</div>
    `;
    return;
  }

  const chips = [...events]
    .slice(-8)
    .reverse()
    .map((event) => {
      const text = `${fmtShortTime(event.started_at)} - ${fmtShortTime(event.ended_at)} 丢包 ${event.failure_count} 次`;
      return `<span class="failure-chip">${escapeHtml(text)}</span>`;
    })
    .join("");

  els.failureSummary.innerHTML = `<strong>失败定位</strong><div class="failure-summary-list">${chips}</div>`;
}

function drawChartFrame(ctx, rect, pad, width, height, yMax, startAt, endAt) {
  ctx.font = "12px Segoe UI, Arial";
  ctx.textBaseline = "middle";
  ctx.strokeStyle = "#d7dde5";
  ctx.fillStyle = "#697386";
  ctx.lineWidth = 1;

  for (let i = 0; i <= 4; i += 1) {
    const value = (yMax / 4) * i;
    const y = pad.top + height - (value / yMax) * height;
    ctx.beginPath();
    ctx.moveTo(pad.left, y);
    ctx.lineTo(pad.left + width, y);
    ctx.stroke();
    ctx.fillText(`${Math.round(value)} ms`, 8, y);
  }

  ctx.strokeStyle = "#9aa8ba";
  ctx.beginPath();
  ctx.moveTo(pad.left, pad.top);
  ctx.lineTo(pad.left, pad.top + height);
  ctx.lineTo(pad.left + width, pad.top + height);
  ctx.stroke();

  timeLabels(startAt, endAt).forEach(({ ratio, label }) => {
    const x = pad.left + ratio * width;
    ctx.strokeStyle = "#edf1f5";
    ctx.beginPath();
    ctx.moveTo(x, pad.top);
    ctx.lineTo(x, pad.top + height);
    ctx.stroke();

    ctx.fillStyle = "#697386";
    ctx.fillText(label, Math.min(rect.width - 58, Math.max(48, x - 22)), pad.top + height + 18);
  });
}

function timeLabels(startAt, endAt) {
  const labels = [];
  const includeDate = endAt.getTime() - startAt.getTime() >= 24 * 60 * 60 * 1000;
  for (let i = 0; i <= 5; i += 1) {
    const ratio = i / 5;
    const time = new Date(startAt.getTime() + (endAt.getTime() - startAt.getTime()) * ratio);
    labels.push({ ratio, label: formatAxisTime(time, includeDate) });
  }
  return labels;
}

function niceMax(value) {
  const magnitude = 10 ** Math.floor(Math.log10(value));
  const normalized = value / magnitude;
  const nice = normalized <= 2 ? 2 : normalized <= 5 ? 5 : 10;
  return nice * magnitude;
}

function syncChartRangeButtons() {
  els.chartRangeButtons.forEach((button) => {
    button.classList.toggle("active", Number(button.dataset.rangeMinutes) === state.chartWindowMinutes);
  });
}

function connectEvents() {
  const source = new EventSource("/api/events");

  source.onopen = () => {
    els.connection.textContent = "实时连接已建立";
  };

  source.onerror = () => {
    els.connection.textContent = "实时连接重试中";
  };

  source.onmessage = async (event) => {
    const payload = JSON.parse(event.data);
    if (payload.type === "status") {
      state.statuses.set(payload.status.target_id, payload.status);
      renderTargets();
    }

    if (payload.type === "result" && payload.result.target_id === state.selectedId) {
      const results = state.results.get(state.selectedId) || [];
      results.unshift(payload.result);
      state.results.set(state.selectedId, results.slice(0, HISTORY_LIMIT));
      renderResults(state.results.get(state.selectedId));
      scheduleChartReload();
    }

    if (payload.type === "targets_changed") {
      await loadAll();
    }
  };
}

function openTargetDialog(target = null) {
  state.editingId = target?.id || null;
  els.targetDialogTitle.textContent = target ? "编辑目标" : "添加目标";
  formField(els.targetForm, "name").value = target?.name || "";
  formField(els.targetForm, "host").value = target?.host || "";
  formField(els.targetForm, "probe_type").value = target?.probe_type || "icmp";
  formField(els.targetForm, "port").value = target?.port || "";
  formField(els.targetForm, "interval_ms").value = target?.interval_ms || settings.defaultIntervalMs;
  formField(els.targetForm, "timeout_ms").value = target?.timeout_ms || settings.defaultTimeoutMs;
  els.targetDialog.showModal();
}

els.targetForm.addEventListener("submit", async (event) => {
  event.preventDefault();
  const data = new FormData(els.targetForm);
  const payload = targetPayloadFromForm(data);
  if (state.editingId) {
    await api(`/api/targets/${state.editingId}`, { method: "PUT", body: JSON.stringify(payload) });
  } else {
    await api("/api/targets", { method: "POST", body: JSON.stringify(payload) });
  }
  els.targetDialog.close();
  state.editingId = null;
  await loadAll();
});

function targetPayloadFromForm(data) {
  const probeType = data.get("probe_type");
  const portValue = data.get("port");
  return {
    name: String(data.get("name")).trim(),
    host: String(data.get("host")).trim(),
    probe_type: probeType,
    port: probeType === "tcp" && portValue ? Number(portValue) : null,
    interval_ms: Number(data.get("interval_ms") || settings.defaultIntervalMs),
    timeout_ms: Number(data.get("timeout_ms") || settings.defaultTimeoutMs),
    enabled: true,
    group_name: null,
    description: null,
  };
}

els.settings.addEventListener("click", () => {
  formField(els.settingsForm, "default_interval_ms").value = settings.defaultIntervalMs;
  formField(els.settingsForm, "default_timeout_ms").value = settings.defaultTimeoutMs;
  els.settingsDialog.showModal();
});

els.settingsForm.addEventListener("submit", (event) => {
  event.preventDefault();
  settings.defaultIntervalMs = Number(formField(els.settingsForm, "default_interval_ms").value || 1000);
  settings.defaultTimeoutMs = Number(formField(els.settingsForm, "default_timeout_ms").value || 1000);
  saveSettings();
  els.settingsDialog.close();
  redrawCurrentChart();
});

els.list.addEventListener("click", () => {
  els.targetsText.value = state.targets
    .map((target) => `${target.host}${target.port ? `:${target.port}` : ""}`)
    .join("\n");
  els.listDialog.showModal();
});

els.listForm.addEventListener("submit", async (event) => {
  event.preventDefault();
  const existing = new Set(state.targets.map((target) => `${target.host}:${target.port || ""}`));
  const lines = els.targetsText.value.split(/\r?\n/).map((line) => line.trim()).filter(Boolean);
  const targets = [];

  for (const line of lines) {
    const parsed = parseListLine(line);
    if (!parsed || existing.has(`${parsed.host}:${parsed.port || ""}`)) continue;
    targets.push({
      name: line,
      host: parsed.host,
      probe_type: parsed.port ? "tcp" : "icmp",
      port: parsed.port,
      interval_ms: settings.defaultIntervalMs,
      timeout_ms: settings.defaultTimeoutMs,
      enabled: true,
      group_name: null,
      description: null,
    });
  }

  if (targets.length > 0) {
    await api("/api/targets/bulk", {
      method: "POST",
      body: JSON.stringify({ targets }),
    });
  }

  els.listDialog.close();
  await loadAll();
});

function parseListLine(line) {
  const ipv6 = line.includes(":") && line.split(":").length > 2;
  if (ipv6) return { host: line, port: null };
  const match = line.match(/^(.+):(\d+)$/);
  if (!match) return { host: line, port: null };
  const port = Number(match[2]);
  if (!Number.isInteger(port) || port < 1 || port > 65535) return null;
  return { host: match[1], port };
}

function formField(form, name) {
  return form.elements.namedItem(name);
}

els.add.addEventListener("click", () => openTargetDialog());
els.refresh.addEventListener("click", loadAll);
els.filter.addEventListener("input", () => {
  state.filter = els.filter.value;
  renderTargets();
});
els.edit.addEventListener("click", () => {
  const target = state.targets.find((item) => item.id === state.selectedId);
  if (target) openTargetDialog(target);
});
els.start.addEventListener("click", () => selectedAction("enable"));
els.stop.addEventListener("click", () => selectedAction("disable"));
els.clearData.addEventListener("click", async () => {
  if (!state.selectedId) return;
  const target = state.targets.find((item) => item.id === state.selectedId);
  const label = target ? `${target.name} (${target.host}${target.port ? `:${target.port}` : ""})` : "当前目标";
  if (!window.confirm(`确定清空 ${label} 的历史数据吗？`)) return;

  await api(`/api/targets/${state.selectedId}/clear-data`, { method: "POST" });
  state.results.set(state.selectedId, []);
  state.chartResults.set(state.selectedId, []);
  state.failureEvents.set(state.selectedId, []);
  await loadAll();
});
els.delete.addEventListener("click", async () => {
  if (!state.selectedId) return;
  await api(`/api/targets/${state.selectedId}`, { method: "DELETE" });
  state.selectedId = null;
  await loadAll();
});

document.querySelectorAll("[data-close-dialog]").forEach((button) => {
  button.addEventListener("click", () => {
    document.querySelector(`#${button.dataset.closeDialog}`).close();
  });
});

let draggingSplitter = false;
let draggingDetailsSplitter = false;

els.splitter.addEventListener("pointerdown", (event) => {
  draggingSplitter = true;
  els.splitter.setPointerCapture(event.pointerId);
  document.body.classList.add("resizing");
});

els.splitter.addEventListener("pointermove", (event) => {
  if (!draggingSplitter) return;
  const rect = els.contentGrid.getBoundingClientRect();
  const offset = event.clientY - rect.top;
  const percent = Math.min(78, Math.max(28, (offset / rect.height) * 100));
  els.contentGrid.style.setProperty("--main-height", `${percent}%`);
  redrawCurrentChart();
});

els.splitter.addEventListener("pointerup", (event) => {
  draggingSplitter = false;
  els.splitter.releasePointerCapture(event.pointerId);
  document.body.classList.remove("resizing");
  redrawCurrentChart();
});

els.detailsSplitter.addEventListener("pointerdown", (event) => {
  draggingDetailsSplitter = true;
  els.detailsSplitter.setPointerCapture(event.pointerId);
  document.body.classList.add("resizing");
});

els.detailsSplitter.addEventListener("pointermove", (event) => {
  if (!draggingDetailsSplitter) return;
  const rect = els.detailsRow.getBoundingClientRect();
  const percent = Math.min(68, Math.max(28, ((event.clientX - rect.left) / rect.width) * 100));
  els.detailsRow.style.setProperty("--history-width", `${percent}%`);
  localStorage.setItem("pinginfo.historyWidth", String(percent));
  redrawCurrentChart();
});

els.detailsSplitter.addEventListener("pointerup", (event) => {
  draggingDetailsSplitter = false;
  els.detailsSplitter.releasePointerCapture(event.pointerId);
  document.body.classList.remove("resizing");
  redrawCurrentChart();
});

els.chart.addEventListener("pointerdown", (event) => {
  chartDrag = {
    x: event.clientX,
    baseEnd: state.chartWindowEnd || new Date(),
    moved: false,
  };
  els.chart.setPointerCapture(event.pointerId);
  els.chart.classList.add("dragging");
});

els.chart.addEventListener("pointermove", (event) => {
  if (!chartDrag) {
    const point = nearestChartPoint(event);
    if (!point) {
      els.chartTooltip.hidden = true;
      return;
    }
    showChartTooltip(point);
    return;
  }

  const rect = els.chart.getBoundingClientRect();
  const dx = event.clientX - chartDrag.x;
  if (Math.abs(dx) < 4) return;
  chartDrag.moved = true;
  const minutesDelta = (dx / Math.max(1, rect.width)) * state.chartWindowMinutes;
  const nextEnd = new Date(chartDrag.baseEnd.getTime() - minutesDelta * 60 * 1000);
  state.chartWindowEnd = nextEnd > new Date() ? null : nextEnd;
  scheduleChartReload(true);
});

els.chart.addEventListener("pointerup", (event) => {
  if (!chartDrag) return;
  els.chart.releasePointerCapture(event.pointerId);
  els.chart.classList.remove("dragging");
  chartDrag = null;
});

els.chart.addEventListener("pointerleave", () => {
  if (!chartDrag) els.chartTooltip.hidden = true;
});

els.chart.addEventListener("click", (event) => {
  if (chartDrag?.moved) return;
  const point = nearestChartPoint(event);
  if (point) showChartTooltip(point);
});

els.chartLatest.addEventListener("click", async () => {
  state.chartWindowEnd = null;
  await loadSelectedChartResults();
});

els.chartRangeButtons.forEach((button) => {
  button.addEventListener("click", async () => {
    const minutes = Number(button.dataset.rangeMinutes);
    if (!CHART_WINDOW_OPTIONS.includes(minutes) || minutes === state.chartWindowMinutes) return;
    state.chartWindowMinutes = minutes;
    state.chartWindowEnd = null;
    saveSettings();
    syncChartRangeButtons();
    await loadSelectedChartResults();
  });
});

window.addEventListener("resize", redrawCurrentChart);

function redrawCurrentChart() {
  if (!state.selectedId) return;
  const results = state.chartResults.get(state.selectedId) || [];
  const failureEvents = state.failureEvents.get(state.selectedId) || [];
  const endAt = state.chartWindowEnd || new Date();
  const startAt = new Date(endAt.getTime() - state.chartWindowMinutes * 60 * 1000);
  renderLatencyChart(results, startAt, endAt, failureEvents);
}

function scheduleChartReload(immediate = false) {
  if (chartReloadTimer) {
    window.clearTimeout(chartReloadTimer);
    chartReloadTimer = null;
  }

  const delay = immediate ? 120 : 1000;
  chartReloadTimer = window.setTimeout(async () => {
    chartReloadTimer = null;
    await loadSelectedChartResults();
  }, delay);
}

function nearestChartPoint(event) {
  if (chartPoints.length === 0) return null;
  const rect = els.chart.getBoundingClientRect();
  const x = event.clientX - rect.left;
  const y = event.clientY - rect.top;
  const index = nearestPointIndexByX(x);
  const candidates = [];
  for (let i = Math.max(0, index - 2); i <= Math.min(chartPoints.length - 1, index + 2); i += 1) {
    candidates.push(chartPoints[i]);
  }

  let nearest = null;
  let distance = Infinity;
  candidates.forEach((point) => {
    const dx = point.x - x;
    const dy = point.y - y;
    const current = Math.sqrt(dx * dx + dy * dy);
    if (current < distance) {
      distance = current;
      nearest = point;
    }
  });
  return distance <= 16 ? nearest : null;
}

function nearestPointIndexByX(x) {
  let low = 0;
  let high = chartPoints.length - 1;
  while (low < high) {
    const mid = Math.floor((low + high) / 2);
    if (chartPoints[mid].x < x) {
      low = mid + 1;
    } else {
      high = mid;
    }
  }
  if (low === 0) return 0;
  const prev = chartPoints[low - 1];
  const curr = chartPoints[low];
  return Math.abs(prev.x - x) <= Math.abs(curr.x - x) ? low - 1 : low;
}

function showChartTooltip(point) {
  const result = point.result;
  const latencyLine = result.success
    ? `Ping ${fmtMs(result.latency_ms)} ms`
    : `失败 ${escapeHtml(result.error_message || result.error_kind || "timeout")}`;
  const ttlLine = result.ttl !== null && result.ttl !== undefined ? `<br>TTL ${result.ttl}` : "";
  const ipLine = result.resolved_ip ? `<br>IP ${escapeHtml(result.resolved_ip)}` : "";
  els.chartTooltip.innerHTML = `${fmtTime(result.finished_at)}<br>${latencyLine}${ttlLine}${ipLine}`;
  els.chartTooltip.hidden = false;
  els.chartTooltip.style.left = `${Math.min(point.x + 12, els.chart.clientWidth - 220)}px`;
  els.chartTooltip.style.top = `${Math.max(8, point.y - 40)}px`;
}

async function selectedAction(action) {
  if (!state.selectedId) return;
  await api(`/api/targets/${state.selectedId}/${action}`, { method: "POST" });
  await loadAll();
}

function escapeHtml(value) {
  return String(value)
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;")
    .replaceAll("'", "&#039;");
}

renderSettingsHint();
syncChartRangeButtons();

const savedHistoryWidth = Number(localStorage.getItem("pinginfo.historyWidth"));
if (savedHistoryWidth >= 28 && savedHistoryWidth <= 68) {
  els.detailsRow.style.setProperty("--history-width", `${savedHistoryWidth}%`);
}

loadAll().catch((error) => {
  els.connection.textContent = error.message;
});
connectEvents();
