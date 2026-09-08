# smoothie/admin

Development & ops panel for the Smoothie stack. A ViteJS + React web UI
(backed by a small Node server) that turns everyday project chores into
one-click actions:

- **Install dependencies** — cargo fetch + npm/bun install for every crate / app
- **Build / rebuild** — hypervisor, router, or both (clean rebuild supported)
- **Start services** — DragonflyDB and MinIO with the exact Justfile recipes
- **Prefill S3** — creates the bucket and uploads a packaged example app
- **Package example app** — builds + tars the example app (`main` entrypoint)
- **Start hypervisor / router / ui** — long-running jobs with live terminal
- **Config generator** — writes `hypervisor/config.json` with a validated JSON preview
- **Environment check** — probes cargo, docker, node, bun, just… and reports versions

Every action is **previewed before it runs**: you see the exact commands,
their working directories and step-by-step notes in a dialog, then confirm.

## Windows / WSL

On Windows the server detects the platform and WSL availability:

- Linux-toolchain commands (cargo, docker, tar, just, POSIX compounds) are
  transparently wrapped: `wsl -e bash -lc "…"` (WSL maps the project cwd to
  `/mnt/...` automatically)
- Pure Node-ecosystem commands (`npm`, `bun`, `node`…) still run natively
- Pin a distro with the `SMOOTHIE_WSL_DISTRO` env var if you have several
- The Environment tab shows a red alert if WSL is missing + install hint

## Run it

```bash
cd admin
npm install

# dev mode: API on :3111 + Vite dev server on :5174 (hot reload)
npm run dev:server &
npm run dev

# or production mode: build once, serve everything from :3111
npm start
```

Then open http://localhost:3111 (prod) or http://localhost:5174 (dev).

## Architecture

```
admin/
├── server/            # Node/Express backend
│   ├── index.js       # REST API + SSE streaming + static hosting
│   ├── actions.js     # All action definitions (commands, cwd, params)
│   └── runner.js      # WSL detection/adaptation, job runner, tool probes
├── src/
│   ├── components/ui/ # coss ui components (copied via registry)
│   ├── components/    # ActionCard, JobTerminal, ConfigGenerator, …
│   ├── lib/           # api client (SSE), types, utils
│   └── App.tsx
└── package.json
```

- **Preview first**: `POST /api/preview` returns the final command lines
  (params substituted, WSL wrapping applied) without executing anything
- **Jobs**: `POST /api/run` spawns the steps sequentially, `GET /api/stream/:job`
  replays + streams stdout/stderr over SSE, `POST /api/kill/:job` kills the
  whole process group
- **Param safety**: substitutions are validated against a strict charset so
  values can't break out of their placeholder
- **Long-running protection**: only one instance of a long-running action
  (hypervisor/router/ui) can run at a time

## Adding an action

Append an entry to `server/actions.js`:

```js
{
  id: "my-action",
  title: "My action",
  description: "What it does.",
  group: "build",           // environment | build | services | run | package
  steps: [
    { cmd: "echo hello {name}", cwd: "router", note: "greets you" },
  ],
  params: [{ key: "name", label: "Name", type: "string", default: "world" }],
}
```

The frontend picks it up automatically — card, preview dialog, params.
