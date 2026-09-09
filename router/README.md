# smoothie/router

Routes approved domains to app containers started by a Smoothie hypervisor.

## Endpoints

- `GET /allow?domain={domain}` → `200` when the domain resolves to a deployed package, otherwise `403`.
- `GET /route?host={host}&scheme={scheme}&id={request_id}&ip={ip}` → `[{"dial":"host:port"}]`.
- `POST /deployments/{deployment_id}` starts an explicit, domainless deployment.
- `GET /deployments/{deployment_id}` returns an explicit deployment, or `404` when it is absent.
- `DELETE /deployments/{deployment_id}` stops and removes an explicit deployment. Repeated deletes succeed.

For an unbound host, `/route` resolves the deployment through Redis:

1. `domains:{host}.bind` → app ID
2. `apps:{app_id}.build` → build ID
3. `builds:{build_id}.package` → package ID

It ranks configured hypervisors, calls `POST /container/inject/{package_id}` with the default argv `["./main"]` and then `POST /container/run/{container_id}?port=8080`, and returns the host port allocated by the hypervisor. If a hypervisor fails, the router tries the next ranked server.

## Domainless deployments

Create an explicit deployment with:

```http
POST /deployments/deployment-123
Content-Type: application/json

{
  "service_id": "service-123",
  "package_id": "package-123",
  "argv": ["./main", "--production"],
  "port": 8080
}
```

`deployment_id`, `service_id`, and `package_id` must be nonempty, `argv` must contain at least one argument, and `port` must be nonzero. The router ranks hypervisors using `service_id` as the affinity key, forwards `argv` during injection, and publishes the requested container port.

A successful `POST` or `GET` returns:

```json
{
  "deployment_id": "deployment-123",
  "status": "running",
  "host": "127.0.0.1",
  "host_port": 3210,
  "container_port": 8080,
  "container_id": "container-123"
}
```

Posting an ID that is already running returns the existing binding without launching another container only when `service_id`, `package_id`, `argv`, and `port` match the original request. Reusing a running deployment ID with different launch configuration returns `409 Conflict`. `DELETE` returns the same fields with `status` set to `deleted` and the host, port, and container fields set to `null`. A hypervisor `404` during shutdown is treated as already stopped; other shutdown errors preserve the binding so deletion can be retried.

Explicit deployments are stored separately from domain bindings. They remain running until explicitly deleted and are never removed by the 30-minute domain idle reaper.

## Server ranking

Each server starts with a score of 100. A successful request subtracts one point. A server that served the same host within the last 24 hours receives a 500-point cache-affinity boost. `power` is used as a capacity tie-breaker when scores are equal.

Domain bindings are refreshed on every request. A background reaper removes domain bindings that receive no requests for 30 minutes and asks the owning hypervisor to shut down and delete their containers.

## Configuration

Each `servers` entry requires:

- `id`: stable unique server ID
- `address`: hypervisor HTTP base URL, such as `http://127.0.0.1:3100`
- `power`: capacity tie-breaker
- `tunnel` and `tunnel_address`: reserved for tunnel routing

The router and hypervisor must use the same Redis and package storage data.
