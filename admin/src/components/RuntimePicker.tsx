import type { BuildMode, DockerEngineSetting } from "@/lib/types";
import { cn } from "@/lib/utils";

function Seg<T extends string>({
  options,
  value,
  onChange,
  ariaLabel,
}: {
  options: { value: T; label: string; title?: string; disabled?: boolean }[];
  value: T;
  onChange: (v: T) => void;
  ariaLabel?: string;
}) {
  return (
    <div
      role="group"
      aria-label={ariaLabel}
      className="flex w-fit max-w-full flex-wrap items-center gap-0.5 rounded-lg bg-muted p-0.5"
    >
      {options.map((o) => (
        <button
          key={o.value}
          type="button"
          disabled={o.disabled}
          title={o.title}
          onClick={() => onChange(o.value)}
          className={cn(
            "rounded-md px-2.5 py-1 text-xs font-medium whitespace-nowrap text-muted-foreground transition-colors hover:text-foreground disabled:pointer-events-none disabled:opacity-40",
            value === o.value && "bg-background text-foreground shadow-sm"
          )}
        >
          {o.label}
        </button>
      ))}
    </div>
  );
}

export function DockerEnginePicker({
  value,
  onChange,
  dockerNativeAvailable,
  allowGlobal = false,
}: {
  value: DockerEngineSetting | "global";
  onChange: (v: DockerEngineSetting | "global") => void;
  dockerNativeAvailable: boolean;
  /** When true (per-action override), adds a "Global" option that follows the stored pref. */
  allowGlobal?: boolean;
}) {
  return (
    <Seg<DockerEngineSetting | "global">
      ariaLabel="Docker engine"
      value={value}
      onChange={onChange}
      options={[
        ...(allowGlobal
          ? [{ value: "global" as const, label: "Global", title: "Follow the stored workspace preference" }]
          : []),
        {
          value: "auto" as const,
          label: "Auto",
          title: dockerNativeAvailable
            ? "Docker Desktop detected — docker steps run natively via docker.exe"
            : "No native docker.exe — docker steps run inside WSL",
        },
        {
          value: "windows" as const,
          label: "Docker Desktop",
          disabled: !dockerNativeAvailable,
          title: dockerNativeAvailable
            ? "Run docker steps natively on Windows via docker.exe (Docker Desktop)"
            : "docker.exe not found — install Docker Desktop to enable this",
        },
        {
          value: "wsl" as const,
          label: "WSL",
          title: "Run docker steps inside the WSL distro",
        },
      ]}
    />
  );
}

export function BuildModePicker({
  value,
  onChange,
  cargoNativeAvailable,
  zigNativeAvailable = true,
  crossTarget,
  allowGlobal = false,
}: {
  value: BuildMode | "global";
  onChange: (v: BuildMode | "global") => void;
  cargoNativeAvailable: boolean;
  zigNativeAvailable?: boolean;
  crossTarget?: string;
  /** When true (per-action override), adds a "Global" option that follows the stored pref. */
  allowGlobal?: boolean;
}) {
  const crossEnabled = cargoNativeAvailable && zigNativeAvailable;
  const crossTitle = crossEnabled
    ? `Compile natively on Windows for Linux (${crossTarget ?? "musl target"}) — faster, then run the binary in WSL`
    : !cargoNativeAvailable
      ? "cargo.exe not found — install the Rust toolchain on Windows to enable this"
      : "zig.exe not found — install zig (https://ziglang.org) to enable cross-linking";
  return (
    <Seg<BuildMode | "global">
      ariaLabel="Build mode"
      value={value}
      onChange={onChange}
      options={[
        ...(allowGlobal
          ? [{ value: "global" as const, label: "Global", title: "Follow the stored workspace preference" }]
          : []),
        {
          value: "wsl" as const,
          label: "In WSL",
          title: "Compile with the WSL toolchain (cargo inside the distro)",
        },
        {
          value: "windows-cross" as const,
          label: "Windows cross (fast)",
          disabled: !crossEnabled,
          title: crossTitle,
        },
      ]}
    />
  );
}
