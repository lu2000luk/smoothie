import { useEffect, useState } from "react";
import type { SmoothieConfig, RouterConfig, ConfigTarget, RouterServerConfig } from "@/lib/types";
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
import { CheckCircle2, FileJson, Loader2, Plus, Server, Trash2 } from "lucide-react";

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

const DEFAULT_ROUTER_CONFIG: RouterConfig = {
  servers: [
    {
      id: "hypervisor-1",
      address: "127.0.0.1:3200",
      tunnel: false,
      tunnel_address: null,
      power: 100,
    },
  ],
  redis: "redis://localhost:6379",
  port: 3300,
  host: "0.0.0.0",
  s3: {
    access_key: "minioadmin",
    secret_key: "minioadmin",
    bucket: "packages",
    region: "us-east-1",
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

function withRouterDefaults(loaded: Partial<RouterConfig> | null | undefined): RouterConfig {
  if (!loaded || typeof loaded !== "object") return structuredClone(DEFAULT_ROUTER_CONFIG);
  const base = structuredClone(DEFAULT_ROUTER_CONFIG);
  const servers = Array.isArray(loaded.servers) && loaded.servers.length > 0
    ? loaded.servers.map((s) => ({
        id: String(s?.id ?? ""),
        address: String(s?.address ?? ""),
        tunnel: Boolean(s?.tunnel ?? false),
        tunnel_address: s?.tunnel_address ? String(s.tunnel_address) : null,
        power: Number(s?.power ?? 100) || 0,
      }))
    : base.servers;
  return {
    ...base,
    ...loaded,
    servers,
    s3: { ...base.s3, ...((loaded as RouterConfig).s3 ?? {}) },
  };
}

/** Empty tunnel_address must serialize as null — router treats Some(_) as "use tunnel". */
function normalizeRouterForSave(config: RouterConfig): RouterConfig {
  return {
    ...config,
    servers: config.servers.map((s) => ({
      ...s,
      tunnel_address: s.tunnel && s.tunnel_address ? s.tunnel_address : null,
    })),
  };
}

export function ConfigGenerator() {
  const [target, setTarget] = useState<ConfigTarget>("hypervisor");

  return (
    <div className="flex flex-col gap-4">
      <div className="flex w-fit items-center gap-1 rounded-lg border bg-muted/40 p-1">
        {(["hypervisor", "router"] as ConfigTarget[]).map((t) => (
          <Button
            key={t}
            size="sm"
            variant={target === t ? "default" : "ghost"}
            onClick={() => setTarget(t)}
          >
            {t === "hypervisor" ? "Hypervisor" : "Router"}
          </Button>
        ))}
      </div>
      {target === "hypervisor" ? <HypervisorConfigForm /> : <RouterConfigForm />}
    </div>
  );
}

function HypervisorConfigForm() {
  const [config, setConfig] = useState<SmoothieConfig>(() => structuredClone(DEFAULT_CONFIG));
  const [exists, setExists] = useState(false);
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [confirmOpen, setConfirmOpen] = useState(false);
  const [saved, setSaved] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    api
      .config("hypervisor")
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
      const res = await api.saveConfig(full, "hypervisor");
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

function RouterConfigForm() {
  const [config, setConfig] = useState<RouterConfig>(() => structuredClone(DEFAULT_ROUTER_CONFIG));
  const [exists, setExists] = useState(false);
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [confirmOpen, setConfirmOpen] = useState(false);
  const [saved, setSaved] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    api
      .config("router")
      .then(({ exists, config: loaded }) => {
        setExists(exists);
        if (loaded) setConfig(withRouterDefaults(loaded as unknown as Partial<RouterConfig>));
      })
      .catch(() => {})
      .finally(() => setLoading(false));
  }, []);

  const set = (path: (string | number)[], value: unknown) => {
    setConfig((prev) => {
      const next = structuredClone(prev);
      let obj: Record<string, unknown> = next;
      for (let i = 0; i < path.length - 1; i++) {
        const key = path[i];
        if (typeof key === "number") {
          obj = (obj as unknown as unknown[])[key] as unknown as Record<string, unknown>;
        } else {
          if (typeof obj[key] !== "object" || obj[key] === null) obj[key] = {};
          obj = obj[key] as Record<string, unknown>;
        }
      }
      const last = path[path.length - 1];
      (obj as Record<string | number, unknown>)[last] = value;
      return next;
    });
  };

  const updateServer = (index: number, patch: Partial<RouterServerConfig>) => {
    setConfig((prev) => {
      const next = structuredClone(prev);
      next.servers[index] = { ...next.servers[index], ...patch };
      return next;
    });
  };

  const addServer = () => {
    setConfig((prev) => ({
      ...structuredClone(prev),
      servers: [
        ...prev.servers,
        { id: `hypervisor-${prev.servers.length + 1}`, address: "127.0.0.1:3200", tunnel: false, tunnel_address: null, power: 100 },
      ],
    }));
  };

  const removeServer = (index: number) => {
    setConfig((prev) => ({
      ...structuredClone(prev),
      servers: prev.servers.filter((_, i) => i !== index),
    }));
  };

  const preview = normalizeRouterForSave(withRouterDefaults(config));

  const save = async () => {
    setSaving(true);
    setError(null);
    try {
      const full = normalizeRouterForSave(withRouterDefaults(config));
      const res = await api.saveConfig(full, "router");
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
        <Loader2 className="h-4 w-4 animate-spin" /> Loading router config…
      </div>
    );
  }

  return (
    <Card>
      <CardHeader>
        <div className="flex items-center justify-between">
          <div>
            <CardTitle className="flex items-center gap-2 text-base">
              <FileJson className="h-4 w-4" /> router/config.json
            </CardTitle>
            <CardDescription className="mt-1">
              Servers are weighted by power (higher = more traffic). Tunnel servers need a
              tunnel_address — otherwise it is stored as null.
            </CardDescription>
          </div>
          <Badge variant={exists ? "success" : "warning"}>{exists ? "on disk" : "not created"}</Badge>
        </div>
      </CardHeader>
      <CardContent className="flex flex-col gap-5">
        {/* Core section */}
        <section>
          <h4 className="mb-2 text-xs font-semibold uppercase tracking-wider text-muted-foreground">
            Router core
          </h4>
          <div className="grid gap-3 sm:grid-cols-2">
            <Field label="redis url">
              <Input value={config.redis ?? ""} onChange={(e) => set(["redis"], e.target.value)} />
            </Field>
            <Field label="host">
              <Input
                value={config.host ?? ""}
                placeholder="0.0.0.0"
                onChange={(e) => set(["host"], e.target.value)}
              />
            </Field>
            <Field label="port">
              <Input
                type="number"
                value={config.port ?? DEFAULT_ROUTER_CONFIG.port}
                onChange={(e) => set(["port"], Number(e.target.value) || 0)}
              />
            </Field>
          </div>
        </section>

        <Separator />

        {/* S3 section (router/src/main.rs S3Config: 4 keys, no endpoint) */}
        <section>
          <h4 className="mb-2 text-xs font-semibold uppercase tracking-wider text-muted-foreground">
            S3 (package metadata)
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
          </div>
        </section>

        <Separator />

        {/* Servers section */}
        <section>
          <div className="mb-2 flex items-center justify-between">
            <h4 className="text-xs font-semibold uppercase tracking-wider text-muted-foreground">
              Servers ({config.servers.length})
            </h4>
            <Button size="sm" variant="outline" onClick={addServer}>
              <Plus className="h-3.5 w-3.5" /> Add server
            </Button>
          </div>
          {config.servers.length === 0 && (
            <p className="rounded-md border border-dashed px-3 py-4 text-center text-xs text-muted-foreground">
              No servers — /route will answer “No server available”. Add at least one hypervisor.
            </p>
          )}
          <div className="flex flex-col gap-3">
            {config.servers.map((s, i) => (
              <div key={i} className="rounded-lg border p-3">
                <div className="mb-2 flex items-center justify-between">
                  <span className="flex items-center gap-1.5 text-xs font-medium">
                    <Server className="h-3.5 w-3.5 text-muted-foreground" />
                    {s.id || `server-${i + 1}`}
                  </span>
                  <Button size="icon-sm" variant="ghost" onClick={() => removeServer(i)} title="Remove server" aria-label="Remove server">
                    <Trash2 className="h-3.5 w-3.5" />
                  </Button>
                </div>
                <div className="grid gap-3 sm:grid-cols-2">
                  <Field label="id">
                    <Input value={s.id} onChange={(e) => updateServer(i, { id: e.target.value })} placeholder="hypervisor-1" />
                  </Field>
                  <Field label="address (host:port)">
                    <Input value={s.address} onChange={(e) => updateServer(i, { address: e.target.value })} placeholder="127.0.0.1:3200" />
                  </Field>
                  <Field label="power (weight)">
                    <Input
                      type="number"
                      min={0}
                      value={s.power}
                      onChange={(e) => updateServer(i, { power: Number(e.target.value) || 0 })}
                    />
                  </Field>
                  <ToggleField label="tunnel" checked={s.tunnel} onChange={(v) => updateServer(i, { tunnel: v })} />
                  <Field label="tunnel_address" className="sm:col-span-2">
                    <Input
                      value={s.tunnel_address ?? ""}
                      disabled={!s.tunnel}
                      placeholder={s.tunnel ? "tunnel host:port" : "only needed when tunnel is on"}
                      onChange={(e) => updateServer(i, { tunnel_address: e.target.value || null })}
                    />
                  </Field>
                </div>
              </div>
            ))}
          </div>
        </section>

        <div className="flex items-center justify-between gap-3">
          <Button variant="outline" size="sm" onClick={() => setConfig(structuredClone(DEFAULT_ROUTER_CONFIG))}>
            Reset to defaults
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

      <Dialog open={confirmOpen} onOpenChange={setConfirmOpen}>
        <DialogContent className="sm:max-w-2xl">
          <DialogHeader>
            <DialogTitle>Write router/config.json?</DialogTitle>
            <DialogDescription>
              This overwrites router/config.json with exactly the JSON below.
            </DialogDescription>
          </DialogHeader>
          <pre className="max-h-[45vh] overflow-auto rounded-lg border bg-code p-3 font-mono text-[12px] leading-relaxed text-code-foreground">
            <code>{JSON.stringify(preview, null, 2)}</code>
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
