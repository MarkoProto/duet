# Daytona v2 REST spec, read from the Python SDK 2.0.0 source

Derived only by reading the unpacked `daytona-2.0.0` wheel (released 2026-11-13). Nothing was sent to the service and no SDK code was imported or run. Citations are relative to the wheel's `daytona/` folder (for example `v2/daytona.py:153`). `METADATA:n` means line n of `daytona-2.0.0.dist-info/METADATA`, which is the package README.

Two markers:

- **UNCONFIRMED**: server behaviour the source does not prove.
- **INFERRED**: a conclusion drawn from the code's shape, not stated in it.

The companion file `docs/daytona-rest-spec.md` covers the 0.231.1 (v0) requests. This file covers v2 only, plus what changed for the v0 path.

## 0. Short answers

1. **Default API base is different.** v2 uses `https://api.daytona.com`, with no `/api` suffix (`v2/daytona.py:153`). v0 stays on `https://app.daytona.io/api` (`_client.py:57`). `DAYTONA_V2_API_URL` overrides the v2 base; `DAYTONA_API_URL` is ignored by 2.x and only triggers a warning (`v2/daytona.py:1415`, `_client.py:61-88`, `METADATA:160`).
2. **The v0 bundle sends the same requests as 0.231.1.** It is the 0.231.1 hand-written SDK, vendored, on top of the generated clients pinned at `==0.231.1` (`v0/_bundle.py:3-4`, `METADATA:41-46`). Its line numbers still match the old spec's citations, for example the headers at `v0/_sync/daytona.py:227,245-246` and the default URL at `v0/_sync/daytona.py:154`. It still sends `X-Daytona-SDK-Version: 0.231.1` (`v0/_sync/daytona.py:13,245`). So our Rust client, which copies 0.231.1, keeps talking to the v0 server exactly as before. How long the v0 server stays up is UNCONFIRMED. The v2 API has a migration route (`POST /migration/v0/sandboxes/{v0_id}/import`, named in `v2/_gen/api/models/v0_sandbox.py:41`), which suggests v0 and v2 are separate platforms (INFERRED).
3. **Every one of our calls changes under v2.** Paths are plural (`/sandboxes`), control-plane JSON is snake_case, states have new names, exec and background processes use one `/processes` resource, uploads take the path in the query, and signed preview URLs are a POST. Section 12 has the call-by-call table.
4. **Alpha signs.** The package classifier is `Development Status :: 3 - Alpha` and the README still says "published as alpha prereleases … The API may still change between alphas" (`METADATA:11,70-73`). Linux sandbox creation is gated per organization (403 `FEATURE_NOT_ENABLED`, `v2/daytona.py:343-346`). Section 13 lists the rest.

## 1. Conventions used by every v2 call

Placeholders:

- `$API2`: control-plane base, default `https://api.daytona.com`. Paths start with `/`, so URLs are `$API2/sandboxes/...` (`v2/_transport.py:1293-1297`).
- `$KEY`: API key (the README calls it a project API key, `METADATA:247`).
- `$SBX`: the sandbox `id`. Ids look like `sbx_<base58>-<region tag>` (INFERRED from `v2/_events.py:123-135` and the `sbx_` prefix in `v2/services.py:189`). Always percent-encode it as one path segment (`v2/_core.py:2270-2288`).
- `$TOOLBOX`: the per-sandbox toolbox base (below).
- `$SBX_TOKEN`: the sandbox access token (below).

Two base URLs:

| Kind | URL | Used for |
|---|---|---|
| Control plane | `$API2/sandboxes...` | create, get, search, delete, lifecycle, labels, ports and previews, access tokens, network policy, secrets |
| Toolbox | `$TOOLBOX/...` | `/processes`, `/files/...`, `/port`, `/env`, `/system/metrics` |

How the SDK builds `$TOOLBOX`:

1. It reads `toolbox_proxy_url` from the create response (a required field there) or from a sandbox detail or search item (optional there) (`v2/_gen/api/models/sandbox_create_operation.py:84`, `v2/_gen/api/models/sandbox.py:78`).
2. If that URL contains `{sandbox_id}` or `{sandboxId}` in its path, the id is substituted. Otherwise the id is appended: `toolbox_proxy_url.rstrip("/") + "/" + $SBX` (`v2/sandbox.py:2334,2359-2380`). Handle both forms.
3. A later read that returns a different `toolbox_proxy_url` replaces the old one (`v2/sandbox.py:958-970`).
4. The SDK treats a URL whose path ends in `/sandboxes/$SBX` as the normal proxy shape (`v2/sandbox.py:2383-2389`), so the real value is probably like `https://<toolbox host>/sandboxes` before the id is added (INFERRED).

Headers on every request (`v2/_transport.py:68-74,1300-1312`, `v2/_body_limit.py:77`, `v2/_sync/daytona.py:43`):

```
Authorization: Bearer <token>             # $KEY on the control plane, $SBX_TOKEN on the toolbox
X-Daytona-Source: sdk-python              # the sync client; the async one sends sdk-python-async
X-Daytona-SDK-Version: 2.0.0
User-Agent: sdk-python/2.0.0
Accept-Encoding: gzip, deflate
Content-Type: application/json            # set by httpx when there is a JSON body
```

- No `Accept` header is set by the SDK, so httpx sends its default `*/*` (INFERRED from httpx defaults; the one exception is the SSE log follow, section 7).
- There is no organization header in v2. With a JWT instead of a key, the SDK exchanges the JWT for a project token; that path is not relevant to us (`v2/_auth.py`).
- `x-daytona-region: <region>` is sent only on create and `find_by_name` when a region is chosen (`v2/daytona.py:781-784,924-925`). `local`, blank or omitted means "default region" and sends nothing (`v2/_core.py:2250-2267`).
- `Idempotency-Key` is sent on every create (section 3).

Toolbox auth (`$SBX_TOKEN`):

- The create response carries `access_token` (returned once, null on a replay) and `access_token_expires_at` (`v2/_gen/api/models/sandbox_create_operation.py:76-77`, `v2/daytona.py:803-806`).
- When the handle has no fresh token (expiry within 60 s), it mints one: `POST $API2/sandboxes/$SBX/access-token` with body `{}` (`v2/sandbox.py:155,1876-1879,2022-2029`). Response: `{"access_token","token_type":"Bearer","expires_at","operations":[...],"ports":[...]}` (`v2/sandbox.py:2510-2569`).
- After a toolbox 401 it re-mints once and retries once (`v2/sandbox.py:1932-1951`).
- If the mint route answers 404, 405 or 501, the SDK falls back to sending `$KEY` to the toolbox (`v2/sandbox.py:1915-1922,2590-2605`). So the API key probably also works on the toolbox, at least on some deployments (INFERRED; UNCONFIRMED for Daytona Cloud).

Wire format:

- **Control plane JSON is snake_case** (`toolbox_proxy_url`, `region_id`, `lifecycle_policy`, `next_cursor`).
- **Toolbox process JSON is camelCase** (`shellCommand`, `keepLogs`, `exitCode`, `nextCursor`) (`v2/_gen/toolbox/models/create_process_request.py:42-59`). Do not share one serde naming rule across both.
- `None` fields are left out of bodies.
- Query booleans are `true`/`false`; search list filters are comma-joined in one value (`v2/_core.py:2316-2372`).
- A response is parsed as JSON only when its `Content-Type` is JSON; otherwise it is raw bytes (`v2/_transport.py:1315-1332`).
- The SDK never follows redirects; a 3xx is an error (`v2/_core.py:355-359`, `METADATA:1261`).
- Default total request timeout is 60 s; waits and long polls use their own budget plus 5 s slack (`v2/_transport.py:86`, `v2/_core.py:1128`).

Shell shorthand used below:

```bash
API2=https://api.daytona.com
H=(-H "Authorization: Bearer $KEY" -H "X-Daytona-Source: sdk-python" \
   -H "X-Daytona-SDK-Version: 2.0.0" -H "User-Agent: sdk-python/2.0.0")
T=(-H "Authorization: Bearer $SBX_TOKEN" -H "X-Daytona-Source: sdk-python" \
   -H "X-Daytona-SDK-Version: 2.0.0" -H "User-Agent: sdk-python/2.0.0")
```

## 2. Client configuration

Environment variables v2 reads, from the process environment only, never from `.env` files (`METADATA:206-224`, `v2/daytona.py:1410-1468`):

| Variable | Purpose |
|---|---|
| `DAYTONA_API_KEY` | Bearer key |
| `DAYTONA_JWT_TOKEN` + `DAYTONA_PROJECT_ID` | JWT auth, exchanged for a project token |
| `DAYTONA_V2_API_URL` | Another v2 endpoint (a regional endpoint or a test setup) |
| `DAYTONA_OTEL_ENABLED` | Client spans |
| `DAYTONA_V0_API_URL` | Overrides the v0 namespace's URL (dev only) (`_client.py:91-96`) |

- Exactly one of key or JWT. None at all fails before any request.
- There is no default region in v2 config. `DaytonaConfig.target` is deprecated and applies only to v0 (`METADATA:251-254`). In v2 the region is chosen per create.
- Whether a v0 API key is accepted by the v2 API is UNCONFIRMED. Both use the `dtn_` example prefix, but v2 talks about project keys and has projects (`METADATA:247`, `v2/_gen/api/models/create_project_request.py`). A safe probe that changes nothing: `GET $API2/api-keys/current` returns the key's name, project, organization and permissions (`v2/daytona.py:945-952`).
- Linux sandboxes need "Linux access enabled for the organization"; without it create answers 403 `FEATURE_NOT_ENABLED` (`v2/daytona.py:343-346`). Check this before tomorrow.

## 3. Create a sandbox

`POST $API2/sandboxes`, JSON body (`v2/daytona.py:774-788,1862-1871`). The same create also exists as `POST /sandboxes/from-snapshot` and `POST /sandboxes/macos`.

What a room create would look like (built the way `create()` builds it, `v2/daytona.py:1702-1859`):

```bash
curl -sS -X POST "$API2/sandboxes" "${H[@]}" \
  -H "Content-Type: application/json" \
  -H "Idempotency-Key: $(uuidgen)" \
  -d '{
    "image": "duet-room",
    "labels": {"app": "duet", "lobby": "lobby-1", "room": "r-123"},
    "lifecycle_policy": {"timeout_seconds": 1800, "on_timeout": "delete", "idle_expiry_seconds": -1},
    "network_policy": {"block_all": true},
    "wait_seconds": 60,
    "create_timeout_seconds": 120
  }'
```

Body fields (all optional; `v2/_gen/api/models/create_sandbox_request.py:41-63`):

| Field | Notes |
|---|---|
| `image` | Image name or `img_…` id. Omit for the platform default image (`daytona-base` style). A bare registry ref such as `python:3.12` is not accepted here; import it first (below) |
| `name` | Display name, not unique. The SDK also stamps `labels.name = name` and derives the idempotency key from it (below) |
| `labels` | Object of strings. Empty keys are refused |
| `env` | Object of strings. `DAYTONA_*` names (except `DAYTONA_API_URL`, `DAYTONA_OTEL_ENDPOINT`, `DAYTONA_DOCKER_START`) fail with 400 |
| `region_id` | Region. Omitted: project default, then organization default, then `earth` (anywhere) |
| `lifecycle_policy` | See below. This is the v2 "ttl / auto-delete" |
| `network_policy` | `{"block_all","allow_cidrs","allow_domains"}`, or `null` for "no policy". Section 9 |
| `secrets` | List of project secret names. Section 9 |
| `ports` | `[{"port": 3000, "proto": "tcp"}]`, "ports exposed when the sandbox starts" |
| `volumes` | `[{"volume","mount_path","subpath"?,"read_only"?}]` |
| `cpu`, `cores_max` | Cores (1 core = 2 vCPUs); `cores_max` a multiple of 0.5 |
| `memory_mib`, `memory_max_mib`, `disk_mib`, `disk_max_mib` | MiB integers (the SDK converts GiB × 1024) |
| `cpu_arch` | `amd64` or `arm64` |
| `cpu_type`, `user`, `spot` | |
| `wait_seconds` | How long the server may hold the request. The SDK always sends it (default 60). The model says the server default is 3 |
| `create_timeout_seconds` | Server's budget to reach `running` before it deletes the sandbox. Default 3600; `-1` is the server maximum |
| `idempotency_key` | Exists in the body model, but the SDK sends the key as a header only |

Headers beyond the defaults:

- `Idempotency-Key`: the caller's key, else `name:<name>` for a named create (or `name:sha256:<hex>` when the name is not plain ASCII or too long), else a random UUID (`v2/daytona.py:1619-1672`). A 400 `IDEMPOTENCY_KEY_REQUIRED` code exists, so the server may refuse a create without it (`v2/_codes.py:153,475`; INFERRED).
- `x-daytona-region` when a region is given.

Idempotency rules (`METADATA:325-373`):

- A create that timed out or lost its connection should be retried with the same key and the same body. A changed body under the same key is a 409.
- A named create is a get-or-create: a repeat returns the same sandbox, with `access_token` only the first time.
- Delete releases the key, so the same name creates a new sandbox afterwards.

Response 2xx, `SandboxCreateOperation` (`v2/_gen/api/models/sandbox_create_operation.py:76-85`):

```json
{
  "id": "sbx_…", "name": "sandbox-…", "status": "pending",
  "toolbox_proxy_url": "https://…",
  "access_token": "…", "access_token_expires_at": "2026-…Z",
  "create_timeout_at": "…", "wait_cap_seconds": 10,
  "image": {"id": "img_…", "name": "duet-room"}, "reservation_id": null
}
```

(Values illustrative.) Required: `id`, `name`, `status`, `toolbox_proxy_url`, `wait_cap_seconds`. The exact success code is not declared in the SDK; accept any 2xx (UNCONFIRMED).

What the SDK does with it (`v2/daytona.py:793-861`):

1. It keeps `id`, `access_token`, `toolbox_proxy_url` and `status`.
2. If `status` is already `running`, it returns.
3. Otherwise the server has held the request for up to `wait_cap_seconds`. The SDK waits for the remainder of its budget (`wait_seconds - wait_cap_seconds`) with the long poll in section 4. `wait_cap_seconds` is 0 when the server skipped the hold because the organization already has its limit of held creates.
4. The POST's own HTTP timeout is `max(60, wait_seconds + 5)` seconds.
5. A create timeout or connection loss raises an error that carries the idempotency key. Anything after the POST never raises; the handle comes back and the caller must check its state (`METADATA:702-715`).
6. The response header `x-daytona-cell` names the regional endpoint; the SDK uses it only for its events socket (`v2/_events.py:381-383`).

### Hard lifetime and auto-delete (v2 `lifecycle_policy`)

Fields (`v2/_core.py:736-855`, `v2/types.py:263-302`, `v2/_gen/api/models/update_sandbox_lifecycle_policy_request.py:39-42`):

| Field | Meaning |
|---|---|
| `timeout_seconds` | Wall-clock lifetime, at least 1. It cannot be disabled. It re-arms after every transition to running (start, resume). This replaces `ttlMinutes` |
| `on_timeout` | `delete`, `stop` or `pause`. **Linux allows only `delete`** (the SDK refuses the others before sending) |
| `idle_expiry_seconds` | Idle seconds before `on_idle_timeout` fires. Below 60 is raised to 60. `-1` disables. `0` is refused |
| `on_idle_timeout` | Required together with an armed `idle_expiry_seconds`; same actions |

- Linux allows only `delete` (`v2/_core.py:878-889`). There is no "stop then delete" chain, and no separate auto-stop or auto-delete interval. Linux sandboxes cannot be stopped at all in v2; the README says only persistent macOS sandboxes can (`v2/sandbox.py:1338-1340`).
- Fields you omit at a Linux create are inherited from the project's lifecycle defaults, then (for `timeout_seconds` only) from a platform default (`v2/types.py:291-299`). So send `"idle_expiry_seconds": -1` to opt out of an inherited idle timeout. Whether "activity" counts browser traffic to a preview port is UNCONFIRMED; the model says activity can be "a running command, or sandbox traffic" (`v2/_gen/api/models/lifecycle_policy.py:42`).
- An organization or platform cap may shorten a requested timeout (`timeout_capped` in `v2/_gen/api/models/effective_lifecycle_policy.py:42`). Read `lifecycle_policy.timeout_at` back from GET to see the real deadline.
- `on_timeout` omitted gets the server's class default, not the project's (`v2/types.py:285-286,295-297`). Always send `"on_timeout": "delete"`.

Mapping from our v0 body:

| v0 (our client) | v2 |
|---|---|
| `ttlMinutes: N` | `lifecycle_policy.timeout_seconds: N*60`, `on_timeout: "delete"` |
| `autoStopInterval: 0` | `lifecycle_policy.idle_expiry_seconds: -1` |
| `autoDeleteInterval: 0` | Nothing. Linux sandboxes are deleted, never stopped (INFERRED) |
| `public: false` | Nothing at create. Ports are private unless exposed with `public: true` (section 8) |
| `snapshot: "x"` | `image: "x"` (a v0 snapshot is a v2 image; INFERRED from both being the boot template) |
| `env: {}`, `volumes: []` | Leave out |
| `target: "eu"` | `region_id: "eu"` plus header `x-daytona-region: eu` |

Change the deadline later: `PATCH $API2/sandboxes/$SBX/lifecycle-policy` with any subset of the four fields. `timeout_seconds` re-anchors from now. Returns `{"id","lifecycle_policy":{...,"timeout_at","idle_timeout_at","last_activity_at"}}` (`v2/sandbox.py:1537-1593`). Setting a `delete` action needs the key to hold `delete:sandboxes` as well as `write:sandboxes`.

Keep-alive: `POST $API2/sandboxes/$SBX/reset-idle-timer`, no body, same response shape. It does not move `timeout_at` (`v2/sandbox.py:1595-1632`).

### Images instead of snapshots

- `create("name")` takes a registered image name or `img_…` id (`v2/daytona.py:293-302`). Platform images can be used by name; omit `image` for the default (`METADATA:298-304`).
- To use a registry image, import it once: `POST $API2/images` with `{"source": {"type": "registry", "ref": "python:3.12"}, "name": "py312"}` (plus optional `region_id`, `cpu_arch`), then wait with `GET $API2/images/{id}?wait=ready` (`v2/services.py:304-339,341-371`, `v2/image.py:228-231`). Building from a Dockerfile is the same route with a different source. A name already used for different content is 409 `IMAGE_NAME_TAKEN`.
- Image names may not start with `img_`, `snap_`, `sbx_`, `vol_` or `fn_` (`METADATA:127-129`).
- v2 "snapshots" are memory snapshots of a sandbox (`sandbox.create_snapshot`). Creating from one is `POST $API2/sandboxes/from-snapshot` with `{"snapshot": "<name or snap_ id>"}` plus only `name`, `labels`, `lifecycle_policy`, `ports`, `region_id`, `wait_seconds`, `create_timeout_seconds`. It takes no env, resources, volumes, secrets or network policy (`v2/daytona.py:422-469,1742-1858`, `METADATA:720-722`).
- A Linux create of a missing default image answers 400 `DEFAULT_IMAGE_NOT_CONFIGURED`. A region that cannot take creates answers 409 `REGION_UNAVAILABLE` (`METADATA:304,321-322`).

## 4. Wait, read state, delete, confirm gone, list by label

### Read one sandbox

```bash
curl -sS "$API2/sandboxes/$SBX" "${H[@]}"
```

`GET $API2/sandboxes/{id}` (`v2/daytona.py:896`). Response fields that matter (`v2/_gen/api/models/sandbox.py:45-80`):

```json
{
  "id": "sbx_…", "name": "…", "class": "linux", "status": "running",
  "error": null, "region_id": "…", "labels": {"app": "duet"},
  "toolbox_proxy_url": "https://…",
  "lifecycle_policy": {"timeout_seconds": 1800, "on_timeout": "delete", "timeout_at": "…",
                       "idle_expiry_seconds": null, "idle_timeout_at": null, "last_activity_at": "…"},
  "image": {"id": "img_…", "name": "…"}, "cpu": 1, "cores_max": 2, "memory_mib": 1024, "disk_mib": 3072,
  "port_map": [], "created_at": "…", "started_at": "…", "updated_at": "…"
}
```

- Required: `class`, `cores_max`, `cpu`, `disk_mib`, `id`, `memory_mib`, `name`, `port_map`, `region_id`, `status`.
- The failure text is `error` (was `errorReason`). The SDK also accepts `state` as a fallback name for `status` (`v2/sandbox.py:2133`).
- Labels may be absent; treat that as `{}`.

`status` values and what they mean (`v2/_gen/api/models/sandbox_status.py:29-41`, `v2/_core.py:2423-2439`):

| Wire `status` | SDK state | For us |
|---|---|---|
| `queued` | queued | keep waiting |
| `pending`, `assigned` | creating | keep waiting |
| `booting` | starting | keep waiting |
| `running` | started | ready |
| `pausing`, `paused`, `stopping`, `stopped` | same | not ready |
| `deleting` | destroying | terminal |
| `deleted` | destroyed | terminal |
| `failed`, `lost` | error | terminal, read `error` |

Map unknown strings to "unknown" and keep waiting, as the SDK does.

### Wait until started (long poll)

```bash
curl -sS --get "$API2/sandboxes/$SBX" "${H[@]}" \
  --data-urlencode 'wait=running' --data-urlencode 'timeout_ms=30000' --max-time 35
```

- The SDK reads `GET /sandboxes/{id}?wait=<wire status>&timeout_ms=<ms>` in slices of at most 30 000 ms, with the HTTP read bounded by the slice plus 5 s (`v2/sandbox.py:147,885-936,1182-1249`).
- The server answers early when the status matches (INFERRED from the parameter names; how the server treats terminal states during the hold is UNCONFIRMED). After each answer the SDK checks: target reached, return; a terminal state (`deleting`, `deleted`, `failed`, `lost`), fail; else ask again. The minimum spacing between reads is 0.2 s (`v2/_wait.py:6`).
- Wait-state names on the wire: `running`, `booting`, `queued`, `paused`, `stopped`, `deleting`, `deleted` (`v2/sandbox.py:2447-2462`).
- Default budget 60 s. A 404 during a non-delete wait is "not found", not success (`METADATA:1219-1221`).
- Optional push channel: the SDK opens one WebSocket per regional endpoint. It mints a ticket with `POST $API2/events/{region}/ticket` (answer `{"ticket","subprotocol"}`), connects to `wss://<API host><API path>/events/{region}` with the ticket in the subprotocol list, then sends `{"type":"subscribe","ids":[{"id","known"}]}` and receives `{"type":"event"|"snapshot","id","version","status"}` frames (`v2/_events.py:138-150,1157-1177,1250-1290,1545-1566`). A 404 on the ticket means "use the long poll". A plain client can ignore all of this and poll.

### Delete

```bash
curl -sS -X DELETE "$API2/sandboxes/$SBX" "${H[@]}"
```

- `DELETE $API2/sandboxes/{id}`, no body. The SDK ignores the response body (`v2/sandbox.py:1268-1271,1530`).
- It then waits with `GET /sandboxes/{id}?wait_exact=deleted&timeout_ms=<ms>`. The exact form is used because the loose `wait=deleted` already answers on `deleting` (`v2/_events.py:240-258`, `v2/sandbox.py:1013-1017`).
- Gone means `status == "deleted"` or a 404 (`v2/sandbox.py:1517-1535`).
- `failed` or `lost` during the delete wait is an error. A sandbox with volumes can stay `deleting` for minutes and may report `error`, then still reach `deleted` later (`METADATA:1206-1216`).
- What DELETE answers for a sandbox that is already gone (404? 409?) is UNCONFIRMED. Treat 404 as done, as now.

### List by label

```bash
curl -sS --get "$API2/sandboxes/search" "${H[@]}" \
  --data-urlencode 'labels={"app":"duet","lobby":"lobby-1"}' \
  --data-urlencode 'limit=100'
# next page: add  --data-urlencode "cursor=$NEXT_CURSOR"
```

- `GET $API2/sandboxes/search` (`v2/daytona.py:1042-1156`, `v2/_core.py:2316-2372`).
- `labels` is one query value holding compact JSON (`separators=(",", ":")`). It is a containment match: every pair must be present.
- Response: `{"sandboxes": [...], "next_cursor": "…" | null, "total": 12, "total_relation": "eq" | "gte"}`. All four keys are required by the SDK (`v2/_core.py:2375-2420`). Items have the same fields as a detail, plus `cursor` (`v2/_gen/api/models/sandbox_list_item.py`).
- The cursor is bound to the exact filter. Sending it with other filters is a 400.
- `limit` 1 to 200.
- Search is eventually consistent and spans every region. A sandbox created a moment ago may be missing.
- Other filters: `q`, `name_prefix`, `id_prefix`, `status` (comma-joined wire statuses, for example `status=pending,assigned,booting,running`), `class`, `region_ids`, `image_ref`, `image_ref_prefix`, `spot`, `forked_from`, `min_cores`/`max_cores`, `min_memory_mib`/`max_memory_mib`, `min_disk_mib`/`max_disk_mib`, `created_start_at`/`created_end_at`, `updated_start_at`/`updated_end_at`, `last_activity_start_at`/`last_activity_end_at` (RFC 3339), `sort`, `order` (`v2/_core.py:2118-2150`).
- `find_by_name` skips items whose status is `deleted` or `deleting` (`v2/daytona.py:934`). Do the same in the sweep, or filter with `status=`.
- Fallback: a deployment without search answers 501 `SEARCH_NOT_CONFIGURED` (or 404 on an older API); then the SDK uses `GET $API2/sandboxes` with the same `cursor`/`limit` and no filters. Where search exists, `GET /sandboxes` answers 410 `GONE` (`v2/services.py:472-502`, `v2/daytona.py:994-1040`). A 503 `SEARCH_UNAVAILABLE` is an error, not a fallback.

Change labels later: `PUT $API2/sandboxes/$SBX/labels` with `{"labels": {...}}`. It replaces the whole set; the reserved `name` label must be resent or it is dropped by the SDK's own logic. Returns `{"id","labels"}` (`v2/sandbox.py:815-853`).

## 5. Run a command with a timeout

`sandbox.exec()` is `process.run()` (`v2/sandbox.py:658-726`, `v2/process.py:685-809`). It is one toolbox resource, `/processes`.

```bash
curl -sS -X POST "$TOOLBOX/processes?waitMs=15000&logs=text" "${T[@]}" \
  -H "Content-Type: application/json" \
  -d '{"kind":"exec","shellCommand":"uname -m","stdin":"closed","timeoutMs":15000,"keepLogs":"on_exit_ttl"}'
```

Request (`v2/_core.py:1141-1252`, `v2/_gen/toolbox/models/create_process_request.py:42-59`, `create_process_query.py:38-39`):

| Where | Key | Notes |
|---|---|---|
| query | `waitMs` | How long the server holds the request. The SDK sends the caller's `timeout` in ms, or 60 000 when none is given (`v2/process.py:122`). `0` means hold until done |
| query | `logs` | `text` (or `base64`) inlines the first log page. Needs `waitMs` |
| body | `kind` | `exec` |
| body | `shellCommand` | Run by `bash` if the image has it, else `sh` (`v2/sandbox.py:680-684`). Or send `argv: [...]` for no shell |
| body | `stdin` | `closed` (the SDK always sends it for exec) |
| body | `timeoutMs` | Execution deadline; the sandbox kills the process tree when it expires. Omit for none. This is `execution_timeout` in the SDK |
| body | `keepLogs` | `on_exit_ttl` (default), `until_cleanup`, `none` (`none` is refused by `run`) |
| body | `cwd`, `env`, `user`, `shell`, `login`, `name`, `tag` | Optional. `cwd` defaults to the user's home when available |

- The SDK's `timeout` (caller wait) never kills anything; `execution_timeout` does. To mirror v0's `timeout`, send both `waitMs` and `timeoutMs` with the same value (INFERRED as the closest match).
- Success is 201 with a process record, also when the hold timed out with the process still running (`create_process_query.py:39`).

Response (`v2/_gen/toolbox/models/process_dto.py:45-75`):

```json
{
  "id": "…", "kind": "exec", "state": "terminal", "reason": "exited", "exitCode": 0,
  "stdin": "closed", "keepLogs": "on_exit_ttl", "cwd": "/home/daytona",
  "createdAt": "…", "startedAt": "…", "exitedAt": "…", "truncatedHead": false,
  "logs": {"frames": [{"channel": "stdout", "cursor": "…", "data": "x86_64\n", "timestamp": 1763100000000}],
           "nextCursor": "…", "eof": true, "truncatedHead": false}
}
```

- `state`: `starting`, `provisioning`, `running`, `terminal`.
- `reason`: `exited`, `signaled`, `timed_out`, `lost`, `failed`.
- `exitCode` is absent until terminal; `signal` is a name like `SIGKILL`.

How the SDK finishes the job (`v2/process.py:775-809`):

1. If `state == "terminal"` and `logs.eof` is true, the inline page is the whole output.
2. If not terminal, it long-polls `GET $TOOLBOX/processes/{id}/wait?timeoutMs=<remaining ms>`. Answer: `{"exitCode","reason","signal"}`. `timed_out` there can mean "the poll expired"; the SDK then reads `GET /processes/{id}` to tell (`v2/process.py:286-345`, `v2/_core.py:1517-1532`).
3. If the logs were not complete, it pages `GET $TOOLBOX/processes/{id}/logs?encoding=text&cursor=<nextCursor>` until `eof` is true, `nextCursor` stops changing, or a page is empty (`v2/process.py:160-172`).
4. stdout = frames with channel `stdout` or `pty`. stderr = channel `stderr`. "output" = `stdout` + `stderr` + `pty` frames in order (`v2/process.py:796-798`). v0's `result` was stdout only; v2 can give both, in order.

Other notes:

- A command that backgrounds a child holding stdout open returns after about 2 s of drain (`v2/sandbox.py:688-694`).
- The per-process log is bounded; very large output loses its oldest frames (`truncatedHead`).
- Toolbox calls on a stopped sandbox answer 409 `SANDBOX_NOT_RUNNING` (`METADATA:1253-1257`).

## 6. Upload and download files

All sandbox paths must be absolute; the SDK refuses relative ones before sending (`v2/_filesystem_inputs.py:11-15`, `METADATA:1002-1004`). The server answers 400 `INVALID_FILE_PATH` for relative or empty paths (`v2/_gen/toolbox/models/fs_path_query.py:38`).

### One file

```bash
curl -sS -X POST "$TOOLBOX/files/upload?path=/home/daytona/duet-room" "${T[@]}" \
  -F 'file=@./duet-room-x86_64;filename=duet-room'
```

- `POST $TOOLBOX/files/upload?path=<absolute path>[&atomic=true]`, multipart, one part named `file`. Its filename is the last path segment (`v2/_core.py:1912-1926`).
- This replaces v0's `files[0].path` form and the `/files/bulk-upload` single-file trick.
- `atomic=true` writes through a temp file and rename; it fails with `ATOMIC_UPLOAD_UNSUPPORTED` where rename is not atomic (`v2/_gen/toolbox/models/upload_query.py:37-38`).
- An existing destination that is not a regular file is 400 `INVALID_FILE_PATH`.
- Response model: `{"path","bytes"}` (`v2/_gen/toolbox/models/uploaded_file.py:38-39`). The SDK ignores it, so its presence on this route is INFERRED.
- Large files: `upload_file` streams with an explicit `Content-Length` and no total deadline (`v2/filesystem.py:443-495`). No size limit is stated (UNCONFIRMED).
- Whether the upload keeps an executable bit is UNCONFIRMED. Set it after upload: `POST $TOOLBOX/files/permissions?path=<abs>&mode=0755` (`v2/filesystem.py:694-712`), or `chmod` through exec as now.

### Several files

```bash
curl -sS -X POST "$TOOLBOX/files/bulk-upload" "${T[@]}" \
  -F 'files[0].path=/app/a.txt' -F 'files[0].file=@./a.txt;filename=a.txt' \
  -F 'files[1].path=/app/b.txt' -F 'files[1].file=@./b.txt;filename=b.txt'
```

- Field names are the same as v0, but the SDK now interleaves them per file (path 0, file 0, path 1, file 1). The path part has no filename, and the file part's filename is the basename, not the full path (`v2/_core.py:1929-1942`).
- Optional query `atomic=true`.
- Response `{"files": [{"path","bytes"}], "errors": ["…"]}`; both keys are required. A 2xx can still carry per-file `errors` (`v2/_core.py:1951-1961`, `v2/_gen/toolbox/models/bulk_upload_response.py:38-39`).

### Download

```bash
curl -sS "$TOOLBOX/files/download?path=/home/daytona/out.json" "${T[@]}" -o out.json
```

- `GET $TOOLBOX/files/download?path=<abs>` returns the raw bytes (`v2/_core.py:1908-1909`). The SDK caps buffered reads at 1 GiB by default.
- Several: `POST $TOOLBOX/files/bulk-download` with `{"paths": [...]}`, answered as multipart (`v2/_core.py:1945-1948`, `v2/filesystem.py:567-599`).
- Pre-signed links: `POST $API2/sandboxes/$SBX/files/sign` with `{"method":"GET"|"POST","path":"/abs","expires_at":"<RFC 3339>"}`, 1 s to 7 days (`v2/sandbox.py:2049-2068`, `METADATA:1008-1014`).

## 7. Start a long-running server and read its logs later

v0 used a session plus `runAsync`. In v2 a background server is a plain detached process (`v2/process.py:567-683`). Sessions still exist, but they are only for a persistent shell.

Start:

```bash
curl -sS -X POST "$TOOLBOX/processes" "${T[@]}" -H "Content-Type: application/json" \
  -d '{"kind":"exec","shellCommand":"/usr/local/bin/duet-room --port 3000",
       "env":{"DUET_ROOM_SECRET":"…"},"stdin":"closed","keepLogs":"until_cleanup",
       "name":"room","tag":"duet-room"}'
```

- `POST $TOOLBOX/processes` with no `waitMs` returns as soon as the process has started: 201 with a process record. Keep its `id`.
- Pass secrets in `env`, not in the command line. The process record echoes `shellCommand` back on every read and list, but it has no `env` field (`v2/_gen/toolbox/models/process_dto.py:45-75`). Our current v0 code puts `DUET_ROOM_SECRET=…` inside the command string.
- `keepLogs`: `on_exit_ttl` (kept for a while after exit, the default), `until_cleanup` (kept until the record is deleted), `none`. For a server whose crash we want to explain, `until_cleanup` is the safer choice (INFERRED).
- `name` is a live alias usable instead of the id in `/processes/{id}` (`v2/process.py:824-829`, `create_process_request.py:50`). `tag` groups processes for `GET /processes?tag=` and `POST /processes:kill` with `{"tag","timeoutMs"?}` (`v2/_core.py:1310-1388`).

Read logs later:

```bash
curl -sS --get "$TOOLBOX/processes/$PID/logs" "${T[@]}" --data-urlencode 'encoding=text'
```

- `GET $TOOLBOX/processes/{id}/logs?encoding=text[&cursor=…][&limit=…]` returns `{"frames":[{"channel","cursor","data","timestamp"}],"nextCursor","eof","truncatedHead","firstAvailableCursor"?}` (`v2/_core.py:1567-1615`, `v2/_gen/toolbox/models/process_logs_response.py:38-42`). Required: `frames`, `nextCursor`, `eof`, `truncatedHead`.
- Channels: `stdout`, `stderr`, `pty`, `setup`, and more (`log_frame_dto.py:37`). This replaces v0's `output`/`stdout`/`stderr` strings and its version-dependent format.
- Follow live: add `follow=true` with `Accept: text/event-stream` (and `Last-Event-ID: <cursor>` to resume). The answer is server-sent events of type `log`, `state`, `warning` and `eof` (`v2/process.py:364-402`).

Other calls on the same record:

- Status: `GET $TOOLBOX/processes/{id}`. `state` and, once terminal, `exitCode`/`reason`.
- Wait for exit: `GET $TOOLBOX/processes/{id}/wait?timeoutMs=<ms>`.
- Signal: `POST $TOOLBOX/processes/{id}/signals` (`v2/_core.py:1840-1857`). On Linux with process containment this also stops detached children (`METADATA:993-998`).
- Delete record and logs, killing first: `DELETE $TOOLBOX/processes/{id}`.
- List: `GET $TOOLBOX/processes?state=&kind=&sessionId=&name=&pid=&tag=`.

Sessions, if wanted: create with `POST /processes` and `{"kind":"session","cwd"?,"env"?,"shell"?}`. Then run commands with `{"kind":"exec","shellCommand":"…","sessionId":"<session id>"}`. Delete with `DELETE /processes/{session id}` (`v2/process.py:532-556`, `v2/session.py`).

## 8. Make a port reachable from a browser

All preview routes are on the control plane, under `$API2/sandboxes/$SBX/ports` (`v2/ports.py:369-378`). Ports 2280, 2281 and 22220 belong to the sandbox daemon and are refused (`METADATA:1131-1134`).

### Signed URL (no headers needed)

```bash
curl -sS -X POST "$API2/sandboxes/$SBX/ports/3000/signed-preview-urls" "${H[@]}" \
  -H "Content-Type: application/json" -d '{"expires_in_seconds": 3600}'
```

```json
{"url": "https://3000-<token>.<domain>/", "token": "…", "expires_at": "2026-11-15T10:00:00Z"}
```

- It is a POST with a JSON body now. v0 used `GET …/signed-preview-url?expiresInSeconds=` (`v2/ports.py:267-314`).
- `expires_in_seconds`: default 60. A value below 1, or above the deployment ceiling (7 days by default), is 400 (`v2/_gen/api/models/create_sandbox_signed_preview_url_request.py:37`).
- The response now has `expires_at`. All three keys are required (`v2/_gen/api/models/signed_preview_url.py:74-76`).
- The token sits in the hostname: `<scheme>://<port>-<token>.<domain>/`.
- Whether the port must first be exposed (`PUT …/ports/{port}`) before signing is UNCONFIRMED. The SDK does not expose first here, unlike `preview_link`.
- 409 `CONFLICT` for a sandbox that is deleted, being deleted, failed, lost, or stopped without being persistent.
- **Revoke is all or nothing.** `POST $API2/sandboxes/$SBX/signed-preview-urls/revoke` expires every signed link of the sandbox on every port (`v2/ports.py:316-332`). The v0 route that expired one token (`…/signed-preview-url/{token}/expire`) has no v2 equivalent in the SDK. Our "one link each, so one can be withdrawn" idea no longer works with signed links.

### Standard URL plus token

1. `GET $API2/sandboxes/$SBX/ports/3000` returns `{"port","public","url","created_at"}` (`v2/_gen/api/models/sandbox_port.py`). On 404 the SDK exposes the port privately: `PUT $API2/sandboxes/$SBX/ports/3000` with `{"public": false}` (`v2/ports.py:186-246`).
2. If `public` is true, the URL needs nothing.
3. Otherwise `POST $API2/sandboxes/$SBX/ports/3000/preview-url` (no body) returns `{"url": "https://3000-<sandbox_id>.<domain>/", "token": "…"}` (`v2/ports.py:249-252`, `v2/_gen/api/models/preview_url.py:73-74`).
4. The token goes in the `x-daytona-preview-token` header. A sandbox access token also works as `Authorization: Bearer` or as a `daytona_token` query parameter; an opaque preview token works only in the header (`preview_url.py:73`).
5. The SDK then mints a token scoped to that one port and prefers it: `POST $API2/sandboxes/$SBX/access-token` with `{"operations": [], "ports": [3000]}` (`v2/sandbox.py:2031-2047`).

Other preview calls:

- List exposed ports: `GET $API2/sandboxes/$SBX/ports?cursor=&limit=` returns `{"ports":[...],"next_cursor"}`.
- Unexpose: `DELETE $API2/sandboxes/$SBX/ports/{port}`. This still works while the sandbox is being deleted.
- Make public: `PUT …/ports/{port}` with `{"public": true}`. It takes effect at once on event delivery, at most 30 s if an event is missed (`METADATA:1135-1136`).
- Per-person scoped token: `POST …/access-token` with `{"operations": [], "ports": [3000], "ttl_seconds": 3600}`. `ttl_seconds` defaults to 3600 and is capped by the deployment (86 400 by default). The token cannot be refreshed or revoked in the SDK (`v2/sandbox.py:1826-1874`, `v2/_gen/api/models/mint_sandbox_access_token_request.py:38-40`). A browser could use `https://3000-$SBX.<domain>/?daytona_token=<token>`. Whether that survives page navigation and WebSocket connects is UNCONFIRMED.
- Is something listening? `GET $TOOLBOX/port/{port}/in-use` returns `{"isInUse": …}`. It works only for ports 3000 to 9999 (`v2/ports.py:138-151`). `GET $TOOLBOX/port` lists open ports.

### Warning page

- Previews can show a warning page first. Send `X-Daytona-Skip-Preview-Warning: true` to skip it from a headless or non-interactive client (`v2/ports.py:38-41`, `METADATA:1137-1139`).
- An organization can turn the page off for every preview with a `disabled: true` setting. That needs the `preview_warning_opt_out` entitlement "where enforcement is on" (`v2/_gen/api/models/update_preview_warning_request.py:37`, `preview_warning.py:37`). The SDK has no route for it; the path is UNCONFIRMED. It is probably a dashboard setting (INFERRED).
- A browser cannot add that header on navigation, so the two people will likely see the warning page once unless the organization has opted out (INFERRED). Whether it also appears on signed URLs is UNCONFIRMED.
- **Our server-side calls** to `/healthz`, `/internal/arm`, `/internal/status`, `/internal/export` and `/internal/end` should send that header. Without it, a warning page that answers 200 could make the health check pass while the room is not up (INFERRED risk). Whether v0 previews honour the same header is UNCONFIRMED; the v0 SDK source never mentions it.

### WebSockets

Nothing in the v2 SDK says whether preview URLs pass WebSocket upgrades. UNCONFIRMED. The only WebSocket the v2 SDK opens is its own events socket on the API host. With a signed URL the token is in the hostname, so a WebSocket to the same origin carries it without extra work (INFERRED). With a standard URL a browser WebSocket cannot set `x-daytona-preview-token`, so it would need `daytona_token` in the query (INFERRED).

## 9. Other v2 features that fit "one private, short-lived room per pair"

- **Hard lifetime that deletes**: `lifecycle_policy` with `on_timeout: "delete"` (section 3), extended live with `PATCH …/lifecycle-policy`. `create_timeout_seconds` also deletes a sandbox that never reaches `running`.
- **Egress lock-down at create**: `network_policy: {"block_all": true}` blocks all outbound traffic except platform names. Or allow-list hosts: `{"allow_domains": ["api.example.com"]}`, which implies `block_all: true`. `allow_cidrs` alone does nothing and the SDK refuses it. CIDR-only mode is not a DNS exfiltration boundary (`v2/network_policy.py:18-76`, `METADATA:486-509`, `v2/_gen/api/models/set_sandbox_network_policy_request.py:37-39`). Read or replace later: `GET`/`PUT $API2/sandboxes/$SBX/network-policy` with `If-Match: <etag>`; a reset is a PUT with JSON `null` (`v2/sandbox.py:1634-1660`). The response shows desired versus applied state. If the policy cannot be enforced the API answers 503 (`v2/daytona.py:532-535`).
- **Secrets that never enter the sandbox**: `PUT $API2/secrets/{name}` with `{"value","allowed_domains":[...],"description"?}` (add `If-None-Match: *` to create only). Attach at create with `"secrets": ["name"]`. Values are injected only into outbound HTTPS requests to the allowed domains (`v2/services.py:1140-1224`, `v2/daytona.py:528-530`). Per sandbox: `GET`/`PUT $API2/sandboxes/$SBX/secrets` with `{"secrets":[...]}` and `If-Match` (`v2/sandbox.py:1662-1691`).
- **Scoped, short-lived credentials**: per-room or per-person access tokens limited to one port and no toolbox access (section 8). This is a good story for "private room".
- **Region pinning**: `region_id` plus `x-daytona-region`. Regions are listed only by the CLI (`daytona region list`); the SDK has no route (`METADATA:306-323`).
- **Labels and search**: `app`/`lobby`/`room` labels, `GET /sandboxes/search?labels=` to sweep, `name` for get-or-create per room id.
- **Events**: the live status socket (section 4) for instant "room ready" and "room gone" without polling.
- **Read-only checks for `duet doctor`**: `GET $API2/api-keys/current` (key, project, permissions), `GET $API2/usage/budget` (needs `read:spending`), `GET $API2/orgs/{org_id}/limits` (needs `read:limits`) (`v2/daytona.py:941-979`). An organization at its sandbox limit gets `LimitExceededError`, a 429 with `limit`, `current`, `max` (`METADATA:1279-1283`).
- **Diagnostics**: `GET $API2/sandboxes/$SBX/logs` (entrypoint log as text), `GET $TOOLBOX/system/metrics` (CPU, memory, disk) (`v2/sandbox.py:652-656,855-862`).

## 10. Errors and retries

Error body (`v2/_gen/toolbox/models/error_envelope.py:37-40`, `v2/_core.py:177-211,348-375`):

```json
{"code": "SANDBOX_NOT_RUNNING", "message": "…", "source": "DAYTONA_API", "trace_id": "…"}
```

- The SDK reads `message`, else `error`, else `HTTP <status>`. It also reads `code`, `source` and `trace_id`, plus optional detail keys (`retry_after_seconds`, `retryAfterMs`, `limit`, `current`, `max`, `feature`, `region`, `requestId`).
- A body that is not JSON (by Content-Type) becomes the message.
- Our Rust `error_text` already reads `message`, then `error`, then `errors`. It works unchanged.

Codes worth handling (`v2/_codes.py`):

| Status | Code | Meaning for us |
|---|---|---|
| 400 | `DEFAULT_IMAGE_NOT_CONFIGURED`, `IDEMPOTENCY_KEY_REQUIRED`, `BAD_REQUEST` | bad create |
| 401 | `UNAUTHORIZED` | key wrong, or a toolbox token expired (re-mint once) |
| 403 | `FEATURE_NOT_ENABLED` | Linux sandboxes not enabled for the organization |
| 409 | `SANDBOX_NOT_RUNNING` | toolbox or preview call on a stopped sandbox |
| 409 | `REGION_UNAVAILABLE`, `CONFLICT` | region closed; sandbox deleted or being deleted |
| 409 | (any) | same idempotency key with a different body |
| 429 | (limit) | concurrency or other organization limit |
| 501 / 503 | `SEARCH_NOT_CONFIGURED` / `SEARCH_UNAVAILABLE` | fall back to `GET /sandboxes`, or retry later |

Retries:

- The SDK has no general retry. It retries once after a 401 with a fresh token (`v2/_transport.py:717-730,788`).
- A create that timed out or lost its connection must be retried with the same `Idempotency-Key` and the same body. Never create again with a fresh key to recover a lost response (`METADATA:325-336`).
- Process waits retry transport failures a few times (`v2/process.py:333-339`).

## 11. The v0 path inside 2.0.0

- `Daytona().v0` is the 0.231.1 SDK, bundled from `daytona/clients@v0.231.1` (`v0/_bundle.py:3-4`), and needs the `v0-sdk` extra, which pins every generated client at `==0.231.1` (`METADATA:41-46`).
- Its URL is always `https://app.daytona.io/api`, or `DAYTONA_V0_API_URL` for development; the root passes it explicitly, so `DAYTONA_API_URL` and `.env` lookups never win (`_client.py:53-96,387-389`).
- Its headers are the 0.231.1 ones: `X-Daytona-Source: sdk-python`, `X-Daytona-SDK-Version: 0.231.1`, `User-Agent: sdk-python/0.231.1` (`v0/_sync/daytona.py:13,227,245-246`).
- The hand-written files line up with the old spec's citations line for line (for example `v0/_sync/sandbox.py:1025` is still `create_signed_preview_url`, `v0/_sync/process.py:84` is still `exec`, `v0/_sync/filesystem.py:866` is still `upload_file`). So `docs/daytona-rest-spec.md` still describes what v0 sends.
- Conclusion: the 2.0.0 release changes nothing for our existing Rust client, which never used the Python package. It keeps working as long as the v0 server does (INFERRED; the v0 server's lifetime is UNCONFIRMED).

## 12. Our calls, v0 against v2

| `daytona.rs` | v0 request (today) | v2 request |
|---|---|---|
| `create` | `POST $API/sandbox`, camelCase, `ttlMinutes`, `autoStopInterval`, `autoDeleteInterval`, `snapshot`, `target` | `POST $API2/sandboxes`, snake_case, `image`, `labels`, `lifecycle_policy`, `region_id`, `wait_seconds`, header `Idempotency-Key` |
| `sandbox()` parse | `id`, `state`, `errorReason`, `toolboxProxyUrl` | `id`, `status` (new values), `error`, `toolbox_proxy_url` (+ `{sandbox_id}` template rule); create also gives `access_token` |
| `get` | `GET $API/sandbox/{id}` | `GET $API2/sandboxes/{id}` (optionally `?wait=running&timeout_ms=30000`) |
| `wait_started` | poll, `started` / `error`, `build_failed`, `destroyed` | long poll; ready `running`; fail on `failed`, `lost`, `deleting`, `deleted` |
| toolbox URL fallback | `GET …/toolbox-proxy-url` | not needed: `toolbox_proxy_url` is required in the create response |
| `delete` | `DELETE $API/sandbox/{id}` | `DELETE $API2/sandboxes/{id}` |
| `wait_gone` | 404 or `destroyed` | `?wait_exact=deleted`, 404 or `deleted` |
| `exec` | `POST $TOOLBOX/process/execute` `{command, timeout}` → `{exitCode, result}` | `POST $TOOLBOX/processes?waitMs=&logs=text` `{kind:"exec", shellCommand, stdin:"closed", timeoutMs}` → `exitCode` + `logs.frames`; maybe `/wait` and `/logs` |
| `upload` | `POST $TOOLBOX/files/bulk-upload` with `files[0].path`/`files[0].file` | `POST $TOOLBOX/files/upload?path=<abs>`, part `file` |
| `start` | `POST $TOOLBOX/process/session`, then `…/session/{s}/exec` `{runAsync:true}` → `cmdId` | `POST $TOOLBOX/processes` `{kind:"exec", shellCommand, env, keepLogs}` → `id` |
| `logs` | `GET …/session/{s}/command/{c}/logs` → `output` | `GET $TOOLBOX/processes/{id}/logs?encoding=text` → join `frames[].data` |
| `signed_url` | `GET …/ports/{p}/signed-preview-url?expiresInSeconds=` | `POST $API2/sandboxes/{id}/ports/{p}/signed-preview-urls` `{expires_in_seconds}` |
| `list` | `GET $API/sandbox?labels=&limit=` → `items`, `nextCursor` | `GET $API2/sandboxes/search?labels=&limit=` → `sandboxes`, `next_cursor` |
| auth on toolbox | `$KEY` | `access_token` from create (mint on 401; `$KEY` as fallback, UNCONFIRMED) |

Smallest set of changes to support v2 next to v0 (INFERRED from the table):

1. A mode switch (`v0`/`v2`) that picks the base URL (`https://app.daytona.io/api` or `https://api.daytona.com`) and the path set.
2. A v2 create body and the `Idempotency-Key` header (reuse the room id, for example `room:<id>`, so a retry cannot make two rooms).
3. A status mapper: `running` = ready, `failed`/`lost`/`deleting`/`deleted` = terminal, `deleted` or 404 = gone. Read `status`, `error`, `toolbox_proxy_url`, with the `{sandbox_id}` template rule.
4. Keep the create response's `access_token` and use it as the toolbox bearer; on a 401 mint once with `POST …/access-token {}`.
5. `exec` and `start` on `POST /processes` (camelCase body), with `logs` read from `frames`.
6. `upload` on `POST /files/upload?path=`.
7. `signed_url` as a POST with a JSON body.
8. `list` on `/sandboxes/search`, reading `sandboxes` and `next_cursor`, skipping `deleting`/`deleted`.
9. For both modes: send `X-Daytona-Skip-Preview-Warning: true` on our own server-to-room requests.

## 13. Things that look unfinished or alpha-only

1. **Alpha label on a 2.0.0 release.** The classifier says Alpha and the README tells users to pin `X.Y.ZaN` prereleases, while the version is a plain `2.0.0` (`METADATA:11,70-87`). The README was not updated for a stable release.
2. **Per-organization gates.** Linux sandboxes need "Linux access enabled" (403 `FEATURE_NOT_ENABLED`, `v2/daytona.py:343-346`). The preview warning opt-out needs an entitlement.
3. **Fallback paths everywhere**, which suggests a service still rolling out (INFERRED):
   - search 501/404, then the plain list (`v2/services.py:472-502`);
   - the token mint route 404/405/501, then the API key (`v2/sandbox.py:2590-2605`);
   - the events ticket 404, then the long poll (`v2/_events.py:1157-1177`).
4. **Linux sandboxes cannot stop or pause.** Lifecycle actions other than `delete` are refused for Linux, and `stop()` works only for persistent macOS (`v2/_core.py:878-889`, `v2/sandbox.py:1338-1340`).
5. **No custom API URLs.** `DAYTONA_API_URL` is ignored by 2.x; only Daytona Cloud and its regional endpoints are supported (`METADATA:160-162,296`).
6. **Missing SDK routes.** There is no route for the preview-warning setting, no regions list, and no single-link revoke for signed preview URLs.
7. **Unknown key compatibility.** Nothing proves a v0 API key works against `api.daytona.com`. Probe with `GET /api-keys/current` first.
8. **Search limits.** It is eventually consistent, and `find_by_name` raises `SearchNotConfiguredError` on a deployment without search.
9. **Port checks.** `is_in_use` answers only for ports 3000 to 9999.

## Surprises and traps

1. The v2 host is `https://api.daytona.com` with no `/api`; v0 stays on `https://app.daytona.io/api`.
2. Control-plane JSON is snake_case, but toolbox process JSON is camelCase.
3. Sandbox states are renamed: `running`, not `started`; `failed`/`lost`, not `error`; `deleted`, not `destroyed`. The field is `status`, and the error text is `error`.
4. Toolbox calls should use the sandbox `access_token` from the create response, not the API key. The token is returned only once per create.
5. A Linux sandbox's only lifecycle action is `delete`. Always send `on_timeout: "delete"`, and send `idle_expiry_seconds: -1` to avoid an inherited idle timeout.
6. Exec has two timeouts: `waitMs` (how long the server holds the HTTP request) and `timeoutMs` (kills the process). Send both.
7. Upload takes the destination as `?path=` and needs an absolute path.
8. Signed preview URLs are a POST with `expires_in_seconds` in JSON, default 60 s, and revoke kills every link of the sandbox at once.
9. List is `/sandboxes/search` with `sandboxes` and `next_cursor`; the cursor is bound to the filter, and the plain `/sandboxes` list may answer 410.
10. Every create should carry `Idempotency-Key`. A retry with the same key and a different body is a 409.
11. A secret in `shellCommand` is echoed back by every process read; use `env`.
12. Our server-side requests to a preview URL may get a warning page instead of the room unless they send `X-Daytona-Skip-Preview-Warning: true`.
