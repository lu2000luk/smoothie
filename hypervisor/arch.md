# how does this work

(everything expandable for future modes/optimizations)
2 running modes: process/container

container: run with crun (use libcrun directly)
boot modes: - Restart (simple cold boot) - Freeze (docker pause the container and keep it as long as we can, delete it whenever limits are reached) - Hybernate (snapshot ram and keep into disk, have a storage quota and delete with LRU policy)

process: run all processes inside the same container but each process with cgroups stuff (nix crate)

package format: (since we have multiple run modes the apps are packaged in an universal way) (stored in S3, store in LRU disk cache)
.tar file: - main (entrypoint) - ... (any other files, will be injected together with the entrypoint)

container API:
- `select_tarball(path)` validates the selected `.tar` file.
- `inject(tarball, argv)` expands it into an idle container.
- `run(injected, container_port)` starts it and publishes a host port chosen by `port.rs`.
- `kill(running)` stops the container, removes its crun state, and closes its published port.

Published ports use a retained host TCP listener and relay to `127.0.0.1:container_port`. This gives Docker-style `HOST_PORT:CONTAINER_PORT` behaviour: the host port is selected and reserved by Smoothie, while an application can listen on any distinct container port. The current crun networking implementation uses the host network namespace so the relay can reach the application; a future isolated namespace backend must preserve this API and route the relay to that namespace instead.

hypervisor only stuff:
always run 1/2 parent containers (all containers with crun!), one for x86, the other for arm (auto detect the system's aarch and choose the one to emulate)
run everything else inside: - 1 unlimited container for the process run mode - X idle containers ready to get injected - X running containers - X hybernated containers

configs:
idle_containers: num (try to always reach this number, never go higher, dont stress the system to reach this, do it slowly) [default: 5]
max_hybernated_containers: num [default: 20]
snapshots_storage_quota: num (bytes) [default: 209715200 (200mb)]
package_cache_quota: num (bytes) [default: 524288000 (500mb)]
run_multiple_aarch: bool (if true run 2 parent containers, one for each aarch [x84/arm] otherwise only run one for the current aarch)
resource_limits: (per container/process)
ram: num (bytes) [default: 26214400 (25mb)]
cpu_p: num (period) [default: 50000]
cpu_q: num (quota) [default: 12500]
