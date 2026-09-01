# smoothie/hypervisor

Runs app packages in containers through a Docker- or Podman-compatible
engine socket. The socket path is the required `engine.socket` config
option (see `config.json.example`); the hypervisor refuses to start
without a reachable engine.

Every container it creates is labeled `io.smoothie.hypervisor=1`;
leftovers from a killed run are removed automatically on the next
startup, and a graceful shutdown removes everything the run created.

Stop modes:
- Restart (stop and boot container normally)
- CRIU
- Freeze container

Run modes:
- Container (Docker/Podman socket)
- landlock

See `arch.md` for the architecture.
