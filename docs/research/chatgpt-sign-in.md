# Sign in with ChatGPT / ChatGPT plan usage — wire contract

Researched 2026-11-19 from OpenAI's own documentation only (Markdown twins fetched with `curl`, i.e. `<url>.md`).
Every page listed below was fetched successfully (HTTP 200); none failed. Strings in backticks are verbatim.
"Not stated" means the OpenAI docs do not say it; nothing is filled in from general OAuth knowledge or Codex source.

Sources (abbreviations used below):
- [OV] https://developers.openai.com/siwc/token-sharing-open-source
- [SI] https://developers.openai.com/siwc/token-sharing-open-source/sign-in
- [AS] https://developers.openai.com/siwc/token-sharing-open-source/profiles-and-sessions
- [TR] https://developers.openai.com/siwc/token-sharing-open-source/token-reference
- [MI] https://developers.openai.com/siwc/token-sharing-open-source/models-and-inference
- [ER] https://developers.openai.com/siwc/token-sharing-open-source/errors-and-recovery
- [PL] https://developers.openai.com/siwc/token-sharing-open-source/preview-limitations
- [VM] https://developers.openai.com/siwc/token-sharing-open-source/self-hosted-vms
- [CA] https://developers.openai.com/siwc/token-sharing-open-source/codex-app-server
- [UX] https://developers.openai.com/siwc/ui-ux-guidelines
- [QS] https://developers.openai.com/siwc/quickstart · [IX] https://developers.openai.com/siwc · [WEB] https://developers.openai.com/siwc/website
- [FC] https://developers.openai.com/api/docs/guides/function-calling · [TS] https://developers.openai.com/api/docs/guides/tools-tool-search
- [SE] https://developers.openai.com/api/reference/resources/responses/streaming-events · [CS] https://developers.openai.com/api/docs/guides/conversation-state
- [DISC] https://auth.openai.com/.well-known/openid-configuration (live discovery document, named by [WEB] and [AS])

## 1. Authorization request [SI §2, OV]

Endpoint: open the system browser at `https://auth.openai.com/api/accounts/authorize`, "URL-encode each query parameter value".

| Parameter | Value / rule (verbatim where quoted) |
| --- | --- |
| `client_id` | First sign-in: `dynamic_agent_client`. Later: the saved issued client ID (e.g. `oaiapp_...`). "`dynamic_agent_client` is the first-time registration entrypoint, not the client ID to save or use for token exchange." |
| `agent_name_hint` | "Your app's actual name, such as `OpenClaw`. Use it consistently across installations and include it only on initial dynamic-registration requests. Omit it from reauthorization with an issued client ID." User can edit it; "display metadata, not identity". |
| `ext_agent_host_id` | "Required host identifier, distinct for each host." Sent on first sign-in and on every reauthorization. |
| `id_token_hint` | Reauthorization only: the retained ID token (may be expired). Skips the account selector. Omit after sign-out. |
| `login_hint` | Reauthorization only, optional: saved email, e.g. `login_hint=user%40example.com`. |
| `response_type` | `code` |
| `redirect_uri` | See below. |
| `scope` | "Identity scopes: `openid profile email`. ChatGPT plan usage scopes: `offline_access resource.invoke chatgpt.tokens.use.direct`." (space-separated, all six) |
| `resource` | `https://api.openai.com/v1` |
| `state` | "A fresh random value bound to this authorization attempt; validate it on return." |
| `nonce` | "A fresh random value for ID-token validation." |
| `code_challenge_method` | `S256` |
| `code_challenge` | "The base64url-encoded SHA-256 digest of the PKCE verifier, without padding." |

- `redirect_uri` (verbatim): "Use an HTTP loopback callback on `127.0.0.1` from initial registration onward, for example `http://127.0.0.1:1455/auth/callback`. Start the listener before opening the browser. Later sign-ins may use another available port, such as `http://127.0.0.1:54321/auth/callback`; only the port may vary. Keep the scheme, host, and path unchanged: `/callback` does not match `/auth/callback`. Within each authorization attempt, send the exact same URI, including its selected port, in the authorization request and code exchange. Do not substitute with `localhost`." So: port is not fixed; the path is free at first registration but frozen afterwards (whether a specific path is required: not stated).
- `ext_agent_host_id` generation [OV]: "Generate and persist a stable `ext_agent_host_id` for each host separately. Choose and persist the host ID before its first sign-in. Use an opaque value, not an email, user ID, or other user-identifying value. A host ID is not an authentication credential." Accepted formats: `urn:ietf:params:oauth:jwk-thumbprint:…` (recommended, RFC 9278, not verified by OpenAI), `urn:uuid:…` ("generate a UUIDv4 once per host, prefix its hyphenated string form with `urn:uuid:`"), `did:key:<key>`. One issued `client_id` may be reused on several hosts of the same user+workspace; each host needs its own ID.
- `prompt`: not required. Only for re-enabling plan usage after a decline [ER]: "Request consent with `force_reconsent=true` only after OpenAI confirms deployment for your integration; the existing OAuth `prompt=consent` mechanism remains supported before that rollout. Your app should not force consent on every ordinary sign-in."
- Returning sign-in [SI]: without `id_token_hint` the user sees the account selector then redirects, "with no workspace selection or consent screen"; with it, account selection is skipped.
- Not stated: PKCE verifier length/charset; `state`/`nonce` length; `audience` parameter (only `resource` is documented); `response_mode` in the SIWC pages ([DISC] lists `response_modes_supported: ["query"]`); any device-code/out-of-band flow.

## 2. Callback [SI §3]

Example, verbatim:
```text
YOUR_CALLBACK
  ?code=<AUTHORIZATION_CODE>
  &scope=chatgpt.tokens.use.direct+email+offline_access+openid+profile+resource.invoke
  &state=<ORIGINAL_STATE>
  &client_id=<ISSUED_CLIENT_ID>
```
- "A successful **new registration** returns `code`, `state`, and the issued `client_id`. The callback may also include `scope`."
- Issued ID format: "such as `oaiapp_...`". "If a new-registration callback does not include an issued ID, treat registration as incomplete."
- Reauthorization: "the callback may omit `client_id`; retain the exact client ID already associated with the pending request. If the callback supplies a different client ID, reject the result".
- Errors: "If the callback contains `error=access_denied`, validate `state`, stop the attempt, and do not exchange a code." Other `error` values, `error_description`: not stated ("handle an OAuth `error` before using the result").
- "Use the token response's granted scopes when deciding whether ChatGPT plan usage is enabled" (not the callback `scope`).

## 3. Token endpoint [SI §3, AS, TR, ER]

`POST https://auth.openai.com/api/accounts/oauth/token`, form-encoded, no client secret.
- Code exchange (verbatim): "POST a form-encoded `authorization_code` grant to `https://auth.openai.com/api/accounts/oauth/token` with the issued `client_id`, `code`, `code_verifier`, the same `redirect_uri`, and the same `resource`. No client secret is required." Fields: `grant_type=authorization_code`, `client_id`, `code`, `code_verifier`, `redirect_uri`, `resource=https://api.openai.com/v1`. "If code exchange returns `invalid_grant`, discard that code and start a fresh authorization with the issued client ID".
- Refresh (verbatim): "POST form-encoded `grant_type=refresh_token`, the issued `client_id` saved with this token set (not `dynamic_agent_client`), the saved `refresh_token`, and `resource=https://api.openai.com/v1` to `https://auth.openai.com/api/accounts/oauth/token`; omit `scope` to retain the grant." "Use standard OAuth refresh near access-token expiry." "serialize refreshes for the same session so two processes do not race a rotating token."
- Response [TR]: "The token response includes `access_token`, `refresh_token`, `id_token`, `token_type`, `expires_in`, `scope`, and `earliest_refresh_at`." HTTP `200` for both grants. `scope` is space-separated; `token_type` is `Bearer` in the [SI] record example.
- Lifetimes [TR]: "Access tokens are valid for one hour (`expires_in: 3600`)." "Refresh tokens are valid for 30 days. Each successful refresh returns a replacement refresh token with a fresh 30-day lifetime. Successive replacements have no fixed limit". Rotation: yes ("a new access token and a replacement refresh token"); replace access token, expiry, scopes and refresh token together.
- Refresh failure [ER]: "**Unusable refresh token:** `invalid_grant` on refresh, `invalid_refresh_token`, `token_expired`, `refresh_token_expired`, `refresh_token_invalidated`, or `refresh_token_reused`. Clear unusable tokens and repeat OAuth with the saved issued client ID." "**Invalid client:** `invalid_client`. Fix the client configuration." Do not erase credentials "solely because of a temporary network or infrastructure failure".
- ID token [SI §4]: verify signature against JWKS (`https://auth.openai.com/.well-known/jwks.json` [WEB]; `RS256` per [DISC]), issuer `https://auth.openai.com`, audience = issued client ID, expiration, `nonce`; use `sub` as account identity. Then check the token response `scope` contains `chatgpt.tokens.use.direct` before any inference.
- Access-token claims [TR]: `sub`, `aud` (`https://api.openai.com/v1`), `client_id`, `scope`, `https://api.openai.com/auth` (opaque), `iss`, `iat`, `exp`, `jti`, `nbf`.
- Sign-out [AS]: `revocation_endpoint` from discovery (`https://auth.openai.com/api/accounts/oauth/revoke` per [DISC]); form `POST` with `token=<REFRESH_TOKEN>`, `token_type_hint=refresh_token`, issued `client_id`; "An empty HTTP `200`" = success.
- Not stated: exact `Content-Type` header string (only "form-encoded"); whether refresh returns a new `id_token`; meaning, type and unit of `earliest_refresh_at`; shape of token-endpoint error bodies (only the codes above); HTTP status of token errors.

## 4. Model request [MI, PL, CA]

- List models: `GET https://api.openai.com/v1/models` with `Authorization: Bearer $ACCESS_TOKEN`. "The model-list response contains a `models` array"; keep entries with `visibility == "list"`; show `display_name`, "pass the selected `slug` as `model`". Refresh on account switch. No fixed allow-list is documented; doc examples use `gpt-6.1-sol` / `gpt-6.2-sol`.
- Inference: `POST https://api.openai.com/v1/responses`. "do not point it at ChatGPT's `backend-api` endpoints".
- Headers documented: `Authorization: Bearer <ACCESS_TOKEN>` and `Content-Type: application/json`. Nothing else. Account, organization, session, originator and `User-Agent` headers: not stated for the direct HTTP route (the only mention is that Codex app-server uses `clientInfo.name` "as the request originator" and puts the version "in the User-Agent" [CA]). `Accept: text/event-stream`: not stated.
- Required body rules [PL], verbatim: "Set `store: false` and `stream: true`. Send `input` as an array containing the context needed for each request. Use `instructions` or developer messages; explicit `{type: "message", role: "system"}` items are rejected."
- Rejected/unsupported fields [PL]: "Omit `background`, `conversation`, `max_output_tokens`, `max_tool_calls`, `metadata`, `moderation`, `multi_agent`, `prompt`, `prompt_cache_retention`, `safety_identifier`, `temperature`, `top_logprobs`, `top_p`, `truncation`, and `user`." Also "Omit `previous_response_id` over HTTP and send the required history in `input`."
- Tools [PL]: "**Supported tools:** Group function/custom tools in namespaces or supply them through `additional_tools` input items. Web search remains subject to model and account/workspace policy." Unsupported: "Image generation, file search, Code Interpreter, native computer use, hosted MCP/connectors, and Responses `tool_search`"; "`programmatic_tool_calling` is not accepted in top-level `tools` on this route." Whether a bare top-level `{"type":"function"}` tool is rejected: not stated explicitly — the sentence above only names namespaces and `additional_tools`, so use one of those.
- Inputs [PL]: text, images, files when the model accepts them; no audio/video, no Files upload API, no transcription API.

Complete documented request [MI]:
```bash
curl --no-buffer https://api.openai.com/v1/responses \
  -H "Authorization: Bearer ${ACCESS_TOKEN}" \
  -H "Content-Type: application/json" \
  -d '{
    "model": "gpt-6.1-sol",
    "input": [
      { "role": "user", "content": "Say exactly: Hello, world!" }
    ],
    "store": false,
    "stream": true
  }'
```
(whitespace of the `input` item condensed; fields unchanged.) The SIWC pages show no tool example; shapes below are from the general Responses docs.

- Messages [CS]: `{"role": "user", "content": "knock knock."}`, `{"role": "assistant", "content": "Who's there?"}`. `instructions` is a top-level string [FC]. For reasoning models, replay every item of the previous `output` array unchanged (reasoning items included) [FC, CS].
- Function definition [FC]: `{"type":"function","name":…,"description":…,"parameters":{JSON Schema},"strict":true}`.
- Namespace tool (in `tools`) [FC]: `{"type":"namespace","name":"crm","description":"…","tools":[{"type":"function","name":…,"description":…,"parameters":{…}}]}`
- `additional_tools` input item [TS]: `{"type":"additional_tools","role":"developer","tools":[{"type":"function",…}]}` — tools "become available only after that item appears in the input"; keep its position when replaying history.
- Model's call (output item, replay as-is) [FC]: `{"id":"fc_12345xyz","call_id":"call_12345xyz","type":"function_call","name":"get_weather","arguments":"{\"location\":\"Paris, France\"}"}` A namespaced call also carries `"namespace":"crm"` [TS]. `arguments` is a JSON-encoded string.
- Your result (input item) [FC]: `{"type":"function_call_output","call_id":"call_12345xyz","output":"<string>"}`.

Streaming (SSE; each event is a JSON object with `type` and `sequence_number`) [SE, FC, MI]:
- Text: `{"type":"response.output_text.delta","item_id":"msg_123","output_index":0,"content_index":0,"delta":"In","sequence_number":1,"logprobs":[]}`
- Function call start: `{"type":"response.output_item.added","response_id":"resp_1234xyz","output_index":0,"item":{"type":"function_call","id":"fc_1234xyz","call_id":"call_1234xyz","name":"get_weather","arguments":""}}`
- Arguments: `{"type":"response.function_call_arguments.delta","response_id":"resp_1234xyz","item_id":"fc_1234xyz","output_index":0,"delta":"{\""}` then `{"type":"response.function_call_arguments.done","response_id":"resp_1234xyz","item_id":"fc_1234xyz","output_index":0,"arguments":"{\"location\":\"Paris, France\"}"}`
- Function call complete (name + call id + full arguments in one event): `{"type":"response.output_item.done","response_id":"resp_1234xyz","output_index":0,"item":{"type":"function_call","id":"fc_1234xyz","call_id":"call_1234xyz","name":"get_weather","arguments":"{\"location\":\"Paris, France\"}"}}`
- End + usage: `{"type":"response.completed","response":{"id":"resp_123","status":"completed","output":[…],"usage":{"input_tokens":0,"output_tokens":0,"output_tokens_details":{"reasoning_tokens":0},"total_tokens":0,"input_tokens_details":{"cached_tokens":0,"cache_write_tokens":0}},…},"sequence_number":1}`
- Failure: `{"type":"response.failed","response":{"status":"failed","error":{"code":"server_error","message":"…"},"usage":null,…}}`; `response.incomplete` carries `response.incomplete_details.reason`; top-level `{"type":"error","code":…,"message":…,"param":null,"sequence_number":1}`.
- Rule [MI]: "Treat inference as successful only after receiving `response.completed`. Read through the terminal event." "Handle `response.incomplete`, explicit errors, and interrupted streams separately from `response.completed`."
- Not stated: the SSE framing (`event:`/`data:` lines, any `[DONE]` sentinel); whether the SIWC route emits exactly these general events.

## 5. Errors [ER, MI, AS]

General: "ChatGPT plan usage errors stop inference. OpenAI does not silently switch the request to another billing path.
Preserve the actual HTTP status, response body shape, error code when present, and request ID."

Before the stream opens — body may be `{"detail":"..."}` ("diagnostic text, not a stable machine-readable code"):
`401` identity/direct permission not accepted (check account and scopes); `403` policy/permission such as serving region;
`503` direct routing unavailable (keep credentials, bounded backoff).

Structured errors — use `error.code` and `error.param`:

| `error.code` | HTTP | Meaning / action |
| --- | --- | --- |
| `subscription_sharing_user_not_eligible` | 403 | Plan not eligible: "unavailable for the selected user, workspace, or policy. Explain the restriction; do not repeat the same request or loop through OAuth." |
| `subscription_sharing_usage_limit_exceeded` | 429 | Limit reached: "Pause new requests … and link to ChatGPT settings → Usage. Do not assume the entire plan is empty or infer a reset time from this code alone; an app-specific limit can also apply." |
| `subscription_sharing_usage_unavailable` | 503 | Usage could not be checked; keep credentials, retry with bounded backoff. |
| `subscription_sharing_unsupported_capability` | 400 | Inspect `error.param`, remove the unsupported input/tool/feature/model/service-tier; do not retry the same body. |
| `subscription_sharing_route_not_supported` | 403 | Wrong method/endpoint; only `POST /v1/responses`. |
| `subscription_sharing_invalid_user` | 401 | Subscriber context not validated; re-sign-in "after confirmed revocation or a terminal refresh error". |
| `chatpass_v2_scope_not_authorized`, `chatpass_v2_invalid_authorization_context` | 403 | Grant does not authorize the operation; check client/grant, do not retry. |
| `subscription_sharing_user_unavailable` | 503 | Temporary; keep credentials, bounded backoff. |

- Mid-stream [MI]: "A usage-limit error can arrive as `response.failed` with `subscription_sharing_usage_limit_exceeded` or `subscription_sharing_usage_unavailable` after streaming has begun."
- Revoked/disconnected [ER]: "OpenAI does not currently notify your tool when a user disconnects the app in ChatGPT settings. When a request or refresh confirms the disconnection, stop using the invalid token set and ask the user to sign in again."
- Limits [AS]: "For ChatGPT Plus users, the five-hour usage limit is shared across all apps … The five-hour usage limit does not apply for Pro users." Users can set a per-app weekly limit in ChatGPT settings.
- Not stated: a dedicated code/status for an *expired* access token (only the `401` rows above); any reset-time field, `Retry-After` or rate-limit header; the full error body JSON (only `error.code`, `error.param`, `{"detail":…}`); name of the request-ID header.

## 6. UI/UX requirements [UX, SI, QS]

- Button: "show a Sign in with ChatGPT button labeled **Continue with ChatGPT**" [SI]. "Use **Continue with ChatGPT** when the action starts the sign-in flow." [UX]. [QS]: "The recommended labels are **Continue with ChatGPT** and **Sign in with ChatGPT**"; compact variant: ChatGPT logo with **ChatGPT**; "use approved OpenAI branding".
- First sign-in only: "Show a welcome modal only the first time a user signs in … Explain that eligible AI requests in your app will use their ChatGPT plan and that they can manage usage in ChatGPT settings." "Use a clear confirmation, such as **You're using your ChatGPT plan**, with a **Got it** action."
- Settings offer: card "Use your ChatGPT plan" / "Complete eligible AI requests in this app with usage included in your ChatGPT plan or credits balance." (illustration text). "Distinguish ChatGPT plan usage from your app's own subscription or charges."
- Usage page: "place a **Manage usage** link near the usage summary. Open ChatGPT usage settings" → `https://chatgpt.com/settings/usage`.
- Indicator: "When requests use the user's ChatGPT plan, display **Using ChatGPT plan** near the composer or model selector. Include a **Manage usage** link to" `https://chatgpt.com/settings/usage`.
- Limit reached: "Make **Manage usage** the primary action. Open ChatGPT usage settings" (same URL). "If your app offers its own credits, present an option to buy them as a secondary action." Illustration wording: "Usage limit reached". "Keep the ChatGPT identity visible in the compact version".
- Existing users: banner with **Continue with ChatGPT**; illustration text "Use your ChatGPT plan in this app".
- Plans (apps with paid plans): "**Your app must clearly show which of its plans support ChatGPT plan usage.**" Include **Use your ChatGPT plan** in the feature list; link **Learn more** to `https://help.openai.com`.
- Show the active ChatGPT account in settings/account menu [AS]. Wording for non-graphical (chat/CLI) surfaces: not stated.

## 7. Servers / VMs with no browser [VM]

No device-code or paste-the-URL flow is documented. The whole procedure, verbatim:
1. "Before importing credentials, create and persist a stable `ext_agent_host_id` for the VM."
2. "A `127.0.0.1` callback reaches the computer running the browser, not the remote VM. Complete OAuth locally with the same tool/client for the same user and workspace you will use on the VM."
3. "Transfer the selected registration's protected credential file to the tool's documented VM path over a secure channel such as SSH. Include its issued `client_id`, `access_token`, `refresh_token`, and other saved metadata such as a retained `id_token`." "Preserve the host ID assigned to the VM when importing the credentials so a copied laptop ID does not overwrite it."
4. "Let the VM own later refreshes. Use its own host ID for its next authorization."
Caveat: "This procedure transfers an existing credential session. Host-specific usage attribution and revocation of ChatGPT
plan access for transferred sessions are not yet available."
Credential file [SI §5]: JSON with `email`, `issuer`, `subject`, `client_id`, `ext_agent_host_id`, `id_token`, `access_token`,
`refresh_token`, `token_type`, `expires_in`, `scopes` (array), `saved_at`; "owner-only permissions (`0600` on Unix)", written atomically.
Not stated: SSH port-forwarding of the loopback port, or the user pasting the redirected URL back, as accepted alternatives.

## 8. Who qualifies, and terms

- [OV]: "_These docs explain ChatGPT plan usage for open-source and locally hosted apps. If you're interested in offering it in a paid or remotely hosted app, complete the interest form._" (`https://openai.com/form/sign-in-with-chatgpt-interest/`)
- [QS]: "Sign in with ChatGPT is currently available to selected commercial partners through a limited trial. ChatGPT plan usage is available to all open-source partners and selected private clients."
- [QS]: "**For open-source developers:** Let users run AI workloads in your tools with their ChatGPT plan, without requiring them to provide an API key. The open-source sign-in flow registers your client and issues OAuth credentials for eligible Responses API requests, without a client secret or partner API key."
- Users: "Eligible ChatGPT Plus and Pro users" [QS].
- Not stated: any definition of "open-source", "locally hosted" or "open-source partners" (licence, repository, review); the phrase "personal projects" does not appear; no terms of use, agreement or acceptance step is linked from these pages; no application/approval step for the `dynamic_agent_client` flow. [PL] calls the feature a preview ("Preview limitations").

## 9. Seen for real (2026-11-19, one Plus-or-Pro account)

- The pasted-address sign-in is accepted: authorize, callback with the issued `client_id`, code exchange, `GET /v1/models` (ten models, `visibility: "list"`), `POST /v1/responses` with one tool namespace, images as `input_image`, history replayed without item ids.
- **No prompt caching was observed.** Two requests a few seconds apart sharing a 4,000-token beginning reported `cache_write_tokens: 0` and `cached_tokens: 0`, on `gpt-6-luna` and `gpt-6.1-sol`, whether the instructions went in `instructions` or in a first `developer` message (the lever OpenAI's caching guide names). `prompt_cache_options` is refused: HTTP 400, "prompt_cache_options is not supported on this model". The Sign in with ChatGPT pages say nothing about caching, so this may be how the preview behaves; every turn then counts its whole context against the plan.
- The `usage` object also carries an `attribution` map per item, which Orbite does not read.
- Not exercised for real: renewal after an hour, a plan at its limit, a sign-in that ended, revocation.
