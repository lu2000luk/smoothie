# smoothie/router

Routes approved domains to app containers started by a Smoothie hypervisor.

## Endpoints

- `GET /allow?domain={domain}` → `200` when the domain resolves to a deployed package, otherwise `403`.
- `GET /route?host={host}&scheme={scheme}&id={request_id}&ip={ip}` → `[{"dial":"host:port"}]`.

For an unbound host, `/route` resolves the deployment through Redis:

1. `domains:{host}.bind` → app ID
2. `apps:{app_id}.build` → build ID
3. `builds:{build_id}.package` → package ID

It ranks configured hypervisors, calls `POST /container/inject/{package_id}` and then `POST /container/run/{container_id}`, and returns the host port allocated by the hypervisor. If a hypervisor fails, the router tries the next ranked server.

## Server ranking

Each server starts with a score of 100. A successful request subtracts one point. A server that served the same host within the last 24 hours receives a 500-point cache-affinity boost. `power` is used as a capacity tie-breaker when scores are equal.

Bindings are refreshed on every request. A background reaper removes bindings that receive no requests for 30 minutes and asks the owning hypervisor to shut down and delete their containers.

## Configuration

Each `servers` entry requires:

- `id`: stable unique server ID
- `address`: hypervisor HTTP base URL, such as `http://127.0.0.1:3100`
- `power`: capacity tie-breaker
- `tunnel` and `tunnel_address`: reserved for tunnel routing

The router and hypervisor must use the same Redis and package storage data.
