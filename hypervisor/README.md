# smoothie/hypervisor

Runs app packages in containers through a Docker- or Podman-compatible
engine socket. The socket path is the required `engine.socket` config
option (see `config.json.example`); the hypervisor refuses to start
without a reachable engine.

Every container it creates is labeled `io.smoothie.hypervisor=1`;
leftovers from a killed run are removed automatically on the next
startup, and a graceful shutdown removes everything the run created.

Container termination endpoints:
- `POST /container/shutdown/{id}` sends `SIGTERM`, waits up to 30 seconds,
  then force-removes the container so it is killed and deleted if it did
  not stop normally.
- `POST /container/kill/{id}` immediately force-removes the container.

## App packages

App packages are uncompressed tar archives. Before an archive is uploaded to the
container engine, the hypervisor requires:

- at most 256 MiB of archive data, 4,096 entries, and 256 MiB total declared
  extracted file data;
- POSIX-relative entry paths, with no absolute paths, `..`, NUL bytes, or
  backslash separators; safe aliases are normalized for duplicate checks;
- only regular files and directories (no symbolic links, hard links, devices,
  FIFOs, sparse metadata, or other special entry types), with no duplicate
  normalized paths;
- an executable regular file named `main` at the archive root.

The default argv remains `["./main"]`; callers may provide a different argv to
`POST /container/inject/{package_id}`. The requested application port remains
the `port` query parameter on `POST /container/run/{id}` (default `8080`).

## Instant boxes

`POST /box` creates a fresh container from the configured Alpine image, starts
one interactive `/bin/sh`, and returns its WebSocket URL:

```json
{
  "id": "smoothie-...",
  "ws_endpoint": "ws://localhost:3100/box/smoothie-.../ws",
  "expires_in_seconds": 14400
}
```

Connect any number of clients to the returned endpoint. Text and binary frames
are written verbatim to the same shell stdin, so include a newline to execute a
command (for example, `pwd\n`). Shell output is broadcast as binary frames to
all currently connected clients. Disconnecting a client does not stop the box
or create another shell.

The box container is force-removed when `/bin/sh` exits (including `exit` or
EOF) or after four hours, whichever happens first. Connected WebSockets are
then closed, and later connections receive `404 Not Found`.

Stop modes:
- Restart (stop and boot container normally)
- CRIU
- Freeze container

Run modes:
- Container (Docker/Podman socket)
- landlock

See `arch.md` for the architecture.
