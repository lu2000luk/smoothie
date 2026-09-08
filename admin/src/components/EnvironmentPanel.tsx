import { useState } from "react";
import type { EnvInfo } from "@/lib/types";
import { api } from "@/lib/api";
import { BuildModePicker, DockerEnginePicker } from "@/components/RuntimePicker";
import type { BuildMode, DockerEngineSetting } from "@/lib/types";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Separator } from "@/components/ui/separator";
import { Spinner } from "@/components/ui/spinner";
import { Download, Info, Monitor, RefreshCw, TerminalSquare, TriangleAlert, Container, Hammer } from "lucide-react";

const TOOL_NOTES: Record<string, string> = {
  cargo: "Rust package manager — builds hypervisor, router and example_app",
  docker: "Container engine used by the hypervisor and the service actions",
  podman: "Docker-compatible alternative (checked when docker is missing)",
  node: "Node.js runtime for the ui and admin panel servers",
  npm: "Node package manager (bun is preferred when present)",
  bun: "Faster alternative to npm used by the ui",
  just: "Command runner used by the project Justfiles (optional)",
  rustc: "Rust compiler",
};

interface Props {
  env: EnvInfo | null;
  loading?: boolean;
  error?: string | null;
  onRetry?: () => void;
  onJobStarted?: (jobId: string) => void;
}

export function EnvironmentPanel({ env, loading, error, onRetry, onJobStarted }: Props) {
  const [installing, setInstalling] = useState<string | null>(null);
  const [installError, setInstallError] = useState<string | null>(null);
  const [installNotice, setInstallNotice] = useState<string | null>(null);
  const [runtimeSaving, setRuntimeSaving] = useState(false);
  const [runtimeError, setRuntimeError] = useState<string | null>(null);

  if (loading && !env) {
    return (
      <p className="flex items-center gap-2 text-sm text-muted-foreground">
        <Spinner className="h-4 w-4" /> Detecting environment…
      </p>
    );
  }

  if (!env) {
    return (
      <Alert variant="error">
        <TriangleAlert className="h-4 w-4" />
        <AlertTitle>Couldn’t detect environment</AlertTitle>
        <AlertDescription className="mt-1 flex flex-col gap-3">
          <span className="break-words">{error ?? "No environment data."}</span>
          {onRetry && (
            <span>
              <Button size="sm" variant="outline" onClick={onRetry}>
                <RefreshCw className="h-3.5 w-3.5" /> Retry
              </Button>
            </span>
          )}
        </AlertDescription>
      </Alert>
    );
  }

  const distroLabel = env.distro
    ? `${env.distro.pretty}${env.distro.family !== "unknown" ? ` (${env.distro.family})` : ""}`
    : null;

  const missingEntries = Object.entries(env.tools).filter(([, info]) => !info.available);
  const unsupportedMissing = missingEntries.filter(
    ([name]) => !env.installs?.[name]?.supported
  );

  const install = async (tool: string) => {
    setInstalling(tool);
    setInstallError(null);
    setInstallNotice(null);
    try {
      const { jobId } = await api.install(tool);
      setInstallNotice(
        `Install for “${tool}” started — watch it in the Jobs tab, then hit Re-check.`
      );
      onJobStarted?.(jobId);
    } catch (e) {
      setInstallError((e as Error).message);
    } finally {
      setInstalling(null);
    }
  };

  const saveRuntime = async (patch: { dockerEngine?: DockerEngineSetting; buildMode?: BuildMode }) => {
    setRuntimeSaving(true);
    setRuntimeError(null);
    try {
      await api.setRuntime(patch);
      onRetry?.();
    } catch (e) {
      setRuntimeError((e as Error).message);
    } finally {
      setRuntimeSaving(false);
    }
  };

  return (
    <div className="flex flex-col gap-4">
      {/* Platform / WSL card */}
      <Card>
        <CardHeader>
          <CardTitle className="flex items-center gap-2 text-base">
            <Monitor className="h-4 w-4" /> Host
          </CardTitle>
          <CardDescription>
            {env.osInfo} · project root <code className="font-mono text-xs">{env.projectRoot}</code>
          </CardDescription>
        </CardHeader>
        <CardContent className="flex flex-col gap-3">
          <div className="flex flex-wrap items-center gap-2 text-sm">
            <Badge variant={env.isWindows ? "warning" : "success"}>
              platform: {env.platform}
            </Badge>
            {env.isWindows && (
              <Badge variant={env.wslAvailable ? "info" : "destructive"}>
                WSL: {env.wslAvailable ? `available${env.wslDistro ? ` (${env.wslDistro})` : ""}` : "NOT FOUND"}
              </Badge>
            )}
            {env.isWindows && env.useWsl && (
              <Badge variant="secondary">commands route through WSL</Badge>
            )}
            {distroLabel && (
              <Badge variant={env.distro?.supported ? "secondary" : "warning"}>
                distro: {distroLabel}
              </Badge>
            )}
          </div>

          {env.isWindows && (
            <Alert variant={env.useWsl ? "info" : "error"}>
              <Info className="h-4 w-4" />
              <AlertTitle>
                {env.useWsl ? "WSL mode active" : "WSL required but not detected"}
              </AlertTitle>
              <AlertDescription>
                {env.useWsl ? (
                  <>
                    All shell commands (cargo, docker, tar, just) are translated to
                    <code className="mx-1 rounded bg-muted px-1 py-0.5 font-mono text-xs">wsl -e bash -lc "…"</code>
                    and Windows paths are mapped to /mnt/… automatically. Set the
                    <code className="mx-1 rounded bg-muted px-1 py-0.5 font-mono text-xs">SMOOTHIE_WSL_DISTRO</code>
                    env var on the server to pin a distro.
                  </>
                ) : (
                  <>
                    You are on Windows but WSL is not available. Most actions (cargo builds, docker,
                    tar packaging) require a Linux toolchain — install WSL with
                    <code className="mx-1 rounded bg-muted px-1 py-0.5 font-mono text-xs">wsl --install</code>
                    and restart the admin server.
                  </>
                )}
              </AlertDescription>
            </Alert>
          )}
        </CardContent>
      </Card>

      {/* Windows execution card */}
      {env.isWindows && (
        <Card>
          <CardHeader>
            <CardTitle className="flex items-center gap-2 text-base">
              <Container className="h-4 w-4" /> Windows execution
            </CardTitle>
            <CardDescription>
              Applies to previews and runs immediately. Per-action overrides are available in each
              action’s “Preview &amp; run” dialog.
            </CardDescription>
          </CardHeader>
          <CardContent className="flex flex-col gap-4">
            {runtimeError && (
              <Alert variant="error">
                <TriangleAlert className="h-4 w-4" />
                <AlertTitle>Couldn’t save preference</AlertTitle>
                <AlertDescription className="break-words">{runtimeError}</AlertDescription>
              </Alert>
            )}
            <div className="flex flex-col gap-2">
              <div className="flex flex-wrap items-center gap-2">
                <p className="text-xs font-medium">Docker engine</p>
                <Badge variant={env.dockerEngine === "windows" ? "success" : "secondary"}>
                  {env.dockerEngine === "windows" ? "via Docker Desktop" : "via WSL"}
                </Badge>
              </div>
              <DockerEnginePicker
                value={env.dockerEngineSetting ?? "auto"}
                dockerNativeAvailable={!!env.dockerNative?.available}
                onChange={(v) => {
                  if (v !== "global") saveRuntime({ dockerEngine: v });
                }}
              />
              <p className="text-[11px] leading-relaxed text-muted-foreground">
                {env.dockerNative?.available ? (
                  <>
                    Docker Desktop detected
                    {env.dockerNative.version ? (
                      <> (<span className="font-mono">{env.dockerNative.version}</span>)</>
                    ) : null}
                    {" "}— native mode runs <code className="font-mono text-[10px]">docker</code> steps
                    through <code className="font-mono text-[10px]">docker.exe</code> instead of inside
                    WSL. Same daemon, no WSL docker setup needed.
                  </>
                ) : (
                  <>
                    No native <code className="font-mono text-[10px]">docker.exe</code> found — install{" "}
                    <a
                      className="underline"
                      href="https://www.docker.com/products/docker-desktop/"
                      target="_blank"
                      rel="noreferrer"
                    >
                      Docker Desktop
                    </a>{" "}
                    to run docker steps directly on Windows. Until then everything docker goes through WSL.
                  </>
                )}
              </p>
            </div>

            <Separator />

            <div className="flex flex-col gap-2">
              <div className="flex flex-wrap items-center gap-2">
                <p className="flex items-center gap-1.5 text-xs font-medium">
                  <Hammer className="h-3.5 w-3.5" /> Build mode
                </p>
                {env.buildMode === "windows-cross" && (
                  <Badge variant="info">cross {env.crossTarget}</Badge>
                )}
              </div>
              <BuildModePicker
                value={env.buildMode ?? "wsl"}
                cargoNativeAvailable={!!env.cargoNative?.available}
                zigNativeAvailable={!!env.zigNative?.available}
                crossTarget={env.crossTarget}
                onChange={(v) => {
                  if (v !== "global") saveRuntime({ buildMode: v });
                }}
              />
              <p className="text-[11px] leading-relaxed text-muted-foreground">
                {env.cargoNative?.available && env.zigNative?.available ? (
                  <>
                    Native <code className="font-mono text-[10px]">cargo.exe</code> +{" "}
                    <code className="font-mono text-[10px]">zig.exe</code> detected
                    {env.cargoNative.version ? (
                      <> (<span className="font-mono">{env.cargoNative.version}</span>)</>
                    ) : null}
                    . “Windows cross” compiles on Windows for Linux
                    (<span className="font-mono text-[10px]">{env.crossTarget}</span>) — much faster file
                    I/O than WSL — and the Start actions then run that Linux binary inside WSL without
                    rebuilding. There are also one-click “(Windows fast)” variants in the Build group.
                  </>
                ) : (
                  <>
                    Fast cross-compiles need <code className="font-mono text-[10px]">cargo.exe</code> and{" "}
                    <code className="font-mono text-[10px]">zig.exe</code> on Windows
                    {!env.cargoNative?.available && !env.zigNative?.available
                      ? " (both missing — install Rust via rustup and zig from "
                      : !env.cargoNative?.available
                        ? " (cargo missing — install Rust via rustup; zig found; also see "
                        : " (zig missing — install it from "}
                    <a
                      className="underline"
                      href="https://ziglang.org/download/"
                      target="_blank"
                      rel="noreferrer"
                    >
                      ziglang.org
                    </a>
                    ). Until then builds run inside WSL.
                  </>
                )}
              </p>
            </div>
            {runtimeSaving && (
              <p className="flex items-center gap-2 text-[11px] text-muted-foreground">
                <Spinner className="h-3 w-3" /> Saving preference…
              </p>
            )}
          </CardContent>
        </Card>
      )}

      {/* Tools card */}
      <Card>
        <CardHeader>
          <div className="flex flex-wrap items-center justify-between gap-2">
            <CardTitle className="flex items-center gap-2 text-base">
              <TerminalSquare className="h-4 w-4" /> Toolchain
            </CardTitle>
            {onRetry && (
              <Button size="xs" variant="outline" onClick={onRetry} disabled={!!loading}>
                {loading ? <Spinner className="h-3 w-3" /> : <RefreshCw className="h-3 w-3" />}
                Re-check
              </Button>
            )}
          </div>
          <CardDescription>
            {env.useWsl
              ? `Probed inside the default WSL distro${distroLabel ? ` · ${distroLabel}` : ""}`
              : `Probed on the machine running the admin server${distroLabel ? ` · ${distroLabel}` : ""}`}
          </CardDescription>
        </CardHeader>
        <CardContent>
          {installError && (
            <Alert variant="error" className="mb-3">
              <TriangleAlert className="h-4 w-4" />
              <AlertTitle>Install failed to start</AlertTitle>
              <AlertDescription className="break-words">{installError}</AlertDescription>
            </Alert>
          )}
          {installNotice && (
            <Alert variant="info" className="mb-3">
              <Info className="h-4 w-4" />
              <AlertDescription>{installNotice}</AlertDescription>
            </Alert>
          )}
          <div className="grid gap-2 sm:grid-cols-2">
            {Object.entries(env.tools).map(([name, info]) => {
              const recipe = env.installs?.[name];
              const busy = installing === name;
              return (
                <div key={name} className="flex items-center justify-between gap-2 rounded-lg border px-3 py-2">
                  <div className="min-w-0">
                    <p className="font-mono text-xs font-semibold">{name}</p>
                    <p className="truncate text-[11px] text-muted-foreground" title={TOOL_NOTES[name]}>
                      {TOOL_NOTES[name]}
                    </p>
                  </div>
                  <div className="flex shrink-0 items-center gap-2">
                    {info.version && (
                      <span className="hidden font-mono text-[10px] text-muted-foreground sm:inline">
                        {info.version}
                      </span>
                    )}
                    {info.available ? (
                      <Badge variant="success">ok</Badge>
                    ) : (
                      <>
                        <Badge variant="secondary">missing</Badge>
                        {recipe?.supported ? (
                          <Button
                            size="xs"
                            variant="outline"
                            title={`Install ${name} now:\n${recipe.command}`}
                            onClick={() => install(name)}
                            disabled={busy}
                          >
                            {busy ? (
                              <Spinner className="h-3 w-3" />
                            ) : (
                              <Download className="h-3 w-3" />
                            )}
                            {busy ? "Installing…" : "Install"}
                          </Button>
                        ) : (
                          <Button
                            size="xs"
                            variant="outline"
                            disabled
                            title={recipe?.reason ?? `No auto-install for ${name} on this distro yet.`}
                          >
                            <Download className="h-3 w-3" />
                            Install
                          </Button>
                        )}
                      </>
                    )}
                  </div>
                </div>
              );
            })}
          </div>
          <Separator className="my-4" />
          {unsupportedMissing.length > 0 ? (
            <div className="flex flex-col gap-2">
              {unsupportedMissing.map(([name]) => (
                <p key={name} className="text-xs text-warning-foreground">
                  No auto-install for “{name}”{distroLabel ? ` on ${distroLabel}` : ""} yet —{" "}
                  {env.installs?.[name]?.reason ??
                    "install it manually, then hit Re-check."}
                </p>
              ))}
              <p className="text-xs text-muted-foreground">
                Missing tools are not fatal for every action — for example you can run the S3 service
                without cargo. Actions that need a missing tool will fail with a clear error in the
                terminal.
              </p>
            </div>
          ) : (
            <p className="text-xs text-muted-foreground">
              {missingEntries.length === 0
                ? "All tools detected. If something still fails, hit Re-check — installs only take effect for new shells."
                : "Click Install on a missing tool to run the distro-specific setup (hover the button to preview the exact command). After it finishes in the Jobs tab, hit Re-check. Missing tools are not fatal for every action — for example you can run the S3 service without cargo."}
            </p>
          )}
        </CardContent>
      </Card>
    </div>
  );
}
