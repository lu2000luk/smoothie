import type { Action, RunRequest, EnvInfo, JobInfo, RuntimeConfig, RuntimeOverride, PreviewStep, SmoothieConfig } from "./types";

export class BackendUnreachableError extends Error {
  constructor() {
    super(
      "Admin API server is not reachable (http://localhost:4102). Run `npm run dev` in admin/ (starts web + api) or `node server/index.js` alongside Vite."
    );
    this.name = "BackendUnreachableError";
  }
}

export function isBackendUnreachable(err: unknown): boolean {
  return (
    err instanceof BackendUnreachableError ||
    (err instanceof TypeError && /fetch|network|load/i.test((err as Error).message))
  );
}

async function json<T>(res: Response): Promise<T> {
  if (!res.ok) {
    let msg = res.statusText;
    try {
      const body = await res.json();
      msg = body.error ?? msg;
    } catch {
      /* ignore */
    }
    throw new Error(msg);
  }
  return res.json() as Promise<T>;
}

async function fetchJson<T>(input: string, init?: RequestInit): Promise<T> {
  let res: Response;
  try {
    res = await fetch(input, init);
  } catch (err) {
    if ((err as Error)?.name === "AbortError") throw err;
    throw new BackendUnreachableError();
  }
  return json<T>(res);
}

export const api = {
  env: (opts?: { fresh?: boolean; signal?: AbortSignal }) =>
    fetchJson<EnvInfo>(opts?.fresh ? "/api/env?fresh=1" : "/api/env", opts?.signal ? { signal: opts.signal } : undefined),

  actions: () => fetchJson<Action[]>("/api/actions"),

  jobs: () => fetchJson<JobInfo[]>("/api/jobs"),

  config: () => fetchJson<{ exists: boolean; config: SmoothieConfig | null }>("/api/config"),

  saveConfig: (config: unknown) =>
    fetchJson<{ ok: boolean; path: string }>("/api/config", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ config }),
    }),

  /** Preview the exact command lines for an action before running */
  preview: (actionId: string, params?: Record<string, string | number | boolean>, runtime?: RuntimeOverride) =>
    fetchJson<{ steps: PreviewStep[] }>("/api/preview", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ actionId, params, runtime }),
    }),

  /** Stored Windows execution prefs (docker engine / build mode). */
  runtime: () => fetchJson<RuntimeConfig>("/api/runtime"),

  setRuntime: (patch: RuntimeOverride) =>
    fetchJson<RuntimeConfig>("/api/runtime", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(patch),
    }),

  run: (req: RunRequest) =>
    fetchJson<{ jobId: string }>("/api/run", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(req),
    }),

  /** 1-click install of a missing toolchain binary (distro-aware). Returns the job id. */
  install: (tool: string) =>
    fetchJson<{ jobId: string }>("/api/install", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ tool }),
    }),

  kill: (jobId: string) => fetchJson<{ ok: boolean }>(`/api/kill/${jobId}`, { method: "POST" }),

  /** Stop a (long-)running action by action id — no job id needed. */
  stop: (actionId: string) =>
    fetchJson<{ ok: boolean; jobIds: string[] }>(`/api/stop/${encodeURIComponent(actionId)}`, { method: "POST" }),

  /** Restart a job: stops it (if running) and starts a new job with the same spec. */
  restart: (jobId: string) =>
    fetchJson<{ ok: boolean; jobId: string }>(`/api/restart/${jobId}`, { method: "POST" }),

  /** Stop the admin portal api process itself. */
  stopPortal: () =>
    fetchJson<{ ok: boolean; stopped: boolean }>("/api/portal/stop", { method: "POST" }),

  /** Restart the admin portal api (detached replacement + self-exit). */
  restartPortal: () =>
    fetchJson<{ ok: boolean; restarting: boolean }>("/api/portal/restart", { method: "POST" }),

  /** SSE stream of job logs; returns an EventSource */
  stream: (jobId: string, handlers: {
    onLog: (line: { stream: string; text: string; ts: number }) => void;
    onStatus: (status: string, exitCode?: number | null) => void;
    onError?: (err: Event) => void;
  }) => {
    const es = new EventSource(`/api/stream/${jobId}`);
    es.addEventListener("log", (e) => {
      try {
        handlers.onLog(JSON.parse((e as MessageEvent).data));
      } catch { /* ignore */ }
    });
    es.addEventListener("status", (e) => {
      try {
        const d = JSON.parse((e as MessageEvent).data);
        handlers.onStatus(d.status, d.exitCode);
        if (d.status !== "running") es.close();
      } catch { /* ignore */ }
    });
    es.onerror = (e) => {
      handlers.onError?.(e);
      es.close();
    };
    return es;
  },
};
