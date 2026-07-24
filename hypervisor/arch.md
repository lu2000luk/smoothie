# how does this work

(everything expandable for future modes/optimizations)
2 running modes: process/container

container: run with crun (use libcrun directly)
boot modes:
    - Restart (simple cold boot)
    - Freeze (docker pause the container and keep it as long as we can, delete it whenever limits are reached)
    - Hybernate (snapshot ram and keep into disk, have a storage quota and delete with LRU policy)

process: run all processes inside the same container but each process with cgroups stuff (nix crate)

package format: (since we have multiple run modes the apps are packaged in an universal way) (stored in S3, store in LRU disk cache)
    .tar file:
        - main (entrypoint)
        - ... (any other files, will be injected together with the entrypoint)

hypervisor only stuff:
    always run 2 parent containers (all containers with crun!), one for x86, the other for arm (auto detect the system's aarch and choose the one to emulate)
    run everything else inside:
        - 1 unlimited container for the process run mode
        - X idle containers ready to get injected
        - X running containers
        - X hybernated containers

configs:
    idle_containers: num (try to always reach this number, never go higher, dont stress the system to reach this, do it slowly) [default: 5]
    max_hybernated_containers: num [default: 20]
    snapshots_storage_quota: num (bytes) [default: 209715200 (200mb)]
    package_cache_quota: num (bytes) [default: 524288000 (500mb)]
    resource_limits: (per container/process)
        ram: num (bytes) [default: 26214400 (25mb)]
        cpu_p: num (period) [default: 50000]
        cpu_q: num (quota) [default: 12500]
