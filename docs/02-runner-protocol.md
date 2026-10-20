# GitHub Actions Runner Wire Protocol (server-side spec for Reef)

**Runner source:** `actions/runner` tag **v2.337.0** (latest release at time of writing; resolved via `api.github.com/repos/actions/runner/releases/latest`).
**Reference server:** `ChristopherHX/runner.server`, branch `main` (used to confirm status codes, JSON shapes, encryption, and service-definition GUIDs).

This document describes exactly what the *unmodified* runner (`config.sh` + `run.sh`, listener + worker) sends and expects, so that Reef can host it without any runner patches. Everything not marked **inferred** was read directly from the C# source at the tag above; file names are given as `Runner.Listener/...`, `Sdk/...`, `Runner.Worker/...` relative to `src/`.

Conventions used below:

* `{tenant}` = the URL returned as `url` from the registration call (section 1). All `_apis/...` routes are resolved **relative to this URL** (`VssHttpUriUtility.ConcatUri`: `tenant.path.TrimEnd('/') + '/' + route.TrimStart('/')`), so if the tenant URL is `http://localhost:4920/owner/repo`, the pools route becomes `http://localhost:4920/owner/repo/_apis/...`.
* All VSS JSON is **camelCase** with enums serialized as **camelCase strings** (`VssJsonMediaTypeFormatter`: `VssCamelCasePropertyNamesContractResolver` + `StringEnumConverter(CamelCaseNamingStrategy)`). Deserialization is Newtonsoft and therefore case-insensitive for property names and accepts enums as strings *or* integers.
* Any response whose declared type is a list (`List<T>`, `IEnumerable<T>`) **must** be wrapped as `{"count": N, "value": [...]}` (`VssJsonCollectionWrapper`). Request bodies that are lists are wrapped the same way by the runner.
* `Byte[]` fields (RSA exponent/modulus, session key, message IV) are base64 strings. `Guid` fields are `"D"` format (`8-4-4-4-12`, lowercase accepted either case). `DateTime` fields are ISO-8601 (`2025-01-01T12:34:56.1234567Z`); `TimeSpan` is `d.hh:mm:ss` / `hh:mm:ss`.
* Success = any 2xx. Any non-2xx throws in the runner (`VssHttpClientBase.HandleResponseAsync`): if the body is `application/json` it is parsed as a `WrappedException` and re-thrown as the named exception type; otherwise as `VssServiceResponseException(status)`.

---

## 1. Registration (`config.sh --url <url> --token <token>`)

Source: `Runner.Listener/Configuration/ConfigurationManager.cs`, `Runner.Sdk/Util/UrlUtil.cs`, `Runner.Listener/Configuration/CredentialManager.cs` (`GitHubAuthResult`), `Runner.Common/RunnerDotcomServer.cs`.

### 1.1 URL derivation

`UrlUtil.IsHostedServer(uri)` is true when the host is `github.com`, `www.github.com`, `github.localhost`, ends with `.ghe.localhost` or `.ghe.com`. `GITHUB_ACTIONS_RUNNER_FORCE_GHES=1` forces false. Everything else (including `http://localhost:4920/owner/repo`) is treated as **GHES**.

| Purpose | github.com (hosted) | GHES-style (Reef) |
|---|---|---|
| Registration / removal | `{scheme}://api.{host}/actions/runner-registration` | `{scheme}://{host}/api/v3/actions/runner-registration` |
| PAT → registration token (`--pat`) | `{scheme}://api.{host}/repos/{owner}/{repo}/actions/runners/registration-token` | `{scheme}://{host}/api/v3/repos/{owner}/{repo}/actions/runners/registration-token` |
| PAT → removal token | `.../actions/runners/remove-token` | same, GHES prefix |

Path rules for the PAT flow: one path segment → `orgs/{org}`; two segments → `repos/{owner}/{repo}` unless the first segment is `enterprises` (then `enterprises/{name}` without `repos/`). Anything else: `ArgumentException`.

For `--url http://localhost:4920/owner/repo` the runner therefore calls `POST http://localhost:4920/api/v3/actions/runner-registration`.

### 1.2 `POST /api/v3/actions/runner-registration`

Request:

```
POST /api/v3/actions/runner-registration HTTP/1.1
Authorization: RemoteAuth <registration-token>
User-Agent: GitHubActionsRunner-linux-x64/2.337.0 (Linux 6.8.0 ...)
Content-Type: application/json; charset=utf-8

{"url":"http://localhost:4920/owner/repo","runner_event":"register"}
```

* `url` is the `--url` value verbatim. `runner_event` is `"register"` (config) or `"remove"` (`config.sh remove`).
* Retries: 3 attempts, random 1–5 s backoff; a **404 is not retried**. Any non-2xx eventually throws `HttpRequestException`.

Response (`GitHubAuthResult`, exact names, snake_case):

```json
{
  "url": "http://localhost:4920/owner/repo",
  "token": "<opaque or JWT bearer token, valid for the registration session>",
  "token_schema": "OAuthAccessToken",
  "use_v2_flow": false
}
```

* `token_schema` **must** be `OAuthAccessToken` (case-insensitive) or the runner throws `NotSupportedException`.
* `url` becomes `RunnerSettings.ServerUrl` (`{tenant}`). For GHES the runner enforces `UriUtility.IsSubdomainOf(tenant.Authority, githubUrl.Authority)` — the tenant host **must equal or be a subdomain of** the `--url` host (port included in Authority). runner.server returns `{scheme}://{host}[:port]/{owner}/{repo}` (or `/{owner}/server` for org URLs); Reef should do the same.
* `use_v2_flow: true` switches the runner to the "runner-admin" flow (`POST /api/v3/actions/runners/register`, `/runner-groups`, `/runners?name=`, broker). Return `false`.
* The `token` is used as `Authorization: Bearer <token>` (`VssOAuthAccessTokenCredential`) for every VSS call during configuration/removal. It is applied on the first request (no 401 dance). It only needs to live for the duration of `config.sh` (minutes).

### 1.3 Sequence of server calls during `config.sh`

1. `POST .../runner-registration` (above).
2. `RunnerServer.ConnectAsync(tenant, bearer)` opens **three** `VssConnection`s (generic 100 s, message 60 s, request 60 s timeouts). Each does `GET {tenant}/_apis/connectionData?connectOptions=1&lastChangeId=-1&lastChangeId64=-1` (section 2). Expect 3 identical GETs (5 attempts each with 100 ms delay on failure).
3. `GET {tenant}/_apis/.../pools?poolType=Automation` → list of `TaskAgentPool`. Pool selection: `defaultPool` = first pool with `isInternal:true`; if any pool has `isHosted:false` the runner prompts for `--runnergroup` (default = `defaultPool.name`) and matches by name (case-insensitive) among non-hosted pools; otherwise it uses `defaultPool`. No match → `TaskAgentPoolNotFoundException`. Return at least one pool with `isInternal:true, isHosted:false` (runner.server: `{id:1,name:"Agents",isHosted:false,isInternal:true}`).
4. RSA-2048 key pair generated and saved to `.credentials_rsaparams`.
5. `GET {tenant}/_apis/.../pools/0/agents?agentName=<name>` (poolId **0** = "all pools"). Response `{count, value:[TaskAgent...]}`.
   * If a match exists and `--replace`: `PUT .../pools/{poolId}/agents/{agentId}` with the merged agent; without `--replace` and `--unattended`: `TaskAgentExistsException`.
   * Otherwise `POST .../pools/{poolId}/agents` (section 5.2 for body).
6. Response validation of the created/replaced `TaskAgent`:
   * `authorization.clientId` must be a non-empty Guid and `authorization.authorizationUrl` an absolute URL, else `NotSupportedException("Message queue listen OAuth token.")`.
   * If `--disableupdate` was passed, `disableUpdate` in the response must be `true`; if `--ephemeral`, `ephemeral` must be `true` (else `NotSupportedException`). Echo them back.
   * `properties.RequireFipsCryptography` (bool) is read with **default `true`** → if you omit it the runner signs OAuth assertions with **PS256**. Send `"properties":{"RequireFipsCryptography":false}` to get RS256 (see 4.2). `properties.ServerUrlV2` / `properties.UseV2Flow` / `EnableAuthMigrationByDefault` / `AuthorizationUrlV2` switch the runner to the broker flow — do not send them.
   * `id` (ulong) is stored as `AgentId`.
7. `.credentials` saved: `{"scheme":"OAuth","data":{"clientId":"<guid>","authorizationUrl":"<url>","requireFipsCryptography":"False"}}`.
8. Connection test with the *OAuth* credential: `ConnectAsync` again (3 × `GET _apis/connectionData`, now via the 401 → token → retry handshake of section 4) followed by `GET .../pools?poolType=Automation`. If the token endpoint error message contains `Current server time is` the runner prints a clock-skew hint.
9. `.runner` saved (`agentId, agentName, poolId, poolName, serverUrl, gitHubUrl, workFolder, ephemeral, disableUpdate, isHostedServer:false, useV2Flow:false`).

### 1.4 Removal (`config.sh remove --token <token>`)

1. `POST /api/v3/actions/runner-registration` with `{"url": <gitHubUrl>, "runner_event": "remove"}` → same response shape.
2. `ConnectAsync` (3 × connectionData) with the returned bearer.
3. `GET .../pools/0/agents?agentName=<name>`; if found → `DELETE .../pools/0/agents/{agentId}` (poolId 0; "agentPool is ignored server side"). Any 2xx (runner.server returns 204).
4. Local `.credentials`, `.credentials_rsaparams`, `.runner` deleted.

---

## 2. `GET {tenant}/_apis/connectionData`

Source: `Sdk/WebApi/WebApi/HttpClients/LocationHttpClient.cs`, `Sdk/WebApi/WebApi/Location/ServerDataProvider.cs`, `LocationCacheManager.cs`, `LocationService.cs`, `Contracts/Location/*.cs`.

Request: `GET {tenant}/_apis/connectionData?connectOptions=1&lastChangeId=-1&lastChangeId64=-1`, `Accept: application/json`. `connectOptions` is `1` (`IncludeServices`) on the first call (the runner always ORs it in when it has no access mappings / `lastChangeId == -1`). Any query string already on the tenant URL is preserved.

Minimal response the runner accepts (this is what runner.server returns, minus its own extra definitions):

```json
{
  "instanceId": "7f2b1f0a-3d4e-4b6c-9a8f-0123456789ab",
  "deploymentId": "7f2b1f0a-3d4e-4b6c-9a8f-0123456789ab",
  "deploymentType": "onPremises",
  "authenticatedUser": {
    "id": "d2b3c4e5-1111-2222-3333-444455556666",
    "descriptor": { "identityType": "System:ServiceIdentity", "identifier": "runner-1" },
    "providerDisplayName": "runner-1",
    "customDisplayName": "runner-1",
    "isActive": true,
    "isContainer": false,
    "metaTypeId": 0,
    "resourceVersion": 2
  },
  "authorizedUser": { "...same shape as authenticatedUser..." : "" },
  "locationServiceData": {
    "serviceOwner": "00000000-0000-0000-0000-000000000000",
    "defaultAccessMappingMoniker": "PublicAccessMapping",
    "lastChangeId": 1,
    "lastChangeId64": 1,
    "clientCacheFresh": false,
    "clientCacheTimeToLive": 3600,
    "accessMappings": [
      { "moniker": "PublicAccessMapping", "displayName": "Public", "accessPoint": "http://localhost:4920/owner/repo", "virtualDirectory": "" , "serviceOwner": "00000000-0000-0000-0000-000000000000" }
    ],
    "serviceDefinitions": [
      {
        "serviceType": "distributedtask",
        "identifier": "a8c47e17-4d56-4a56-92bb-de7ea7dc65be",
        "displayName": "pools",
        "relativePath": "_apis/distributedtask/pools/{poolId}",
        "relativeToSetting": "context",
        "description": "Resource Location",
        "serviceOwner": "00000000-0000-0000-0000-000000000000",
        "status": "active",
        "toolId": "Framework",
        "locationMappings": [],
        "resourceVersion": 1,
        "minVersion": "1.0",
        "maxVersion": "12.0",
        "releasedVersion": "1.0"
      }
      // ... one entry per row of the table in section 3.2 ...
    ]
  },
  "webApplicationRelativeDirectory": ""
}
```

How the runner consumes it (`VssServerDataProvider.ConnectAsync`):

* `instanceId` — stored; also used as the on-disk location-cache directory name (`~/.local/share/Microsoft/VisualStudio Services/.../Cache/{instanceId}/LocationServiceData.config`, wrapped in try/catch). Use a **stable** value.
* `authenticatedUser` / `authorizedUser` — optional on the first connect. **If present once, they must be present with the same `descriptor` on every later connectionData call** on the same connection (`IdentityDescriptorComparer` → `VssAuthenticationException("CannotAuthenticateAsAnotherUser")`; missing → `NullReferenceException`). runner.server omits both; that is safe.
* `locationServiceData.serviceOwner` — if empty/omitted the runner substitutes `ServiceInstanceTypes.TFSOnPremises`, which makes `LocationService.GetLocationDataAsync(<any area id>)` return the *local* provider. **Leave it empty**; a non-empty value that differs from `instanceId` makes the runner look for a `LocationService2` service definition whose identifier is the client's area id (`a85b8835-c1a1-4aac-ae97-1c3d0ba72dbd`) and fail with `ServerDataProviderNotFound`.
* `accessMappings` — optional. If **non-empty**, `defaultAccessMappingMoniker` **must** name one of them (`m_accessMappings[defaultAccessMappingMoniker]` → `KeyNotFoundException` otherwise). If omitted, the runner synthesizes a client mapping from the tenant URL. Access mappings are only used to resolve `relativeToSetting:"fullyQualified"` definitions via `locationMappings`; the runner never needs that for its own clients because `LocationForCurrentConnection(LocationService2, SelfReferenceIdentifier)` short-circuits to the tenant URL.
* `lastChangeId` — stored only when services were included. If it stays `-1` (omitted → default 0 is fine; explicitly `-1` is not), every `GetResourceLocationsAsync` triggers another connectionData GET. Send a non-negative integer.
* `clientCacheTimeToLive` (seconds, default 3600) — after expiry the runner re-fetches connectionData on the next call that needs location data.
* `serviceDefinitions` with `resourceVersion > 0` are converted into the **resource-location table** (`ApiResourceLocation.FromServiceDefinition`: `id=identifier, area=serviceType, resourceName=displayName, routeTemplate=relativePath, resourceVersion, minVersion, maxVersion, releasedVersion`). When at least one such definition exists, the runner **never sends `OPTIONS`**.
  * `minVersion`, `maxVersion`, `releasedVersion` are strings `"major.minor"`. If a definition omits `maxVersion` the negotiation code dereferences `null` → crash. Always include all three plus `resourceVersion ≥ 1`.
* **If `serviceDefinitions` is empty or absent**: (a) because the runner just wrote a new server-mapping cache entry it immediately issues a **second** `GET _apis/connectionData?connectOptions=1&lastChangeId=-1&lastChangeId64=-1`; (b) `GetResourceLocationsAsync` returns `null`, so each `TaskAgentHttpClient`/`TaskHttpClient` falls back to `OPTIONS {tenant}/_apis/` (section 3.1) the first time it needs a location. Both designs work; runner.server ships definitions in connectionData and answers `OPTIONS _apis/` with an empty 200.

`relativeToSetting` values: `context`=0, `webApplication`=2, `fullyQualified`=3 (strings or ints). For resource locations the value is irrelevant — the runner only uses `relativePath` as a route template.

---

## 3. Resource-location discovery

Source: `Sdk/WebApi/WebApi/VssHttpClientBase.cs`, `VssApiResourceLocation.cs`, `VssApiResourceLocationCollection.cs`, `VssApiResourceVersion.cs`, `VssApiResourceVersionExtensions.cs`, `VssHttpUriUtility.cs`, `Sdk/DTWebApi/WebApi/TaskResourceIds.cs`, `Sdk/DTGenerated/Generated/TaskAgentHttpClientBase.cs`, `TaskHttpClientBase.cs`.

### 3.1 `OPTIONS {tenant}/_apis/`

Only sent when no `serviceDefinitions` with `resourceVersion > 0` were delivered in connectionData. Note the exact path: **`_apis/`** (constant `c_optionsRelativePath = "_apis/"`), not `_apis/{area}`. The `?allHostTypes=true` variant exists but is never used by the runner. Headers: the usual VSS headers (section 10) and `Authorization: Bearer` after the 401 challenge. Response must be `application/json`:

```json
{
  "count": 2,
  "value": [
    {
      "id": "a8c47e17-4d56-4a56-92bb-de7ea7dc65be",
      "area": "distributedtask",
      "resourceName": "pools",
      "routeTemplate": "_apis/{area}/{resource}/{poolId}",
      "resourceVersion": 1,
      "minVersion": "1.0",
      "maxVersion": "12.0",
      "releasedVersion": "1.0"
    },
    { "id": "e298ef32-5878-4cab-993c-043836571f42", "area": "distributedtask", "resourceName": "agents",
      "routeTemplate": "_apis/{area}/pools/{poolId}/{resource}/{agentId}", "resourceVersion": 1,
      "minVersion": "1.0", "maxVersion": "12.0", "releasedVersion": "1.0" }
  ]
}
```

Rules: `id` must be unique (duplicate ids with different content → `VssApiResourceDuplicateIdException`); missing/empty version strings default to `1.0`. A location that the runner needs but cannot find by `id` → `VssResourceNotFoundException(locationId)`.

### 3.2 Route-template filling and version negotiation

* `VssHttpUriUtility.ReplaceRouteValues`: placeholders `{name}` are replaced case-insensitively from the route values the client passes **plus** `{area}` = `location.area` and `{resource}` = `location.resourceName`. A placeholder whose value is `null`/absent is removed **together with its slash**, so one template `.../pools/{poolId}/agents/{agentId}` serves both the collection (`agentId` absent) and the item. Double `{{`/`}}` are literals. Query parameters are appended afterwards.
* Version negotiation (`NegotiateRequestVersion`): the client asks for `ApiResourceVersion(api, resourceVersion)` (per-call constants below). If `location.minVersion > api` → `VssVersionNotSupportedException`. If `location.maxVersion < api` → the request uses `maxVersion` with `-preview` when `releasedVersion < maxVersion`. Otherwise it uses `api`, `resourceVersion = min(requested, location.resourceVersion)`, and `-preview` if `releasedVersion < api` or the requested version was preview (any requested resourceVersion > 0 is preview). With the recommended `minVersion 1.0 / maxVersion 12.0 / releasedVersion 1.0 / resourceVersion 1` every request below becomes e.g. `6.0-preview.1`, `5.1-preview.1`, `2.0-preview.1`.
* The negotiated version is sent as **`Accept: application/json; api-version=6.0-preview.1`** and, when there is a body, also inside **`Content-Type: application/json; charset=utf-8; api-version=6.0-preview.1`** (`ObjectContent` default charset is utf-8). It is **never** a query parameter. If the API major version is ≤ 1 the legacy form `api-version=1.0; res-version=N` is used — avoid by keeping `minVersion` ≥ 1.0 and not forcing max below 2.0. The server should accept and ignore these parameters.

### 3.3 Complete table of locationIds used by the runner

`TaskAgentHttpClient` and `TaskHttpClient` both carry `[ResourceArea("A85B8835-C1A1-4AAC-AE97-1C3D0BA72DBD")]` (area name `distributedtask`); that GUID is *not* a resource location and never appears in a URL. Route templates in the table are the Azure-DevOps-style templates (**inferred** from the route-value names; the runner only requires that the placeholder names match). runner.server's concrete templates are listed for comparison. Versions are `ApiResourceVersion(api, res)` as requested by the runner.

| locationId | area / resourceName | Verbs used | Route values passed | Recommended routeTemplate | runner.server relativePath | Requested version |
|---|---|---|---|---|---|---|
| `a8c47e17-4d56-4a56-92bb-de7ea7dc65be` | distributedtask / pools | GET | (none) | `_apis/distributedtask/pools/{poolId}` | `/_apis/v1/AgentPools` | 5.1, res 1 |
| `e298ef32-5878-4cab-993c-043836571f42` | distributedtask / agents | GET (list & item), POST, PUT, PATCH¹, DELETE | poolId, agentId | `_apis/distributedtask/pools/{poolId}/agents/{agentId}` | `/_apis/v1/Agent/{poolId}/{agentId}` | 6.0, res 2 |
| `134e239e-2df3-4794-a6f6-24f1f19ec8dc` | distributedtask / sessions | POST, DELETE | poolId, sessionId | `_apis/distributedtask/pools/{poolId}/sessions/{sessionId}` | `/_apis/v1/AgentSession/{poolId}/{sessionId}` | 5.1, res 1 |
| `c3a054f6-7a8a-49c0-944e-3a8e5d7adfd7` | distributedtask / messages | GET, DELETE (POST¹) | poolId, messageId | `_apis/distributedtask/pools/{poolId}/messages/{messageId}` | `/_apis/v1/Message/{poolId}/{messageId}` | GET 6.0 res 1; DELETE 5.1 res 1 |
| `fc825784-c92a-4299-9221-998a02d1b54f` | distributedtask / jobrequests | GET, PATCH, DELETE¹ | poolId, requestId | `_apis/distributedtask/pools/{poolId}/jobrequests/{requestId}` | `/_apis/v1/AgentRequest/{poolId}/{requestId}` | 5.1, res 1 |
| `8ffcd551-079c-493a-9c02-54346299d144` | distributedtask / packages | GET (list & item) | packageType, platform, version | `_apis/distributedtask/packages/{packageType}/{platform}/{version}` | (not implemented) | 5.1, res 2 |
| `8cc1b02b-ae49-4516-b5ad-4f9b29967c30` | distributedtask / updates | PUT | poolId, agentId | `_apis/distributedtask/pools/{poolId}/agents/{agentId}/updates` | (not implemented) | 5.1, res 1 |
| `13b5d709-74aa-470b-a8e9-bf9f3ded3f18` | distributedtask / (runner config refresh)² | POST | agentId, configType | `_apis/distributedtask/runners/{agentId}/config/{configType}` (inferred) | (not implemented) | 6.0, res 1 |
| `83597576-cc2c-453c-bea6-2882ae6a1653` | distributedtask / timelines | POST, GET | scopeIdentifier, hubName, planId, timelineId | `{scopeIdentifier}/_apis/distributedtask/hubs/{hubName}/plans/{planId}/timelines/{timelineId}` | `/_apis/v1/Timeline/{scopeIdentifier}/{hubName}/{planId}/timeline/{timelineId}` | 5.1, res 1 |
| `8893bc5b-35b2-4be7-83cb-99e683551db4` | distributedtask / records | PATCH | scopeIdentifier, hubName, planId, timelineId | `{scopeIdentifier}/_apis/distributedtask/hubs/{hubName}/plans/{planId}/timelines/{timelineId}/records` | `/_apis/v1/Timeline/{scopeIdentifier}/{hubName}/{planId}/{timelineId}` | 5.1, res 1 |
| `858983e4-19bd-4c5e-864c-507b59b58b12` | distributedtask / feed | POST | scopeIdentifier, hubName, planId, timelineId, recordId | `{scopeIdentifier}/_apis/distributedtask/hubs/{hubName}/plans/{planId}/timelines/{timelineId}/records/{recordId}/feed` | `/_apis/v1/TimeLineWebConsoleLog/{scopeIdentifier}/{hubName}/{planId}/{timelineId}/{recordId}` | 5.1, res 1 |
| `46f5667d-263a-4684-91b1-dff7fdcf64e2` | distributedtask / logs | POST (create & append) | scopeIdentifier, hubName, planId, logId | `{scopeIdentifier}/_apis/distributedtask/hubs/{hubName}/plans/{planId}/logs/{logId}` | `/_apis/v1/Logfiles/{scopeIdentifier}/{hubName}/{planId}/{logId}` | 5.1, res 1 |
| `7898f959-9cdf-4096-b29e-7f293031629e` | distributedtask / attachments | PUT | scopeIdentifier, hubName, planId, timelineId, recordId, type, name | `{scopeIdentifier}/_apis/distributedtask/hubs/{hubName}/plans/{planId}/timelines/{timelineId}/records/{recordId}/attachments/{type}/{name}` | `/_apis/v1/Timeline/{scopeIdentifier}/{hubName}/{planId}/{timelineId}/attachments/{recordId}/{type}/{name}` | 5.1, res 1 |
| `557624af-b29e-4c20-8ab0-0399d2204f3f` | distributedtask / events | POST | scopeIdentifier, hubName, planId | `{scopeIdentifier}/_apis/distributedtask/hubs/{hubName}/plans/{planId}/events` | `/_apis/v1/FinishJob/{scopeIdentifier}/{hubName}/{planId}` | 2.0, res 1 |
| `27d7f831-88c1-4719-8ca1-6a061dad90eb` | distributedtask / actiondownloadinfo | POST | scopeIdentifier, hubName, planId (+ `?jobId=`) | `{scopeIdentifier}/_apis/distributedtask/hubs/{hubName}/plans/{planId}/actiondownloadinfo` | `/_apis/v1/ActionDownloadInfo/{scopeIdentifier}/{hubName}/{planId}` | 6.0, res 1 |

¹ Exists in the generated client but is never called by the runner (`UpdateAgentAsync` PATCH, `SendMessageAsync` POST, `DeleteAgentRequestAsync` DELETE). ² Only used when the server sends a `RunnerRefreshConfig` message (section 5.6); optional.

Not required: the Location area itself (`00d9565f-ed9c-4a06-9a50-00e7896ccab4` connectionData, `d810a47d-...` serviceDefinitions) — connectionData is fetched by fixed path and `GetServiceDefinitionAsync` is only reached when `serviceOwner` is the root identifier, which section 2 tells you to avoid. The route values `hubName` is filled from `plan.planType`, `scopeIdentifier` from `plan.scopeIdentifier`, `planId` from `plan.planId`, `timelineId` from `timeline.id`.

---

## 4. OAuth (listener credential)

Source: `Runner.Listener/Configuration/OAuthCredential.cs`, `RSAFileKeyManager.cs`, `Sdk/WebApi/WebApi/OAuth/*.cs`, `Sdk/WebApi/WebApi/Jwt/JsonWebToken.cs`, `VssSigningCredentials.cs`, `Sdk/Common/Common/VssHttpMessageHandler.cs`, `Authentication/VssCredentials.cs`, `IssuedTokenProvider.cs`.

### 4.1 Public-key registration

`TaskAgent.authorization.publicKey = { "exponent": base64(RSAParameters.Exponent), "modulus": base64(RSAParameters.Modulus) }` — raw big-endian unsigned integers as .NET exports them (exponent is typically `AQAB` = 65537, modulus 256 bytes for 2048-bit). Sent in the `POST/PUT agents` body; the server must store it against the `clientId` it assigns and return `authorization: { clientId, authorizationUrl, publicKey }`.

### 4.2 Token request

`OAuthCredential.GetVssCredentials`: `clientId`, `authorizationUrl` (and optional `oauthEndpointUrl`, defaults to `authorizationUrl`) from `.credentials`. It builds `VssOAuthJwtBearerClientCredential(clientId, audience = authorizationUrl, signingCredentials)` and `VssOAuthCredential(new Uri(oauthEndpointUrl), ClientCredentials grant, ...)`.

```
POST {authorizationUrl}
Accept: application/json
Content-Type: application/x-www-form-urlencoded
User-Agent: GitHubActionsRunner-linux-x64/2.337.0 (...)

grant_type=client_credentials
&client_assertion_type=urn%3Aietf%3Aparams%3Aoauth%3Aclient-assertion-type%3Ajwt-bearer
&client_assertion=<JWT>
```

JWT (`VssOAuthJwtBearerAssertion.GetBearerToken` → `JsonWebToken.Create`):

* Header: `{"typ":"JWT","alg":"RS256"}` — or `"alg":"PS256"` when `requireFipsCryptography` is true. `VssSigningCredentials.Create(rsa, requireFips)` picks `RSASignaturePadding.Pss` for FIPS, `Pkcs1` otherwise; hash is always SHA-256; key size must be ≥ 2048.
* Payload: `{"iss": "<clientId>", "sub": "<clientId>", "aud": "<authorizationUrl exactly as stored, AbsoluteUri>", "nbf": <unix now>, "exp": <unix now + 300>, "jti": "<new guid>"}`. No `iat`. Lifetime 5 minutes → server clock tolerance matters (runner reports skew if the error text contains `Current server time is`).
* Encoding: base64url without padding, standard `header.payload.signature`.

Because `RequireFipsCryptography` defaults to **true** when the agent JSON has no `properties.RequireFipsCryptography`, a server that only verifies RS256 **must** return `"properties": {"RequireFipsCryptography": false}` (a JSON boolean; `PropertiesCollection` also accepts `{"$type":"System.Boolean","$value":false}`) from `POST`/`PUT agents`, or verify both PS256 and RS256.

Response:

* `200` `{"access_token":"<token>","token_type":"Bearer","expires_in":3600,"scope":"..."}` (`VssOAuthTokenResponse`; `token_type`/`scope`/`refresh_token` are ignored, `expires_in` only sets an informational `ValidTo`).
* `400` with `application/json` `{"error":"invalid_client","error_description":"..."}` → `VssOAuthTokenRequestException`. `invalid_client` makes the listener exit (`CreateSessionResult.Failure`); `server_error` is retried; anything else is fatal for session creation.
* Any other status → `VssServiceResponseException` (retried with backoff during session creation).

The token endpoint is unauthenticated (no `Authorization` header). runner.server: `POST {tenant}/_apis/v1/auth/`, returns a 7-day RS256 JWT with claims `Agent=oauth`, `Agent=job`.

### 4.3 When the bearer is sent (401 challenge handshake)

`VssHttpMessageHandler.SendAsync` with `VssOAuthCredential`: the provider's `CurrentToken` is `null` initially, so **the first request on each connection is sent without `Authorization`**. The response is an authentication challenge when `VssOAuthCredential.IsAuthenticationChallenge` is true: status **302** or **401** *and* a `WWW-Authenticate` header containing `Bearer`. Then the runner POSTs the token endpoint, retries the request with `Authorization: Bearer <access_token>` (plus header `X-VSS-UserData:` empty), and caches the token on the connection. There is no proactive expiry check — the cached token is reused until the **next 401 challenge**, at which point it is invalidated and re-fetched (up to `m_maxAuthRetries` per request). A 401 that persists after a fresh token → `VssUnauthorizedException` → the listener stops (non-retriable).

Therefore the server **must** answer unauthenticated `_apis/...` requests with `401` + `WWW-Authenticate: Bearer` (a JSON body is optional). This applies to `GET _apis/connectionData`, `OPTIONS _apis/`, and every listener call. Registration-time calls (bearer from `runner-registration`) and worker calls (bearer from `SystemVssConnection.authorization.parameters.AccessToken`) use `VssOAuthAccessTokenCredential`, which applies the token immediately; a 401 there cannot be recovered.

Token lifetime recommendation: at least the maximum expected job duration plus polling interval (runner.server uses 7 days; GitHub uses ~1 h and relies on the 401 refresh). Both work.

---

## 5. Pools, agents, sessions, messages, job requests, updates

Source: `Runner.Common/RunnerServer.cs`, `Sdk/DTGenerated/Generated/TaskAgentHttpClientBase.cs`, `Sdk/DTWebApi/WebApi/TaskAgentHttpClient.cs`, `Runner.Listener/MessageListener.cs`, `Runner.cs`, `JobDispatcher.cs`, DTOs under `Sdk/DTWebApi/WebApi/`; runner.server `AgentController`, `AgentSessionController`, `MessageController`, `AgentRequestController`.

Common headers on every VSS request (see 10 for the full list): `Accept: application/json; api-version=X`, `X-TFS-FedAuthRedirect: Suppress`, `X-TFS-Session: <guid>`, `X-VSS-E2EID: <guid>`, `User-Agent: GitHubActionsRunner-linux-x64/2.337.0 (...) VSServices/... (NetStandard; ...)`.

### 5.1 Pools

`GET {tenant}/_apis/distributedtask/pools?poolType=Automation` (also `poolName=` when a name is given — the runner never does). Response:

```json
{ "count": 1, "value": [ { "id": 1, "name": "Default", "isHosted": false, "isInternal": true, "poolType": "automation", "size": 0, "scope": "00000000-0000-0000-0000-000000000000", "createdOn": "2025-01-01T00:00:00Z", "autoProvision": null, "autoSize": null, "targetSize": null, "agentCloudId": null } ] }
```

`TaskAgentPoolType` has the single member `automation = 1`. Only `id`, `name`, `isHosted`, `isInternal` matter (see 1.3 step 3).

### 5.2 Agents

`TaskAgent` (extends `TaskAgentReference`) JSON as the runner sends it on `POST .../pools/{poolId}/agents` (`api-version=6.0-preview.2`):

```json
{
  "id": 0,
  "name": "my-runner",
  "version": "2.337.0",
  "osDescription": "Linux 6.8.0-45-generic #45-Ubuntu SMP ...",
  "ephemeral": false,
  "disableUpdate": false,
  "status": 0,
  "provisioningState": null,
  "maxParallelism": 1,
  "createdOn": "0001-01-01T00:00:00",
  "authorization": { "publicKey": { "exponent": "AQAB", "modulus": "sZ1...base64...==" } },
  "labels": [
    { "id": 0, "name": "self-hosted", "type": "system" },
    { "id": 0, "name": "Linux", "type": "system" },
    { "id": 0, "name": "X64", "type": "system" },
    { "id": 0, "name": "gpu", "type": "user" }
  ]
}
```

Field notes: `id` is `ulong`; `status` is `TaskAgentStatus` (`offline=1, online=2, busy=3`; the default `0` serializes as the integer 0 — accept it); `labels` is a set of `AgentLabel {id:int, name, type: "system"|"user"}` (`LabelType System=0, User=1`); `--no-default-labels` omits the three system labels; `enabled`, `accessPoint`, `statusChangedOn`, `assignedRequest`, `lastCompletedRequest`, `_links`, `properties` are omitted when default. `PUT` (replace) sends the previously fetched agent merged with the new key/labels/version/ephemeral/disableUpdate and `maxParallelism:1`.

Response (`TaskAgent`) — required content:

```json
{
  "id": 7, "name": "my-runner", "version": "2.337.0", "osDescription": "...", "enabled": true,
  "ephemeral": false, "disableUpdate": false, "status": "offline", "provisioningState": "Provisioned",
  "maxParallelism": 1, "createdOn": "2025-01-01T00:00:00Z",
  "authorization": {
    "clientId": "b0d9e6c4-6b4a-4c2e-9d0a-55a1e2f3c4d5",
    "authorizationUrl": "http://localhost:4920/owner/repo/_apis/v1/auth/",
    "publicKey": { "exponent": "AQAB", "modulus": "..." }
  },
  "labels": [ ...echo... ],
  "properties": { "RequireFipsCryptography": false }
}
```

Other agent calls: `GET .../pools/{poolId}/agents?agentName=<name>` (poolId 0 = all pools; other optional query params `includeCapabilities`, `includeAssignedRequest`, `includeLastCompletedRequest`, `propertyFilters`, `demands` are never sent) → `{count,value:[TaskAgent]}`; `GET .../pools/{poolId}/agents/{agentId}` (unused by the runner); `DELETE .../pools/0/agents/{agentId}` → 2xx, empty body. Runner.server keys everything by `agentId` alone and treats `poolId` 0/-1 as wildcard.

### 5.3 Sessions

`POST {tenant}/_apis/distributedtask/pools/{poolId}/sessions` (`api-version=5.1-preview.1`), body `TaskAgentSession`:

```json
{
  "sessionId": "00000000-0000-0000-0000-000000000000",
  "ownerName": "myhost (PID: 12345)",
  "agent": { "id": 7, "name": "my-runner", "version": "2.337.0", "osDescription": "Linux ...", "status": 0 },
  "useFipsEncryption": false
}
```

Response:

```json
{
  "sessionId": "c1d2e3f4-1111-2222-3333-444455556666",
  "ownerName": "myhost (PID: 12345)",
  "agent": { "id": 7, "name": "my-runner", "version": "2.337.0", "osDescription": "..." , "status": 0 },
  "useFipsEncryption": true,
  "encryptionKey": { "encrypted": true, "value": "<base64 RSA-OAEP ciphertext of the 32-byte AES key>" }
}
```

* `sessionId` must be non-empty (used in every message call; `Guid.Empty` disables delete/message calls).
* `encryptionKey`: if omitted/empty, message bodies are sent in clear text (section 5.4). If present: `encrypted:true` → `value` is the AES key encrypted with the agent's RSA public key using OAEP-**SHA256** when `useFipsEncryption:true`, OAEP-**SHA1** otherwise (`MessageListener.GetMessageDecryptor`). `encrypted:false` → `value` is the raw AES key (base64). runner.server: `Aes.Create()` (256-bit key), `useFipsEncryption:true`, OAEP-SHA256.
* `brokerMigrationMessage` (optional) — do not send; it moves the runner to the broker protocol.
* Errors: a JSON `WrappedException` with `typeKey` `TaskAgentSessionConflictException` (runner retries every 30 s for up to 4 min, then exits with code `SessionConflict`); `TaskAgentNotFoundException`, `TaskAgentPoolNotFoundException`, `AccessDeniedException`, `VssUnauthorizedException` (401) → exit; `VssOAuthTokenRequestException` other than `server_error` → exit; anything else → retry every 30 s forever. Session creation is preceded by a fresh `ConnectAsync` (3 × connectionData) every time.
* `DELETE .../pools/{poolId}/sessions/{sessionId}` on shutdown (30 s timeout) → any 2xx; body ignored. Not sent after a 404/`TaskAgentNotFoundException` or after `RunnerShutdown`.

Session expiry on the server: runner.server drops a session that has not polled for 60 s. Expired session → respond to `GET messages` with `403` (any 4xx works) and body `WrappedException{typeKey:"TaskAgentSessionExpiredException"}` → the runner transparently creates a new session and continues.

### 5.4 Messages (long poll)

`GET {tenant}/_apis/distributedtask/pools/{poolId}/messages?sessionId={sid}&lastMessageId={n}&status=Online&runnerVersion=2.337.0&os=Linux&architecture=X64&disableUpdate=false` (`api-version=6.0-preview.1`).

* `lastMessageId` is omitted until the first message has been received; afterwards it is the `messageId` of the last message returned (**not** necessarily deleted). `status` is `Online` or `Busy` (PascalCase enum `ToString()`); a job start/finish cancels the in-flight poll and re-polls with the new status. `disableUpdate` is lowercase `true`/`false`. `os` is `VarUtil.OS` (`Linux`/`Windows`/`macOS`), `architecture` `X64`/`Arm64`/... .
* The message connection has a 60 s `SendTimeout`; runner.server long-polls for **50 s** then returns **`204 No Content`**. The runner treats an empty body (204, or 200 with `Content-Length: 0`) as "no message" and polls again immediately; after 50 consecutive empties (reset every 30 min) it sleeps 15–30 s between polls. A timeout/exception → 15–30 s (then 30–60 s) backoff plus `RefreshConnectionAsync` (new connection → new connectionData GET). **Do not use 202**; the runner has no special handling for it beyond "2xx with body → parse".
* Response `200` `application/json`:

```json
{ "messageId": 42, "messageType": "PipelineAgentJobRequest", "iv": "<base64 16 bytes>", "body": "<base64 ciphertext>" }
```

`TaskAgentMessage`: `messageId` (int64, `EmitDefaultValue=false`), `messageType` (string), `iv` (`Byte[]`, optional), `body` (string). Decryption (`MessageListener.DecryptMessage`) is **skipped** when the session has no `encryptionKey` (or empty value) **or** the message has no/empty `iv`; then `body` is the plain UTF-8 JSON. Otherwise: `Aes.Create()` (CBC, PKCS7) with the session key and `iv`, `body = base64(ciphertext)`, plaintext = UTF-8 JSON. runner.server generates a fresh IV per message (`session.Key.GenerateIV()`).

* After handling, the runner calls `DELETE {tenant}/_apis/distributedtask/pools/{poolId}/messages/{messageId}?sessionId={sid}` (`api-version=5.1-preview.1`, 30 s timeout, any 2xx; failures are logged and ignored). runner.server treats the delete as the job **acknowledgement**: a job whose message is not deleted before the next `GET` is re-queued (`DropMessage`). Deletion is skipped for a job message that arrives while a self-update or a run-once job is in progress (so the server re-delivers it), and for `RunnerShutdown`.
* A `TaskAgentSessionExpiredException` body on any error status → new session (unless `skipSessionRecover`). `TaskAgentNotFoundException`/`RunnerNotFoundException` → the runner deletes its local config and exits (used for ephemeral cleanup). `TaskAgentPoolNotFoundException`, `AccessDeniedException`, `VssUnauthorizedException` → exit; other errors retry with backoff.

Message types dispatched by `Runner.cs`:

| `messageType` | Body | Runner behaviour |
|---|---|---|
| `PipelineAgentJobRequest` | `AgentJobRequestMessage` (section 6) | Starts the worker; renews the job request every 60 s. |
| `JobCancellation` | `{"jobId":"<guid>","timeout":"00:05:00"}` (`JobCancelMessage`; `timeout` is a `TimeSpan`) | Cancels the running job with that id; the worker gets `timeout` to finish post steps, then the listener kills it. Returns `false` (message still deleted unless update/run-once in progress) if the job is unknown. |
| `AgentRefresh` | `{"agentId":7,"timeout":"01:00:00","targetVersion":"2.338.0"}` | Legacy self-update via `packages` (5.7). |
| `RunnerRefresh` | `{"target_version":"...","download_url":"...","sha256_checksum":"...","os":"linux-x64"}` | Broker-style self-update (direct download). |
| `ForceTokenRefresh` | (none) | Refreshes the message connection (new connectionData + token). |
| `RunnerRefreshConfig` | `{"runnerQualifiedId":"...","configType":"runner"|"credentials","serviceType":"pipelines","configRefreshURL":"..."}` | Calls the config-refresh endpoint (`13b5d709-...`); optional feature. |
| `BrokerMigration` | `{"brokerBaseUrl":"https://..."}` | Switches polling to the broker `GET {brokerBaseUrl}/message` — **do not send**. |
| `RunnerJobRequest` | `RunnerJobRequestRef {"id","runner_request_id","should_acknowledge","run_service_url","billing_owner_id"}` | Fetches the job from the Run Service / ActionsRunServer — **do not send**. |
| `RunnerShutdown` | `{"reason":"..."}` | Exits without deleting message or session (hosted runners). |

Unknown types are ignored (and deleted).

### 5.5 Job requests (renew / finish)

`PATCH {tenant}/_apis/distributedtask/pools/{poolId}/jobrequests/{requestId}?lockToken=00000000-0000-0000-0000-000000000000` (`api-version=5.1-preview.1`).

* `lockToken` is always `Guid.Empty` ("lockToken has never been used, keep this here of compat").
* Header `X-VSS-OrchestrationId: <orch_id>` is added when the `SystemVssConnection` AccessToken JWT contains an `orch_id` (or `orchid`) claim.
* **Renew** body: `{"requestId": 42, "lockedUntil": null}` (`lockedUntil` omitted because `EmitDefaultValue=false`) → sent immediately on job receipt and then every **60 s** while the job runs. The worker is not started until the first renew succeeds (up to 5 failures, 1–10 s apart; then the job result is `Abandoned` and `FinishAgentRequest` is skipped).
* Renew response (`TaskAgentJobRequest`) must include **`lockedUntil`** (used in the error path as `request.LockedUntil.Value` → `NullReferenceException` if missing), and may include `reservedAgent: {"id":7,"name":"..."}` — if `reservedAgent.name` differs from the runner's name the runner **renames itself** and rewrites `.runner`. runner.server: `patch.LockedUntil = now + 1 day; return patch`. Recommended: `now + 10 min`.

```json
{ "requestId": 42, "lockedUntil": "2025-01-01T12:10:00Z", "planId": "...", "jobId": "...", "result": null }
```

* If a renew fails, the runner retries until `lockedUntil + 5 min` has passed, then stops renewing and the job is cancelled with result `Abandoned`. Error bodies with `typeKey` `TaskAgentJobNotFoundException` or `TaskAgentJobTokenExpiredException` stop renewal immediately.
* **Finish** (`FinishAgentRequestAsync`) body: `{"requestId": 42, "finishTime": "2025-01-01T12:05:00Z", "result": "succeeded"}` — sent **only when `plan.version < 8`** (`PlanFeatures.JobCompletedPlanEvent` absent) and never for run-service jobs. With `plan.version >= 8` job completion is reported solely by the worker's `JobCompleted` event (section 7.5). Recommended: set `plan.version: 12` and treat `events` as the source of truth; still accept the PATCH.
* `GET .../pools/{poolId}/jobrequests/{requestId}` is used by the listener when a worker exits abnormally to check whether the request already has a `result`; respond with `{ "requestId": 42, "result": null|"canceled"|... }`. `DELETE` is never called.
* `TaskResult` values: `succeeded`, `succeededWithIssues`, `failed`, `canceled`, `skipped`, `abandoned` (ints 0–5).

### 5.6 Agent update state / config refresh (optional)

* `PUT .../pools/{poolId}/agents/{agentId}/updates?currentState=<state>&updateTrace=<text>` (`5.1-preview.1`) → returns `TaskAgent`. Called by the self-updater to report progress and by `RunnerConfigUpdater` for telemetry. Return the agent JSON (200).
* `POST` location `13b5d709-...` with route values `agentId`, `configType` (`runner`|`credentials`) and body = JSON string (the base64-encoded, RSA-signed current config) → returns a JSON string with the refreshed config. Only after a `RunnerRefreshConfig` message; Reef should simply never send that message.

### 5.7 Packages (self-update; optional)

* `GET .../packages/agent/{platform}?$top=1&includeToken=True` (`5.1-preview.2`; `platform` = `linux-x64`, `linux-arm64`, `osx-x64`, `osx-arm64`, `win-x64`, `win-arm64`) → `{count, value:[PackageMetadata]}`; `GET .../packages/agent/{platform}/{version}?includeToken=True` → single `PackageMetadata`:

```json
{ "type": "agent", "platform": "linux-x64", "version": { "major": 2, "minor": 338, "patch": 0 }, "downloadUrl": "https://github.com/actions/runner/releases/download/v2.338.0/actions-runner-linux-x64-2.338.0.tar.gz", "hashValue": "<sha256 hex>", "token": null, "infoUrl": "...", "filename": "actions-runner-linux-x64-2.338.0.tar.gz", "createdOn": "..." }
```

`token` if present is sent as `Authorization: Bearer` on the download. Called (a) on `AgentRefresh` messages and (b) by the worker after a job when the runner was configured with `--disableupdate` (`WarningOutdatedRunnerAsync`: `$top=5&includeToken=False`; failures are swallowed). Returning `{count:0,value:[]}` or 404 is harmless. Never send `AgentRefresh` and the update path is dormant.

---

## 6. `AgentJobRequestMessage`

Source: `Sdk/DTPipelines/Pipelines/AgentJobRequestMessage.cs`, `JobResources.cs`, `WorkspaceOptions.cs`, `ActionStep.cs`, `ActionStepDefinitionReference.cs`, `Step.cs`/`StepConverter.cs`, `ContextData/*.cs`, `ObjectTemplating/Tokens/*.cs`, `Sdk/DTWebApi/WebApi/ServiceEndpointLegacy/*.cs`, `TaskOrchestrationPlanReference.cs`, `TimelineReference.cs`; worker: `Runner.Worker/JobRunner.cs`, `ExecutionContext.cs`, `JobExtension.cs`, `GitHubContext.cs`, `ActionManager.cs`, `Runner.Common/JobServerQueue.cs`; runner.server `MessageController` (`queueJob`).

### 6.1 Encodings used inside the message

**PipelineContextData** (`contextData` values, `PipelineContextDataJsonConverter`). Scalars may be written as bare JSON (`"str"`, `true`, `1.5`); objects carry a numeric type tag `t`:

| Type | `t` | Payload field | Example |
|---|---|---|---|
| String | 0 | `s` | `"text"` or `{"t":0,"s":"text"}` |
| Array | 1 | `a` (array of context data) | `{"t":1,"a":["x",{"t":2,"d":[...]}]}` |
| Dictionary (case-insensitive) | 2 | `d` (array of `{"k":"key","v":<data>}`) | `{"t":2,"d":[{"k":"ref","v":"refs/heads/main"}]}` |
| Boolean | 3 | `b` | `true` or `{"t":3,"b":true}` |
| Number | 4 | `n` (double) | `42` or `{"t":4,"n":42}` |
| CaseSensitiveDictionary | 5 | `d` | same as 2 |

A bare JSON `null` deserializes to `null` context data. Objects without `t` are treated as strings. Empty dictionaries/arrays omit `d`/`a`.

**TemplateToken** (`steps[].inputs`, `environment`, `jobContainer`, `jobServiceContainers`, `jobOutputs`, `defaults[]`, `environmentVariables[]`, `actionsEnvironment.url`, `continueOnError`, `timeoutInMinutes`, `displayNameToken`, `snapshot`; `TemplateTokenJsonConverter`). Scalars without position info are bare JSON (`"str"`, `true`, `1`, `null`); otherwise objects with numeric `type` plus optional `file` (index into `fileTable`), `line`, `col`:

| Token | `type` | Fields |
|---|---|---|
| String | 0 | `lit` |
| Sequence | 1 | `seq`: array of tokens |
| Mapping | 2 | `map`: array of `{"key": <scalar token>, "value": <token>}` |
| BasicExpression (`${{ ... }}`) | 3 | `expr`: expression text without the `${{ }}` |
| InsertExpression (`${{ insert }}`) | 4 | `directive` |
| Boolean | 5 | `bool` |
| Number | 6 | `num` |
| Null | 7 | (none) |

Objects without `type` are strings. Mapping keys are usually `{"type":0,"lit":"..."}` or bare strings.

**Steps**: `Step` is polymorphic on `type` (`StepType`): `"action"` (4) → `ActionStep`; `"backgroundStepControl"` (5) → `BackgroundStepControl`. `ActionStep.reference` is polymorphic on `type` (`ActionSourceType`): `"repository"` (1) → `RepositoryPathReference {name, ref, repositoryType, path}`; `"containerRegistry"` (2) → `ContainerRegistryReference {image}`; `"script"` (3) → `ScriptReference`. Both converters read the discriminator case-insensitively and accept the string or the integer. For repository references the worker requires `repositoryType == "GitHub"` (anything else → `NotSupportedException`), non-empty `name` and `ref`; `path` is the sub-directory inside the repo (omit or `null` for the root).

**ServiceEndpoint** (legacy `GitHub.DistributedTask.WebApi.ServiceEndpoint`): `{id, name, type, url, authorization:{scheme, parameters:{...}}, data:{...}, isShared:false, isReady:true}`. Dictionary keys (`parameters`, `data`) keep their exact case (`AccessToken`, `CacheServerUrl`).

**Variables**: `variables` is a dictionary `name → VariableValue {"value": "...", "isSecret": false}` (`isSecret` omitted when false). Names are case-insensitive.

### 6.2 Complete annotated example

Minimal job: one `uses: actions/checkout@v4` step and one `run: echo hi` step, `runs-on: self-hosted`, plan version 12 so completion is signalled via the `JobCompleted` event. Comments (`//`) are annotations, not part of the JSON.

```jsonc
{
  "messageType": "PipelineAgentJobRequest",              // REQUIRED. Worker stores it in system.jobRequestType; "RunnerJobRequest" switches to run-service mode.
  "plan": {                                              // REQUIRED (ArgUtil.NotNull). Supplies route values for all callbacks.
    "scopeIdentifier": "00000000-0000-0000-0000-000000000000", // {scopeIdentifier} route value
    "planType": "free",                                  // {hubName} route value ("free" in runner.server; GitHub uses "Actions")
    "version": 12,                                       // >= 8 enables PlanFeatures.JobCompletedPlanEvent (worker POSTs JobCompleted; listener skips finish PATCH)
    "planId": "3f7c2b1e-9d44-4a1c-8f2e-7a1b2c3d4e5f",   // {planId} route value
    "planGroup": "free",
    "owner": { "id": 0, "name": "Community" },
    "definition": null, "artifactUri": null, "artifactLocation": null
  },
  "timeline": {                                          // REQUIRED. timeline.id is the main timeline for this job.
    "id": "9a8b7c6d-5e4f-4a3b-9c2d-1e0f9a8b7c6d",
    "changeId": 1,
    "location": null
  },
  "jobId": "1b2c3d4e-5f60-4718-8293-a4b5c6d7e8f9",       // REQUIRED. Also the id of the job TimelineRecord.
  "jobDisplayName": "build",                             // REQUIRED. Shown in logs; timeline record name.
  "jobName": "build",                                    // REQUIRED. Timeline record refName; used for github.job? no (that comes from system.github.job)
  "requestId": 42,                                       // REQUIRED (int64). Used for jobrequests/{requestId} renew and in JobCompleted.
  "lockedUntil": "2025-01-01T12:10:00Z",                 // REQUIRED (non-nullable DateTime; value unused by worker)
  "resources": {                                         // REQUIRED (ArgUtil.NotNull)
    "endpoints": [                                       // EXACTLY ONE endpoint named SystemVssConnection (Enumerable.Single → crash if 0 or >1)
      {
        "id": "00000000-0000-0000-0000-000000000000",
        "name": "SystemVssConnection",
        "type": "SystemVssConnection",
        "url": "http://localhost:4920/owner/repo",       // base URL for all job callbacks (timeline/logs/feed/events/actiondownloadinfo)
        "authorization": {
          "scheme": "OAuth",                             // must be "OAuth"
          "parameters": {
            "AccessToken": "eyJhbGciOiJSUzI1NiIsInR5cCI6IkpXVCJ9.eyJ...fQ.sig" // MUST be a syntactically valid JWT (3 base64url parts): the listener parses it for an orch_id claim. Sent as Bearer on every job callback.
          }
        },
        "data": {
          "CacheServerUrl": "http://localhost:4920/owner/repo/",   // optional (actions/cache)
          "FeedStreamUrl": "ws://localhost:4920/owner/repo/_apis/v1/TimeLineWebConsoleLog/feedstream/9a8b7c6d-.../ws", // optional; omit → REST feed POSTs
          "ResultsServiceUrl": "http://localhost:4920/owner/repo/", // optional; unused unless system.github.results_endpoint is set
          "GenerateIdTokenUrl": "http://localhost:4920/owner/repo/_apis/v1/Message/idtoken?sig=..." // optional; enables ACTIONS_ID_TOKEN_REQUEST_URL
        },
        "isShared": false,
        "isReady": true
      }
    ],
    "containers": [],
    "repositories": []
  },
  "variables": {                                         // REQUIRED dictionary (may be empty, but see notes)
    "system.github.token":   { "value": "ghs_xxx", "isSecret": true },   // → github.token / GITHUB_TOKEN; also default auth for action downloads. Without it checkout fails (not a crash).
    "github_token":          { "value": "ghs_xxx", "isSecret": true },   // runner.server sends both
    "system.github.job":     { "value": "build" },                       // → github.job (only source of that key)
    "system.workflowFilePath": { "value": ".github/workflows/ci.yml" },
    "system.workflowFileFullPath": { "value": "owner/repo/.github/workflows/ci.yml@refs/heads/main" }, // optional; printed as "Uses:"; if present and contextData.inputs exists it must be a dictionary
    "system.phaseDisplayName": { "value": "build" },
    "system.runnerGroupName": { "value": "Default" },                    // printed as "Runner group name"
    "system.runner.lowdiskspacethreshold": { "value": "100" },           // MB; optional
    "DistributedTask.NewActionMetadata":  { "value": "true" },          // feature flags (all optional, default false)
    "DistributedTask.EnableCompositeActions": { "value": "true" },
    "DistributedTask.EnhancedAnnotations": { "value": "true" },
    "DistributedTask.UploadStepSummary":   { "value": "true" },
    "DistributedTask.AddWarningToNode12Action": { "value": "true" },
    "DistributedTask.AllowRunnerContainerHooks": { "value": "true" },
    "DistributedTask.DeprecateStepOutputCommands": { "value": "true" },
    "DistributedTask.ForceGithubJavascriptActionsToNode16": { "value": "true" },
    "actions_uses_cache_service_v2": { "value": "true" },
    "build.buildId": { "value": "1001" }, "build.buildNumber": { "value": "1001" }, "build.containerId": { "value": "1" } // legacy artifact v1 support (optional)
    // NOT set: system.github.results_endpoint (Results service off), system.github.launch_endpoint, ACTIONS_STEP_DEBUG / ACTIONS_RUNNER_DEBUG (set to "true" to enable debug logging)
  },
  "mask": [],                                            // MaskHint list; optional
  "contextData": {                                       // dictionary name → PipelineContextData
    "github": {                                          // REQUIRED. Worker does ExpressionValues["github"].AssertDictionary → KeyNotFound/type crash if missing or not a dictionary
      "t": 2,
      "d": [
        { "k": "server_url",      "v": "http://localhost:4920" },
        { "k": "api_url",         "v": "http://localhost:4920/api/v3" },
        { "k": "graphql_url",     "v": "http://localhost:4920/api/graphql" },
        { "k": "repository",      "v": "owner/repo" },              // REQUIRED (ArgUtil.NotNull(repoFullName) in JobExtension)
        { "k": "repository_owner","v": "owner" },
        { "k": "repository_id",   "v": "1" },
        { "k": "repository_owner_id", "v": "1" },
        { "k": "repositoryUrl",   "v": "http://localhost:4920/owner/repo.git" },
        { "k": "run_id",          "v": "1001" },
        { "k": "run_number",      "v": "7" },
        { "k": "run_attempt",     "v": "1" },
        { "k": "retention_days",  "v": "90" },
        { "k": "sha",             "v": "0123456789abcdef0123456789abcdef01234567" },
        { "k": "ref",             "v": "refs/heads/main" },
        { "k": "ref_name",        "v": "main" },
        { "k": "ref_type",        "v": "branch" },
        { "k": "ref_protected",   "v": false },
        { "k": "head_ref",        "v": "" },
        { "k": "base_ref",        "v": "" },
        { "k": "event_name",      "v": "push" },
        { "k": "event",           "v": { "t": 2, "d": [ { "k": "ref", "v": "refs/heads/main" }, { "k": "repository", "v": { "t": 2, "d": [ { "k": "full_name", "v": "owner/repo" }, { "k": "default_branch", "v": "main" } ] } } ] } }, // full webhook payload as nested dictionaries; written to $GITHUB_EVENT_PATH
        { "k": "workflow",        "v": "CI" },
        { "k": "workflow_ref",    "v": "owner/repo/.github/workflows/ci.yml@refs/heads/main" },
        { "k": "workflow_sha",    "v": "0123456789abcdef0123456789abcdef01234567" },
        { "k": "actor",           "v": "octocat" },
        { "k": "actor_id",        "v": "1" },
        { "k": "triggering_actor","v": "octocat" },
        { "k": "job",             "v": "build" }          // overwritten by system.github.job when that variable exists
        // "token" is injected by the worker from system.github.token; do not rely on sending it here.
      ]
    },
    "needs":    { "t": 2, "d": [] },                     // optional; job outputs of dependencies
    "strategy": { "t": 2, "d": [ { "k": "fail-fast", "v": true }, { "k": "max-parallel", "v": 1 }, { "k": "job-index", "v": 0 }, { "k": "job-total", "v": 1 } ] }, // optional
    "matrix":   null,                                    // optional (null when no matrix)
    "inputs":   { "t": 2, "d": [] },                     // optional; MUST be a dictionary if present and system.workflowFileFullPath is set
    "vars":     { "t": 2, "d": [ { "k": "MY_VAR", "v": "1" } ] }, // optional; repository/org variables
    "job":      { "t": 2, "d": [ { "k": "check_run_id", "v": 1 } ] } // optional; merged into the job context
    // "env" and "secrets" are built by the worker and ignored/overwritten if sent.
  },
  "workspace": { "clean": null },                        // WorkspaceOptions; send {} (runner.server) – clean: "all"|"resources"|"outputs" optional
  "jobContainer": null,                                  // TemplateToken (string image or mapping {image, options, env, ports, volumes, credentials}); null/omit for host jobs
  "jobServiceContainers": null,                          // TemplateToken mapping alias → container mapping; omit
  "jobOutputs": null,                                    // TemplateToken mapping of job outputs (expressions); omit or {"type":2,"map":[...]}
  "defaults": [],                                        // list of TemplateToken mappings, e.g. {"type":2,"map":[{"key":"run","value":{"type":2,"map":[{"key":"shell","value":"bash"}]}}]}
  "environmentVariables": [],                            // list of TemplateToken mappings (workflow/job `env:`)
  "fileTable": [ ".github/workflows/ci.yml" ],           // names referenced by tokens' "file" ids (1-based); may be empty
  "actionsEnvironment": null,                            // {"name":"production","url":<TemplateToken>} for `environment:`; optional
  "steps": [                                             // REQUIRED list (may be empty). Pre/post steps are derived by the worker.
    {
      "type": "action",
      "id": "5f0e1d2c-3b4a-4958-8776-655443322110",     // step TimelineRecord id
      "name": "__run",                                   // unique within job; used for steps.<name>.outputs when contextName absent
      "displayName": "Run actions/checkout@v4",          // or omit and provide displayNameToken
      "displayNameToken": null,
      "contextName": "checkout",                         // steps.checkout.*; optional
      "condition": "success()",                          // expression string; default success()
      "continueOnError": false,                          // TemplateToken (bool/expression); optional
      "timeoutInMinutes": null,                          // TemplateToken; optional
      "enabled": true,
      "reference": {
        "type": "repository",
        "name": "actions/checkout",                      // nameWithOwner
        "ref": "v4",
        "repositoryType": "GitHub",                      // MUST be "GitHub"
        "path": null
      },
      "inputs": {                                        // MappingToken → `with:`
        "type": 2,
        "map": [
          { "key": { "type": 0, "lit": "fetch-depth" }, "value": { "type": 0, "lit": "1" } }
        ]
      },
      "environment": null                                // MappingToken → step `env:`
    },
    {
      "type": "action",
      "id": "6a1f2e3d-4c5b-4a69-9887-766554433221",
      "name": "__run_2",
      "displayName": "Run echo hi",
      "condition": "success()",
      "enabled": true,
      "reference": { "type": "script" },                 // `run:` step
      "inputs": {
        "type": 2,
        "map": [
          { "key": { "type": 0, "lit": "script" }, "value": { "type": 0, "lit": "echo hi" } },
          { "key": { "type": 0, "lit": "shell" },  "value": { "type": 0, "lit": "bash" } },      // optional (falls back to defaults.run.shell / platform default)
          { "key": { "type": 0, "lit": "workingDirectory" }, "value": { "type": 0, "lit": "." } } // optional
        ]
      },
      "environment": { "type": 2, "map": [ { "key": "FOO", "value": { "type": 3, "expr": "github.sha" } } ] } // expression example
    }
  ],
  "snapshot": null, "billingOwnerId": null, "enableDebugger": false, "debuggerTunnel": null, "debuggerWelcomeMessage": null, "dependencies": null, "jobSidecarContainers": null
}
```

The runner.server constructor call for reference (`MessageController.cs` ~L5108): `new AgentJobRequestMessage(plan{PlanType:"free",ContainerId:0,ScopeIdentifier:Guid.NewGuid(),PlanGroup:"free",PlanId:Guid.NewGuid(),Owner{Id:0,Name:"Community"},Version:12}, timeline{Id,ChangeId:1}, jobId, displayname, name, jobContainer, jobServiceContainer, environment, variables, maskHints:[], resources, contextData, new WorkspaceOptions(), steps, fileTable, outputs, jobDefaults, deploymentEnvironment, snapshot:null)`.

### 6.3 Fields the worker crashes without

| Missing / wrong | Failure |
|---|---|
| `resources`, `variables`, `steps`, `plan` | `ArgUtil.NotNull` → `ArgumentNullException` before any timeline write (`JobRunner.RunAsync`, `ExecutionContext.InitializeJob`). |
| `timeline` / `timeline.id` | `NullReferenceException`/`ArgumentNullException` in `JobServerQueue.Start`. |
| exactly one endpoint named `SystemVssConnection` | `InvalidOperationException` from `Enumerable.Single` (listener `JobDispatcher.RunAsync` uses `SingleOrDefault`, but `JobRunner` and `JobServerQueue` use `Single`). |
| `SystemVssConnection.authorization.parameters.AccessToken` that is not a 3-part JWT | `JsonWebTokenDeserializationException` in `JobDispatcher.Run` — thrown in the **listener** before the worker starts, terminating the runner process. |
| `authorization.scheme != "OAuth"` or missing AccessToken | `VssUtil.GetVssCredential` returns `null` → `ArgumentNullException` when connecting the job server. |
| `contextData.github` missing or not a dictionary | `KeyNotFoundException` / `ArgumentException` at `ExpressionValues["github"].AssertDictionary("github")`. |
| `contextData.github.repository` missing | `ArgumentNullException(repoFullName)` in `JobExtension.InitializeJob` (after the job record is created; job fails with an error). |
| `contextData.inputs` present but not a dictionary while `system.workflowFileFullPath` is set | `ArgumentException` (`AssertDictionary("inputs")`). |
| a repository step with `repositoryType` other than `GitHub`, or empty `name`/`ref` | `NotSupportedException` / `ArgumentNullException` in `ActionManager`. |
| `messageType` absent | fine for the worker (`null` stored in `system.jobRequestType`), but `Runner.cs` dispatches on the *outer* `TaskAgentMessage.messageType`, so the outer value must be `PipelineAgentJobRequest`. |

Everything else (`workspace`, `jobContainer`, `jobServiceContainers`, `jobOutputs`, `defaults`, `environmentVariables`, `fileTable`, `actionsEnvironment`, `mask`, `snapshot`, all `variables`, all other `contextData` keys) may be omitted; the getters lazily create empty collections.

---

## 7. Job execution callbacks (worker → server)

Source: `Runner.Common/JobServer.cs`, `Runner.Common/JobServerQueue.cs`, `Sdk/DTGenerated/Generated/TaskHttpClientBase.cs`, `Sdk/DTWebApi/WebApi/TaskHttpClient.cs`, `Runner.Worker/ExecutionContext.cs`, `JobRunner.cs`; runner.server `TimelineController`, `LogfilesController`, `TimeLineWebConsoleLogController`, `FinishJobController`.

All calls go to `SystemVssConnection.url` with `Authorization: Bearer <AccessToken>` (applied on the first attempt), the same VSS headers, and `Accept: application/json; api-version=5.1-preview.1` (events: `2.0-preview.1`). Before the first call the worker does `GET {url}/_apis/connectionData?connectOptions=1&lastChangeId=-1&lastChangeId64=-1` once (`JobServer.ConnectAsync`, retried with exponential backoff up to 3.2 s) and resolves `TaskHttpClient` locations from it (or via `OPTIONS _apis/`). The listener also connects with the job token for `RenewAgentRequest` when it has no other connection; and on worker crash it fetches the timeline to append an error.

Order of operations for a normal job (`plan.version >= 8`, no Results service):

1. `GET _apis/connectionData` (job token).
2. `PATCH .../timelines/{timelineId}/records` with the **job record** in state `pending` (from `InitializeJob`), followed ~immediately by the same record `inProgress` with `startTime` (`ExecutionContext.Start`). The job record has `parentId: null`, `type: "Job"`, `name: jobDisplayName`, `refName: jobName`, `order: null` (server assigns), `workerName: <runner name>`, `agentPlatform`. The listener waits for this first successful update (`JobRecordUpdated`, ≤ 1 s) before continuing.
3. Feed lines (`POST .../records/{jobId}/feed`) begin flowing every 250–500 ms for console output of the job record (setup steps).
4. Step records are added (`type:"Task"`, `parentId: jobId`, `order: 1..n`, `state: pending`) and then each step transitions `inProgress` → `completed` with `result`, `startTime`, `finishTime`, `percentComplete: 100`. Timeline PATCHes are batched every **500 ms**; each PATCH contains only records that changed since the last one, and one PATCH per timeline id.
5. Actions are resolved (`POST .../actiondownloadinfo?jobId=`) and downloaded before step 4's records leave `pending` (section 8).
6. Per step, when the step's log is complete: `POST .../logs` (create) → `POST .../logs/{logId}` (append content) → `PATCH records` with `{ "id": <stepId>, "log": {"id": <logId>, ...} }`. Same for the job log at the end. Attachments (`PUT .../attachments/...`) for step summaries (`Checks.Step.Summary`, only when `DistributedTask.UploadStepSummary` is true and Results is off it still goes through `QueueResultsUpload` → dropped without Results) and diagnostic logs (`DistributedTask.Core.DiagnosticLog`, only with `ACTIONS_RUNNER_DEBUG`).
7. Final job record `completed` with `result`, `finishTime`, `percentComplete: 100`, `issues`/counts, and `variables` (job-level).
8. `JobServerQueue.ShutdownAsync` drains everything (`runOnce` flush).
9. `POST .../events` with `JobCompletedEvent` (retried 5 × with backoff; `TaskOrchestrationPlanNotFoundException` typeKey aborts retries).
10. Listener: renewal loop stops; `FinishAgentRequest` PATCH only if `plan.version < 8`; status goes back to `Online`; next `GET messages`.

### 7.1 Timeline records — `PATCH .../timelines/{timelineId}/records`

Body is a collection wrapper of `TimelineRecord`:

```json
{
  "count": 2,
  "value": [
    {
      "id": "1b2c3d4e-5f60-4718-8293-a4b5c6d7e8f9",
      "parentId": null,
      "type": "Job",
      "name": "build",
      "refName": "build",
      "startTime": "2025-01-01T12:00:01.1234567Z",
      "finishTime": null,
      "currentOperation": null,
      "percentComplete": 0,
      "state": "inProgress",
      "result": null,
      "resultCode": null,
      "changeId": 0,
      "lastModified": "0001-01-01T00:00:00",
      "workerName": "my-runner",
      "order": null,
      "log": null,
      "details": null,
      "errorCount": 0,
      "warningCount": 0,
      "noticeCount": 0,
      "issues": [],
      "location": null,
      "attempt": 0,
      "identifier": null,
      "agentPlatform": "Linux",
      "variables": {}
    },
    {
      "id": "5f0e1d2c-3b4a-4958-8776-655443322110",
      "parentId": "1b2c3d4e-5f60-4718-8293-a4b5c6d7e8f9",
      "type": "Task",
      "name": "Run actions/checkout@v4",
      "refName": "checkout",
      "order": 2,
      "state": "completed",
      "result": "failed",
      "startTime": "2025-01-01T12:00:05.0000000Z",
      "finishTime": "2025-01-01T12:00:09.0000000Z",
      "percentComplete": 100,
      "errorCount": 1, "warningCount": 0, "noticeCount": 0,
      "issues": [
        { "type": "error", "category": "General", "message": "Process completed with exit code 1.",
          "isInfrastructureIssue": null,
          "data": { "stepNumber": "2", "logFileLineNumber": "17", "file": "src/x.js", "line": "3", "col": "1", "title": "..." } }
      ],
      "log": { "id": 12, "location": null },
      "variables": {},
      "isBackground": false
    }
  ]
}
```

* `type` is `"Job"` or `"Task"` (exact case; `TimelineRecord.RecordType` with `DataMember(Name="Type")`). `state`: `pending`/`inProgress`/`completed`/`delayed` (0–3). `result`: see 5.5. `issues[].type`: `error`(1)/`warning`(2)/`notice`(3). `issues[].data` keys seen: `stepNumber`, `logFileLineNumber`, `file`, `line`, `col`, `endLine`, `endColumn`, `title`.
* Records are **partial updates**: a later PATCH may carry a record with only `id` and `log` (from the log upload) — merge by `id` within the timeline (runner.server `MergeTimelineRecords`). Fields with `EmitDefaultValue=false` (`order`, `refName`, `issues`, `variables`, `previousAttempts`, `agentPlatform`, `isBackground`, ...) are simply absent when unset; the runner never sends `null` to clear them.
* `details` (`TimelineReference {id, changeId, location}`) appears on records that own a sub-timeline; before PATCHing such a record the worker calls `POST .../timelines` (7.4) for `details.id` unless it created it already.
* Response must be `{ "count": N, "value": [TimelineRecord...] }` (the runner deserializes `List<TimelineRecord>`, which requires the wrapper; an unwrapped array → `JsonReaderException` → "Mismatched response content type" error). Returning the same records back is sufficient. Failures are retried on the next tick (records buffered); persistent failure of the **job** record on final flush marks the job `failed`.
* `variables` on the job record carries job-level output variables / `set-output` legacy; runner.server persists them.

### 7.2 Logs — `POST .../logs` then `POST .../logs/{logId}`

Create: `POST {url}/{scopeIdentifier}/_apis/distributedtask/hubs/{hubName}/plans/{planId}/logs` body `{"path": "logs\\5f0e1d2c-3b4a-4958-8776-655443322110", "createdOn": "0001-01-01T00:00:00", "lastChangedOn": "0001-01-01T00:00:00", "id": 0, "lineCount": 0}` (`TaskLog(path)`; the path is `logs\<recordId>` with a backslash). Response **must** contain an integer `id`:

```json
{ "id": 12, "location": null, "path": "logs\\5f0e1d2c-...", "lineCount": 0, "indexLocation": null, "createdOn": "...", "lastChangedOn": "..." }
```

Append: `POST .../logs/12` with the whole log file as the body, `Content-Type: application/octet-stream` (plus `; api-version=5.1-preview.1` appended by the client), not chunked (`Content-Length` set; the runner buffers request content). Exactly **one** append per log (the paging logger uploads the complete file when the step finishes; there is no incremental append in the legacy path). Content is UTF-8 text with the runner's `2025-01-01T12:00:05.1234567Z <line>` timestamps. Response: `TaskLog` JSON or empty 2xx (ignored). Then the worker PATCHes the record with `"log": {"id": 12, "location": null, "path": "...", "lineCount": 0, ...}`.

### 7.3 Live console feed — `POST .../records/{recordId}/feed`

`POST {url}/{scopeIdentifier}/_apis/distributedtask/hubs/{hubName}/plans/{planId}/timelines/{timelineId}/records/{jobId}/feed` — note `recordId` in the route is always the **job** record id; the step is identified in the body:

```json
{ "count": 3, "value": ["2025-01-01T12:00:05.1234567Z ##[group]Run actions/checkout@v4", "2025-01-01T12:00:05.1234568Z with:", "..."], "stepId": "5f0e1d2c-3b4a-4958-8776-655443322110", "startLine": 1 }
```

`TimelineRecordFeedLinesWrapper`: `count`, `value` (≤ 100 lines per batch), `stepId` (record the lines belong to), `startLine` (1-based line number of the first line in that step's log; present when known). Batches are sent every 250 ms for the first 60 s of a job, then every 500 ms; each batch has a 60 s timeout; failures are dropped. Response ignored — any 2xx (runner.server `Ok()`). If `SystemVssConnection.data.FeedStreamUrl` is set the worker opens a WebSocket (`Authorization: Bearer`, `User-Agent`) and sends the same JSON as text frames, falling back to REST after repeated failures.

### 7.4 Timelines — `POST .../timelines` / `GET .../timelines/{timelineId}?includeRecords=True`

Create body: `{"id": "<details.id>", "changeId": 0, "location": null, "lastChangedBy": "00000000-0000-0000-0000-000000000000", "lastChangedOn": "0001-01-01T00:00:00"}` → return the same `Timeline` (`records` may be empty). Used only for sub-timelines. GET (used by the listener after a worker crash and returned with `records:[...]`) → `Timeline` with `records` list; the listener then PATCHes the job record with an extra error issue.

### 7.5 Attachments — `PUT .../records/{recordId}/attachments/{type}/{name}`

Body `application/octet-stream`; response `TaskAttachment` (`{"type","name","createdOn","lastChangedOn","lastChangedBy","timelineId","recordId","_links"}`) or empty 2xx. Types: `DistributedTask.Core.DiagnosticLog` (runner diagnostics when `ACTIONS_RUNNER_DEBUG`), `DistributedTask.Core.Summary`, `DistributedTask.Core.FileAttachment`; `Checks.Step.Summary` and `Results.Core.*` go to the Results service only.

### 7.6 Job completion — `POST .../events`

`POST {url}/{scopeIdentifier}/_apis/distributedtask/hubs/{hubName}/plans/{planId}/events`, `Content-Type: application/json; charset=utf-8; api-version=2.0-preview.1`:

```json
{
  "name": "JobCompleted",
  "jobId": "1b2c3d4e-5f60-4718-8293-a4b5c6d7e8f9",
  "requestId": 42,
  "result": "succeeded",
  "outputs": { "artifact": { "value": "dist.zip", "isSecret": false } },
  "actionsEnvironment": { "name": "production", "url": "https://prod.example" },
  "actionsStepsTelemetry": [ { "action": "actions/checkout", "ref": "v4", "type": "repository", "stage": "Main", "stepId": "5f0e...", "stepContextName": "checkout", "result": "succeeded", "executionTimeInSeconds": 4, "startTime": "...", "finishTime": "...", "errorMessages": [] } ],
  "jobTelemetry": [ { "type": "general", "message": "..." } ]
}
```

`JobEvent` is polymorphic on `name` (`JobAssigned`, `JobStarted`, `JobCompleted`); the runner only ever sends `JobCompleted`. `outputs` is `name → VariableValue`; `actionsEnvironment.url` is a `TemplateToken` (string). Response: any 2xx, body ignored. This is the authoritative "job finished" signal when `plan.version >= 8`; the server should mark the job/request complete and (for ephemeral runners) deregister the agent on the next `GET messages`.

---

## 8. Action download info

Source: `Runner.Worker/ActionManager.cs` (`GetDownloadInfoAsync`, `DownloadRepositoryActionAsync`, `DownloadRepositoryArchive`, `CreateAuthHeader`), `Sdk/DTWebApi/WebApi/ActionDownloadInfo.cs`, `ActionReference.cs`, `ActionReferenceList.cs`; runner.server `ActionDownloadInfoController`.

Request (once per job for all `uses:` repository references, deduplicated; nested composite actions cause additional requests): `POST {url}/{scopeIdentifier}/_apis/distributedtask/hubs/{hubName}/plans/{planId}/actiondownloadinfo?jobId={jobId}`, `api-version=6.0-preview.1`:

```json
{ "actions": [ { "nameWithOwner": "actions/checkout", "ref": "v4", "path": null } ], "dependencies": null }
```

Response (`ActionDownloadInfoCollection`) — a dictionary keyed by **`"{nameWithOwner}@{ref}"` exactly as sent** (ordinal match; a missing key → `Exception("Missing download info for actions/checkout@v4")`):

```json
{
  "actions": {
    "actions/checkout@v4": {
      "nameWithOwner": "actions/checkout",
      "ref": "v4",
      "resolvedNameWithOwner": "actions/checkout",
      "resolvedSha": "11bd71901bbe5b1630ceea73d27597364c9af683",
      "tarballUrl": "http://localhost:4920/api/v3/repos/actions/checkout/tarball/11bd71901bbe5b1630ceea73d27597364c9af683",
      "zipballUrl": "http://localhost:4920/api/v3/repos/actions/checkout/zipball/11bd71901bbe5b1630ceea73d27597364c9af683",
      "authentication": { "token": "ghs_download_token", "expiresAt": "2025-01-01T13:00:00Z" },
      "packageDetails": null
    }
  }
}
```

* Required non-empty: `nameWithOwner`, `ref` (the `ArgUtil` checks for `resolvedNameWithOwner`/`resolvedSha` actually test `ref` — a runner bug — but `resolvedNameWithOwner` and `resolvedSha` are used for the archive-cache path and log output, so send them). `tarballUrl` is used on Linux/macOS, `zipballUrl` on Windows.
* `authentication.token` is optional; when absent the worker substitutes `github.token` (from `system.github.token`).
* `packageDetails {version, manifestDigest}` is for immutable OCI-packaged actions; omit.
* Errors: the runner retries 3 × unless the JSON `typeKey` is `UnresolvableActionDownloadInfoException` or `NonRetryableActionDownloadInfoException`; then fails the job with `FailedToResolveActionDownloadInfoException`.

Download: `GET {tarballUrl}` with `User-Agent: GitHubActionsRunner-...` and `Authorization: Basic base64("x-access-token:" + token)` (default). If the variable `actions_use_bearer_token_for_codeload` is `true` **and** the host starts with `codeload.` or the path starts with `/_codeload/`, `Authorization: Bearer <token>` is used instead. Timeout `GITHUB_ACTIONS_RUNNER_ACTION_ARCHIVE_DOWNLOAD_TIMEOUT` (default 300 s), 3 attempts. The response must be a gzip tarball (extracted with `tar -xzf`) or zip containing **exactly one top-level directory** (`'<archive>' contains 'N' directories` otherwise); GitHub-style `owner-repo-sha/` naming is fine. The URL may redirect (the runner's `HttpClientHandler` follows redirects, dropping `Authorization` on cross-host redirects per .NET rules).

Special cases: `repositoryType: "GitHub"` with `nameWithOwner` equal to the workflow repository and `ref` equal to the workflow sha is *not* special-cased in the legacy path — it is downloaded like any other action. `$/`-prefixed self references require `actions_self_repository`; do not emit them.

---

## 9. Results service and other optional subsystems

* **Results service** (`ResultsHttpClient`, `twirp/results.services.receiver.Receiver/*`, blob uploads) is enabled **only** when `variables["system.github.results_endpoint"]` is non-empty *and* the job token exists. When omitted, `JobServerQueue` uses the legacy timeline/log/feed path described in section 7 exclusively (`_resultsClientInitiated=false`; step summaries queued via `QueueResultsUpload` are silently dropped, diagnostic logs still go to `attachments`). `system.github.results_upload_with_sdk` is irrelevant without it. `SystemVssConnection.data.ResultsServiceUrl` is likewise unused. **Confirmed**: runner.server sets `ResultsServiceUrl` but not `results_endpoint`, and jobs run on the legacy path.
* **Run service / Launch** (`RunnerJobRequest` messages, `run_service_url`, `system.github.launch_endpoint`, `acquirejob`/`renewjob`/`completejob`, `actions/runner-registration` `use_v2_flow`, broker `message`/`session`/`acknowledge` endpoints, `BrokerMigration`) — all opt-in by the server; never emit them and the runner stays on the VSS protocol above.
* **Self-update**: dormant unless the server sends `AgentRefresh`/`RunnerRefresh`. `packages` GET should return an empty list (or 404 — the worker's outdated-version warning swallows exceptions). `--disableupdate` runners still call `GET packages` after every job and warn if a newer version is listed.
* **Telemetry**: `JobCompletedEvent.jobTelemetry`/`actionsStepsTelemetry` are informational. `UpdateAgentUpdateStateAsync` (`PUT .../updates`) is only used by updater/config-refresh paths.
* **OIDC**: `SystemVssConnection.data.GenerateIdTokenUrl` → worker exports `ACTIONS_ID_TOKEN_REQUEST_URL`/`ACTIONS_ID_TOKEN_REQUEST_TOKEN` (token = AccessToken); omit to disable.
* **Cache/Artifacts**: `data.CacheServerUrl` → `ACTIONS_CACHE_URL`; `ACTIONS_RESULTS_URL` is derived from `results_endpoint`. The toolkit's cache/artifact clients call `{CacheServerUrl}_apis/artifactcache/...` and `{url}/_apis/pipelines/workflows/{runId}/artifacts` — out of scope for the runner protocol proper.
* **WebSocket feed**: omit `data.FeedStreamUrl` to keep everything on REST.
* **Connectivity check**: `system.runner.serviceconnectivitycheckinput` (JSON) triggers HTTP probes; omit.

---

## 10. Gotchas checklist (from runner source and runner.server)

1. **First request is unauthenticated.** Every new listener connection sends `GET _apis/connectionData` (and later calls) without `Authorization` until it sees `401` + `WWW-Authenticate: Bearer`. Serve that challenge on all `_apis/*` routes; never return 403/404 for a missing token or the runner will not authenticate.
2. **Client-assertion algorithm**: default is **PS256** (RSA-PSS) because `RequireFipsCryptography` defaults to `true` when absent. Return `"properties":{"RequireFipsCryptography":false}` (JSON boolean) from `POST/PUT agents` to get RS256, or verify both. `aud` equals the `authorizationUrl` string you returned; `nbf`/`exp` are 5 minutes apart — allow a few minutes of skew.
3. **JSON casing**: emit camelCase (`sessionId`, `encryptionKey`, `messageType`, `lockedUntil`, `tarballUrl`, `oSDescription` is *not* used — Newtonsoft camel-cases `OSDescription` to `osDescription`). Parsing is case-insensitive, so accept anything. Enums: camelCase strings (`inProgress`, `succeeded`, `automation`, `error`); also accept integers. Dictionary keys (`variables`, `contextData`, `parameters`, `data`, `outputs`, `actions`) keep their exact case and are matched case-insensitively by the runner except `actiondownloadinfo.actions` (ordinal).
4. **`api-version` lives in the `Accept` header** (`application/json; api-version=6.0-preview.2`) and is echoed into `Content-Type: application/json; charset=utf-8; api-version=...` on requests with bodies. It is never a query parameter. Do not reject unknown `api-version` parameters or the `charset`.
5. **Collections are wrapped**: `{count, value}` for every list response (pools, agents, packages, timeline-records PATCH response, `OPTIONS _apis/`) and request (timeline PATCH body, feed body). An unwrapped array fails with `Mismatched response content type`.
6. **Content-Type of responses must be exactly `application/json`** (parameters allowed). Anything else with a body → `Invalid response content type` error. Empty bodies (204 or `Content-Length: 0`) deserialize to `null`, which is the "no message" signal for the long poll and is accepted for fire-and-forget calls (feed, DELETE, events, log append, attachments).
7. **Errors**: return `application/json` `{"$id":"1","innerException":null,"message":"...","typeName":"GitHub.DistributedTask.WebApi.TaskAgentSessionExpiredException, GitHub.DistributedTask.WebApi","typeKey":"TaskAgentSessionExpiredException","errorCode":0,"eventId":3000}` with a 4xx to trigger typed behaviour (`TaskAgentSessionExpiredException` → re-create session; `TaskAgentSessionConflictException`; `TaskAgentNotFoundException` → runner wipes its config and exits; `TaskAgentJobNotFoundException`/`TaskAgentJobTokenExpiredException` → stop renewing; `TaskOrchestrationPlanNotFoundException` → stop JobCompleted retries; `UnresolvableActionDownloadInfoException`). Only `typeKey` (class name) matters; `X-TFS-ServiceError` header text is used for the message otherwise.
8. **GUID formats**: `D` format everywhere; the runner parses any Newtonsoft-accepted Guid string. `Guid.Empty` (`00000000-...`) has semantics: `sessionId` empty disables message calls; `lockToken` is always empty; `authorization.clientId` empty fails registration.
9. **Timestamps**: ISO-8601 with 7 fractional digits and `Z` from the runner (`DateTime.UtcNow`); accept any ISO-8601. `TimeSpan` (`JobCancellation.timeout`, `AgentRefresh.timeout`) is `[d.]hh:mm:ss`. `createdOn`/`lastModified` default `0001-01-01T00:00:00` are sent — accept them.
10. **Empty array vs null**: the runner omits most defaults (`EmitDefaultValue=false`) and never sends explicit `null` to clear a field; treat absent as "unchanged" when merging timeline records. Empty `issues: []` and `variables: {}` are sent on job records; `mask: []`, `defaults: []`, `fileTable: []`, `dependencies` may be absent in the job message — all fine.
11. **Long poll timing**: hold `GET messages` ≤ 50 s and return `204` on timeout; the message connection's client timeout is 60 s and a timeout counts as an error (backoff + connection refresh + new connectionData GET). Do **not** use 202 (it is just "2xx with body" to the runner and an empty 202 is the same as 204).
12. **Message acknowledgement** is `DELETE messages/{messageId}?sessionId=`; re-deliver a job if it is not deleted before the next poll (runner.server behaviour) — and bear in mind the runner intentionally skips the delete while a self-update or run-once job is active. `lastMessageId` on the next GET is a weaker signal.
13. **Renew interval**: `PATCH jobrequests/{id}` every 60 s; response must carry `lockedUntil` (recommend now + 10 min) and should not carry a different `reservedAgent.name` unless you want to rename the runner. Give up on the request only after `lockedUntil + 5 min` of failed renews.
14. **Job-request completion**: with `plan.version >= 8` the listener never PATCHes `finishTime/result`; the worker's `POST events` (`JobCompleted`) is the completion signal. With `version < 8` you get both. Pick ≥ 8.
15. **Session encryption**: AES key delivered via `encryptionKey.value` encrypted with RSA-OAEP — SHA-1 unless `useFipsEncryption:true` (then SHA-256); AES-CBC/PKCS7 with a per-message 16-byte `iv`; `body` = base64(ciphertext of UTF-8 JSON). Omit `encryptionKey` **and** `iv` for plaintext (`body` = raw JSON string).
16. **Ephemeral runners** (`--ephemeral` → `ephemeral:true` must be echoed): the listener runs in run-once mode. It keeps one `GET messages` in flight while the job runs (so a `JobCancellation` can still arrive), and when the worker finishes it cancels that poll, `DELETE`s the session, deletes its local `.runner`/`.credentials` and exits with code 0 (`Runner.cs`: `settings.Ephemeral && runOnceJobCompleted → DeleteLocalRunnerConfig`). It never deregisters the agent itself, so the server must delete the agent when it receives `JobCompleted` for that runner's job (or on the session `DELETE`). Any job message delivered to an ephemeral runner after its first job is left undeleted (`skipMessageDeletion`), so do not assign a second job. runner.server additionally deletes the agent and answers a stray later poll with 403 `TaskAgentSessionExpiredException`.
17. **Route templates**: placeholders are filled case-insensitively; a `null` route value removes its path segment, so one template per resource covers list + item URLs. `{area}`/`{resource}` are auto-filled with `area`/`resourceName`. Routes are always relative to the tenant URL path — if the tenant URL is `http://host/owner/repo`, every route is under `/owner/repo/`.
18. **connectionData**: keep `serviceOwner` empty, ship `serviceDefinitions` with `resourceVersion ≥ 1` and all three version strings, set `lastChangeId ≥ 0`, and if you send `accessMappings` make `defaultAccessMappingMoniker` match one. Expect the runner to call it 3 × per `ConnectAsync` (three connections) plus once per job (worker) and after every polling error.
19. **Headers you will see** and can ignore: `X-TFS-FedAuthRedirect: Suppress`, `X-TFS-Session: <guid>[, <operation>]`, `X-VSS-E2EID: <guid>`, `X-VSS-Agent` (when set), `X-VSS-UserData` (empty, on the authenticated retry), `X-VSS-OrchestrationId` (renew), `Accept-Language`. Headers the runner reads: `WWW-Authenticate` (challenge), `X-TFS-ServiceError` (error text), `X-VSS-AuthenticateError`, `X-TFS-FedAuthRealm` (302 challenge), `Retry-After`/`X-RateLimit-Delay`/`X-RateLimit-Reset` (throttling → `ThrottlingReportHandler`), `x-vss-e2eid`/`x-github-request-id` (logging only).
20. **Job token must be a JWT** (three base64url segments with JSON header/payload) because the listener decodes it to look for `orch_id`; signature is not verified client-side. Include an `orch_id` claim if you want `X-VSS-OrchestrationId` on renews.
21. **runner.server quirks to not copy blindly**: it answers `OPTIONS _apis/` with an empty 200 (works only because its connectionData already lists every service definition), keys agents by id ignoring `poolId`, returns `lockedUntil = now + 1 day`, uses `PlanType "free"`, `Version 12`, random `planId`/`scopeIdentifier` per job, and issues 7-day OAuth tokens.
