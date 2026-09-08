/**
 * Action definitions for the Smoothie admin panel.
 *
 * Every action is a list of command steps executed in order.
 * Commands are written in POSIX style and are transparently adapted:
 *   - On native Linux / macOS: run as-is.
 *   - On Windows: if WSL is available, commands are translated
 *     (paths -> /mnt/..., wrapped in `wsl -e bash -lc`) because Docker,
 *     cargo builds and POSIX tooling live inside the WSL distro.
 *
 * Params can be referenced with {paramName} and are substituted at run
 * time (after validation) by the runner.
 *
 * Step hints honored by buildPlan (see runner.js):
 *   forceNativeWindows — run this step via cmd.exe on Windows (Docker
 *     Desktop, native cargo). Ignored on other platforms.
 *   forceWsl — always route this step through WSL on Windows.
 *   cross — this `cargo build` targets Linux (x86_64-unknown-linux-musl)
 *     via zig and runs natively on Windows; one-click fast path.
 */

const DOCKER = "docker";

import { CROSS_TARGET } from "./runner.js";

/**
 * One-click fast path for Windows: cross-compile `crate` for Linux with the
 * NATIVE Windows cargo (fast NTFS, warm local cache) instead of building
 * inside WSL. The `cross: true` marker makes buildPlan emit the
 * platform-appropriate recipe (cmd.exe + zig linker on Windows, bash
 * elsewhere); on Linux/macOS the same steps run normally. Needs `zig` on
 * PATH — first run also installs cargo-zigbuild (one-time, ~1 min).
 */
const crossBuildSteps = (crate) => [
  {
    cmd: "cargo build --release",
    cwd: crate,
    cross: true,
    note: `Fast native cross-compile for Linux (${CROSS_TARGET}) via zig — much faster than building inside WSL. Run the result in WSL.`,
  },
];

/**
 * DragonflyDB (redis-compatible) — the Justfile uses different flags per OS.
 * Windows needs -p because --network=host doesn't work through WSL port forwarding.
 */
const startDbSteps = [
  {
    cmd: `${DOCKER} run -d --name smoothie-dragonfly -p 6379:6379 --ulimit memlock=-1 docker.dragonflydb.io/dragonflydb/dragonfly`,
    note: "Start DragonflyDB (redis-compatible) detached on port 6379. Uses -p port mapping so it works identically on WSL and Linux.",
  },
  {
    cmd: `${DOCKER} ps --filter name=smoothie-dragonfly`,
    note: "Verify the container is up.",
  },
];

/** MinIO S3 — same command as hypervisor/Justfile, detached for convenience. */
const startS3Steps = [
  {
    cmd: `${DOCKER} run -d --name smoothie-minio -p 9000:9000 -p 9001:9001 -e MINIO_ROOT_USER={accessKey} -e MINIO_ROOT_PASSWORD={secretKey} minio/minio server /data --console-address ":9001"`,
    note: "Start MinIO detached: API on :9000, web console on :9001 (login with the credentials below).",
  },
  {
    cmd: `${DOCKER} ps --filter name=smoothie-minio`,
    note: "Verify the container is up.",
  },
];

export const actions = [
  // ─── environment ───────────────────────────────────────────────
  {
    id: "env-check",
    title: "Check environment",
    description:
      "Verifies that every tool the Smoothie stack needs is installed and reachable: Rust/Cargo, Docker (or Podman), Just, and a C compiler toolchain.",
    group: "environment",
    steps: [
      {
        cmd: 'echo "== Rust =="; cargo --version; rustc --version; echo "== Docker =="; docker --version 2>/dev/null || podman --version 2>/dev/null || echo "MISSING: docker/podman"; echo "== Just =="; just --version 2>/dev/null || echo "MISSING: just (optional)"; echo "== cc =="; cc --version 2>/dev/null || gcc --version 2>/dev/null || echo "MISSING: cc/gcc"',
        note: "Prints versions of rust, docker/podman, just and the C toolchain.",
      },
    ],
  },
  {
    id: "install-deps",
    title: "Install dependencies",
    description:
      "Fetches Cargo crates for hypervisor, router and example_app, and installs the admin panel + ui npm dependencies. Safe to re-run at any time.",
    group: "environment",
    steps: [
      {
        cmd: "cargo fetch",
        cwd: "hypervisor",
        note: "Downloads all Cargo dependencies for the hypervisor workspace.",
      },
      {
        cmd: "cargo fetch",
        cwd: "router",
        note: "Downloads all Cargo dependencies for the router.",
      },
      {
        cmd: "(command -v bun >/dev/null 2>&1 && bun install) || npm install",
        cwd: "ui",
        note: "Installs SvelteKit ui dependencies (prefers bun when available).",
      },
      {
        cmd: "(command -v bun >/dev/null 2>&1 && bun install) || npm install",
        cwd: "admin",
        note: "Installs admin panel dependencies.",
      },
    ],
  },

  // ─── build ─────────────────────────────────────────────────────
  {
    id: "build-hypervisor",
    title: "Build hypervisor",
    description: "Compiles smoothie/hypervisor in release mode. Output lands in hypervisor/target/release/.",
    group: "build",
    steps: [{ cmd: "cargo build --release", cwd: "hypervisor", note: "Release build of the hypervisor." }],
  },
  {
    id: "build-router",
    title: "Build router",
    description: "Compiles smoothie/router in release mode. Output lands in router/target/release/.",
    group: "build",
    steps: [{ cmd: "cargo build --release", cwd: "router", note: "Release build of the router." }],
  },
  {
    id: "build-all",
    title: "Build everything",
    description: "Release-builds hypervisor and router back to back. Use Rebuild-* for a clean build.",
    group: "build",
    steps: [
      { cmd: "cargo build --release", cwd: "hypervisor", note: "Release build of the hypervisor." },
      { cmd: "cargo build --release", cwd: "router", note: "Release build of the router." },
    ],
  },
  {
    id: "rebuild-hypervisor",
    title: "Rebuild hypervisor (clean)",
    description: "Wipes hypervisor/target and recompiles from scratch. Slow but fixes stale-artifact weirdness.",
    group: "build",
    dangerous: false,
    steps: [
      { cmd: "cargo clean", cwd: "hypervisor", note: "Deletes target/ (all compiled artifacts)." },
      { cmd: "cargo build --release", cwd: "hypervisor", note: "Full recompile." },
    ],
  },
  {
    id: "rebuild-router",
    title: "Rebuild router (clean)",
    description: "Wipes router/target and recompiles from scratch.",
    group: "build",
    steps: [
      { cmd: "cargo clean", cwd: "router", note: "Deletes target/ (all compiled artifacts)." },
      { cmd: "cargo build --release", cwd: "router", note: "Full recompile." },
    ],
  },
  {
    id: "build-hypervisor-win",
    title: "Build hypervisor (Windows fast)",
    description: `Cross-compiles the hypervisor for Linux (${CROSS_TARGET}) with the native Windows cargo — much faster than building inside WSL. Pair with “Start hypervisor” while build mode is “Windows cross” to run the result in WSL without rebuilding.`,
    group: "build",
    steps: crossBuildSteps("hypervisor"),
  },
  {
    id: "build-router-win",
    title: "Build router (Windows fast)",
    description: `Cross-compiles the router for Linux (${CROSS_TARGET}) with the native Windows cargo — much faster than building inside WSL. Pair with “Start router” while build mode is “Windows cross” to run the result in WSL without rebuilding.`,
    group: "build",
    steps: crossBuildSteps("router"),
  },
  {
    id: "build-all-win",
    title: "Build everything (Windows fast)",
    description: `Cross-compiles hypervisor and router back to back with the native Windows cargo (${CROSS_TARGET}). Same outputs as the per-crate fast actions.`,
    group: "build",
    steps: [...crossBuildSteps("hypervisor"), ...crossBuildSteps("router")],
  },

  // ─── services ──────────────────────────────────────────────────
  {
    id: "start-db",
    title: "Start DragonflyDB",
    description:
      "Starts the redis-compatible database in Docker on port 6379. Matches the `just db` recipe, adapted so the same command works through WSL and native Linux.",
    group: "services",
    longRunning: false,
    steps: startDbSteps,
    params: [],
    docsUrl: "https://github.com/dragonflydb/dragonfly",
  },
  {
    id: "stop-db",
    title: "Stop DragonflyDB",
    description: "Stops and removes the smoothie-dragonfly container.",
    group: "services",
    steps: [
      { cmd: `${DOCKER} rm -f smoothie-dragonfly`, note: "Force-removes the container (data is not persisted)." },
    ],
  },
  {
    id: "start-s3",
    title: "Start MinIO (S3)",
    description:
      "Starts MinIO in Docker: S3 API on :9000 and web console on :9001. Same image and defaults as the hypervisor/Justfile s3 recipe (minioadmin/minioadmin).",
    group: "services",
    steps: startS3Steps,
    params: [
      {
        key: "accessKey",
        label: "MinIO root user",
        type: "string",
        default: "minioadmin",
        description: "Becomes MINIO_ROOT_USER and the s3.access_key in config.json.",
      },
      {
        key: "secretKey",
        label: "MinIO root password",
        type: "string",
        default: "minioadmin",
        description: "Becomes MINIO_ROOT_PASSWORD and the s3.secret_key in config.json.",
      },
    ],
    docsUrl: "https://min.io",
  },
  {
    id: "stop-s3",
    title: "Stop MinIO",
    description: "Stops and removes the smoothie-minio container.",
    group: "services",
    steps: [{ cmd: `${DOCKER} rm -f smoothie-minio`, note: "Force-removes the MinIO container." }],
  },
  {
    id: "prefill-s3",
    title: "Prefill S3 with example app",
    description:
      "Builds the example_app, packages it as a Smoothie package (.tar with a `main` entrypoint), creates the bucket in the running MinIO and uploads the package. Requires MinIO to be running (Start MinIO action) and Docker available.",
    group: "services",
    steps: [
      {
        cmd: `${DOCKER} run --rm --network=host -e MC_HOST_local=http://{accessKey}:{secretKey}@127.0.0.1:9000 minio/mc mb --ignore-existing local/{bucket}`,
        note: "Creates the bucket (ignored if it already exists). Runs inside a throwaway minio/mc container that talks to MinIO over the host network.",
      },
      {
        cmd: `${DOCKER} run --rm --network=host -e MC_HOST_local=http://{accessKey}:{secretKey}@127.0.0.1:9000 -v {tmpDir}:/pkg minio/mc cp /pkg/{tarName} local/{bucket}/{tarName}`,
        note: "Uploads the built package to the bucket via a throwaway minio/mc container with {tmpDir} mounted at /pkg.",
      },
    ],
    params: [
      { key: "accessKey", label: "S3 access key", type: "string", default: "minioadmin" },
      { key: "secretKey", label: "S3 secret key", type: "string", default: "minioadmin" },
      { key: "bucket", label: "Bucket name", type: "string", default: "packages", description: "Same as s3.bucket in config.json." },
    ],
  },
  {
    id: "docker-managed",
    title: "List hypervisor containers",
    description:
      "Runs `docker ps -a` filtered on the io.smoothie.hypervisor label — shows every container the hypervisor currently owns (same as `just managed`).",
    group: "services",
    steps: [
      {
        cmd: `${DOCKER} ps -a --filter label=io.smoothie.hypervisor`,
        note: "Lists containers labeled io.smoothie.hypervisor=1.",
      },
    ],
  },

  // ─── run ───────────────────────────────────────────────────────
  {
    id: "start-hypervisor",
    title: "Start hypervisor",
    description:
      "Runs the hypervisor with cargo run (release). Refuses to start without a reachable engine socket — use the config generator first if hypervisor/config.json doesn't exist yet. Keeps running until you stop it.",
    group: "run",
    longRunning: true,
    steps: [
      {
        cmd: "test -f config.json && echo 'config.json found' || (echo 'MISSING config.json — generate it in the Config tab first' && exit 1)",
        cwd: "hypervisor",
        note: "Sanity check: hypervisor refuses to boot without config.json.",
      },
      { cmd: "cargo run --release", cwd: "hypervisor", note: "Starts the hypervisor (stays running; stop it from the Jobs panel)." },
    ],
  },
  {
    id: "start-router",
    title: "Start router",
    description: "Runs the router with cargo run (release). Keeps running until you stop it.",
    group: "run",
    longRunning: true,
    steps: [{ cmd: "cargo run --release", cwd: "router", note: "Starts the router (stays running; stop it from the Jobs panel)." }],
  },
  {
    id: "start-ui",
    title: "Start ui (dev server)",
    description: "Starts the SvelteKit ui dev server with npm (or bun when available). Keeps running until you stop it.",
    group: "run",
    longRunning: true,
    steps: [
      {
        cmd: "(command -v bun >/dev/null 2>&1 && bun run dev) || npm run dev",
        cwd: "ui",
        note: "Vite dev server for the SvelteKit ui.",
      },
    ],
  },

  // ─── package ───────────────────────────────────────────────────
  {
    id: "package-example",
    title: "Package example app",
    description:
      "Builds example_app in release mode and packs the binary into a Smoothie package tar (binary renamed to `main`, per the package format in hypervisor/arch.md). The tar path is printed at the end and reused by the Prefill S3 action.",
    group: "package",
    steps: [
      { cmd: "cargo build --release", cwd: "example_app", note: "Compile the example HTTP app." },
      {
        cmd: `BIN=$(ls target/${CROSS_TARGET}/release/example_app target/release/example_app 2>/dev/null | head -1); test -n "$BIN" || (echo "no built binary found (looked for cross + native target dirs)" && exit 1); mkdir -p /tmp/smoothie-pkg && cp "$BIN" /tmp/smoothie-pkg/main && tar -cf /tmp/smoothie-pkg/example_app.tar -C /tmp/smoothie-pkg main && rm /tmp/smoothie-pkg/main && echo "Package: /tmp/smoothie-pkg/example_app.tar"`,
        cwd: "example_app",
        note: "Renames the binary to `main` and tars it — matches the .tar package format (main = entrypoint). Picks the Windows cross-built binary when present, else the native one.",
      },
    ],
  },
];

export const actionGroups = {
  environment: "Environment",
  build: "Build",
  services: "Services",
  run: "Run",
  package: "Package",
  config: "Config",
};

export function getAction(id) {
  return actions.find((a) => a.id === id);
}
