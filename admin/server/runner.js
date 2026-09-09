import { spawn } from "node:child_process";
import path from "node:path";
import fs from "node:fs";
import os from "node:os";
import { randomUUID } from "node:crypto";
import { fileURLToPath } from "node:url";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
export const PROJECT_ROOT = path.resolve(__dirname, "..", "..");

const IS_WIN = process.platform === "win32";

/** Which WSL distro to use (overridable via SMOOTHIE_WSL_DISTRO). */
const WSL_DISTRO = process.env.SMOOTHIE_WSL_DISTRO || null;

/**
 * Linux target used for fast Windows cross-compiles. Static musl ELF runs
 * as-is inside WSL. Linking on stock Windows needs a capable linker driver —
 * the cross recipe uses cargo-zigbuild (needs `zig` on PATH), because the
 * default `cc` (e.g. MinGW) rejects musl's linker flags.
 */
export const CROSS_TARGET = "x86_64-unknown-linux-musl";

/**
 * Runtime execution preferences (Windows only — no-ops elsewhere).
 *   dockerEngine: 'auto' | 'wsl' | 'windows'
 *     'windows' runs `docker`/`podman` steps natively via Docker Desktop
 *     instead of routing them through WSL. 'auto' picks 'windows' when a
 *     native docker.exe is detected, otherwise 'wsl'.
 *   buildMode: 'wsl' | 'windows-cross' (default 'windows-cross')
 *     'windows-cross' compiles cargo steps natively on Windows (fast NTFS,
 *     warm local cache) with `--target <CROSS_TARGET>`, then runs the
 *     resulting Linux binary inside WSL ("build fast, run in WSL").
 * Overridable via SMOOTHIE_DOCKER_ENGINE / SMOOTHIE_BUILD_MODE and via
 * POST /api/runtime (in-memory, per server process).
 * SMOOTHIE_BUILD_MODE=wsl opts out to in-WSL builds; anything else (or
 * unset) uses Windows-fast cross-compiles.
 */
export const runtimeConfig = {
  dockerEngine: ["auto", "wsl", "windows"].includes(process.env.SMOOTHIE_DOCKER_ENGINE)
    ? process.env.SMOOTHIE_DOCKER_ENGINE
    : "auto",
  buildMode: process.env.SMOOTHIE_BUILD_MODE === "wsl" ? "wsl" : "windows-cross",
};

export function getRuntimeConfig() {
  return { ...runtimeConfig };
}

export function setRuntimeConfig(patch = {}) {
  if (patch.dockerEngine !== undefined) {
    if (!["auto", "wsl", "windows"].includes(patch.dockerEngine)) {
      throw new Error(`Unknown dockerEngine: ${patch.dockerEngine} (want auto|wsl|windows)`);
    }
    runtimeConfig.dockerEngine = patch.dockerEngine;
  }
  if (patch.buildMode !== undefined) {
    if (!["wsl", "windows-cross"].includes(patch.buildMode)) {
      throw new Error(`Unknown buildMode: ${patch.buildMode} (want wsl|windows-cross)`);
    }
    runtimeConfig.buildMode = patch.buildMode;
  }
  return getRuntimeConfig();
}

/** Resolve 'auto' to a concrete engine using native docker availability. */
export function effectiveDockerEngine(nativeDockerAvailable) {
  const setting = runtimeConfig.dockerEngine;
  if (setting === "windows") return "windows";
  if (setting === "wsl") return "wsl";
  return nativeDockerAvailable ? "windows" : "wsl";
}

let wslAvailability = null;

/**
 * Detect whether WSL is available on this Windows machine.
 * Checks `wsl.exe --status` output and wsl.conf presence.
 */
export async function detectWsl() {
  if (!IS_WIN) return { available: false, distro: null, reason: "not-windows" };
  if (wslAvailability) return wslAvailability;
  const available = await new Promise((resolve) => {
    let done = false;
    const finish = (v) => {
      if (done) return;
      done = true;
      clearTimeout(timer);
      resolve(v);
    };
    const p = spawn("wsl.exe", ["--status"], { windowsHide: true });
    let out = "";
    p.stdout.on("data", (d) => (out += d));
    p.on("error", () => finish(false));
    p.on("close", (code) => finish(code === 0 && /linux/i.test(out) || code === 0));
    const timer = setTimeout(() => {
      try {
        p.kill();
      } catch { /* already exited */ }
      finish(false);
    }, 5000);
  });
  wslAvailability = { available, distro: available ? WSL_DISTRO : null };
  return wslAvailability;
}

/**
 * Commands that can run natively on Windows (they exist as .exe/.cmd and
 * don't depend on a POSIX toolchain). Everything else goes through WSL.
 */
const NATIVE_ON_WINDOWS = /^(node|npm|npx|bun|bunx|pnpm|yarn)(\s|$)/;

/**
 * Node-ecosystem fallback compounds used by the ui/admin actions, e.g.
 * `(command -v bun >/dev/null 2>&1 && bun install) || npm install`.
 * They only invoke node/npm/bun, so they can run natively on Windows
 * (via cmd.exe after POSIX→cmd rewriting in buildPlan) instead of
 * requiring a duplicate Node install inside WSL.
 */
const NATIVE_FALLBACK_COMPOUND =
  /^\(command -v (bun|node|npm)\b.*\)\s*\|\|\s*(npm|bun|node|npx|bunx)\b/;

/**
 * True when the command is a direct `docker`/`podman` invocation (the thing
 * Docker Desktop provides natively on Windows). Compound `sh -c '... docker
 * ...'` wrappers stay classified as shell scripts — they keep working through
 * WSL, they just don't get the native fast-path.
 */
export function isDockerCommand(cmd) {
  return /^\s*(docker|podman)(\.exe)?(\s|$)/.test(cmd);
}

/**
 * Coarse step classification used to apply the runtime prefs:
 * 'docker' | 'cargo-build' | 'cargo-run' | 'other'.
 */
export function classifyStep(cmd) {
  const c = cmd.trim();
  if (isDockerCommand(c)) return "docker";
  if (/^cargo\s+build\b/.test(c)) return "cargo-build";
  if (/^cargo\s+(run|install|test)\b/.test(c)) return "cargo-run";
  return "other";
}

/**
 * Decide whether a given command needs to go through WSL.
 * On Windows we route the Linux toolchain (cargo, docker, just, tar, sh
 * compounds, …) through WSL, while plain Node-ecosystem commands run
 * natively. Explicit `wsl ...` commands pass through untouched.
 *
 * `overrides.dockerEngine` ('wsl'|'windows') forces where docker steps go:
 * with Docker Desktop installed, 'windows' runs `docker` natively via
 * docker.exe instead of inside WSL. Defaults to the global runtimeConfig.
 */
export function needsWsl(cmd, overrides = {}) {
  if (!IS_WIN) return false;
  const c = cmd.trim();
  if (/^wsl(\.exe)?\b/.test(c)) return false;
  if (isDockerCommand(c)) {
    // Docker steps honor the engine pref; 'auto' is resolved properly in
    // buildPlan (which knows whether docker.exe exists). Standalone, 'auto'
    // conservatively reports WSL (the historical behavior).
    const engine = overrides.dockerEngine ?? runtimeConfig.dockerEngine;
    return engine !== "windows";
  }
  // Node-ecosystem bun/npm fallbacks run natively (rewritten for cmd.exe).
  if (NATIVE_FALLBACK_COMPOUND.test(c)) return false;
  // Compound/subshell scripts use POSIX syntax -> WSL bash
  if (/^[(<{|&]/.test(c)) return true;
  if (NATIVE_ON_WINDOWS.test(c)) return false;
  return true;
}

/** Convert a Windows path (C:\foo\bar) to a WSL path (/mnt/c/foo/bar). */
export function toWslPath(p) {
  if (!/^[a-zA-Z]:[\\/]/.test(p)) return p;
  const drive = p[0].toLowerCase();
  const rest = p.slice(2).replace(/\\/g, "/").replace(/^\//, "");
  return `/mnt/${drive}/${rest}`;
}

/** Wrap a bash command line for WSL execution. */
export function wslWrap(cmd) {
  const escaped = cmd.replace(/"/g, '\\"');
  const distroArg = WSL_DISTRO ? ["-d", WSL_DISTRO] : [];
  return ["wsl", ...distroArg, "-e", "bash", "-lc", escaped].join(" ");
}

/**
 * Probe a native Windows tool (docker, cargo, zig, node, npm, bun, …) via
 * cmd.exe — no WSL. `bin` is given without extension so cmd resolves it
 * through PATHEXT (npm/node are often .cmd shims — e.g. nvm-windows,
 * volta, vite-plus — not .exe, so a hardcoded `.exe` suffix misses them).
 */
function probeNative(bin, flag = "--version") {
  return new Promise((resolve) => {
    let done = false;
    const finish = (v) => {
      if (done) return;
      done = true;
      clearTimeout(timer);
      resolve(v);
    };
    const proc = spawn(`${bin} ${flag}`, { shell: "cmd.exe", windowsHide: true });
    let out = "";
    let err = "";
    const timer = setTimeout(() => {
      try {
        proc.kill();
      } catch { /* already exited */ }
      finish({ available: false });
    }, 6000);
    proc.stdout?.on("data", (d) => (out += d));
    proc.stderr?.on("data", (d) => (err += d));
    proc.on("error", () => {
      finish({ available: false });
    });
    proc.on("close", (code) => {
      const combined = (out + "\n" + err).trim();
      if (code === 0 && combined) {
        finish({ available: true, version: combined.split("\n")[0].slice(0, 60) });
      } else {
        finish({ available: false });
      }
    });
  });
}

let windowsNativeCache = null;

/**
 * Detect Docker Desktop / Rust toolchain / zig installed natively on Windows
 * (outside WSL). Used to offer "run docker through Windows" and fast
 * Windows cross-compiles. Always {docker,cargo,zig} shaped; non-Windows
 * hosts report unavailable. (`zig version`, not `--version`.)
 */
export async function detectWindowsNative(force = false) {
  if (!IS_WIN) return { docker: { available: false }, cargo: { available: false }, zig: { available: false } };
  if (windowsNativeCache && !force) return windowsNativeCache;
  const [docker, cargo, zig] = await Promise.all([
    probeNative("docker"),
    probeNative("cargo"),
    probeNative("zig", "version"),
  ]);
  windowsNativeCache = { docker, cargo, zig };
  return windowsNativeCache;
}

/**
 * Build a fast Windows-native cross-compile command for `cargo build ...`.
 * Uses cargo-zigbuild (auto-installed once) with the zig linker — plain
 * `cargo build --target musl` does NOT link on stock Windows because the
 * default `cc` (e.g. MinGW) rejects musl's linker flags.
 *
 * QUOTING: this runs via `cmd.exe /s /c "…"` (see spawnStep), where nested
 * double-quotes truncate the command and silently no-op. Keep it QUOTELESS.
 */
export function toCrossBuildCmd(cmd) {
  const build = /--target[\s=]/.test(cmd)
    ? cmd.replace(/^cargo\s+build\b/, "cargo zigbuild")
    : cmd.replace(/^cargo\s+build\b/, "cargo zigbuild") + ` --target ${CROSS_TARGET}`;
  const ensureTarget = `(rustup target list --installed 2>NUL | findstr /C:${CROSS_TARGET} >NUL 2>&1 || rustup target add ${CROSS_TARGET} >NUL 2>&1)`;
  const ensureZigbuild = `(cargo zigbuild --version >NUL 2>&1 || cargo install cargo-zigbuild)`;
  return `${ensureTarget} & ${ensureZigbuild} & ${build}`;
}

/**
 * Same as toCrossBuildCmd but for POSIX shells (Linux/macOS running a
 * `cross: true` step, e.g. the "(Windows fast)" actions on a Linux box).
 */
export function toCrossBuildCmdSh(cmd) {
  const build = /--target[\s=]/.test(cmd)
    ? cmd.replace(/^cargo\s+build\b/, "cargo zigbuild")
    : cmd.replace(/^cargo\s+build\b/, "cargo zigbuild") + ` --target ${CROSS_TARGET}`;
  return `rustup target list --installed 2>/dev/null | grep -q ${CROSS_TARGET} || rustup target add ${CROSS_TARGET} >/dev/null 2>&1; (cargo zigbuild --version >/dev/null 2>&1 || cargo install cargo-zigbuild); ${build}`;
}

/**
 * Rewrite `cargo run ...` so a windows-cross build isn't rebuilt: execute
 * the previously cross-built Linux ELF directly inside WSL instead of
 * `cargo run` (which would recompile for the WSL toolchain from scratch).
 * Binary name follows the cargo convention (package dir == binary name).
 */
export function toCrossRunCmd(cmd, cwdRel) {
  const inner = cmd.trim().replace(/^cargo\s+run\b/, "");
  if (/--target[\s=]/.test(inner)) return cmd;
  const bin = path.basename(cwdRel);
  const rest = inner.replace(/--release\b/, "").trim();
  return [`./target/${CROSS_TARGET}/release/${bin}`, rest].filter(Boolean).join(" ");
}

/**
 * Shared package staging dir — lives INSIDE the project so it is visible
 * from Windows (native Docker Desktop), WSL (via /mnt/c/…) and native
 * Linux at the same time. The old `/tmp/smoothie-pkg` split-brain put the
 * tar in WSL's /tmp (invisible to Docker Desktop) so `mc cp /pkg/*.tar`
 * failed with "Requested path not found".
 */
export const PKG_DIR_ABS = path.join(PROJECT_ROOT, ".tmp", "smoothie-pkg");
export const PKG_TAR = "example_app.tar";
/** Legacy location kept for backward-compat rewriting (see buildPlan). */
export const LEGACY_PKG_DIR = "/tmp/smoothie-pkg";

/**
 * Build the execution plan for a list of steps.
 * Returns resolved absolute cwd + final command string per step.
 *
 * `overrides` (all optional):
 *   dockerEngine: 'auto'|'wsl'|'windows' — where docker steps run.
 *   buildMode: 'wsl'|'windows-cross' — native fast cross-compile or not.
 *   nativeDocker: boolean — probed docker.exe availability used to resolve
 *     'auto' (defaults to false = conservative WSL when unknown).
 * Steps may also carry author hints: `forceNativeWindows` / `forceWsl`,
 * plus `cross: true` ("this cargo build should target Linux" — used by the
 * one-click Windows-fast actions and honored on every platform).
 */
export function buildPlan(steps, overrides = {}) {
  const buildMode = overrides.buildMode ?? runtimeConfig.buildMode;
  const crossActive = IS_WIN && buildMode === "windows-cross";
  const engineSetting = overrides.dockerEngine ?? runtimeConfig.dockerEngine;
  const engine =
    engineSetting === "windows" ? "windows" : engineSetting === "wsl" ? "wsl" : overrides.nativeDocker ? "windows" : "wsl";

  return steps.map((step) => {
    const relCwd = step.cwd ? path.resolve(PROJECT_ROOT, step.cwd) : PROJECT_ROOT;
    const scope = classifyStep(step.cmd);
    let cmd = step.cmd;
    let note = step.note;
    let useWsl = needsWsl(step.cmd, { dockerEngine: engine });
    let nativeWindows = false;
    let crossTarget = null;

    if (step.forceNativeWindows && IS_WIN) {
      useWsl = false;
      nativeWindows = true;
    } else if (step.forceWsl && IS_WIN) {
      useWsl = true;
    } else if (scope === "docker" && IS_WIN) {
      useWsl = engine !== "windows";
      nativeWindows = !useWsl;
    } else if (scope === "cargo-build" && (crossActive || step.cross)) {
      // Fast cross-compile: native cargo + zig linker on Windows (cmd.exe),
      // zigbuild recipe via bash elsewhere. `cross: true` steps always take
      // this path; plain cargo builds only when buildMode is windows-cross.
      if (IS_WIN) {
        cmd = toCrossBuildCmd(cmd);
        useWsl = false;
        nativeWindows = true;
      } else {
        cmd = toCrossBuildCmdSh(cmd);
      }
      note = note ? `${note} [cross-compile → ${CROSS_TARGET}${IS_WIN ? ", runs natively" : ""}]` : `Cross-compile for Linux ${CROSS_TARGET} (needs zig on PATH; first run installs cargo-zigbuild).`;
      crossTarget = CROSS_TARGET;
    } else if (scope === "cargo-run" && crossActive) {
      cmd = toCrossRunCmd(cmd, relCwd);
      note = note ? `${note} [runs Windows cross-built Linux binary in WSL]` : `Runs the Windows cross-built Linux binary (${CROSS_TARGET}) inside WSL — no rebuild.`;
      useWsl = true;
    } else if (IS_WIN && NATIVE_FALLBACK_COMPOUND.test(cmd.trim())) {
      // Node-ecosystem fallback compounds run natively on Windows.
      useWsl = false;
      nativeWindows = true;
    }

    // POSIX → cmd.exe rewrite for natively-run Node fallback compounds:
    // `command -v` → `where`, `/dev/null` → `NUL`. cmd.exe supports the
    // same `( … ) || …` grouping and `&&`/`||` chaining used here.
    if (IS_WIN && nativeWindows && NATIVE_FALLBACK_COMPOUND.test(cmd.trim())) {
      cmd = cmd.replace(/command -v/g, "where").replace(/\/dev\/null/g, "NUL");
    }

    // ── shared pkg-dir translation (Windows split-brain fix) ──
    // {tmpDir} is a project-relative staging dir (forward-slash form, e.g.
    // `C:/…/smoothie/.tmp/smoothie-pkg`). WSL bash needs the /mnt/c/… view,
    // native Docker Desktop needs the Windows view. Rewrite per execution
    // target; also rewrite the legacy /tmp/smoothie-pkg so old preview URLs
    // and hardcoded scripts keep working.
    if (IS_WIN) {
      const winSlash = PKG_DIR_ABS.replace(/\\/g, "/");
      const winBack = PKG_DIR_ABS;
      const wslPkg = toWslPath(PKG_DIR_ABS);
      if (useWsl) {
        if (scope === "docker") {
          // Docker via WSL still talks to the Docker Desktop daemon (Windows),
          // so `-v` must keep the Windows `C:/…` form — /mnt/c/… is unknown
          // to the daemon. Only map the legacy /tmp path to the shared dir.
          if (cmd.includes("\\")) cmd = cmd.split(winBack).join(winSlash);
          if (cmd.includes(LEGACY_PKG_DIR)) cmd = cmd.split(LEGACY_PKG_DIR).join(winSlash);
        } else {
          cmd = cmd.split(winBack).join(wslPkg).split(winSlash).join(wslPkg);
          if (cmd.includes(LEGACY_PKG_DIR)) cmd = cmd.split(LEGACY_PKG_DIR).join(wslPkg);
        }
      } else {
        // Native Windows (cmd.exe / Docker Desktop): normalize backslashes
        // to forward slashes for `docker -v`, and map legacy /tmp path.
        if (cmd.includes("\\")) cmd = cmd.split(winBack).join(winSlash);
        if (cmd.includes(LEGACY_PKG_DIR)) cmd = cmd.split(LEGACY_PKG_DIR).join(winSlash);
        // Docker Desktop has no --network=host: mc must reach MinIO via
        // host.docker.internal instead of 127.0.0.1.
        if (scope === "docker" && cmd.includes("minio/mc")) {
          cmd = cmd.replace(/\s--network=host\b/, "");
          cmd = cmd.replace(/127\.0\.0\.1:9000/g, "host.docker.internal:9000");
        }
      }
    }

    let wslCwd = relCwd;
    // bashCmd is the raw command handed to `wsl.exe -e bash -lc` at spawn
    // time. `cmd` keeps the wsl-wrapped form for display (preview + logs),
    // i.e. exactly what you'd paste into PowerShell.
    let bashCmd = cmd;
    if (useWsl) {
      wslCwd = toWslPath(relCwd);
      cmd = wslWrap(cmd);
    } else if (IS_WIN) {
      // Any step that stays on Windows (docker via Docker Desktop, native
      // cargo, node/npm/…) runs under cmd.exe — never `bash`, which may not
      // exist on a stock Windows box.
      nativeWindows = true;
    }
    return { cmd, bashCmd, cwd: relCwd, wslCwd, useWsl, note, engine: scope, nativeWindows, crossTarget };
  });
}

/**
 * Runtime-computed params, always merged into user params at
 * preview/run time: {tmpDir}, {tarName}, {pkgDir}.
 * Forward-slash form so the same value is valid in bash, cmd.exe and
 * `docker -v` (Docker Desktop accepts `C:/…` as well as `C:\…`).
 */
export function runtimeParams() {
  const dir = PKG_DIR_ABS.replace(/\\/g, "/");
  return {
    tmpDir: dir,
    tarName: PKG_TAR,
    pkgDir: dir,
  };
}

/**
 * Substitute {param} placeholders with validated values.
 * Values are checked against a safe charset — they must never break out
 * of a placeholder or inject shell syntax.
 */
export function substituteParams(text, params) {
  return text.replace(/\{(\w+)\}/g, (match, key) => {
    const val = params?.[key];
    if (val === undefined || val === null) return match;
    const strVal = String(val);
    if (!/^[\w .:@/=+~\\\-]+$/.test(strVal)) {
      throw new Error(`Parameter "${key}" contains unsupported characters: ${strVal}`);
    }
    return strVal;
  });
}

/**
 * Replace known project paths with the WSL-compatible view for display.
 */
export function displayPath(p) {
  return p;
}

export class JobRunner {
  constructor() {
    /** @type {Map<string, import('../src/lib/types').JobInfo & {proc:any, logs:any[]}>} */
    this.jobs = new Map();
  }

  getJob(id) {
    return this.jobs.get(id);
  }

  listJobs() {
    return [...this.jobs.values()]
      .sort((a, b) => b.startedAt - a.startedAt)
      .slice(0, 50)
      .map(({ proc, logs, subscribers, spec, ...rest }) => rest);
  }

  getLogs(id) {
    return this.jobs.get(id)?.logs ?? [];
  }

  kill(jobId) {
    const job = this.jobs.get(jobId);
    if (!job || job.status !== "running" || !job.proc) return false;
    if (IS_WIN) {
      spawn("taskkill", ["/pid", String(job.proc.pid), "/T", "/F"], { windowsHide: true });
    } else {
      try {
        // Kill the whole process group (bash -c wraps everything)
        process.kill(-job.proc.pid, "SIGKILL");
      } catch {
        job.proc.kill("SIGKILL");
      }
    }
    return true;
  }

  /**
   * Stop all running jobs for a given action id (RUN category tasks are
   * long-running with at most one live job per action, but kill all matches
   * to be safe). Returns the list of stopped job ids (empty = nothing running).
   */
  killAction(actionId) {
    const targets = [...this.jobs.values()].filter(
      (j) => j.actionId === actionId && j.status === "running"
    );
    for (const job of targets) this.kill(job.id);
    return targets.map((j) => j.id);
  }

  /**
   * Restart a job: stop the old job (if still running) and start a new job
   * with the same action spec (steps already planned, incl. runtime prefs).
   * Waits briefly for the old process to exit so ports / files are freed
   * before the replacement binds. Returns the new job id, or null when the
   * job is unknown / has no restartable spec.
   */
  async restart(jobId) {
    const old = this.jobs.get(jobId);
    if (!old || !old.spec) return null;
    if (old.status === "running") {
      this.kill(old.id);
      // Give taskkill / SIGKILL a moment to free ports before rebinding.
      for (let i = 0; i < 30; i++) {
        if (old.status !== "running") break;
        await this.sleep(100);
      }
      await this.sleep(400);
    }
    const { actionId, title, steps, longRunning, params } = old.spec;
    return this.run({ actionId, title, steps, longRunning, params });
  }

  /**
   * Run an action: spawn step 1, wait, spawn step 2, ... streaming logs.
   * Returns the job id immediately.
   */
  run({ actionId, title, steps, longRunning = false, params = {} }) {
    const jobId = randomUUID();
    const job = {
      id: jobId,
      actionId,
      title,
      status: "running",
      startedAt: Date.now(),
      endedAt: undefined,
      exitCode: null,
      proc: null,
      logs: [],
      subscribers: new Set(),
      // Kept for POST /api/restart/:jobId (stop + start again with same spec).
      spec: { actionId, title, steps, longRunning, params },
    };
    this.jobs.set(jobId, job);

    (async () => {
      for (let i = 0; i < steps.length; i++) {
        const plan = steps[i];
        if (job.status !== "running") return; // killed mid-run
        if (i > 0 && !longRunning) {
          await this.sleep(150); // small gap so step boundaries render separately
        }
        this.pushLog(job, {
          stream: "system",
          text: `── Step ${i + 1}/${steps.length}${plan.note ? ` · ${plan.note}` : ""}`,
        });
        this.pushLog(job, { stream: "system", text: `$ ${plan.cmd}` });

        const exitCode = await this.spawnStep(job, plan, longRunning && i === steps.length - 1);
        if (exitCode !== 0) {
          job.status = exitCode === -1 ? "killed" : "failed";
          job.exitCode = exitCode;
          job.endedAt = Date.now();
          this.pushLog(job, {
            stream: "system",
            text: exitCode === -1 ? "✗ Stopped by user" : `✗ Step failed with exit code ${exitCode}`,
          });
          return;
        }
      }
      job.status = "success";
      job.exitCode = 0;
      job.endedAt = Date.now();
      this.pushLog(job, { stream: "system", text: "✓ Done" });
    })();

    return jobId;
  }

  spawnStep(job, plan, keepAlive) {
    return new Promise((resolve) => {
      let proc;
      if (plan.nativeWindows && IS_WIN) {
        // Native Windows execution (Docker Desktop docker.exe, native cargo
        // cross-compiles). cmd.exe syntax — NOT bash.
        proc = spawn(plan.cmd, {
          cwd: plan.cwd, // node handles win32 cwd natively
          windowsHide: true,
          shell: "cmd.exe",
          env: { ...process.env, TERM: "dumb", FORCE_COLOR: "0" },
        });
      } else if (plan.useWsl) {
        // plan.cmd is the display form (`wsl -e bash -lc "…"`); spawn the
        // raw bash command directly — passing the wrapped form here would
        // execute `wsl …` *inside* Linux and fail with exit 127.
        proc = spawn("wsl.exe", [...(WSL_DISTRO ? ["-d", WSL_DISTRO] : []), "-e", "bash", "-lc", plan.bashCmd ?? plan.cmd], {
          cwd: plan.cwd, // node handles win32 cwd natively
          windowsHide: true,
          shell: false,
          env: { ...process.env, TERM: "dumb", FORCE_COLOR: "0" },
        });
      } else {
        // bash -lc: login shell so toolchains on PATH added by .profile
        // (~/.cargo/bin, nvm, etc.) resolve, and bash features like
        // process substitution / [[ ]] work in action scripts.
        proc = spawn("bash", ["-lc", plan.cmd], {
          cwd: plan.cwd,
          windowsHide: true,
          env: { ...process.env, TERM: "dumb", FORCE_COLOR: "0" },
          detached: !IS_WIN, // own process group for clean group-kill
        });
      }
      job.proc = proc;

      const onData = (stream) => (chunk) => {
        for (const line of String(chunk).split(/\r?\n/)) {
          if (line.length) this.pushLog(job, { stream, text: line });
        }
      };
      proc.stdout?.on("data", onData("stdout"));
      proc.stderr?.on("data", onData("stderr"));

      proc.on("error", (err) => {
        this.pushLog(job, { stream: "system", text: `spawn error: ${err.message}` });
        resolve(-2);
      });

      proc.on("close", (code, signal) => {
        if (signal === "SIGKILL" || signal === "SIGTERM") resolve(-1);
        else resolve(code ?? 0);
      });
    });
  }

  pushLog(job, line) {
    const entry = { ...line, ts: Date.now() };
    job.logs.push(entry);
    if (job.logs.length > 5000) job.logs.shift();
    for (const sub of job.subscribers) {
      try {
        sub(entry);
      } catch { /* subscriber vanished */ }
    }
  }

  subscribe(jobId, fn) {
    const job = this.jobs.get(jobId);
    if (!job) return null;
    job.subscribers.add(fn);
    return () => job.subscribers.delete(fn);
  }

  sleep(ms) {
    return new Promise((r) => setTimeout(r, ms));
  }
}

/**
 * Tools that execute natively on Windows (see NATIVE_ON_WINDOWS): they are
 * probed via cmd.exe even when WSL is active, so a Windows install counts
 * as "available" instead of demanding a duplicate install inside WSL.
 */
const NATIVE_WINDOWS_TOOLS = new Set(["node", "npm", "bun"]);

/** Detect installed tools and versions for the env endpoint. */
export async function detectTools(useWsl) {
  const tools = {
    node: ["node", "--version"],
    npm: ["npm", "--version"],
    bun: ["bun", "--version"],
    cargo: ["cargo", "--version"],
    rustc: ["rustc", "--version"],
    just: ["just", "--version"],
    docker: ["docker", "--version"],
    podman: ["podman", "--version"],
  };
  // Probe in parallel: sequential WSL spawns took 5×6s (cold boot) and kept
  // the frontend's Re-check button disabled with a spinner the whole time.
  const probeOne = async ([bin, flag], nativeFirst) => {
    if (nativeFirst) {
      // Prefer the native Windows install (that's where these run);
      // fall back to the WSL distro so a Linux-only install still shows.
      const native = await probeNative(bin, flag);
      if (native.available) return native;
      if (useWsl) return probeTool(bin, flag, true);
      return native;
    }
    return probeTool(bin, flag, useWsl);
  };
  const entries = Object.entries(tools);
  const results = await Promise.all(
    entries.map(([name, spec]) =>
      probeOne(spec, IS_WIN && NATIVE_WINDOWS_TOOLS.has(name)).then((r) => [name, r])
    )
  );
  return Object.fromEntries(results);
}

/**
 * Shell snippet that makes tool probes see the same binaries an interactive
 * user shell sees. `wsl -e bash -lc` is a login (non-interactive) shell, so
 * PATH tweaks from ~/.bashrc (bun installer, nvm, volta, asdf, pipx, …)
 * are NOT applied — tools installed per-user (bun → ~/.bun/bin, node via
 * nvm/fnm/volta, cargo → ~/.cargo/bin, just → ~/.local/bin) were invisible
 * and reported as "missing" even though they work fine in the user's
 * terminal. We re-apply the known locations explicitly instead of sourcing
 * ~/.bashrc wholesale (which could print MOTD/noise into version output).
 */
function probeScript(bin, flag) {
  // `bin` is always one of the fixed keys in detectTools — safe to interpolate.
  return [
    // Drop Windows interop entries (/mnt/c/…) from PATH: WSL appends the
    // Windows PATH, so `command -v node` can resolve to a Windows shim
    // (e.g. a shell wrapper exec'ing a .exe) that fails or misbehaves under
    // Linux bash. Detection must reflect Linux-native binaries only — the
    // separate detectWindowsNative() already covers Docker Desktop etc.
    'export PATH="$(printf \'%s\' "$PATH" | tr \':\' \'\\n\' | grep -v \'^/mnt/\' | paste -sd: -)"',
    'export PATH="$HOME/.bun/bin:$HOME/.local/bin:$HOME/.cargo/bin:$HOME/.volta/bin:$HOME/.asdf/shims:/snap/bin:/usr/local/bin:$PATH"',
    '[ -f "$HOME/.cargo/env" ] && . "$HOME/.cargo/env" >/dev/null 2>&1 || true',
    'export NVM_DIR="${NVM_DIR:-$HOME/.nvm}"; [ -s "$NVM_DIR/nvm.sh" ] && . "$NVM_DIR/nvm.sh" >/dev/null 2>&1 || true',
    // fnm / volta shims that live outside PATH until their init runs
    '(command -v fnm >/dev/null 2>&1 && eval "$(fnm env --shell bash 2>/dev/null)" >/dev/null 2>&1) || true',
    // Direct fallbacks when the binary exists but still isn't on PATH
    // (e.g. user's default shell is zsh/fish so bash never got the export).
    `command -v ${bin} >/dev/null 2>&1 || { [ -x "$HOME/.bun/bin/${bin}" ] && export PATH="$HOME/.bun/bin:$PATH"; }`,
    `command -v ${bin} >/dev/null 2>&1 || { [ -x "$HOME/.cargo/bin/${bin}" ] && export PATH="$HOME/.cargo/bin:$PATH"; }`,
    `command -v ${bin} >/dev/null 2>&1 || { [ -x "$HOME/.local/bin/${bin}" ] && export PATH="$HOME/.local/bin:$PATH"; }`,
    // Merge stderr into stdout: some tools print --version to stderr.
    `${bin} ${flag} 2>&1`,
  ].join('; ');
}

function probeTool(bin, flag, useWsl) {
  return new Promise((resolve) => {
    let done = false;
    const finish = (v) => {
      if (done) return;
      done = true;
      clearTimeout(timer);
      resolve(v);
    };
    let proc;
    if (useWsl && IS_WIN) {
      proc = spawn("wsl.exe", [...(WSL_DISTRO ? ["-d", WSL_DISTRO] : []), "-e", "bash", "-lc", probeScript(bin, flag)], {
        windowsHide: true,
      });
    } else if (IS_WIN) {
      // Native Windows (no WSL): cmd.exe syntax — no POSIX `export`/`command -v`.
      proc = spawn(`${bin} ${flag} 2>&1`, { shell: "cmd.exe", windowsHide: true });
    } else {
      proc = spawn(probeScript(bin, flag), { shell: true, windowsHide: true });
    }
    let out = "";
    const timer = setTimeout(() => {
      try {
        proc.kill();
      } catch { /* already exited */ }
      finish({ available: false });
    }, 6000);
    proc.stdout?.on("data", (d) => (out += d));
    proc.on("error", () => {
      finish({ available: false });
    });
    proc.on("close", (code) => {
      if (code === 0 && out.trim()) {
        finish({ available: true, version: out.trim().split("\n")[0].slice(0, 60) });
      } else {
        finish({ available: false });
      }
    });
  });
}

export const runtime = {
  isWindows: IS_WIN,
  osInfo: `${os.type()} ${os.release()}`,
  tmpDir: os.tmpdir(),
};

/* ─── distro detection + 1-click install recipes ─────────────────── */

/** Tools that can be installed with one click (allowlist for /api/install). */
export const INSTALLABLE_TOOLS = ["node", "npm", "bun", "cargo", "rustc", "just", "docker", "podman"];

/** Map an /etc/os-release id (+ ID_LIKE) to a package-manager family. */
export function distroFamily(id, like = "") {
  const lid = String(id || "").toLowerCase();
  const llike = String(like || "").toLowerCase();
  const has = (...names) => names.some((n) => lid === n || llike.includes(n));
  if (lid === "macos" || lid === "darwin") return "brew";
  if (has("ubuntu", "debian", "linuxmint", "pop", "elementary", "zorin", "raspbian", "kali")) return "apt";
  if (has("fedora", "rhel", "centos", "rocky", "almalinux", "oracle", "amzn")) return "dnf";
  if (has("arch", "manjaro", "endeavouros", "garuda")) return "pacman";
  if (has("opensuse", "sles", "suse")) return "zypper";
  if (has("alpine")) return "apk";
  return "unknown";
}

/** Read a shell `VAR="value"` assignment out of /etc/os-release text. */
function osReleaseField(text, key) {
  const m = text.match(new RegExp(`^${key}=(.*)$`, "m"));
  if (!m) return "";
  return m[1].trim().replace(/^"|"$/g, "").replace(/^'|'$/g, "");
}

function runOsRelease(useWsl) {
  return new Promise((resolve) => {
    let proc;
    let done = false;
    const finish = (v) => {
      if (done) return;
      done = true;
      clearTimeout(timer);
      resolve(v);
    };
    if (useWsl && IS_WIN) {
      proc = spawn("wsl.exe", [...(WSL_DISTRO ? ["-d", WSL_DISTRO] : []), "-e", "bash", "-lc", "cat /etc/os-release 2>/dev/null"], {
        windowsHide: true,
      });
    } else if (process.platform === "darwin") {
      resolve(null); // macOS has no /etc/os-release — handled as brew
      return;
    } else {
      proc = spawn("cat /etc/os-release 2>/dev/null", { shell: true, windowsHide: true });
    }
    let out = "";
    // Cold WSL boots can take a while on first spawn — allow extra time.
    const timer = setTimeout(() => {
      try {
        proc.kill();
      } catch { /* already exited */ }
      finish(out.trim() || null);
    }, 15000);
    proc.stdout?.on("data", (d) => (out += d));
    proc.on("error", () => {
      finish(null);
    });
    proc.on("close", () => {
      finish(out.trim() || null);
    });
  });
}

/**
 * Detect the Linux distro that tool installs will run in
 * (inside the WSL distro when useWsl, otherwise on the server host).
 */
export async function detectDistro(useWsl) {
  if (IS_WIN && !useWsl) {
    return { id: "windows", family: "windows", pretty: "Windows (no WSL)", version: "", supported: false };
  }
  if (process.platform === "darwin" && !useWsl) {
    return { id: "macos", family: "brew", pretty: "macOS (Homebrew)", version: "", supported: true };
  }
  const text = await runOsRelease(useWsl);
  if (!text) {
    return { id: "unknown", family: "unknown", pretty: "Unknown distro", version: "", supported: false };
  }
  const id = osReleaseField(text, "ID") || "unknown";
  const like = osReleaseField(text, "ID_LIKE");
  const pretty = osReleaseField(text, "PRETTY_NAME") || id;
  const version = osReleaseField(text, "VERSION_ID");
  const family = distroFamily(id, like);
  return { id, family, pretty, version, supported: family !== "unknown" };
}

// `sudo` may not exist (root container) — fall back to running directly.
const SUDO_PREAMBLE = 'if command -v sudo >/dev/null 2>&1; then SUDO=sudo; else SUDO=; fi';

const UNIVERSAL = {
  bun: {
    command:
      'curl -fsSL https://bun.sh/install | bash && echo \'--- bun installed to ~/.bun/bin (restart your shell or run: export PATH="$HOME/.bun/bin:$PATH") ---\' && "$HOME/.bun/bin/bun" --version',
    note: "Installs bun via the official install script (works on any distro).",
  },
  rust: {
    command:
      'curl --proto \'=https\' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y && source "$HOME/.cargo/env" && rustc --version && cargo --version',
    note: "Installs the Rust toolchain via rustup (provides both cargo and rustc).",
  },
  justFallback:
    'curl --proto \'=https\' --tlsv1.2 -sSf https://just.systems/install.sh | bash -s -- --to ~/.local/bin && (just --version || ~/.local/bin/just --version)',
};

const RECIPES = {
  node: {
    apt: `${SUDO_PREAMBLE}; $SUDO apt-get update && $SUDO apt-get install -y curl ca-certificates && curl -fsSL https://deb.nodesource.com/setup_lts.x | $SUDO -E bash - && $SUDO apt-get install -y nodejs && node --version && npm --version`,
    dnf: `${SUDO_PREAMBLE}; $SUDO dnf install -y nodejs npm && node --version && npm --version`,
    pacman: `${SUDO_PREAMBLE}; $SUDO pacman -Sy --noconfirm nodejs npm && node --version && npm --version`,
    zypper: `${SUDO_PREAMBLE}; $SUDO zypper install -y nodejs npm && node --version && npm --version`,
    apk: `${SUDO_PREAMBLE}; $SUDO apk add --no-cache nodejs npm && node --version && npm --version`,
    brew: `brew install node && node --version && npm --version`,
    note: "Installs Node.js LTS (npm ships with it).",
  },
  npm: {
    apt: `${SUDO_PREAMBLE}; $SUDO apt-get update && $SUDO apt-get install -y nodejs npm && node --version && npm --version`,
    dnf: `${SUDO_PREAMBLE}; $SUDO dnf install -y nodejs npm && npm --version`,
    pacman: `${SUDO_PREAMBLE}; $SUDO pacman -Sy --noconfirm nodejs npm && npm --version`,
    zypper: `${SUDO_PREAMBLE}; $SUDO zypper install -y nodejs npm && npm --version`,
    apk: `${SUDO_PREAMBLE}; $SUDO apk add --no-cache nodejs npm && npm --version`,
    brew: `brew install node && npm --version`,
    note: "npm ships with Node.js — this installs nodejs + npm together.",
  },
  bun: {
    apt: UNIVERSAL.bun.command, dnf: UNIVERSAL.bun.command, pacman: UNIVERSAL.bun.command,
    zypper: UNIVERSAL.bun.command, apk: UNIVERSAL.bun.command, brew: UNIVERSAL.bun.command,
    note: UNIVERSAL.bun.note,
  },
  cargo: {
    apt: UNIVERSAL.rust.command, dnf: UNIVERSAL.rust.command, pacman: UNIVERSAL.rust.command,
    zypper: UNIVERSAL.rust.command, apk: UNIVERSAL.rust.command, brew: UNIVERSAL.rust.command,
    note: UNIVERSAL.rust.note,
  },
  rustc: {
    apt: UNIVERSAL.rust.command, dnf: UNIVERSAL.rust.command, pacman: UNIVERSAL.rust.command,
    zypper: UNIVERSAL.rust.command, apk: UNIVERSAL.rust.command, brew: UNIVERSAL.rust.command,
    note: UNIVERSAL.rust.note,
  },
  just: {
    apt: `${SUDO_PREAMBLE}; $SUDO apt-get update && ($SUDO apt-get install -y just || (${UNIVERSAL.justFallback}))`,
    dnf: `${SUDO_PREAMBLE}; $SUDO dnf install -y just && just --version`,
    pacman: `${SUDO_PREAMBLE}; $SUDO pacman -Sy --noconfirm just && just --version`,
    zypper: `${SUDO_PREAMBLE}; $SUDO zypper install -y just && just --version`,
    apk: `${SUDO_PREAMBLE}; $SUDO apk add --no-cache just && just --version`,
    brew: `brew install just && just --version`,
    note: "Installs the `just` command runner (apt falls back to the official install script).",
  },
  docker: {
    apt: `${SUDO_PREAMBLE}; curl -fsSL https://get.docker.com | $SUDO sh && ($SUDO systemctl enable --now docker 2>/dev/null || $SUDO service docker start 2>/dev/null || true) && (docker --version || $SUDO docker --version)`,
    dnf: `${SUDO_PREAMBLE}; curl -fsSL https://get.docker.com | $SUDO sh && ($SUDO systemctl enable --now docker 2>/dev/null || true) && (docker --version || $SUDO docker --version)`,
    pacman: `${SUDO_PREAMBLE}; curl -fsSL https://get.docker.com | $SUDO sh && ($SUDO systemctl enable --now docker 2>/dev/null || true) && (docker --version || $SUDO docker --version)`,
    zypper: `${SUDO_PREAMBLE}; curl -fsSL https://get.docker.com | $SUDO sh && ($SUDO systemctl enable --now docker 2>/dev/null || true) && (docker --version || $SUDO docker --version)`,
    apk: `${SUDO_PREAMBLE}; curl -fsSL https://get.docker.com | $SUDO sh && ($SUDO rc-update add docker boot 2>/dev/null && $SUDO service docker start 2>/dev/null || true) && (docker --version || $SUDO docker --version)`,
    brew: `brew install --cask docker && echo '--- Open Docker Desktop once to finish setup ---' && docker --version`,
    note: "Installs Docker via the official get.docker.com script (detects your distro itself). On macOS installs Docker Desktop.",
  },
  podman: {
    apt: `${SUDO_PREAMBLE}; $SUDO apt-get update && $SUDO apt-get install -y podman && podman --version`,
    dnf: `${SUDO_PREAMBLE}; $SUDO dnf install -y podman && podman --version`,
    pacman: `${SUDO_PREAMBLE}; $SUDO pacman -Sy --noconfirm podman && podman --version`,
    zypper: `${SUDO_PREAMBLE}; $SUDO zypper install -y podman && podman --version`,
    apk: `${SUDO_PREAMBLE}; $SUDO apk add --no-cache podman && podman --version`,
    brew: `brew install podman && podman --version`,
    note: "Installs Podman (Docker-compatible alternative).",
  },
};

/**
 * Return the 1-click install recipe for a tool on the detected distro.
 * Never throws for unknown tools/distros — returns { supported: false }.
 */
export function getInstallRecipe(tool, distro) {
  const name = String(tool || "").toLowerCase();
  if (!INSTALLABLE_TOOLS.includes(name)) {
    return { tool: name, supported: false, reason: `Unknown tool: ${tool}` };
  }
  const family = distro?.family ?? "unknown";
  if (family === "windows") {
    return {
      tool: name, supported: false,
      reason: "Installs run inside WSL — enable WSL first (`wsl --install`), then restart the admin server.",
    };
  }
  const recipe = RECIPES[name];
  const command = recipe?.[family];
  if (!command) {
    const label = distro?.pretty && distro.pretty !== "Unknown distro" ? distro.pretty : `distro "${distro?.id ?? "unknown"}"`;
    return {
      tool: name, supported: false,
      reason: `No auto-install for ${name} on ${label} yet — install it manually, then hit Re-check.`,
    };
  }
  return { tool: name, supported: true, command, note: recipe.note, family, distro: distro?.id ?? "unknown" };
}
