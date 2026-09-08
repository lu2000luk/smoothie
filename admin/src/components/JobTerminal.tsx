import { useEffect, useRef, useState } from "react";
import type { JobInfo, LogLine } from "@/lib/types";
import { api } from "@/lib/api";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Square, Trash2 } from "lucide-react";

const COLORS: Record<LogLine["stream"], string> = {
  stdout: "text-code-foreground",
  stderr: "text-red-400",
  system: "text-muted-foreground italic",
};

export function JobTerminal({
  jobId,
  jobs,
  onKill,
}: {
  jobId: string | null;
  jobs: JobInfo[];
  onKill?: () => void;
}) {
  const [logs, setLogs] = useState<LogLine[]>([]);
  const [status, setStatus] = useState<string>("idle");
  const [autoScroll, setAutoScroll] = useState(true);
  const viewportRef = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    if (!jobId) {
      setLogs([]);
      setStatus("idle");
      return;
    }
    setLogs([]);
    setStatus("connecting");
    const es = api.stream(jobId, {
      onLog: (line) => {
        setLogs((prev) => [...prev.slice(-4999), line as LogLine]);
      },
      onStatus: (s) => {
        setStatus(s);
        if (s !== "running") onKill?.();
      },
      onError: () => setStatus("disconnected"),
    });
    return () => es.close();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [jobId]);

  useEffect(() => {
    if (autoScroll && viewportRef.current) {
      viewportRef.current.scrollTop = viewportRef.current.scrollHeight;
    }
  }, [logs, autoScroll]);

  const job = jobs.find((j) => j.id === jobId);
  const isRunning = status === "running";

  return (
    <Card className="flex min-h-[560px] flex-col">
      <CardHeader className="flex-row items-center justify-between space-y-0 border-b pb-3">
        <CardTitle className="flex items-center gap-2 text-sm">
          {job ? job.title : "Terminal"}
          {job && (
            <Badge
              variant={
                job.status === "running"
                  ? "info"
                  : job.status === "success"
                    ? "success"
                    : job.status === "failed"
                      ? "destructive"
                      : "secondary"
              }
            >
              {status}
            </Badge>
          )}
        </CardTitle>
        <div className="flex items-center gap-1">
          <Button
            size="sm"
            variant="ghost"
            onClick={() => setAutoScroll((v) => !v)}
            className="text-[11px]"
          >
            auto-scroll: {autoScroll ? "on" : "off"}
          </Button>
          {isRunning && job && (
            <Button size="sm" variant="destructive-outline" onClick={async () => {
              await api.kill(job.id);
              onKill?.();
            }}>
              <Square className="h-3 w-3" /> Stop
            </Button>
          )}
          {job && !isRunning && (
            <Button size="icon-sm" variant="ghost" onClick={() => setLogs([])}>
              <Trash2 className="h-3.5 w-3.5" />
            </Button>
          )}
        </div>
      </CardHeader>
      <CardContent className="min-h-0 flex-1 p-0">
        <ScrollArea
          viewportRef={viewportRef}
          className="h-[500px] bg-code"
          onViewportScroll={(e) => {
            const el = e.currentTarget;
            setAutoScroll(el.scrollHeight - el.scrollTop - el.clientHeight < 40);
          }}
        >
          <div className="p-4 font-mono text-[12px] leading-relaxed">
            {!jobId && (
              <p className="text-muted-foreground">
                Select a job on the left — or start an action from the Actions tab.
              </p>
            )}
            {logs.map((l, i) => (
              <div key={i} className={`whitespace-pre-wrap break-all ${COLORS[l.stream] ?? ""}`}>
                {l.stream === "system" ? `· ${l.text}` : l.text}
              </div>
            ))}
            {isRunning && (
              <div className="mt-1 inline-block h-3.5 w-2 animate-pulse bg-muted-foreground align-middle" />
            )}
          </div>
        </ScrollArea>
      </CardContent>
    </Card>
  );
}
