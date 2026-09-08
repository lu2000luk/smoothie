import { useState } from "react";
import type { Action, EnvInfo, JobInfo, PreviewStep, RunRequest, RuntimeOverride, BuildMode, DockerEngineSetting } from "@/lib/types";
import { api } from "@/lib/api";
import { BuildModePicker, DockerEnginePicker } from "@/components/RuntimePicker";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { TerminalSquare, Info, Loader2, Zap, Square, Eye, RotateCcw } from "lucide-react";

export function ActionCard({
  action,
  env,
  runningJob,
  onStarted,
  onStopped,
  onViewLogs,
}: {
  action: Action;
  env?: EnvInfo | null;
  /** Live running job for this action (if any) — enables the Stop control. */
  runningJob?: JobInfo | null;
  onStarted?: (jobId: string) => void;
  onStopped?: () => void;
  onViewLogs?: (jobId: string) => void;
}) {
  const [previewOpen, setPreviewOpen] = useState(false);
  const [preview, setPreview] = useState<PreviewStep[] | null>(null);
  const [previewing, setPreviewing] = useState(false);
  const [running, setRunning] = useState(false);
  const [stopping, setStopping] = useState(false);
  const [restarting, setRestarting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  // Per-run Windows execution overrides. "global" follows the stored prefs.
  const [dockerSel, setDockerSel] = useState<DockerEngineSetting | "global">("global");
  const [buildSel, setBuildSel] = useState<BuildMode | "global">("global");
  const [params, setParams] = useState<Record<string, string>>(() =>
    Object.fromEntries(
      (action.params ?? []).map((p) => [p.key, p.default != null ? String(p.default) : ""])
    )
  );

  const toOverride = (
    d: DockerEngineSetting | "global" = dockerSel,
    b: BuildMode | "global" = buildSel
  ): RuntimeOverride => ({
    ...(d !== "global" ? { dockerEngine: d } : {}),
    ...(b !== "global" ? { buildMode: b } : {}),
  });

  const loadPreview = async (override?: RuntimeOverride) => {
    setPreviewing(true);
    setError(null);
    try {
      const res = await api.preview(action.id, params, override ?? toOverride());
      setPreview(res.steps);
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setPreviewing(false);
    }
  };

  const openPreview = () => {
    setPreviewOpen(true);
    setPreview(null);
    setDockerSel("global");
    setBuildSel("global");
    loadPreview({});
  };

  const run = async () => {
    setRunning(true);
    setError(null);
    try {
      const req: RunRequest = { actionId: action.id, params };
      const override = toOverride();
      if (Object.keys(override).length > 0) req.runtime = override;
      const { jobId } = await api.run(req);
      setPreviewOpen(false);
      onStarted?.(jobId);
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setRunning(false);
    }
  };

  const stop = async () => {
    setStopping(true);
    setError(null);
    try {
      // Prefer kill-by-job (exact), fall back to stop-by-action when the
      // job list is stale and the server reports "not running" for the id.
      if (runningJob) {
        try {
          await api.kill(runningJob.id);
        } catch (e) {
          await api.stop(action.id);
        }
      } else {
        await api.stop(action.id);
      }
      onStopped?.();
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setStopping(false);
    }
  };

  const restart = async () => {
    setRestarting(true);
    setError(null);
    try {
      // Stop the live job first (exact kill-by-job, fallback to stop-by-action),
      // then start a fresh job with the same params + runtime overrides.
      if (runningJob) {
        try {
          await api.kill(runningJob.id);
        } catch {
          await api.stop(action.id);
        }
        await new Promise((r) => setTimeout(r, 600));
      }
      const req: RunRequest = { actionId: action.id, params };
      const override = toOverride();
      if (Object.keys(override).length > 0) req.runtime = override;
      const { jobId } = await api.run(req);
      onStarted?.(jobId);
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setRestarting(false);
    }
  };

  const showRuntimePickers = !!env?.isWindows;
  const isRunning = !!runningJob;

  return (
    <>
      <Card className="flex h-full flex-col transition-colors hover:border-ring/40">
        <CardHeader className="pb-2">
          <div className="flex items-start justify-between gap-2">
            <CardTitle className="text-sm font-semibold">{action.title}</CardTitle>
            <div className="flex shrink-0 gap-1">
              {action.longRunning && (
                <Badge variant="info" className="text-[10px]">
                  long-running
                </Badge>
              )}
              {action.dangerous && (
                <Badge variant="warning" className="text-[10px]">
                  destructive
                </Badge>
              )}
            </div>
          </div>
          <CardDescription className="line-clamp-3 text-xs leading-relaxed">
            {action.description}
          </CardDescription>
        </CardHeader>
        <CardContent className="mt-auto flex flex-col gap-2">
          {action.params && action.params.length > 0 && (
            <div className="grid gap-1.5">
              {action.params.map((p) => (
                <div key={p.key} className="grid grid-cols-[110px_1fr] items-center gap-2">
                  <Label className="truncate text-[11px] text-muted-foreground" title={p.label}>
                    {p.label}
                  </Label>
                  <Input
                    className="h-7 text-xs"
                    value={params[p.key] ?? ""}
                    placeholder={p.placeholder}
                    onChange={(e) => setParams((prev) => ({ ...prev, [p.key]: e.target.value }))}
                  />
                </div>
              ))}
            </div>
          )}
          <div className="flex flex-col gap-2">
            {isRunning ? (
              <>
                <div className="flex items-center gap-2">
                  <Button
                    size="sm"
                    variant="destructive-outline"
                    onClick={stop}
                    disabled={stopping || restarting}
                    className="flex-1"
                    title={`Stop ${action.title}`}
                  >
                    {stopping ? (
                      <>
                        <Loader2 className="h-3.5 w-3.5 animate-spin" /> Stopping…
                      </>
                    ) : (
                      <>
                        <Square className="h-3.5 w-3.5" /> Stop
                      </>
                    )}
                  </Button>
                  <Button
                    size="sm"
                    variant="outline"
                    onClick={restart}
                    disabled={restarting || stopping}
                    className="flex-1"
                    title={`Restart ${action.title} (stop + start again)`}
                  >
                    {restarting ? (
                      <>
                        <Loader2 className="h-3.5 w-3.5 animate-spin" /> Restarting…
                      </>
                    ) : (
                      <>
                        <RotateCcw className="h-3.5 w-3.5" /> Restart
                      </>
                    )}
                  </Button>
                  {action.docsUrl && (
                    <Button size="icon-sm" variant="ghost" render={<a href={action.docsUrl} target="_blank" rel="noreferrer" />}>
                      <Info className="h-3.5 w-3.5" />
                    </Button>
                  )}
                </div>
                <Button
                  size="sm"
                  variant="ghost"
                  onClick={() => runningJob && onViewLogs?.(runningJob.id)}
                  disabled={!runningJob}
                  className="w-full"
                  title="View live logs in the Jobs tab"
                >
                  <Eye className="h-3.5 w-3.5" /> View logs
                </Button>
              </>
            ) : (
              <div className="flex items-center gap-2">
                <Button size="sm" onClick={openPreview} disabled={previewing} className="flex-1">
                  {previewing ? (
                    <>
                      <Loader2 className="h-3.5 w-3.5 animate-spin" /> Building preview…
                    </>
                  ) : (
                    <>
                      <TerminalSquare className="h-3.5 w-3.5" /> Preview &amp; run
                    </>
                  )}
                </Button>
                {action.docsUrl && (
                  <Button size="icon-sm" variant="ghost" render={<a href={action.docsUrl} target="_blank" rel="noreferrer" />}>
                    <Info className="h-3.5 w-3.5" />
                  </Button>
                )}
              </div>
            )}
          </div>
          {isRunning && runningJob && (
            <p className="flex items-center gap-1.5 text-[11px] text-muted-foreground">
              <span className="h-1.5 w-1.5 animate-pulse rounded-full bg-emerald-500" />
              Running since {new Date(runningJob.startedAt).toLocaleTimeString()}
            </p>
          )}
          {error && <p className="text-[11px] text-destructive-foreground">{error}</p>}
        </CardContent>
      </Card>

      {/* Preview dialog */}
      <Dialog open={previewOpen} onOpenChange={setPreviewOpen}>
        <DialogContent className="sm:max-w-2xl">
          <DialogHeader>
            <DialogTitle className="flex items-center gap-2">
              <Zap className="h-4 w-4 text-warning-foreground" />
              {action.title}
            </DialogTitle>
            <DialogDescription>
              These exact commands will run on this machine{". "}
              <span className="text-muted-foreground">
                (paths are relative to the project root)
              </span>
            </DialogDescription>
          </DialogHeader>

          {showRuntimePickers && (
            <div className="flex flex-col gap-2 rounded-lg border px-3 py-2">
              <p className="text-[11px] font-medium text-muted-foreground">
                Run this time via{" "}
                <span className="font-normal">
                  (global: docker {env?.dockerEngineSetting ?? "auto"} · build {env?.buildMode ?? "wsl"})
                </span>
              </p>
              <div className="flex flex-wrap items-center gap-x-4 gap-y-2">
                <div className="flex items-center gap-2">
                  <span className="text-[11px] text-muted-foreground">Docker</span>
                  <DockerEnginePicker
                    allowGlobal
                    value={dockerSel}
                    dockerNativeAvailable={!!env?.dockerNative?.available}
                    onChange={(v) => {
                      setDockerSel(v);
                      setPreview(null);
                      loadPreview(toOverride(v, buildSel));
                    }}
                  />
                </div>
                <div className="flex items-center gap-2">
                  <span className="text-[11px] text-muted-foreground">Build</span>
                  <BuildModePicker
                    allowGlobal
                    value={buildSel}
                    cargoNativeAvailable={!!env?.cargoNative?.available}
                    zigNativeAvailable={!!env?.zigNative?.available}
                    crossTarget={env?.crossTarget}
                    onChange={(v) => {
                      setBuildSel(v);
                      setPreview(null);
                      loadPreview(toOverride(dockerSel, v));
                    }}
                  />
                </div>
              </div>
            </div>
          )}

          <div className="max-h-[50vh] overflow-y-auto rounded-lg border bg-code p-3">
            {preview === null ? (
              <div className="flex items-center gap-2 py-6 text-sm text-muted-foreground">
                <Loader2 className="h-4 w-4 animate-spin" /> Building command preview…
              </div>
            ) : (
              <div className="flex flex-col gap-3">
                {preview.map((step, i) => (
                  <div key={i} className="flex flex-col gap-1">
                    {step.note && (
                      <p className="text-[11px] text-muted-foreground">
                        <span className="font-semibold text-muted-foreground/80">#{i + 1}</span>{" "}
                        {step.note}
                      </p>
                    )}
                    <pre className="overflow-x-auto rounded-md bg-code-highlight px-3 py-2 font-mono text-[12px] leading-relaxed text-code-foreground">
                      <code>
                        <span className="select-none text-muted-foreground">$ </span>
                        {step.cmd}
                      </code>
                    </pre>
                    <div className="flex items-center gap-1.5 text-[10px] text-muted-foreground">
                      <span className="font-mono">{step.cwd}</span>
                      {step.wsl && <Badge variant="secondary" className="h-4 px-1 text-[9px]">WSL</Badge>}
                      {step.nativeWindows && <Badge variant="success" className="h-4 px-1 text-[9px]">NATIVE</Badge>}
                      {step.crossTarget && <Badge variant="info" className="h-4 px-1 text-[9px]">CROSS {step.crossTarget}</Badge>}
                    </div>
                  </div>
                ))}
              </div>
            )}
          </div>

          <DialogFooter>
            <Button variant="outline" onClick={() => setPreviewOpen(false)} disabled={running}>
              Cancel
            </Button>
            <Button onClick={run} disabled={running || preview === null}>
              {running ? (
                <>
                  <Loader2 className="h-4 w-4 animate-spin" /> Starting…
                </>
              ) : (
                "Run"
              )}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </>
  );
}
