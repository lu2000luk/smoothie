import { useEffect, useState, useCallback, useRef } from "react";
import { api, isBackendUnreachable } from "@/lib/api";
import type { Action, EnvInfo, JobInfo } from "@/lib/types";
import { Tabs, TabsList, TabsTrigger, TabsContent } from "@/components/ui/tabs";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Spinner } from "@/components/ui/spinner";
import { Separator } from "@/components/ui/separator";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { ActionCard } from "@/components/ActionCard";
import { JobTerminal } from "@/components/JobTerminal";
import { ConfigGenerator } from "@/components/ConfigGenerator";
import { EnvironmentPanel } from "@/components/EnvironmentPanel";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { CupSoda, Moon, Sun, TriangleAlert, RefreshCw, ServerOff, Square, Power, RotateCcw } from "lucide-react";

const GROUPS: { id: string; label: string }[] = [
  { id: "environment", label: "Environment" },
  { id: "build", label: "Build" },
  { id: "services", label: "Services" },
  { id: "run", label: "Run" },
  { id: "package", label: "Package" },
];

function useDarkMode() {
  const [dark, setDark] = useState<boolean>(() => {
    if (typeof window === "undefined") return true;
    const stored = localStorage.getItem("smoothie-theme");
    if (stored) return stored === "dark";
    return true; // dark by default
  });

  useEffect(() => {
    const root = document.documentElement;
    root.classList.toggle("dark", dark);
    root.style.colorScheme = dark ? "dark" : "light";
    localStorage.setItem("smoothie-theme", dark ? "dark" : "light");
  }, [dark]);

  return { dark, toggle: () => setDark((v) => !v) };
}

export default function App() {
  const [env, setEnv] = useState<EnvInfo | null>(null);
  const [actions, setActions] = useState<Action[]>([]);
  const [jobs, setJobs] = useState<JobInfo[]>([]);
  const [activeJobId, setActiveJobId] = useState<string | null>(null);

  const [envLoading, setEnvLoading] = useState(true);
  const [actionsLoading, setActionsLoading] = useState(true);
  const [envError, setEnvError] = useState<string | null>(null);
  const [actionsError, setActionsError] = useState<string | null>(null);
  const [jobsError, setJobsError] = useState<string | null>(null);
  const [backendDown, setBackendDown] = useState(false);
  const [activeTab, setActiveTab] = useState("dashboard");
  const [stoppingId, setStoppingId] = useState<string | null>(null);
  const [restartingId, setRestartingId] = useState<string | null>(null);
  const [portalConfirm, setPortalConfirm] = useState<"restart" | "stop" | null>(null);
  const [portalBusy, setPortalBusy] = useState<"restart" | "stop" | null>(null);
  const [portalNotice, setPortalNotice] = useState<
    { kind: "restarting" | "restarted" | "stopped" | "error"; text: string } | null
  >(null);

  const { dark, toggle } = useDarkMode();
  const pollRef = useRef<ReturnType<typeof setInterval> | null>(null);
  // Env refresh bookkeeping: Re-check must stay clickable while a probe is
  // in flight — abort the stale request and ignore out-of-order responses
  // so a slow WSL cold-boot can't wedge the button on "loading" forever.
  const envSeqRef = useRef(0);
  const envAbortRef = useRef<AbortController | null>(null);
  // Snapshot of values refreshJobs needs, without making it unstable.
  // (If refreshJobs depended on `env`/`actions`, every loadInitial() would
  // create a new callback identity, re-trigger the effect below, call
  // loadInitial() again → infinite refresh / flicker.)
  const statusRef = useRef({ hasEnv: false, actionsLen: 0, envError: null as string | null, actionsError: null as string | null });
  statusRef.current = { hasEnv: !!env, actionsLen: actions.length, envError, actionsError };

  const refreshEnv = useCallback(async () => {
    const seq = ++envSeqRef.current;
    envAbortRef.current?.abort();
    const ctrl = new AbortController();
    envAbortRef.current = ctrl;
    setEnvLoading(true);
    setEnvError(null);
    try {
      // fresh=1 bypasses the server's 8s dedup cache (StrictMode double-mount).
      const data = await api.env({ fresh: true, signal: ctrl.signal });
      if (envSeqRef.current !== seq) return; // superseded by a newer click
      setEnv(data);
      setEnvError(null);
      setBackendDown(false);
    } catch (err) {
      if ((err as Error)?.name === "AbortError") return;
      if (envSeqRef.current !== seq) return;
      setEnvError((err as Error)?.message ?? "Failed to load environment");
    } finally {
      if (envSeqRef.current === seq) setEnvLoading(false);
    }
  }, []);

  const loadInitial = useCallback(async () => {
    const seq = ++envSeqRef.current;
    setEnvLoading(true);
    setActionsLoading(true);
    setEnvError(null);
    setActionsError(null);
    setBackendDown(false);

    // Independent fetches: one failing must not block the other.
    const [envRes, actionsRes] = await Promise.allSettled([api.env(), api.actions()]);

    if (envSeqRef.current !== seq) return; // superseded by a Re-check click

    if (envRes.status === "fulfilled") {
      setEnv(envRes.value);
      setEnvError(null);
    } else {
      setEnvError((envRes.reason as Error)?.message ?? "Failed to load environment");
    }

    if (actionsRes.status === "fulfilled") {
      setActions(actionsRes.value);
      setActionsError(null);
    } else {
      setActionsError((actionsRes.reason as Error)?.message ?? "Failed to load actions");
    }

    setEnvLoading(false);
    setActionsLoading(false);

    if (
      envRes.status === "rejected" &&
      actionsRes.status === "rejected" &&
      (isBackendUnreachable((envRes as PromiseRejectedResult).reason) ||
        isBackendUnreachable((actionsRes as PromiseRejectedResult).reason))
    ) {
      setBackendDown(true);
    }
  }, []);

  const refreshJobs = useCallback(async () => {
    const snap = statusRef.current;
    try {
      setJobs(await api.jobs());
      setJobsError(null);
      // If jobs recover, clear a backend-down banner that was jobs-only.
      setBackendDown((prev) => (snap.envError || snap.actionsError ? prev : false));
    } catch (err) {
      const msg = (err as Error)?.message ?? "Failed to load jobs";
      setJobsError(msg);
      if (isBackendUnreachable(err) && !snap.hasEnv && snap.actionsLen === 0) {
        setBackendDown(true);
      }
    }
  }, []);

  useEffect(() => {
    loadInitial();
    refreshJobs();
    pollRef.current = setInterval(refreshJobs, 3000);
    return () => {
      if (pollRef.current) clearInterval(pollRef.current);
    };
  }, [loadInitial, refreshJobs]);

  const runningJobs = jobs.filter((j) => j.status === "running");

  const runningJobFor = useCallback(
    (actionId: string) => jobs.find((j) => j.actionId === actionId && j.status === "running") ?? null,
    [jobs]
  );

  const stopJob = useCallback(
    async (jobId: string) => {
      setStoppingId(jobId);
      try {
        await api.kill(jobId);
      } catch {
        /* already finished — next poll will clear it */
      } finally {
        setStoppingId(null);
        refreshJobs();
      }
    },
    [refreshJobs]
  );

  const viewJobLogs = useCallback((jobId: string) => {
    setActiveJobId(jobId);
    setActiveTab("jobs");
  }, []);

  const restartJob = useCallback(
    async (jobId: string) => {
      setRestartingId(jobId);
      try {
        const { jobId: newJobId } = await api.restart(jobId);
        setActiveJobId(newJobId);
      } catch {
        /* restart failures surface on next poll / terminal; keep old selection */
      } finally {
        setRestartingId(null);
        refreshJobs();
      }
    },
    [refreshJobs]
  );

  const waitForPortalBack = useCallback(async () => {
    for (let i = 0; i < 30; i++) {
      await new Promise((r) => setTimeout(r, 1000));
      try {
        await api.env();
        return true;
      } catch {
        /* still down — keep polling */
      }
    }
    return false;
  }, []);

  const confirmPortalAction = useCallback(async () => {
    const kind = portalConfirm;
    if (!kind) return;
    setPortalConfirm(null);
    setPortalBusy(kind);
    setPortalNotice(
      kind === "restart"
        ? { kind: "restarting", text: "Restarting admin portal — the api is respawning…" }
        : { kind: "stopped", text: "Stopping admin portal…" }
    );
    try {
      if (kind === "restart") {
        await api.restartPortal();
        const back = await waitForPortalBack();
        if (back) {
          setPortalNotice({ kind: "restarted", text: "Admin portal restarted." });
          await loadInitial();
          await refreshJobs();
        } else {
          setPortalNotice({
            kind: "error",
            text: "Restart was requested but the api did not come back within 30s. Restart it manually: cd admin && npm run dev.",
          });
        }
      } else {
        await api.stopPortal();
        setBackendDown(true);
        setPortalNotice({
          kind: "stopped",
          text: "Admin portal stopped. Restart it manually: cd admin && npm run dev (or node server/index.js).",
        });
      }
    } catch (err) {
      // A fetch failure right after POST usually means the process already
      // exited — for restart, still wait for it to come back.
      if (kind === "restart") {
        const back = await waitForPortalBack();
        if (back) {
          setPortalNotice({ kind: "restarted", text: "Admin portal restarted." });
          await loadInitial();
          await refreshJobs();
        } else {
          setPortalNotice({
            kind: "error",
            text: `Restart request failed: ${(err as Error)?.message ?? "unknown error"}`,
          });
        }
      } else {
        setPortalNotice({
          kind: "error",
          text: `Stop request failed: ${(err as Error)?.message ?? "unknown error"}`,
        });
      }
    } finally {
      setPortalBusy(null);
    }
  }, [portalConfirm, loadInitial, refreshJobs, waitForPortalBack]);

  return (
    <div className="flex min-h-screen bg-background text-foreground">
      {/* Sidebar */}
      <aside className="sticky top-0 hidden h-screen w-60 shrink-0 flex-col border-r bg-sidebar p-4 md:flex">
        <div className="flex items-center gap-2 px-2 py-3">
          <CupSoda className="h-6 w-6 text-primary" />
          <div>
            <h1 className="text-sm font-semibold leading-tight">Smoothie Admin</h1>
            <p className="text-xs text-muted-foreground">dev &amp; ops panel</p>
          </div>
        </div>
        <Separator className="my-2" />
        <nav className="flex flex-col gap-1">
          {GROUPS.map((g) => {
            const count = actions.filter((a) => a.group === g.id).length;
            return (
              <a
                key={g.id}
                href={`#group-${g.id}`}
                className="flex items-center justify-between rounded-md px-2 py-1.5 text-sm text-sidebar-foreground/70 hover:bg-sidebar-accent hover:text-sidebar-accent-foreground"
              >
                {g.label}
                <Badge variant="secondary" className="h-5 px-1.5 text-[10px]">
                  {actionsLoading ? "…" : count}
                </Badge>
              </a>
            );
          })}
          <a
            href="#group-config"
            className="flex items-center justify-between rounded-md px-2 py-1.5 text-sm text-sidebar-foreground/70 hover:bg-sidebar-accent hover:text-sidebar-accent-foreground"
          >
            Config
            <Badge variant="secondary" className="h-5 px-1.5 text-[10px]">
              1
            </Badge>
          </a>
        </nav>

        <Separator className="my-2" />
        <div className="px-2">
          <p className="mb-1 text-xs font-medium text-muted-foreground">Running now</p>
          {jobsError && runningJobs.length === 0 ? (
            <p className="text-xs text-muted-foreground/60">Jobs unavailable</p>
          ) : runningJobs.length === 0 ? (
            <p className="text-xs text-muted-foreground/60">Nothing running</p>
          ) : (
            <div className="flex flex-col gap-1">
              {runningJobs.map((j) => (
                <div
                  key={j.id}
                  className="group flex w-full items-center gap-1 rounded-md px-1 py-0.5 hover:bg-sidebar-accent"
                >
                  <button
                    onClick={() => viewJobLogs(j.id)}
                    className="flex min-w-0 flex-1 items-center gap-2 rounded px-1 py-0.5 text-left text-xs"
                    title={`View logs for ${j.title}`}
                  >
                    <Spinner className="h-3 w-3 shrink-0" />
                    <span className="truncate">{j.title}</span>
                  </button>
                  <button
                    onClick={() => stopJob(j.id)}
                    disabled={stoppingId === j.id}
                    title={`Stop ${j.title}`}
                    aria-label={`Stop ${j.title}`}
                    className="shrink-0 rounded p-1 text-muted-foreground opacity-60 hover:bg-destructive/10 hover:text-destructive-foreground hover:opacity-100 disabled:opacity-40"
                  >
                    <Square className="h-3 w-3" />
                  </button>
                  <button
                    onClick={() => restartJob(j.id)}
                    disabled={restartingId === j.id}
                    title={`Restart ${j.title} (stop + start again)`}
                    aria-label={`Restart ${j.title}`}
                    className="shrink-0 rounded p-1 text-muted-foreground opacity-60 hover:bg-accent hover:text-foreground hover:opacity-100 disabled:opacity-40"
                  >
                    <RotateCcw className={`h-3 w-3 ${restartingId === j.id ? "animate-spin" : ""}`} />
                  </button>
                </div>
              ))}
            </div>
          )}
        </div>

        <div className="mt-auto px-2 text-[11px] text-muted-foreground/60">
          {envLoading ? (
            "connecting…"
          ) : env ? (
            <>
              <p>{env.osInfo}</p>
              {env.isWindows && (
                <p className={env.useWsl ? "text-success-foreground" : "text-destructive-foreground"}>
                  WSL: {env.useWsl ? "active" : "NOT FOUND"}
                </p>
              )}
            </>
          ) : (
            <p className="text-destructive-foreground">env unavailable</p>
          )}
        </div>
      </aside>

      {/* Main */}
      <main className="min-w-0 flex-1">
        <div className="mx-auto max-w-5xl px-4 py-6 md:px-8">
          {/* Header */}
          <header className="mb-6 flex flex-wrap items-center justify-between gap-3">
            <div>
              <h2 className="text-xl font-semibold tracking-tight">Smoothie control panel</h2>
              <p className="text-sm text-muted-foreground">
                One-click dev scripts for the hypervisor, router and services. Every command is
                previewed before it runs.
              </p>
            </div>
            <div className="flex items-center gap-2">
              {env?.isWindows && (
                <Badge variant={env.useWsl ? "secondary" : "destructive"} className="gap-1.5">
                  {env.useWsl ? "WSL mode" : "Windows · WSL missing"}
                </Badge>
              )}
              {env?.isWindows && env.dockerEngine && (
                <Badge variant={env.dockerEngine === "windows" ? "success" : "secondary"} className="gap-1.5" title={`docker steps run ${env.dockerEngine === "windows" ? "natively via Docker Desktop" : "inside WSL"} (setting: ${env.dockerEngineSetting ?? "auto"})`}>
                  {env.dockerEngine === "windows" ? "Docker: Desktop" : "Docker: WSL"}
                </Badge>
              )}
              {env?.isWindows && env.buildMode === "windows-cross" && (
                <Badge variant="info" className="gap-1.5" title={`cargo builds cross-compile natively for ${env.crossTarget ?? "Linux"} and run in WSL`}>
                  Build: Win cross
                </Badge>
              )}
              <Button
                size="icon-sm"
                variant="ghost"
                onClick={toggle}
                title={dark ? "Switch to light mode" : "Switch to dark mode"}
                aria-label="Toggle color theme"
              >
                {dark ? <Sun className="h-4 w-4" /> : <Moon className="h-4 w-4" />}
              </Button>
              <Button
                size="sm"
                variant="outline"
                onClick={() => setPortalConfirm("restart")}
                disabled={portalBusy !== null}
                title="Restart the admin portal api (respawns node server/index.js on port 4102)"
              >
                <RotateCcw className="h-3.5 w-3.5" /> Restart portal
              </Button>
              <Button
                size="sm"
                variant="destructive-outline"
                onClick={() => setPortalConfirm("stop")}
                disabled={portalBusy !== null}
                title="Stop the admin portal api (needs a manual restart)"
              >
                <Power className="h-3.5 w-3.5" /> Stop portal
              </Button>
            </div>
          </header>

          {backendDown && (
            <Alert variant="error" className="mb-6">
              <ServerOff className="h-4 w-4" />
              <AlertTitle>Admin API server is not reachable</AlertTitle>
              <AlertDescription className="mt-1 flex flex-col gap-3">
                <span>
                  Vite is running but nothing answers <code className="font-mono">/api/*</code> on
                  port 4102 (ECONNREFUSED). Start the backend, then retry:
                  <code className="mx-1 rounded bg-muted px-1 py-0.5 font-mono text-xs">
                    cd admin && npm run dev
                  </code>
                  (starts web + api together). Or run
                  <code className="mx-1 rounded bg-muted px-1 py-0.5 font-mono text-xs">
                    node server/index.js
                  </code>
                  in a second terminal.
                </span>
                <span>
                  <Button size="sm" variant="outline" onClick={() => { loadInitial(); refreshJobs(); }}>
                    <RefreshCw className="h-3.5 w-3.5" /> Retry connection
                  </Button>
                </span>
              </AlertDescription>
            </Alert>
          )}

          {portalNotice && (
            <Alert
              variant={
                portalNotice.kind === "error" || portalNotice.kind === "stopped" ? "error" : "warning"
              }
              className="mb-6"
            >
              {portalNotice.kind === "restarting" ? (
                <RefreshCw className="h-4 w-4 animate-spin" />
              ) : (
                <ServerOff className="h-4 w-4" />
              )}
              <AlertTitle>
                {portalNotice.kind === "restarting"
                  ? "Restarting admin portal…"
                  : portalNotice.kind === "restarted"
                    ? "Admin portal restarted"
                    : portalNotice.kind === "stopped"
                      ? "Admin portal stopped"
                      : "Portal action failed"}
              </AlertTitle>
              <AlertDescription className="mt-1 flex flex-col gap-3">
                <span>{portalNotice.text}</span>
                {(portalNotice.kind === "restarted" || portalNotice.kind === "error") && (
                  <span>
                    <Button
                      size="sm"
                      variant="outline"
                      onClick={() => {
                        setPortalNotice(null);
                        loadInitial();
                        refreshJobs();
                      }}
                    >
                      <RefreshCw className="h-3.5 w-3.5" /> Refresh
                    </Button>
                  </span>
                )}
              </AlertDescription>
            </Alert>
          )}

          {jobsError && !backendDown && (
            <Alert variant="warning" className="mb-6">
              <TriangleAlert className="h-4 w-4" />
              <AlertTitle>Jobs feed unavailable</AlertTitle>
              <AlertDescription>{jobsError}</AlertDescription>
            </Alert>
          )}

          <Tabs value={activeTab} onValueChange={setActiveTab} className="gap-6">
            <TabsList className="w-fit">
              <TabsTrigger value="dashboard">Actions</TabsTrigger>
              <TabsTrigger value="config">Config</TabsTrigger>
              <TabsTrigger value="environment">Environment</TabsTrigger>
              <TabsTrigger value="jobs">
                Jobs
                {runningJobs.length > 0 && (
                  <Badge className="ml-1.5 h-4 px-1 text-[10px]" variant="secondary">
                    {runningJobs.length}
                  </Badge>
                )}
              </TabsTrigger>
            </TabsList>

            {/* ── Dashboard: action cards ── */}
            <TabsContent value="dashboard" className="flex flex-col gap-10 outline-none">
              {actionsLoading && (
                <div className="flex items-center gap-2 text-sm text-muted-foreground">
                  <Spinner className="h-4 w-4" /> Loading actions…
                </div>
              )}
              {!actionsLoading && actionsError && (
                <Alert variant="error">
                  <TriangleAlert className="h-4 w-4" />
                  <AlertTitle>Couldn’t load actions</AlertTitle>
                  <AlertDescription className="mt-1 flex flex-col gap-3">
                    <span className="break-words">{actionsError}</span>
                    <span>
                      <Button size="sm" variant="outline" onClick={loadInitial}>
                        <RefreshCw className="h-3.5 w-3.5" /> Retry
                      </Button>
                    </span>
                  </AlertDescription>
                </Alert>
              )}
              {!actionsLoading && !actionsError && actions.length === 0 && (
                <p className="text-sm text-muted-foreground">
                  No actions returned by the server. Check
                  <code className="mx-1 font-mono text-xs">server/actions.js</code>.
                </p>
              )}
              {GROUPS.map((group) => {
                const groupActions = actions.filter((a) => a.group === group.id);
                if (groupActions.length === 0) return null;
                return (
                  <section key={group.id} id={`group-${group.id}`} className="scroll-mt-6">
                    <h3 className="mb-3 text-xs font-semibold uppercase tracking-wider text-muted-foreground">
                      {group.label}
                    </h3>
                    <div className="grid gap-3 sm:grid-cols-2">
                      {groupActions.map((a) => (
                        <ActionCard
                          key={a.id}
                          action={a}
                          env={env}
                          runningJob={runningJobFor(a.id)}
                          onStarted={(jobId) => {
                            setActiveJobId(jobId);
                            refreshJobs();
                          }}
                          onStopped={refreshJobs}
                          onViewLogs={viewJobLogs}
                        />
                      ))}
                    </div>
                  </section>
                );
              })}
            </TabsContent>

            {/* ── Config generator ── */}
            <TabsContent value="config" className="outline-none">
              <ConfigGenerator />
            </TabsContent>

            {/* ── Environment ── */}
            <TabsContent value="environment" className="outline-none">
              <EnvironmentPanel
                env={env}
                loading={envLoading}
                error={envError}
                onRetry={refreshEnv}
                onJobStarted={(jobId) => {
                  setActiveJobId(jobId);
                  refreshJobs();
                }}
              />
            </TabsContent>

            {/* ── Jobs / terminal ── */}
            <TabsContent value="jobs" className="outline-none">
              <div className="grid gap-4 lg:grid-cols-[260px_1fr]">
                <ScrollArea className="h-[560px] rounded-lg border">
                  <div className="flex flex-col gap-1 p-2">
                    {jobs.length === 0 && !jobsError && (
                      <p className="p-2 text-xs text-muted-foreground">No jobs yet.</p>
                    )}
                    {jobsError && jobs.length === 0 && (
                      <div className="flex flex-col gap-2 p-2">
                        <p className="text-xs text-destructive-foreground">{jobsError}</p>
                        <Button size="sm" variant="outline" onClick={refreshJobs}>
                          <RefreshCw className="h-3.5 w-3.5" /> Retry
                        </Button>
                      </div>
                    )}
                    {jobs.map((j) => (
                      <div
                        key={j.id}
                        className={`group flex items-center gap-1 rounded-md px-1 py-0.5 hover:bg-accent ${
                          activeJobId === j.id ? "bg-accent" : ""
                        }`}
                      >
                        <button
                          onClick={() => setActiveJobId(j.id)}
                          className="flex min-w-0 flex-1 items-center gap-2 rounded px-1 py-1 text-left text-xs"
                        >
                          <JobDot status={j.status} />
                          <span className="min-w-0 flex-1 truncate">{j.title}</span>
                          <span className="shrink-0 text-[10px] text-muted-foreground">
                            {new Date(j.startedAt).toLocaleTimeString()}
                          </span>
                        </button>
                        {j.status === "running" && (
                          <button
                            onClick={() => stopJob(j.id)}
                            disabled={stoppingId === j.id}
                            title={`Stop ${j.title}`}
                            aria-label={`Stop ${j.title}`}
                            className="shrink-0 rounded p-1 text-muted-foreground opacity-60 hover:bg-destructive/10 hover:text-destructive-foreground hover:opacity-100 disabled:opacity-40"
                          >
                            <Square className="h-3 w-3" />
                          </button>
                        )}
                        <button
                          onClick={() => restartJob(j.id)}
                          disabled={restartingId === j.id}
                          title={j.status === "running" ? `Restart ${j.title} (stop + start again)` : `Run ${j.title} again`}
                          aria-label={`Restart ${j.title}`}
                          className="shrink-0 rounded p-1 text-muted-foreground opacity-60 hover:bg-accent hover:text-foreground hover:opacity-100 disabled:opacity-40"
                        >
                          <RotateCcw className={`h-3 w-3 ${restartingId === j.id ? "animate-spin" : ""}`} />
                        </button>
                      </div>
                    ))}
                  </div>
                </ScrollArea>
                <JobTerminal
                  jobId={activeJobId}
                  jobs={jobs}
                  onKill={refreshJobs}
                  onRestarted={(newJobId) => {
                    setActiveJobId(newJobId);
                    refreshJobs();
                  }}
                />
              </div>
            </TabsContent>
          </Tabs>

          <Dialog open={portalConfirm !== null} onOpenChange={(open) => !open && setPortalConfirm(null)}>
            <DialogContent className="sm:max-w-md">
              <DialogHeader>
                <DialogTitle>
                  {portalConfirm === "restart" ? "Restart admin portal?" : "Stop admin portal?"}
                </DialogTitle>
                <DialogDescription>
                  {portalConfirm === "restart"
                    ? "The admin api (node server/index.js on port 4102) will respawn. Running jobs are stopped first. The page reconnects automatically once it is back."
                    : "The admin api (node server/index.js on port 4102) will exit and this panel will go offline. Running jobs are stopped first. You will need to restart it manually: cd admin && npm run dev."}
                </DialogDescription>
              </DialogHeader>
              <DialogFooter>
                <Button variant="outline" onClick={() => setPortalConfirm(null)} disabled={portalBusy !== null}>
                  Cancel
                </Button>
                <Button
                  variant={portalConfirm === "stop" ? "destructive" : "default"}
                  onClick={confirmPortalAction}
                  disabled={portalBusy !== null}
                >
                  {portalConfirm === "restart" ? (
                    <>
                      <RotateCcw className="h-4 w-4" /> Restart
                    </>
                  ) : (
                    <>
                      <Power className="h-4 w-4" /> Stop
                    </>
                  )}
                </Button>
              </DialogFooter>
            </DialogContent>
          </Dialog>
        </div>
      </main>
    </div>
  );
}

function JobDot({ status }: { status: JobInfo["status"] }) {
  const cls =
    status === "running"
      ? "bg-blue-500 animate-pulse"
      : status === "success"
        ? "bg-emerald-500"
        : status === "failed"
          ? "bg-red-500"
          : "bg-neutral-400";
  return <span className={`h-2 w-2 shrink-0 rounded-full ${cls}`} />;
}
