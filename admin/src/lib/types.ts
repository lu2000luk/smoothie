export interface DistroInfo {
  id: string;
  family: string;
  pretty: string;
  version: string;
  supported: boolean;
}

export interface InstallRecipe {
  tool: string;
  supported: boolean;
  command?: string;
  note?: string;
  reason?: string;
  family?: string;
  distro?: string;
}

export interface EnvInfo {
  platform: "win32" | "linux" | "darwin";
  isWindows: boolean;
  wslAvailable: boolean;
  useWsl: boolean;
  wslDistro?: string;
  tools: Record<string, { available: boolean; version?: string }>;
  distro?: DistroInfo;
  installs?: Record<string, InstallRecipe>;
  projectRoot: string;
  osInfo: string;
  /** Native Windows docker.exe (Docker Desktop) — Windows only. */
  dockerNative?: { available: boolean; version?: string };
  /** Native Windows cargo.exe — Windows only. */
  cargoNative?: { available: boolean; version?: string };
  /** Native Windows zig.exe (linker for cross-compiles) — Windows only. */
  zigNative?: { available: boolean; version?: string };
  /** Stored docker preference: auto|wsl|windows. */
  dockerEngineSetting?: DockerEngineSetting;
  /** Effective docker engine after resolving auto. */
  dockerEngine?: DockerEngine;
  /** Stored build mode: wsl|windows-cross. */
  buildMode?: BuildMode;
  /** Linux target used for Windows cross-compiles. */
  crossTarget?: string;
}

/** Stored preference: where should `docker` steps run on Windows. */
export type DockerEngineSetting = "auto" | "wsl" | "windows";

/** Resolved engine: native Docker Desktop (windows) or inside WSL. */
export type DockerEngine = "wsl" | "windows";

/**
 * Where cargo steps compile on Windows: inside WSL, or natively as a
 * Linux cross-compile (fast) whose binary then runs in WSL.
 */
export type BuildMode = "wsl" | "windows-cross";

export interface RuntimeConfig {
  dockerEngine: DockerEngineSetting;
  buildMode: BuildMode;
  crossTarget?: string;
}

export interface RuntimeOverride {
  dockerEngine?: DockerEngineSetting;
  buildMode?: BuildMode;
}

export interface PreviewStep {
  cmd: string;
  cwd: string;
  note?: string;
  wsl: boolean;
  /** Step scope: docker|cargo-build|cargo-run|other. */
  engine?: string;
  /** True when this step runs natively via cmd.exe on Windows. */
  nativeWindows?: boolean;
  /** Linux target when this step is a cross-compile. */
  crossTarget?: string;
}

export interface CommandStep {
  /** Shell command to run */
  cmd: string;
  /** Working directory relative to project root */
  cwd?: string;
  /** Human-readable explanation of this step */
  note?: string;
}

export interface ActionParam {
  key: string;
  label: string;
  type: "string" | "number" | "boolean";
  default?: string | number | boolean;
  placeholder?: string;
  description?: string;
  optional?: boolean;
}

export interface Action {
  id: string;
  title: string;
  description: string;
  group: "environment" | "build" | "run" | "services" | "config" | "package";
  /** Long-running actions (server-like) keep running; short ones exit */
  longRunning?: boolean;
  dangerous?: boolean;
  steps: CommandStep[];
  params?: ActionParam[];
  docsUrl?: string;
}

export interface RunRequest {
  actionId: string;
  params?: Record<string, string | number | boolean>;
  /** Per-run execution overrides (Windows: docker engine / build mode). */
  runtime?: RuntimeOverride;
}

export interface JobInfo {
  id: string;
  actionId: string;
  title: string;
  status: "running" | "success" | "failed" | "killed";
  startedAt: number;
  endedAt?: number;
  exitCode?: number | null;
}

export interface LogLine {
  stream: "stdout" | "stderr" | "system";
  text: string;
  ts: number;
}

export interface SmoothieConfig {
  s3: {
    access_key: string;
    secret_key: string;
    bucket: string;
    region: string;
    endpoint: string;
    supports_range: boolean;
    force_path_style: boolean;
  };
  redis: string;
  port: number;
  host: string;
  engine: {
    socket: string;
    image: string;
  };
  [key: string]: unknown;
}
