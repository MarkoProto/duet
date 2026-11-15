# Daytona REST spec, read from the Python SDK 0.231.1 source

Derived only by reading source under `SP=/tmp/dcheck.rsLxkF/v/lib/python3.12/site-packages` (all citations are relative to `SP`). Nothing was sent to the service and no SDK code was executed. **UNCONFIRMED** marks anything the source does not prove (mostly server-side behaviour).

Packages found, all 0.231.1: `daytona` (hand-written), `daytona_api_client`, `daytona_toolbox_api_client`, their `_async` twins, `daytona_analytics_api_client(_async)`. Only the sync variants were read; the async twins are generated from the same OpenAPI documents.

## 0. Conventions used by every call

Placeholders:

- `$API`: control-plane base, default `https://app.daytona.io/api`. It already contains `/api`; the SDK builds URLs as `host + path` with no slash normalisation.
- `$KEY`: API key.
- `$SANDBOX_ID`: the sandbox `id` (not its name).
- `$TOOLBOX`: the sandbox's `toolboxProxyUrl`, with any trailing `/` removed.

Two base URLs:

| Kind | URL | Used for |
|---|---|---|
| Control plane | `$API/sandbox...` | create, get, list, delete, ttl, autostop, previews |
| Toolbox | `$TOOLBOX/$SANDBOX_ID/...` | files, process, sessions, work-dir |

How the SDK gets the toolbox base:

1. It reads `toolboxProxyUrl` from the Sandbox JSON returned by create, get or list (the field is required in both `Sandbox` and `SandboxListItem`).
2. If that is empty, it calls `GET $API/sandbox/$SANDBOX_ID/toolbox-proxy-url`, which returns `{"url": "..."}`.
3. Every toolbox request URL is `toolboxProxyUrl + "/" + sandbox.id + resource_path`. A proxy object prepends the id to the path and swaps the host. It always uses the id, never the name.
4. If a later GET returns a different `toolboxProxyUrl`, the SDK switches to it.

The generated toolbox client's header comment says: "Default for Daytona Cloud: `https://proxy.app.daytona.io/toolbox/{sandboxId}`". Treat that as an example and read the real value from the sandbox JSON (the actual value per region is UNCONFIRMED).

Headers on every request, identical for control plane and toolbox (the toolbox client gets a deep copy of the control-plane defaults):

```
Authorization: Bearer $KEY
X-Daytona-Source: sdk-python
X-Daytona-SDK-Version: 0.231.1
User-Agent: sdk-python/0.231.1
X-Daytona-Organization-ID: <org id>      # only with JWT auth, never with an API key
Accept: application/json                  # per operation, see each section
Content-Type: application/json            # only when there is a JSON body
```

Whether the server validates or branches on `X-Daytona-Source` is UNCONFIRMED. One toolbox endpoint is documented as depending on the caller's SDK version (section 9), so a client that omits `X-Daytona-SDK-Version` may see different response formats.

Shell shorthand used in the curl examples below:

```bash
H=(-H "Authorization: Bearer $KEY" -H "X-Daytona-Source: sdk-python" \
   -H "X-Daytona-SDK-Version: 0.231.1" -H "User-Agent: sdk-python/0.231.1")
```

Encoding rules in the generated clients:

- Path parameters are percent-encoded with no safe characters (`/` becomes `%2F`). Booleans in paths are `true` / `false`.
- Query values are percent-encoded with `/` left as is; booleans are `true` / `false`; list parameters marked `multi` repeat the key (`states=started&states=stopped`).
- JSON bodies use camelCase keys. `None` fields are omitted (`exclude_none=True`), never sent as `null`.
- Any status outside 200-299 is an error. Success codes vary (200, 201, 202, 204), so accept any 2xx.

Sources: `daytona/_sync/daytona.py:154,201,219,226-227,245-246,254,843-862`; `daytona/_sync/sandbox.py:226-234,1544-1548`; `daytona/internal/toolbox_api_client_proxy.py:71-81`; `daytona_api_client/api_client.py:184-185,201-206,235-248,324-329,510-547`; `daytona_api_client/configuration.py:209,212`; `daytona_toolbox_api_client/api_client.py:4,95`; `daytona_api_client/api/sandbox_api.py:7390-7442`; `daytona_api_client/models/toolbox_proxy_url.py:33`.

## 1. Client configuration

Environment variables read:

| Variable | Read from | Purpose |
|---|---|---|
| `DAYTONA_API_KEY` | process env, then `.env.local`, then `.env` | Bearer token. Not read from env if a JWT was given in config |
| `DAYTONA_JWT_TOKEN` | same | Alternative Bearer token |
| `DAYTONA_ORGANIZATION_ID` | same | Required with JWT; sent as `X-Daytona-Organization-ID` |
| `DAYTONA_API_URL` | process env only (never dotenv) | API base |
| `DAYTONA_SERVER_URL` | process env only | Deprecated alias of `DAYTONA_API_URL` |
| `DAYTONA_TARGET` | process env, `.env.local`, `.env` | Default region; sent as `target` in the create body |
| `DAYTONA_USE_DEPRECATED_POLLING` | same | `"true"` disables the event stream (polling only) |
| `DAYTONA_OTEL_ENABLED`, `DAYTONA_EXPERIMENTAL_OTEL_ENABLED` | same | `"true"` turns on tracing |

- Explicit `DaytonaConfig` values win. Env is consulted only when config is missing or incomplete.
- Default API URL: `https://app.daytona.io/api`.
- Auth header: `Authorization: Bearer <api_key or jwt_token>`; the API key wins if both exist. Without an API key, the organization id is mandatory.
- No credentials at all raises a client-side error before any request.
- Default headers attached to all requests: the block in section 0. Nothing else.
- Side effect, not REST: unless polling mode is on, the constructor opens a socket.io WebSocket to the API origin with path `<api path>/socket.io/`, auth `{"token": key}` and query `organizationId`, `source`, `sdkVersion`. A plain REST client can ignore it and poll.

Sources: `daytona/_sync/daytona.py:154-216,226-254,279-300`; `daytona/_utils/env.py:26-51`; `daytona/common/daytona.py:90-104,127-131`; `daytona/internal/event_dispatcher.py:408-410,450-464`.

## 2. Create a sandbox

`POST $API/sandbox`, JSON body, returns 200 with a Sandbox object. No query parameters. Headers: defaults plus `Accept: application/json`, `Content-Type: application/json`.

What `Daytona().create()` with no arguments sends:

```bash
curl -sS -X POST "$API/sandbox" "${H[@]}" \
  -H "Accept: application/json" -H "Content-Type: application/json" \
  -d '{"env":{},"labels":{"code-toolbox-language":"python"},"volumes":[]}'
```

`env`, `labels` and `volumes` are always present. `snapshot` is absent, which is how the default snapshot is requested. `target` appears only if `DAYTONA_TARGET` or a per-call target is set. The `code-toolbox-language` label is an SDK convention for `code_run`; whether the server needs it is UNCONFIRMED (the SDK defaults to python when it is missing on read).

With the options you listed:

```json
{
  "snapshot": "my-snapshot",
  "env": {"FOO": "bar"},
  "labels": {"purpose": "ci", "code-toolbox-language": "python"},
  "public": true,
  "autoStopInterval": 15,
  "autoDeleteInterval": 0,
  "ttlMinutes": 30,
  "volumes": []
}
```

SDK parameter to wire key (every body key is optional in the model):

| SDK param | Wire key | Notes |
|---|---|---|
| `snapshot` | `snapshot` | string, id or name; omitted when unset |
| `env_vars` | `env` | object of strings; always sent, `{}` when empty |
| `labels` | `labels` | object of strings; always sent |
| `public` | `public` | bool |
| `ttl_minutes` | `ttlMinutes` | int minutes from creation; 0 disables |
| `auto_stop_interval` | `autoStopInterval` | int minutes; 0 disables; SDK docs say default 15 |
| `auto_delete_interval` | `autoDeleteInterval` | int minutes; negative disables; 0 deletes on stop |
| `ephemeral` | none | Not a wire field. The SDK turns it into `autoDeleteInterval: 0` |
| `auto_archive_interval` | `autoArchiveInterval` | int minutes; 0 means maximum |
| `auto_pause_interval` | `autoPauseInterval` | int minutes; exclusive with a non-zero auto-stop |
| `name` | `name` | |
| `os_user` | `user` | |
| `target` | `target` | region |
| `queue_timeout` | `queueTimeout` | int minutes, at least 1 |
| `volumes` | `volumes` | `[{"volumeId","mountPath","subpath"?}]`; always sent, `[]` when none |
| `secrets` | `secrets` | list of one-entry objects: `[{"ENV_NAME":"secret-name"}]` |
| `network_block_all` | `networkBlockAll` | bool |
| `network_allow_list` | `networkAllowList` | comma-separated CIDRs |
| `domain_allow_list` | `domainAllowList` | comma-separated domains |
| `outbound_proxy_url` | `outboundProxyUrl` | |
| `otel_endpoint_override` | `otelEndpointOverride` | |
| `linked_sandbox` | `linkedSandbox` | |
| `kvm`, `spot` | `kvm`, `spot` | bool |
| `resources` | `cpu`, `memory`, `disk`, `gpu`, `gpuType` | ints (GiB for memory and disk); `gpuType` is a list. The SDK sends these only for image-based creation, together with `buildInfo`. `CreateSandboxFromSnapshotParams` has no `resources` field, so whether the server accepts them with a snapshot is UNCONFIRMED |
| `image` | `buildInfo` | `{"dockerfileContent": "...", "contextHashes"?: [...]}` |

Client-side validation the SDK does before the POST: intervals and `ttl_minutes` must be non-negative, `queue_timeout` at least 1, auto-stop and auto-pause not both non-zero.

Response: the Sandbox object (fields in section 3). The model declares only status 200.

Waiting for ready, after the POST:

- If the POST response already has `state == "started"`, the SDK returns at once.
- Otherwise it polls `GET $API/sandbox/$SANDBOX_ID` (section 3). The first GET is immediate.
- Ready: `state == "started"`.
- Failed: `state` is `error` or `build_failed`. The error message includes `errorReason`.
- Failed: `state == "destroyed"` with `queueTimedOutAt` set (queue timeout) or `spotEvictedAt` set (spot eviction).
- A plain `destroyed` without those fields is not treated as terminal by the SDK; it keeps polling until the timeout.
- Anything else (`creating`, `starting`, `pulling_snapshot`, `pending_build`, `building_snapshot`, `restoring`, `unknown`, ...) means keep waiting.
- Poll interval, default mode: socket.io push events plus a safety-net GET every 1.0 s.
- Poll interval, polling-only mode: 0.1 s for the first 5 s, then multiplied by 1.1 per poll, capped at 1.0 s.
- Timeout: `create(timeout=60)` seconds for POST plus wait; 0 means no limit. The POST gets a 60 s HTTP timeout; the polling GETs have none.
- On timeout after the POST succeeded, the SDK raises an error that carries the sandbox id. It does not delete the sandbox.

All `state` values in the enum: `creating`, `restoring`, `destroyed`, `destroying`, `started`, `stopped`, `starting`, `stopping`, `error`, `build_failed`, `pending_build`, `building_snapshot`, `unknown`, `pulling_snapshot`, `archived`, `archiving`, `resizing`, `snapshotting`, `forking`, `pausing`, `paused`, `resuming`. The SDK maps unknown strings to a catch-all, so tolerate new values.

Sources: `daytona/_sync/daytona.py:447-469,481-586,639-644`; `daytona/common/daytona.py:17,198-235,248-259`; `daytona_api_client/api/sandbox_api.py:625-691,830-906`; `daytona_api_client/models/create_sandbox.py:36-66,103-107`; `daytona_api_client/models/create_build_info.py:33-36`; `daytona_api_client/models/sandbox_volume.py:33-37`; `daytona_api_client/models/sandbox_state.py:30-62`; `daytona/_sync/sandbox.py:659-679,1401-1415,1417-1514`; `daytona/_utils/timeout.py:79-123`; `daytona/common/errors.py:204-212`.

## 3. Get one sandbox

```bash
curl -sS "$API/sandbox/$SANDBOX_ID" "${H[@]}" -H "Accept: application/json"
```

- `GET $API/sandbox/{sandboxIdOrName}`: accepts id or name. Returns 200.
- Optional query `verbose` (bool) exists; the SDK never sends it.
- The SDK's own refresh always uses the id.

Response fields that matter (camelCase on the wire):

```json
{
  "id": "…", "name": "…", "organizationId": "…",
  "state": "started", "desiredState": "started",
  "errorReason": "…", "recoverable": false,
  "toolboxProxyUrl": "https://…",
  "labels": {}, "env": {}, "public": false,
  "snapshot": "…", "user": "…", "target": "…",
  "cpu": 1, "gpu": 0, "memory": 1, "disk": 3,
  "autoStopInterval": 15, "autoArchiveInterval": 10080, "autoDeleteInterval": -1,
  "autoDestroyAt": "…", "queueTimedOutAt": "…", "spotEvictedAt": "…",
  "createdAt": "…", "updatedAt": "…", "lastActivityAt": "…",
  "daemonVersion": "…", "runnerId": "…"
}
```

(Values above are illustrative, not observed.)

- Declared required: `id`, `organizationId`, `name`, `user`, `env`, `labels`, `public`, `networkBlockAll`, `kvm`, `target`, `cpu`, `gpu`, `memory`, `disk`, `toolboxProxyUrl`.
- Everything else is optional, including `state` and `errorReason`.
- The four `auto*Interval` values are typed "float or int"; parse them as numbers, not strictly integers.
- Timestamps are strings (format UNCONFIRMED).
- `desiredState` values: `destroyed`, `started`, `stopped`, `resized`, `archived`, `paused`.
- A 404 here is what the SDK treats as "destroyed" when waiting for a delete.

Sources: `daytona/_sync/daytona.py:695-701`; `daytona/_sync/sandbox.py:293,1576-1584`; `daytona_api_client/api/sandbox_api.py:3779-3840,3999-4056`; `daytona_api_client/models/sandbox.py:38-85`; `daytona_api_client/models/sandbox_desired_state.py:30-35`.

## 4. List sandboxes filtered by labels

Supported. `GET $API/sandbox`, returns 200 with `{"items": [...], "nextCursor": "…" | null}`.

```bash
curl -sS --get "$API/sandbox" "${H[@]}" -H "Accept: application/json" \
  --data-urlencode 'labels={"purpose":"ci"}' \
  --data-urlencode 'limit=100'
# next page: add  --data-urlencode "cursor=$NEXT_CURSOR"
```

- `labels` is one query parameter holding a JSON object as a string. The SDK sends Python `json.dumps` output, for example `labels=%7B%22purpose%22%3A%20%22ci%22%7D`. Matching semantics for several labels (all must match?) are UNCONFIRMED.
- Pagination is by cursor: repeat with `cursor=<nextCursor>` until `nextCursor` is null or empty. `limit` is the page size, 1 to 200.
- The generated docstring says "Eventually consistent", so a sandbox created moments ago may be missing.
- Other query parameters, all optional: `id` and `name` (prefix, case-insensitive), `states` (repeated key), `snapshots` (repeated), `regionIds` (repeated), `sandboxClasses` (repeated), `minCpu`, `maxCpu`, `minMemoryGiB`, `maxMemoryGiB`, `minDiskGiB`, `maxDiskGiB`, `minGpu`, `maxGpu`, `isPublic`, `isRecoverable`, `createdAtAfter`, `createdAtBefore`, `lastEventAfter`, `lastEventBefore`, `autoDestroyAtAfter`, `autoDestroyAtBefore`, `sort` (`name`, `cpu`, `memoryGib`, `diskGib`, `lastActivityAt`, `createdAt`), `order` (`asc`, `desc`), `includeErroredDeleted`, `includeWarm`.
- The SDK never sends `includeErroredDeleted`, `includeWarm`, `sandboxClasses`, `minGpu` or `maxGpu`.
- Datetimes are sent as `strftime("%Y-%m-%dT%H:%M:%S.%f%z")`, for example `2026-11-11T10:00:00.000000+0000` (the `+` must be encoded as `%2B`). Other accepted formats are UNCONFIRMED.
- List items are a reduced shape. They include `id`, `name`, `state`, `desiredState`, `labels`, `errorReason`, `toolboxProxyUrl`, `createdAt`, `lastActivityAt`, `autoDestroyAt` and the interval fields. They do not include `env`, `volumes`, `buildInfo` or `backupCreatedAt`.

Sources: `daytona/_sync/daytona.py:740-793`; `daytona/common/sandbox.py:83-105`; `daytona_api_client/api/sandbox_api.py:7459-7506,7627-7628,8043-8255`; `daytona_api_client/models/list_sandboxes_response.py:34-35`; `daytona_api_client/models/sandbox_list_item.py:37-76`; `daytona_api_client/models/sandbox_list_sort_field.py:30-35`.

## 5. Delete a sandbox

```bash
curl -sS -X DELETE "$API/sandbox/$SANDBOX_ID" "${H[@]}" -H "Accept: application/json"
```

- `DELETE $API/sandbox/{sandboxIdOrName}`. No query parameters and no body. There is no `force` on delete in this version (`force` exists only on `POST .../stop`).
- Returns 200 with a Sandbox object; the SDK tolerates an empty body.
- The SDK returns as soon as the request is accepted. With `wait=True` it polls `GET $API/sandbox/$SANDBOX_ID` until `state == "destroyed"` or the GET returns 404; `error` and `build_failed` count as failure. Default timeout 60 s.
- What the server answers for an already-deleted sandbox is UNCONFIRMED (the SDK maps a 404 to "not found").

Sources: `daytona/_sync/sandbox.py:627-654`; `daytona/_sync/daytona.py:647-669`; `daytona_api_client/api/sandbox_api.py:1505-1571,1710-1773,12248-12250`.

## 6. `set_ttl` and `set_autostop_interval`

Both are POSTs with the value in the URL path, no body, no query. Both return 200 with a Sandbox object, which the SDK ignores.

```bash
curl -sS -X POST "$API/sandbox/$SANDBOX_ID/ttl/30" "${H[@]}" -H "Accept: application/json"
curl -sS -X POST "$API/sandbox/$SANDBOX_ID/autostop/15" "${H[@]}" -H "Accept: application/json"
```

- `POST $API/sandbox/{sandboxIdOrName}/ttl/{ttlMinutes}`: minutes counted from now ("re-anchored from the current time"); 0 disables. The SDK rejects negatives client-side.
- `POST $API/sandbox/{sandboxIdOrName}/autostop/{interval}`: minutes of inactivity; 0 disables. The SDK rejects negatives client-side.
- Same pattern for the related calls: `POST .../autodelete/{interval}` (negative disables, 0 deletes on stop), `POST .../autoarchive/{interval}`, `POST .../autopause/{interval}`.

Sources: `daytona/_sync/sandbox.py:734-738,822-825,887-890`; `daytona_api_client/api/sandbox_api.py:11147-11207,11364-11419,11436-11496,11653-11708,10786-10841`.

## 7. `sandbox.fs.upload_file(bytes, remote_path)` and `upload_files`

`upload_file` is a thin wrapper over `upload_files` with one entry, so both use the same endpoint:

`POST $TOOLBOX/$SANDBOX_ID/files/bulk-upload`, body `multipart/form-data`, no query parameters. Headers: defaults plus `Accept: */*`. The destination path travels as a form field, not as a query parameter.

```bash
curl -sS -X POST "$TOOLBOX/$SANDBOX_ID/files/bulk-upload" "${H[@]}" -H "Accept: */*" \
  -F 'files[0].path=tmp/hello.txt' \
  -F 'files[0].file=@./hello.txt;filename=tmp/hello.txt'
```

Exact multipart layout the SDK produces (through httpx):

```
--BOUNDARY
Content-Disposition: form-data; name="files[0].path"

tmp/hello.txt
--BOUNDARY
Content-Disposition: form-data; name="files[0].file"; filename="tmp/hello.txt"
Content-Type: text/plain

<file bytes>
--BOUNDARY--
```

- Field names are literally `files[i].path` (text) and `files[i].file` (file part), with `i` counting from 0.
- The file part's `filename` is the destination path. Its `Content-Type` is guessed from that name, falling back to `application/octet-stream`.
- With several files, the SDK sends all `files[i].path` parts first, then all `files[i].file` parts. Whether the server needs a path part before its file part is UNCONFIRMED, so keep that order.
- Success: any 2xx. The generated model for 200 is `{"files": [{"path": "...", "bytes": 123}], "errors": ["..."]}`, both keys optional. The SDK ignores the success body. Whether a 200 can carry partial `errors` is UNCONFIRMED, so check for a non-empty `errors`.
- Failure: non-2xx. The SDK first tries to read `{"errors": ["..."]}` and joins them; otherwise it uses the raw body text.
- Relative paths: the SDK sends the path exactly as given. Its docstrings say relative paths are resolved by the sandbox against the sandbox working directory (the image WORKDIR, or the user's home when there is none). There is no client-side resolution.
- To read those directories: `GET $TOOLBOX/$SANDBOX_ID/work-dir` and `GET $TOOLBOX/$SANDBOX_ID/user-home-dir`, each returning `{"dir": "/abs/path"}`.
- Size limits and chunking: none in the SDK. Bytes are sent from memory in one request; a local file is streamed from disk in 64 KiB reads, still one request. Any server or proxy size limit is UNCONFIRMED.
- Client timeouts: read and write 30 min by default (0 means none), connect 30 s. Connection establishment is retried up to 3 times.

A single-file endpoint also exists in the generated client, but the hand-written SDK never calls it with Bearer auth (it is used only for pre-signed upload URLs): `POST $TOOLBOX/$SANDBOX_ID/files/upload-v2?path=<dest>`, multipart field `file` or a raw request body, 200 returns `{"name","path","type"}`. Parent directories are created and an existing file is overwritten, per its docstring. Use with Bearer auth is UNCONFIRMED.

Sources: `daytona/_sync/filesystem.py:866-919,950-987`; `daytona/common/filesystem.py:23-35`; `daytona_toolbox_api_client/api/file_system_api.py:3204-3206,3243-3245,3398-3463,3529-3530,3667-3722`; `daytona_toolbox_api_client/models/upload_files_response.py:33-34`; `daytona_toolbox_api_client/models/fs_file_result.py:32-33`; `daytona_toolbox_api_client/models/uploaded_file.py:32-34`; `httpx/_multipart.py:115-169,244-256`; `daytona/internal/http_client.py:16-25,39-50,69-80`; `daytona/_sync/sandbox.py:310-311,324-338,530-532`; `daytona_toolbox_api_client/api/info_api.py:270,760`.

## 8. `sandbox.process.exec(command, cwd, env, timeout)`

`POST $TOOLBOX/$SANDBOX_ID/process/execute`, JSON body, returns 200.

```bash
curl -sS -X POST "$TOOLBOX/$SANDBOX_ID/process/execute" "${H[@]}" \
  -H "Accept: application/json" -H "Content-Type: application/json" \
  -d '{"command":"echo hello","cwd":"/tmp","envs":{"FOO":"bar"},"timeout":30}'
```

Request body:

| Key | Type | Required | Notes |
|---|---|---|---|
| `command` | string | yes | Sent exactly as passed. This SDK version does no `sh -c` wrapping and no base64 encoding |
| `cwd` | string | no | Omitted when unset; docstring says the sandbox working directory is used |
| `envs` | object of strings | no | Note the plural key; the SDK parameter is `env` |
| `timeout` | int, seconds | no | Omitted when unset. The generated model says "defaults to 10 seconds"; the real daemon default is UNCONFIRMED, so always send it |

Response body:

```json
{"exitCode": 0, "result": "hello\n"}
```

- `result` is a required string. The SDK docs call it standard output; whether stderr is merged into it is UNCONFIRMED.
- `exitCode` is optional in the model. If it is missing the SDK falls back to a legacy `code` key.
- The SDK returns the exit code without raising, so a non-zero exit presumably still arrives as HTTP 200 (UNCONFIRMED).
- Declared error statuses: 400 and 408, both with the toolbox error body (section 11). The SDK has a daemon error code `PROCESS_EXECUTION_TIMEOUT`; that this is what a 408 carries is UNCONFIRMED.
- The SDK docstring says the command is run by the first available of zsh, bash, sh in the sandbox.
- Client HTTP timeout: `timeout + 5` seconds when a timeout is given, otherwise none.

Sources: `daytona/_sync/process.py:84-156`; `daytona_toolbox_api_client/api/process_api.py:1701-1756,1903-1963`; `daytona_toolbox_api_client/models/execute_request.py:32-37,75-77`; `daytona_toolbox_api_client/models/execute_response.py:32-35`.

## 9. Sessions

All on the toolbox base. JSON keys are camelCase.

Create a session: `POST /process/session`, returns 201 with an empty body. 409 is declared (presumably "already exists", UNCONFIRMED).

```bash
curl -sS -X POST "$TOOLBOX/$SANDBOX_ID/process/session" "${H[@]}" \
  -H "Accept: application/json" -H "Content-Type: application/json" \
  -d '{"sessionId":"my-session"}'
```

Run a command in it: `POST /process/session/{sessionId}/exec`, returns 200 or 202.

```bash
curl -sS -X POST "$TOOLBOX/$SANDBOX_ID/process/session/my-session/exec" "${H[@]}" \
  -H "Accept: application/json" -H "Content-Type: application/json" \
  -d '{"command":"sleep 5 && echo done","runAsync":true}'
```

- Request: `command` (string, required), `runAsync` (bool, optional), `suppressInputEcho` (bool, optional), `async` (bool, deprecated alias of `runAsync`). No timeout field exists in the body.
- Response: `{"cmdId": "..."}` is required. `exitCode`, `output`, `stdout`, `stderr` are optional and filled for synchronous runs.
- Both 200 and 202 map to the same response type. Which mode returns which is UNCONFIRMED (presumably 202 for `runAsync`), so accept both.
- Declared errors: 400, 404, 409, 410, 500.

Command status and exit code: `GET /process/session/{sessionId}/command/{commandId}`, returns 200 with `{"id": "...", "command": "...", "exitCode": 0}`. `exitCode` is optional; presumably absent while the command is running (UNCONFIRMED), so poll until it appears.

```bash
curl -sS "$TOOLBOX/$SANDBOX_ID/process/session/my-session/command/$CMD_ID" "${H[@]}" -H "Accept: application/json"
```

Command logs: `GET /process/session/{sessionId}/command/{commandId}/logs`, returns 200 with `{"output": "...", "stdout": "...", "stderr": "..."}` (all three required strings in the model).

```bash
curl -sS "$TOOLBOX/$SANDBOX_ID/process/session/my-session/command/$CMD_ID/logs" "${H[@]}" -H "Accept: application/json"
```

- The generated docstring says: "Returns JSON with separated stdout/stderr for SDK >= 0.167.0, plain text otherwise." So the format depends on the caller's declared SDK version. Which header the daemon inspects is UNCONFIRMED (most likely `X-Daytona-SDK-Version`).
- Send the version header and `Accept: application/json`, and make the parser accept plain text as a fallback.
- The WebSocket stream variant marks stdout chunks with the bytes `01 01 01` and stderr chunks with `02 02 02`. Whether the plain-text REST form contains those markers is UNCONFIRMED.
- Optional query `follow=true` is "WebSocket only". The SDK uses it by changing the URL scheme to `ws`/`wss` on the same path with the same headers.
- Declared errors: 400, 403, 404, 500.

Other session calls:

- Session info: `GET /process/session/{sessionId}` returns `{"sessionId": "...", "commands": [{"id","command","exitCode"?}]}`.
- List sessions: `GET /process/session` returns an array of the same objects.
- Send stdin: `POST /process/session/{sessionId}/command/{commandId}/input` with `{"data": "..."}`, returns 204.
- Delete a session: `DELETE /process/session/{sessionId}`, returns 204 with no body. Declared errors: 400, 404, 500.

```bash
curl -sS -X DELETE "$TOOLBOX/$SANDBOX_ID/process/session/my-session" "${H[@]}" -H "Accept: */*"
```

Sources: `daytona/_sync/process.py:262-287,340-366,370-425,429-462,465-503,563-582,586-631`; `daytona/common/process.py:20-22,41-62`; `daytona_toolbox_api_client/api/process_api.py:884-940,1089-1149,1432-1488,1637-1684,2968-3015,3032-3091,3246-3296,3313-3378,3545-3601,4053-4097,4411-4476,4643-4709,4726-4789,4952-5015`; models `create_session_request.py:32`, `session_execute_request.py:32-37`, `session_execute_response.py:32-38`, `command.py:32-35`, `session.py:33-34`, `session_command_logs_response.py:32-36`, `session_send_input_request.py:32` (all under `daytona_toolbox_api_client/models/`).

## 10. Preview links

All on the control plane.

Standard preview link: `GET $API/sandbox/{sandboxIdOrName}/ports/{port}/preview-url`, returns 200.

```bash
curl -sS "$API/sandbox/$SANDBOX_ID/ports/3000/preview-url" "${H[@]}" -H "Accept: application/json"
```

```json
{"sandboxId": "…", "url": "https://…", "token": "…"}
```

For a private sandbox, requests to `url` must carry `X-Daytona-Preview-Token: <token>` or `Authorization: Bearer $KEY`, otherwise 401 (per the SDK docstring). Public sandboxes need nothing.

Signed preview URL: it is a GET, not a POST. `GET $API/sandbox/{sandboxIdOrName}/ports/{port}/signed-preview-url?expiresInSeconds=N`, returns 200.

```bash
curl -sS "$API/sandbox/$SANDBOX_ID/ports/3000/signed-preview-url?expiresInSeconds=3600" "${H[@]}" -H "Accept: application/json"
```

```json
{"sandboxId": "…", "port": 3000, "token": "…", "url": "https://…"}
```

- `expiresInSeconds` is an optional integer query parameter; omitted means 60 seconds.
- The response has no expiry timestamp field; track expiry yourself.
- The token is embedded in `url`, so the URL works with no headers.

Expire a signed URL: `POST $API/sandbox/{sandboxIdOrName}/ports/{port}/signed-preview-url/{token}/expire`, no body, returns 200 with no body. The token is a path segment, so percent-encode it. The generated client sets no `Accept` header here.

```bash
curl -sS -X POST "$API/sandbox/$SANDBOX_ID/ports/3000/signed-preview-url/$TOKEN/expire" "${H[@]}"
```

Sources: `daytona/_sync/sandbox.py:985-1022,1025-1062,1065-1078`; `daytona_api_client/api/sandbox_api.py:1779-1853,2008-2059,3216-3276,3433-3488,6879-6943,7108-7168`; `daytona_api_client/models/port_preview_url.py:33-37`; `daytona_api_client/models/signed_port_preview_url.py:33-38`.

## 11. Errors and retries

Toolbox error body (generated `ErrorResponse` model):

```json
{"statusCode": 404, "message": "…", "path": "…", "timestamp": "…",
 "code": "FILE_NOT_FOUND", "method": "GET", "source": "DAYTONA_DAEMON"}
```

- Required: `statusCode`, `message`, `path`, `timestamp`. Optional: `code`, `method`, `source`.
- `source` values known to the SDK: `DAYTONA_API`, `DAYTONA_DAEMON`, `DAYTONA_PROXY`.
- `code` values: `BAD_REQUEST`, `INVALID_REQUEST_BODY`, `UNAUTHORIZED`, `FORBIDDEN`, `NOT_FOUND`, `CONFLICT`, `REQUEST_TIMEOUT`, `GONE`, `UNPROCESSABLE_ENTITY`, `INTERNAL_SERVER_ERROR`, `FILE_NOT_FOUND`, `FILE_ACCESS_DENIED`, `INVALID_FILE_PATH`, `FILE_READ_FAILED`, `PROCESS_EXECUTION_TIMEOUT`, `PROCESS_NOT_FOUND`, `SESSION_ENDED`, `COMMAND_ALREADY_COMPLETED`, plus git, LSP and computer-use codes.

Control-plane error body: the generated client has no error model, so the full shape is UNCONFIRMED. What the SDK reads from any error body:

- `message` if it is a string, otherwise `error` if it is a string.
- `code` if it is a string, otherwise `error_code`.
- `source` if it is a string.
- If the body is not a JSON object, the whole body text becomes the message.
- The status comes from the HTTP status line, not from the body.

Bulk upload is the one exception: its error body is `{"errors": ["..."]}`.

Status mapping in the SDK:

| Status | SDK error class |
|---|---|
| 400 | bad request |
| 401 | authentication |
| 403 | forbidden |
| 404 | not found |
| 408, 504 | timeout |
| 409 | conflict |
| 410 | gone |
| 422 | unprocessable entity |
| 429 | rate limit |
| 500 | internal server error |
| 502 | bad gateway |
| 503 | service unavailable |
| other | generic error |

A matching `(source, code)` pair overrides the status mapping, for example `DAYTONA_DAEMON` with `FILE_NOT_FOUND` or `SESSION_ENDED`.

Retries:

- The SDK has no application-level retry and no backoff on 429. It does not read `Retry-After` itself.
- Control plane transport (urllib3 defaults, since the SDK sets nothing): connection failures are retried up to 3 times. Read errors are retried only for GET, HEAD, PUT, DELETE, OPTIONS, TRACE. A 413, 429 or 503 response that carries `Retry-After` is re-sent automatically for those methods, never for POST. Redirects are followed.
- Toolbox transport: the same, plus "remote end closed the connection without a response" is retried for every method including POST (total 3).
- Uploads (httpx): connection establishment retried up to 3 times.
- Timeouts raised client-side during create carry the sandbox id (section 2).

Sources: `daytona/_utils/errors.py:99-130,142-178`; `daytona/common/errors.py:16-22,345-359,368-395,420-440`; `daytona_toolbox_api_client/models/error_response.py:33-41`; `daytona_toolbox_api_client/models/daemon_error_code.py:21-60`; `daytona_api_client/exceptions.py:133-162`; `daytona_api_client/rest.py:92-93`; `daytona/internal/urllib3_retry.py:12-53`; `daytona/_sync/daytona.py:858`; `urllib3/util/retry.py:199-205,423-443,569`; `daytona/internal/http_client.py:16,73-77`; `daytona/_sync/filesystem.py:979-987`.

## Surprises and traps

1. Toolbox URLs are `toolboxProxyUrl + "/" + sandbox id + path`. The proxy URL comes per sandbox from the API; do not hardcode it, and use the id even if you fetched the sandbox by name.
2. `upload_file` is not `POST /files/upload?path=`. It is multipart to `/files/bulk-upload` with form fields named `files[0].path` and `files[0].file`; there is no `path` query parameter.
3. `exec` sends the raw command string. No `sh -c`, no base64. The env key is `envs`, and `timeout` is integer seconds.
4. An unset `exec` timeout is simply omitted, and the generated model says the server default is 10 seconds. Always send one.
5. `ephemeral` does not exist on the wire; send `autoDeleteInterval: 0`. Create uses `env` (not `envVars`) and `user` (not `osUser`).
6. Resources (`cpu`, `memory`, `disk`) are never sent by the SDK for snapshot-based creation.
7. `set_ttl` and `set_autostop_interval` put the number in the URL path and send no body.
8. Creating a signed preview URL is a GET with `expiresInSeconds`; the default lifetime is 60 seconds and the response has no expiry field.
9. Session async flag is `runAsync` (`async` is deprecated). Success may be 200 or 202, and `exitCode` is absent until the command finishes.
10. The session logs endpoint returns JSON only for callers that declare a recent SDK version; otherwise plain text. A client without `X-Daytona-SDK-Version` may get a different format.
11. Delete has no `force` parameter and returns 200 with a body. A later 404 on GET means destroyed.
12. While waiting for start, the SDK does not treat a plain `destroyed` state as failure. Your own client should fail fast on `destroyed`.
13. The create response can already be `started`; check it before polling.
14. List is cursor-paginated, `labels` is a JSON string inside one query parameter, and results are eventually consistent.
15. Success statuses are not uniform: create sandbox 200, create session 201 (empty body), session exec 200 or 202, delete session and send input 204.
16. Interval fields in responses may be floats; `state` may be a string you have not seen before.
17. An error `message` may be a non-string; fall back to `error`.
18. `$API` already ends in `/api`. A trailing slash in `DAYTONA_API_URL` would produce `//sandbox`, because the SDK does not trim it.
