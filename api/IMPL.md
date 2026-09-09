# Smoothie API implementation

## Resource model

Smoothie treats **Service**, **Package**, and **Deployment** as separate resources.

### Service

A service is an app-owned canvas node and runtime configuration. It can exist before any code is uploaded. Its mutable fields are:

- `name`
- integer `position_x` and `position_y` canvas coordinates
- `container_port`
- `argv`
- `active_package_id` and `active_deployment_id`

`package_ids` is persisted as an oldest-to-newest history. Package and deployment list endpoints return newest-first for UI use, while quota pruning traverses the persisted package history oldest-first. Package history is retained when a blob is pruned or explicitly deleted. Older service JSON using `owner_id` is accepted and missing runtime/canvas fields receive defaults; the legacy Docker `image` value is ignored when records are decoded.

Service list and get responses add a transient `active_deployment` object loaded from the deployment hash. It is never persisted into `Service`. `PATCH` accepts `active_package_id: null` to stop the active runtime and clear package selection; selecting a non-null package is only allowed through the activation endpoint.

### Package

A package is immutable build/archive metadata linked to one service. Metadata includes the sanitized original upload `filename`; old records default this field for serde compatibility. The package archive is stored in the configured S3 bucket as `{package_id}.tar`. The mutable `blob_status` (`available`, `pruned`, or `deleted`) describes storage lifecycle without rewriting package identity, checksum, size, ownership, or creation metadata.

Uploads use `multipart/form-data` with a file field named `package`. Before S3 upload, the API checks:

- archive size is at most exactly `209715200` bytes
- tar paths are relative, normalized paths with no traversal
- entries are regular files or directories (no links, devices, or other special entries)
- entry count and declared content size limits
- an executable regular file named `main` exists at the archive root
- the complete archive SHA-256 checksum

Each service has an available-blob cap of exactly `209715200` bytes. Upload activates the new package. After activation, oldest non-active available packages are deleted from S3 and marked `pruned` until available package bytes are within the cap. Metadata and IDs remain queryable. An active package cannot be explicitly deleted.

### Deployment

A deployment is an immutable-ish record of one request to run one package with a snapshot of the service arguments and port. Service `argv` contains only arguments to the required root executable; deployment and router `argv` are always `["./main", ...service.argv]`, including `["./main"]` when the service has no arguments. Router-derived status and runtime fields may change, while deployment identity and requested package/configuration remain stable.

The router contract is:

- `POST {router}/deployments/{deployment_id}` with `{service_id, package_id, argv, port}`
- `GET {router}/deployments/{deployment_id}` for current runtime state
- `DELETE {router}/deployments/{deployment_id}` to stop it

Router responses contain `status`, `host`, `host_port`, `container_port`, and `container_id`. A failed deploy is not rolled back: the active service selection, package, and deployment stay persisted, and the deployment is marked `failed` with an actionable error. Upload, activation, and start therefore return their normal success HTTP status after persistence even when the router fails; clients inspect deployment `status` and `error`. This makes retry explicit and preserves failure history.

Starting a service creates a new deployment for its active package. Retrying reuses the selected deployment ID. Stopping removes the active runtime association but does not remove the active package selection.

## Persistence

Existing user/app Redis ownership boundaries are preserved:

- `user:{owner}:app:{app_id}:services` — service hash
- `user:{owner}:app:{app_id}:service:{service_id}:packages` — package hash
- `user:{owner}:app:{app_id}:service:{service_id}:deployments` — deployment hash

All routes authenticate first and load resources through the authenticated owner's app/service namespace. Cross-owner resources therefore resolve as not found rather than leaking their existence. An in-process lock registry keyed by `(owner, app, service)` serializes package upload, activation, deletion, quota pruning, and related runtime/service mutations. State is reloaded after lock acquisition. This protects one API process; distributed serialization would require a Redis lock if multiple API processes are introduced.

Service deletion stops its active deployment, deletes all available S3 package blobs, and removes service/package/deployment Redis records. App deletion enumerates its services and runs the same cleanup before deleting the app.

## HTTP surface

In addition to service CRUD:

- `PATCH /apps/{app_id}/services/{service_id}/position`
- `GET /apps/{app_id}/services/{service_id}/packages`
- `POST /apps/{app_id}/services/{service_id}/packages`
- `DELETE /apps/{app_id}/services/{service_id}/packages/{package_id}`
- `POST /apps/{app_id}/services/{service_id}/packages/{package_id}/activate`
- `GET /apps/{app_id}/services/{service_id}/deployments`
- `GET /apps/{app_id}/services/{service_id}/deployments/{deployment_id}`
- `POST /apps/{app_id}/services/{service_id}/deployments/start`
- `POST /apps/{app_id}/services/{service_id}/deployments/{deployment_id}/retry`
- `POST /apps/{app_id}/services/{service_id}/deployments/stop`

## Canvas and domainless routing

The canvas is currently organizational only: integer service coordinates have no runtime effect. Runtime access is represented by router-reported host/port information on deployments. Services and apps are deliberately domainless in this phase—there is no hostname, TLS certificate, route binding, or DNS ownership in these records.

Future domain integration should add a separate app/service route or domain-binding resource rather than embedding domain state in packages or deployments. That resource can point at a service's active healthy deployment, own hostname verification and TLS lifecycle, and update router ingress independently when deployments change. Keeping this boundary allows package history, runtime retries, canvas layout, and domain management to evolve without coupling their lifecycles.
