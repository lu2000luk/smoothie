import express from "express";
import path from "node:path";
import fs from "node:fs";
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

const CONFIG_PATH = path.join(PROJECT_ROOT, "hypervisor", "config.json");
const PKG_DIR = "/tmp/smoothie-pkg";
const PKG_TAR = "example_app.tar";

app.use(express.json());

/* ─── environment detection ─────────────────────────────────────── */

app.get("/api/env", async (_req, res) => {
  const wsl = await detectWsl();
  const useWsl = runtime.isWindows && wsl.available;
  const [tools, distro, native] = await Promise.all([
    detectTools(useWsl),
    detectDistro(useWsl),
    detectWindowsNative(),
  ]);
  const installs = {};
  for (const name of Object.keys(tools)) {
    installs[name] = getInstallRecipe(name, distro);
  }
  const runtimeCfg = getRuntimeConfig();
  res.json({
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
  });
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

app.post("/api/run", async (req, res) => {
  const { actionId, params = {}, runtime: runtimeOverride } = req.body ?? {};
  const action = getAction(actionId);
  if (!action) return res.status(404).json({ error: `Unknown action: ${actionId}` });

  // Refuse to run two long-running actions of the same id simultaneously
  if (action.longRunning) {
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

app.get("/api/config", (_req, res) => {
  if (fs.existsSync(CONFIG_PATH)) {
    try {
      res.json({ exists: true, config: JSON.parse(fs.readFileSync(CONFIG_PATH, "utf8")) });
      return;
    } catch (err) {
      res.status(500).json({ error: `config.json is not valid JSON: ${err.message}` });
      return;
    }
  }
  res.json({ exists: false, config: null });
});

app.post("/api/config", (req, res) => {
  const { config } = req.body ?? {};
  if (!config || typeof config !== "object") {
    return res.status(400).json({ error: "Body must be { config: {...} }" });
  }
  // Basic shape validation mirroring hypervisor expectations
  const required = ["s3", "redis", "port", "host", "engine"];
  for (const key of required) {
    if (!(key in config)) return res.status(400).json({ error: `Missing required key: ${key}` });
  }
  for (const key of ["socket"]) {
    if (!config.engine || typeof config.engine !== "object" || !(key in config.engine))
      return res.status(400).json({ error: `Missing engine.${key}` });
  }
  try {
    fs.writeFileSync(CONFIG_PATH, JSON.stringify(config, null, 2) + "\n");
    res.json({ ok: true, path: CONFIG_PATH });
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
});

/* ─── static frontend (production) ──────────────────────────────── */

const DIST = path.join(__dirname, "..", "dist");
app.use(express.static(DIST));
app.get(/^\/(?!api|stream).*/, (_req, res) => {
  res.sendFile(path.join(DIST, "index.html"));
});

/* ─── boot ──────────────────────────────────────────────────────── */

const PORT = process.env.ADMIN_PORT || 3111;
app.listen(PORT, () => {
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
