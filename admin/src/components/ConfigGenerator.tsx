import { useEffect, useState } from "react";
import type { SmoothieConfig } from "@/lib/types";
import { api } from "@/lib/api";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Switch } from "@/components/ui/switch";
import { Badge } from "@/components/ui/badge";
import { Separator } from "@/components/ui/separator";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { CheckCircle2, FileJson, Loader2 } from "lucide-react";

const DEFAULT_CONFIG: SmoothieConfig = {
  s3: {
    access_key: "minioadmin",
    secret_key: "minioadmin",
    bucket: "packages",
    region: "us-east-1",
    endpoint: "http://localhost:9000",
    supports_range: true,
    force_path_style: true,
  },
  redis: "redis://localhost:6379",
  port: 3100,
  host: "0.0.0.0",
  engine: {
    socket: "/var/run/docker.sock",
    image: "docker.io/library/alpine:3.24",
  },
};

function withDefaults(loaded: Partial<SmoothieConfig> | null | undefined): SmoothieConfig {
  if (!loaded || typeof loaded !== "object") return structuredClone(DEFAULT_CONFIG);
  return {
    ...structuredClone(DEFAULT_CONFIG),
    ...loaded,
    s3: { ...DEFAULT_CONFIG.s3, ...((loaded as SmoothieConfig).s3 ?? {}) },
    engine: { ...DEFAULT_CONFIG.engine, ...((loaded as SmoothieConfig).engine ?? {}) },
  };
}

export function ConfigGenerator() {
  const [config, setConfig] = useState<SmoothieConfig>(() => structuredClone(DEFAULT_CONFIG));
  const [exists, setExists] = useState(false);
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [confirmOpen, setConfirmOpen] = useState(false);
  const [saved, setSaved] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    api
      .config()
      .then(({ exists, config }) => {
        setExists(exists);
        if (config) setConfig(withDefaults(config));
      })
      .catch(() => {})
      .finally(() => setLoading(false));
  }, []);

  const set = (path: (string | number)[], value: unknown) => {
    setConfig((prev) => {
      const next = structuredClone(prev);
      let obj: Record<string, unknown> = next;
      for (let i = 0; i < path.length - 1; i++) {
        const key = path[i] as string;
        if (typeof obj[key] !== "object" || obj[key] === null) obj[key] = {};
        obj = obj[key] as Record<string, unknown>;
      }
      obj[path[path.length - 1] as string] = value;
      return next;
    });
  };

  const save = async () => {
    setSaving(true);
    setError(null);
    try {
      const full = withDefaults(config);
      const res = await api.saveConfig(full);
      setConfig(full);
      setSaved(res.path);
      setExists(true);
      setConfirmOpen(false);
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setSaving(false);
    }
  };

  if (loading) {
    return (
      <div className="flex items-center gap-2 text-sm text-muted-foreground">
        <Loader2 className="h-4 w-4 animate-spin" /> Loading config…
      </div>
    );
  }

  return (
    <Card>
      <CardHeader>
        <div className="flex items-center justify-between">
          <div>
            <CardTitle className="flex items-center gap-2 text-base">
              <FileJson className="h-4 w-4" /> hypervisor/config.json
            </CardTitle>
            <CardDescription className="mt-1">
              The hypervisor refuses to start without this file. Preset matches
              config.json.example — MinIO on :9000, DragonflyDB on :6379.
            </CardDescription>
          </div>
          <Badge variant={exists ? "success" : "warning"}>{exists ? "on disk" : "not created"}</Badge>
        </div>
      </CardHeader>
      <CardContent className="flex flex-col gap-5">
        {/* S3 section */}
        <section>
          <h4 className="mb-2 text-xs font-semibold uppercase tracking-wider text-muted-foreground">
            S3 (package storage)
          </h4>
          <div className="grid gap-3 sm:grid-cols-2">
            <Field label="access_key">
              <Input value={config.s3?.access_key ?? ""} onChange={(e) => set(["s3", "access_key"], e.target.value)} />
            </Field>
            <Field label="secret_key">
              <Input value={config.s3?.secret_key ?? ""} onChange={(e) => set(["s3", "secret_key"], e.target.value)} />
            </Field>
            <Field label="bucket">
              <Input value={config.s3?.bucket ?? ""} onChange={(e) => set(["s3", "bucket"], e.target.value)} />
            </Field>
            <Field label="region">
              <Input value={config.s3?.region ?? ""} onChange={(e) => set(["s3", "region"], e.target.value)} />
            </Field>
            <Field label="endpoint" className="sm:col-span-2">
              <Input value={config.s3?.endpoint ?? ""} onChange={(e) => set(["s3", "endpoint"], e.target.value)} />
            </Field>
            <ToggleField
              label="supports_range"
              checked={config.s3?.supports_range ?? DEFAULT_CONFIG.s3.supports_range}
              onChange={(v) => set(["s3", "supports_range"], v)}
            />
            <ToggleField
              label="force_path_style"
              checked={config.s3?.force_path_style ?? DEFAULT_CONFIG.s3.force_path_style}
              onChange={(v) => set(["s3", "force_path_style"], v)}
            />
          </div>
        </section>

        <Separator />

        {/* Core section */}
        <section>
          <h4 className="mb-2 text-xs font-semibold uppercase tracking-wider text-muted-foreground">
            Hypervisor core
          </h4>
          <div className="grid gap-3 sm:grid-cols-2">
            <Field label="redis url">
              <Input value={config.redis ?? ""} onChange={(e) => set(["redis"], e.target.value)} />
            </Field>
            <Field label="host">
              <Input value={config.host ?? ""} onChange={(e) => set(["host"], e.target.value)} />
            </Field>
            <Field label="port">
              <Input
                type="number"
                value={config.port ?? DEFAULT_CONFIG.port}
                onChange={(e) => set(["port"], Number(e.target.value) || 0)}
              />
            </Field>
          </div>
        </section>

        <Separator />

        {/* Engine section */}
        <section>
          <h4 className="mb-2 text-xs font-semibold uppercase tracking-wider text-muted-foreground">
            Engine (Docker/Podman)
          </h4>
          <div className="grid gap-3 sm:grid-cols-2">
            <Field label="socket path">
              <Input value={config.engine?.socket ?? ""} onChange={(e) => set(["engine", "socket"], e.target.value)} />
            </Field>
            <Field label="base image">
              <Input value={config.engine?.image ?? ""} onChange={(e) => set(["engine", "image"], e.target.value)} />
            </Field>
          </div>
        </section>

        <div className="flex items-center justify-between gap-3">
          <Button variant="outline" size="sm" onClick={() => setConfig(structuredClone(DEFAULT_CONFIG))}>
            Reset to example defaults
          </Button>
          <Button size="sm" onClick={() => setConfirmOpen(true)}>
            Write config.json
          </Button>
        </div>
        {saved && (
          <p className="flex items-center gap-1.5 text-xs text-success-foreground">
            <CheckCircle2 className="h-3.5 w-3.5" /> Written to {saved}
          </p>
        )}
        {error && <p className="text-xs text-destructive-foreground">{error}</p>}
      </CardContent>

      {/* Write confirmation with JSON preview */}
      <Dialog open={confirmOpen} onOpenChange={setConfirmOpen}>
        <DialogContent className="sm:max-w-2xl">
          <DialogHeader>
            <DialogTitle>Write config.json?</DialogTitle>
            <DialogDescription>
              This overwrites hypervisor/config.json with exactly the JSON below.
            </DialogDescription>
          </DialogHeader>
          <pre className="max-h-[45vh] overflow-auto rounded-lg border bg-code p-3 font-mono text-[12px] leading-relaxed text-code-foreground">
            <code>{JSON.stringify(config, null, 2)}</code>
          </pre>
          <DialogFooter>
            <Button variant="outline" onClick={() => setConfirmOpen(false)} disabled={saving}>
              Cancel
            </Button>
            <Button onClick={save} disabled={saving}>
              {saving ? (
                <>
                  <Loader2 className="h-4 w-4 animate-spin" /> Writing…
                </>
              ) : (
                "Write file"
              )}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </Card>
  );
}

function Field({
  label,
  children,
  className = "",
}: {
  label: string;
  children: React.ReactNode;
  className?: string;
}) {
  return (
    <div className={`flex flex-col gap-1 ${className}`}>
      <Label className="font-mono text-[11px] text-muted-foreground">{label}</Label>
      {children}
    </div>
  );
}

function ToggleField({
  label,
  checked,
  onChange,
}: {
  label: string;
  checked: boolean;
  onChange: (v: boolean) => void;
}) {
  return (
    <div className="flex items-center justify-between rounded-md border px-3 py-2">
      <Label className="font-mono text-[11px] text-muted-foreground">{label}</Label>
      <Switch checked={checked} onCheckedChange={onChange} />
    </div>
  );
}
