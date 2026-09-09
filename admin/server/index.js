import express from "express";
import path from "node:path";
import fs from "node:fs";
import { spawn } from "node:child_process";
import { fileURLToPath } from "node:url";
import { actions, getAction } from "./actions.js";
import {
  JobRunner,
  detectWsl,
  detectTools,
  detectDistro,
  detectWindowsNative,
  getRuntimeConfig,
  setRuntimeConfig,
  effectiveDockerEngine,
  CROSS_TARGET,
  getInstallRecipe,
  INSTALLABLE_TOOLS,
  buildPlan,
  substituteParams,
  runtimeParams,
  needsWsl,
  PROJECT_ROOT,
  runtime,
} from "./runner.js";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const app = express();
const runner = new JobRunner();

const CONFIG_PATHS = {
  hypervisor: path.join(PROJECT_ROOT, "hypervisor", "config.json"),
  router: path.join(PROJECT_ROOT, "router", "config.json"),
  api: path.join(PROJECT_ROOT, "api", "config.json"),
};
// Back-compat alias for the single-config UI.
const CONFIG_PATH = CONFIG_PATHS.hypervisor;

function resolveConfigTarget(input) {
  const t = String(input ?? "hypervisor").toLowerCase();
  if (t === "hypervisor" || t === "router" || t === "api") return t;
  return null;
}

app.use(express.json());

/* ─── environment detection ─────────────────────────────────────── */

// Short-lived cache: React StrictMode (dev) mounts effects twice, which
// fired two full WSL probe sweeps back-to-back and kept the UI in
// "loading" with Re-check disabled. Re-check bypasses via ?fresh=1.
let envCache = { at: 0, payload: null };
const ENV_CACHE_TTL_MS = 8000;

function withTimeout(promise, ms, label) {
  let timer;
  const timeout = new Promise((resolve) =>
    (timer = setTimeout(() => resolve({ __timeout: true, label }), ms))
  );
  return Promise.race([promise.then((v) => ({ __timeout: false, value: v })), timeout]).finally(() =>
    clearTimeout(timer)
  );
}

app.get("/api/env", async (req, res) => {
  try {
    const fresh = req.query.fresh === "1";
    if (!fresh && envCache.payload && Date.now() - envCache.at < ENV_CACHE_TTL_MS) {
      return res.json(envCache.payload);
    }
    const wsl = await detectWsl();
    const useWsl = runtime.isWindows && wsl.available;
    const [toolsRes, distroRes, nativeRes] = await Promise.all([
      withTimeout(detectTools(useWsl), 20000, "tools"),
      withTimeout(detectDistro(useWsl), 18000, "distro"),
      withTimeout(detectWindowsNative(), 12000, "native"),
    ]);
    const tools = toolsRes.__timeout ? {} : toolsRes.value;
    const distro = distroRes.__timeout
      ? { id: "unknown", family: "unknown", pretty: "Detection timed out", version: "", supported: false }
      : distroRes.value;
    const native = nativeRes.__timeout
      ? { docker: { available: false }, cargo: { available: false }, zig: { available: false } }
      : nativeRes.value;
    if (toolsRes.__timeout) {
      console.warn("GET /api/env: tool detection timed out after 20s — returning partial env");
    }
  const installs = {};
  for (const name of Object.keys(tools)) {
    installs[name] = getInstallRecipe(name, distro);
  }
  const runtimeCfg = getRuntimeConfig();
  const payload = {
    platform: process.platform,
    isWindows: runtime.isWindows,
    wslAvailable: wsl.available,
    wslDistro: wsl.distro ?? undefined,
    useWsl,
    tools,
    distro,
    installs,
    projectRoot: PROJECT_ROOT,
    osInfo: runtime.osInfo,
    // Windows execution prefs: Docker Desktop native vs WSL, fast cross-compile.
    dockerNative: native.docker,
    cargoNative: native.cargo,
    zigNative: native.zig,
    dockerEngineSetting: runtimeCfg.dockerEngine,
    dockerEngine: runtime.isWindows
      ? effectiveDockerEngine(native.docker.available)
      : "wsl",
    buildMode: runtimeCfg.buildMode,
    crossTarget: CROSS_TARGET,
  };
  envCache = { at: Date.now(), payload };
  res.json(payload);
  } catch (err) {
    console.error(`GET /api/env failed: ${err?.message ?? err}`);
    res.status(500).json({ error: `Environment detection failed: ${err?.message ?? err}` });
  }
});

/* ─── runtime prefs (Windows: Docker Desktop vs WSL, cross-compile) ─── */

/**
 * Validate a per-request or stored runtime override.
 * Returns a clean {dockerEngine?, buildMode?} object or throws (→ 400).
 */
function parseRuntimeOverride(input) {
  const out = {};
  if (input == null) return out;
  if (typeof input !== "object") throw new Error("runtime must be { dockerEngine?, buildMode? }");
  if (input.dockerEngine !== undefined) {
    if (!["auto", "wsl", "windows"].includes(input.dockerEngine)) {
      throw new Error(`Unknown dockerEngine: ${input.dockerEngine} (want auto|wsl|windows)`);
    }
    out.dockerEngine = input.dockerEngine;
  }
  if (input.buildMode !== undefined) {
    if (!["wsl", "windows-cross"].includes(input.buildMode)) {
      throw new Error(`Unknown buildMode: ${input.buildMode} (want wsl|windows-cross)`);
    }
    out.buildMode = input.buildMode;
  }
  return out;
}

/** Resolve plan overrides, probing native docker.exe when 'auto' needs it. */
async function planOverrides(requested = {}) {
  const cfg = getRuntimeConfig();
  const dockerEngine = requested.dockerEngine ?? cfg.dockerEngine;
  let nativeDocker = false;
  if (runtime.isWindows && dockerEngine === "auto") {
    nativeDocker = (await detectWindowsNative()).docker.available;
  }
  return {
    dockerEngine,
    buildMode: requested.buildMode ?? cfg.buildMode,
    nativeDocker,
  };
}

app.get("/api/runtime", (_req, res) => {
  const cfg = getRuntimeConfig();
  res.json({ ...cfg, crossTarget: CROSS_TARGET });
});

app.post("/api/runtime", (req, res) => {
  try {
    const patch = parseRuntimeOverride(req.body);
    res.json({ ...setRuntimeConfig(patch), crossTarget: CROSS_TARGET });
  } catch (err) {
    res.status(400).json({ error: err.message });
  }
});

/**
 * 1-click install: runs the distro-specific install command for a tool
 * as a job (logs stream like any other action). Body: { tool }.
 */
app.post("/api/install", async (req, res) => {
  const { tool } = req.body ?? {};
  const name = String(tool ?? "").toLowerCase();
  if (!INSTALLABLE_TOOLS.includes(name)) {
    return res.status(400).json({ error: `Unknown tool: ${tool}` });
  }
  const wsl = await detectWsl();
  const useWsl = runtime.isWindows && wsl.available;
  const distro = await detectDistro(useWsl);
  const recipe = getInstallRecipe(name, distro);
  if (!recipe.supported) {
    return res.status(400).json({ error: recipe.reason });
  }
  const steps = buildPlan([{ cmd: recipe.command, note: recipe.note ?? `Install ${name}` }]);
  const jobId = runner.run({
    actionId: `install-${name}`,
    title: `Install ${name}`,
    steps,
    longRunning: false,
    params: {},
  });
  res.json({ jobId });
});

/** Preview the exact install command for a tool without running it. */
app.get("/api/install/:tool", async (req, res) => {
  const name = String(req.params.tool ?? "").toLowerCase();
  if (!INSTALLABLE_TOOLS.includes(name)) {
    return res.status(400).json({ error: `Unknown tool: ${req.params.tool}` });
  }
  const wsl = await detectWsl();
  const useWsl = runtime.isWindows && wsl.available;
  const distro = await detectDistro(useWsl);
  const recipe = getInstallRecipe(name, distro);
  if (!recipe.supported) {
    return res.status(400).json({ error: recipe.reason });
  }
  const [plan] = buildPlan([{ cmd: recipe.command, note: recipe.note }]);
  res.json({ tool: name, distro, command: recipe.command, note: recipe.note, step: { cmd: plan.cmd, cwd: plan.cwd, wsl: plan.useWsl } });
});

/* ─── actions ───────────────────────────────────────────────────── */

app.get("/api/actions", (_req, res) => {
  res.json(actions);
});

/**
 * Preview: returns the exact command lines (with params substituted and
 * WSL wrapping applied) that would run — without running anything.
 */
app.post("/api/preview", async (req, res) => {
  const { actionId, params, runtime: runtimeOverride } = req.body ?? {};
  const action = getAction(actionId);
  if (!action) return res.status(404).json({ error: `Unknown action: ${actionId}` });
  try {
    const runtimeOpts = parseRuntimeOverride(runtimeOverride);
    const allParams = { ...runtimeParams(), ...params };
    const steps = action.steps.map((s) => ({
      ...s,
      cmd: substituteParams(s.cmd, allParams),
    }));
    const plan = buildPlan(steps, await planOverrides(runtimeOpts));
    res.json({
      steps: plan.map((p) => ({
        cmd: p.cmd,
        cwd: p.cwd,
        note: p.note,
        wsl: p.useWsl,
        engine: p.engine,
        nativeWindows: p.nativeWindows,
        crossTarget: p.crossTarget ?? undefined,
      })),
    });
  } catch (err) {
    res.status(400).json({ error: err.message });
  }
});

/* ─── run / kill / jobs / logs ──────────────────────────────────── */

/**
 * Stop actions also terminate the matching tracked long-running job
 * (shell pkill alone can't reach it: the tracked proc is the wsl.exe /
 * cmd.exe wrapper, while the real server binary lives inside WSL or in a
 * child process group). Killed first so the cleanup step below can verify
 * a quiet system; orphans are caught by the pkill patterns in actions.js.
 */
const STOP_TARGETS = {
  "stop-hypervisor": "start-hypervisor",
  "stop-router": "start-router",
  "stop-api": "start-api",
  "stop-ui": "start-ui",
};

/**
 * Pipelines (stop → rebuild → run) replace any previous run of themselves
 * plus the matching Start-* service job, so re-running a pipeline never
 * 409s and never leaves two servers fighting over one port. The in-job
 * stop step (pkill by pattern) then clears orphans started outside the
 * panel; by the time the Build stage finishes, the port is free for Run.
 */
const PIPELINE_TARGETS = {
  "pipeline-hypervisor": "start-hypervisor",
  "pipeline-router": "start-router",
  "pipeline-api": "start-api",
};

app.post("/api/run", async (req, res) => {
  const { actionId, params = {}, runtime: runtimeOverride } = req.body ?? {};
  const action = getAction(actionId);
  if (!action) return res.status(404).json({ error: `Unknown action: ${actionId}` });

  // Running a Stop-* action first kills the tracked Start-* job (if any).
  // Never 409s: stopping when nothing runs is fine (shell step reports it).
  const stopTarget = STOP_TARGETS[actionId];
  if (stopTarget) runner.killAction(stopTarget);

  // Running a Pipeline-* action first replaces any previous pipeline run of
  // the same service plus the matching Start-* job (if any). Never 409s:
  // re-running a pipeline is the main operation (stop → rebuild → run).
  const pipelineTarget = PIPELINE_TARGETS[actionId];
  if (pipelineTarget) {
    runner.killAction(actionId);
    runner.killAction(pipelineTarget);
  }

  // Refuse to run two long-running actions of the same id simultaneously
  // (pipelines opt out — they auto-replace above instead of conflicting).
  if (action.longRunning && !pipelineTarget) {
    const already = [...runner.jobs.values()].find(
      (j) => j.actionId === actionId && j.status === "running"
    );
    if (already) {
      return res.status(409).json({ error: `"${action.title}" is already running`, jobId: already.id });
    }
  }

  let steps;
  try {
    const runtimeOpts = parseRuntimeOverride(runtimeOverride);
    const allParams = { ...runtimeParams(), ...params };
    const substituted = action.steps.map((s) => ({ ...s, cmd: substituteParams(s.cmd, allParams) }));
    // runtime path fixes for steps that reference /tmp inside package-example on Windows
    steps = buildPlan(substituted, await planOverrides(runtimeOpts));
  } catch (err) {
    return res.status(400).json({ error: err.message });
  }

  const jobId = runner.run({
    actionId,
    title: action.title,
    steps,
    longRunning: action.longRunning,
    params,
  });
  res.json({ jobId });
});

app.post("/api/kill/:jobId", (req, res) => {
  const ok = runner.kill(req.params.jobId);
  if (!ok) return res.status(409).json({ error: "Job not found or not running" });
  res.json({ ok: true });
});

/**
 * Stop RUN-category (or any) tasks by action id without needing the job id.
 * Body or param: { actionId } / :actionId. Kills every running job for that
 * action and returns the stopped job ids.
 */
app.post("/api/stop", (req, res) => {
  const actionId = req.body?.actionId;
  if (!actionId) return res.status(400).json({ error: "Body must be { actionId }" });
  const jobIds = runner.killAction(String(actionId));
  if (jobIds.length === 0) {
    return res.status(409).json({ error: `"${actionId}" is not running` });
  }
  res.json({ ok: true, jobIds });
});

app.post("/api/stop/:actionId", (req, res) => {
  const jobIds = runner.killAction(String(req.params.actionId));
  if (jobIds.length === 0) {
    return res.status(409).json({ error: `"${req.params.actionId}" is not running` });
  }
  res.json({ ok: true, jobIds });
});

/**
 * Restart a job by id: stops it (if running) and starts a new job with the
 * same action spec. Returns the replacement job id. Used by the Run-jobs
 * Restart buttons (hypervisor / router / ui keep running until stopped, so
 * a one-click stop + start again is the common operation).
 */
app.post("/api/restart/:jobId", async (req, res) => {
  const job = runner.getJob(req.params.jobId);
  if (!job) return res.status(404).json({ error: "Unknown job" });
  const jobId = await runner.restart(req.params.jobId);
  if (!jobId) return res.status(400).json({ error: `Job "${req.params.jobId}" cannot be restarted` });
  res.json({ ok: true, jobId });
});

app.get("/api/jobs", (_req, res) => {
  res.json(runner.listJobs());
});

app.get("/api/stream/:jobId", (req, res) => {
  const job = runner.getJob(req.params.jobId);
  if (!job) return res.status(404).json({ error: "Unknown job" });

  res.writeHead(200, {
    "Content-Type": "text/event-stream",
    "Cache-Control": "no-cache",
    Connection: "keep-alive",
  });
  res.write("retry: 1000\n\n");

  // Replay existing logs
  for (const line of job.logs) {
    res.write(`event: log\ndata: ${JSON.stringify(line)}\n\n`);
  }
  // Current status
  res.write(
    `event: status\ndata: ${JSON.stringify({ status: job.status, exitCode: job.exitCode })}\n\n`
  );

  const unsub = runner.subscribe(job.id, (line) => {
    res.write(`event: log\ndata: ${JSON.stringify(line)}\n\n`);
  });

  const statusInterval = setInterval(() => {
    res.write(
      `event: status\ndata: ${JSON.stringify({ status: job.status, exitCode: job.exitCode })}\n\n`
    );
    if (job.status !== "running") {
      clearInterval(statusInterval);
      res.end();
    }
  }, 1000);

  req.on("close", () => {
    clearInterval(statusInterval);
    unsub?.();
  });
});

/* ─── config ────────────────────────────────────────────────────── */

app.get("/api/config", (req, res) => {
  const target = resolveConfigTarget(req.query.target ?? "hypervisor");
  if (!target) return res.status(400).json({ error: "Unknown config target (want hypervisor|router|api)" });
  const file = CONFIG_PATHS[target];
  if (fs.existsSync(file)) {
    try {
      res.json({ exists: true, config: JSON.parse(fs.readFileSync(file, "utf8")), target });
      return;
    } catch (err) {
      res.status(500).json({ error: `${target}/config.json is not valid JSON: ${err.message}` });
      return;
    }
  }
  res.json({ exists: false, config: null, target });
});

function validateHypervisorConfig(config) {
  // Basic shape validation mirroring hypervisor expectations
  const required = ["s3", "redis", "port", "host", "engine"];
  for (const key of required) {
    if (!(key in config)) return `Missing required key: ${key}`;
  }
  for (const key of ["socket"]) {
    if (!config.engine || typeof config.engine !== "object" || !(key in config.engine))
      return `Missing engine.${key}`;
  }
  return null;
}

function validateRouterConfig(config) {
  // Mirrors router/src/main.rs Config + ServerConfig + S3Config
  const required = ["servers", "redis", "s3"];
  for (const key of required) {
    if (!(key in config)) return `Missing required key: ${key}`;
  }
  if (!Array.isArray(config.servers)) return "servers must be an array";
  for (let i = 0; i < config.servers.length; i++) {
    const s = config.servers[i];
    if (!s || typeof s !== "object") return `servers[${i}] must be an object`;
    for (const key of ["id", "address", "power"]) {
      if (!(key in s)) return `Missing servers[${i}].${key}`;
    }
    if (typeof s.id !== "string" || !s.id) return `servers[${i}].id must be a non-empty string`;
    if (typeof s.address !== "string" || !s.address) return `servers[${i}].address must be a non-empty string`;
    if (typeof s.power !== "number" || !Number.isFinite(s.power) || s.power < 0)
      return `servers[${i}].power must be a number >= 0`;
    if ("tunnel" in s && typeof s.tunnel !== "boolean") return `servers[${i}].tunnel must be a boolean`;
    if (s.tunnel_address != null && typeof s.tunnel_address !== "string")
      return `servers[${i}].tunnel_address must be a string`;
  }
  if (typeof config.redis !== "string" || !config.redis) return "redis must be a non-empty string";
  if (!config.s3 || typeof config.s3 !== "object") return "s3 must be an object";
  for (const key of ["access_key", "secret_key", "bucket", "region"]) {
    if (!(key in config.s3)) return `Missing s3.${key}`;
  }
  if (config.port != null && (!Number.isInteger(config.port) || config.port < 1 || config.port > 65535))
    return "port must be an integer 1-65535";
  if (config.host != null && typeof config.host !== "string") return "host must be a string";
  return null;
}

function validateApiConfig(config) {
  // Mirrors api/src/main.rs Config + S3Config.
  // router_address accepts the `router` / `router_url` aliases the Rust
  // side deserializes, but canonical written form is `router_address`.
  const routerAddress = config.router_address ?? config.router ?? config.router_url;
  if (routerAddress == null) return "Missing required key: router_address";
  const required = ["s3", "redis"];
  for (const key of required) {
    if (!(key in config)) return `Missing required key: ${key}`;
  }
  if (typeof config.redis !== "string" || !config.redis) return "redis must be a non-empty string";
  if (typeof routerAddress !== "string" || !routerAddress)
    return "router_address must be a non-empty string";
  if (!config.s3 || typeof config.s3 !== "object") return "s3 must be an object";
  for (const key of ["access_key", "secret_key", "bucket", "region"]) {
    if (!(key in config.s3)) return `Missing s3.${key}`;
  }
  if (config.port != null && (!Number.isInteger(config.port) || config.port < 1 || config.port > 65535))
    return "port must be an integer 1-65535";
  if (config.host != null && typeof config.host !== "string") return "host must be a string";
  return null;
}

app.post("/api/config", (req, res) => {
  const { config, target: rawTarget } = req.body ?? {};
  const target = resolveConfigTarget(rawTarget ?? req.query.target ?? "hypervisor");
  if (!target) return res.status(400).json({ error: "Unknown config target (want hypervisor|router|api)" });
  if (!config || typeof config !== "object") {
    return res.status(400).json({ error: "Body must be { config: {...} }" });
  }
  const err =
    target === "router"
      ? validateRouterConfig(config)
      : target === "api"
        ? validateApiConfig(config)
        : validateHypervisorConfig(config);
  if (err) return res.status(400).json({ error: err });
  try {
    const file = CONFIG_PATHS[target];
    fs.writeFileSync(file, JSON.stringify(config, null, 2) + "\n");
    res.json({ ok: true, path: file, target });
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
});

/* ─── admin portal self-management (stop / restart) ─────────────── */

function stopTrackedJobs() {
  for (const job of runner.jobs.values()) {
    if (job.status === "running") runner.kill(job.id);
  }
}

/**
 * Exit the portal process. `server.close()` first frees port 4102 so a
 * restart replacement can bind immediately (no EADDRINUSE race).
 * Exit code matters in dev (`concurrently --kill-others-on-fail`):
 *   Stop → exit 1 → vite is torn down too (full portal stop).
 *   Restart → exit 0 → vite survives, only the api process is replaced.
 */
function exitPortal(code) {
  stopTrackedJobs();
  try {
    server.close(() => process.exit(code));
    setTimeout(() => process.exit(code), 1500);
  } catch {
    setTimeout(() => process.exit(code), 200);
  }
}

/** Stop the admin portal (this api process). Frontend shows a "stopped" state. */
app.post("/api/portal/stop", (_req, res) => {
  res.json({ ok: true, stopped: true });
  setTimeout(() => exitPortal(1), 250);
});

/**
 * Restart the admin portal api: spawn a detached replacement
 * (`node server/index.js` with the same args) then exit this process.
 * In production (`npm start`, single process serving dist/) the portal
 * comes back on the same port. In dev the vite process survives
 * (exit 0 + --kill-others-on-fail) and its /api proxy reconnects.
 */
app.post("/api/portal/restart", (_req, res) => {
  res.json({ ok: true, restarting: true });
  setTimeout(() => {
    try {
      const entry = path.join(__dirname, "index.js");
      const args = process.argv.slice(2);
      const child = spawn(process.execPath, [entry, ...args], {
        detached: true,
        stdio: "ignore",
        cwd: path.join(__dirname, ".."),
        env: process.env,
      });
      child.unref();
    } catch (err) {
      console.error(`portal restart: failed to spawn replacement: ${err.message}`);
      return;
    }
    exitPortal(0);
  }, 250);
});

/* ─── static frontend (production) ──────────────────────────────── */

const DIST = path.join(__dirname, "..", "dist");
app.use(express.static(DIST));
app.get(/^\/(?!api|stream).*/, (_req, res) => {
  res.sendFile(path.join(DIST, "index.html"));
});

/* ─── boot ──────────────────────────────────────────────────────── */

const PORT = process.env.ADMIN_PORT || 4102;
const server = app.listen(PORT, () => {
  console.log(`Smoothie admin server → http://localhost:${PORT}`);
  console.log(`Project root: ${PROJECT_ROOT}`);
  if (runtime.isWindows) {
    detectWsl().then((w) =>
      console.log(`Windows detected · WSL available: ${w.available ? "yes" : "NO (commands will fail)"}`)
    );
    detectWindowsNative().then((n) =>
      console.log(
        `Windows native tools · docker.exe: ${n.docker.available ? n.docker.version ?? "yes" : "not found"} · cargo.exe: ${n.cargo.available ? n.cargo.version ?? "yes" : "not found"} · zig.exe: ${n.zig.available ? n.zig.version ?? "yes" : "not found"}`
      )
    );
    const cfg = getRuntimeConfig();
    console.log(`Runtime prefs · dockerEngine=${cfg.dockerEngine} buildMode=${cfg.buildMode}`);
  }
});
