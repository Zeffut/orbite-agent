# Running Orbite on a GitHub Copilot subscription

Research note, 2026-11-19. Question: through which paths that GitHub documents and permits may a third-party, self-hosted program
(Orbite: Rust service, headless Debian/Ubuntu, own agent loop and tools) use its owner's Copilot subscription for model inference?

Sources: GitHub's own material only, read on 2026-11-19: the `github/copilot-sdk` and `github/copilot-cli` repositories (READMEs,
`docs/`, licences, release assets), docs.github.com, the GitHub Changelog, GitHub's terms. Nothing was installed or signed in:
everything below is documented, not tested. "Not stated" means the sources read do not say. Fetch failures: none of the pages
cited failed. The GitHub Generative AI Services Terms were read through a summarising fetcher, not as raw text (see §6).

Recent changes that contradict older write-ups (billing by tokens and the GA Rust SDK are in §4 and §5):
- GitHub Models is gone: "As of July 30, 2026, GitHub Models has been fully retired. The playground, model catalog, inference API,
  and bring your own key (BYOK) are no longer available to any customer."
  ([docs](https://docs.github.com/en/github-models/use-github-models/prototyping-with-ai-models))
- The SDK README is now titled "Microsoft Copilot SDK": "The SDK repository will move to the Microsoft organization as part of a
  phased transition."

## 1. Official ways for a third-party program to use a Copilot subscription

**a. Copilot SDK** ([repo](https://github.com/github/copilot-sdk),
[docs](https://docs.github.com/en/copilot/how-tos/copilot-sdk)).
- "Python, TypeScript, Go, .NET, Java, and Rust"; SDK licence MIT. Status: "The Microsoft Copilot SDK is generally available and
  follows semantic versioning." GA on 2026-06-02
  ([changelog](https://github.blog/changelog/2026-06-02-copilot-sdk-is-now-generally-available/)).
- Purpose, in GitHub's words: "You can embed GitHub Copilot's agentic engine into your own applications, services, and developer
  tools". It is an agent runtime, not a completions API: "you define agent behavior, Copilot handles planning, tool invocation,
  file edits, and more."
- Subscription: "a GitHub Copilot subscription is required [...] **unless you are using BYOK**". CLI needed? No: "A separate CLI
  installation is not required." "By default, SDK v2 loads the native Rust runtime into your application through a C ABI and
  carries framed JSON-RPC messages over an in-memory connection."
- Other hostings: a spawned child process over stdio or TCP (`RuntimeConnection::for_spawned_stdio`), or a server you run,
  `copilot-runtime --server --port 4321`, attached with `RuntimeConnection::for_tcp(host, port)` ("plaintext JSON-RPC over TCP,
  not HTTP or TLS"; loopback only unless `COPILOT_CONNECTION_TOKEN` is set).

**b. Copilot CLI** ([repo](https://github.com/github/copilot-cli),
[install](https://docs.github.com/en/copilot/how-tos/copilot-cli/set-up-copilot-cli/install-copilot-cli)).
- Install: `curl -fsSL https://gh.io/copilot-install | bash`, `npm install -g @github/copilot`, or release archives.
- Non-interactive: `copilot -p "..."` or a piped prompt; `-s`, `--no-ask-user`, `--model=MODEL`, `--output-format=FORMAT` ("`text`
  (default) or `json` (outputs JSONL: one JSON object per line)"), `--available-tools=TOOL ...`, `--excluded-tools=TOOL ...`
  ([programmatic use](https://docs.github.com/en/copilot/how-tos/copilot-cli/automate-copilot-cli/run-cli-programmatically),
  [reference](https://docs.github.com/en/copilot/reference/copilot-cli-reference/cli-command-reference)).
- Server modes: `copilot --acp` (Agent Client Protocol, NDJSON over `--stdio` or `--port`; "in public preview and subject to
  change", [docs](https://docs.github.com/en/copilot/reference/copilot-cli-reference/acp-server)); and a container image
  `ghcr.io/github/copilot-cli` "that runs the CLI as a headless server for Copilot SDK clients".
- Caller-defined function tools through `-p` or ACP: not stated. ACP "`session/new` [...] does not carry tool-filtering or
  reasoning settings"; the CLI's documented extension path is MCP servers.

**c. GitHub Models**: retired (above); it "was a separate service from GitHub Copilot". **d. A documented HTTP API for Copilot
chat completions**: none. The [REST API endpoints for Copilot](https://docs.github.com/en/rest/copilot) cover settings, usage
metrics and user management only. The SDK docs mention the backend ("Copilot API", "CAPI", `/chat/completions`, `/responses`,
`/v1/messages`, the variables `GITHUB_COPILOT_API_TOKEN` with `COPILOT_API_URL`) as things the runtime uses; no base URL, request
format or third-party use of them is documented.

## 2. Authentication

Credential order, same in CLI and SDK docs
([CLI](https://docs.github.com/en/copilot/how-tos/copilot-cli/set-up-copilot-cli/authenticate-copilot-cli),
[SDK](https://github.com/github/copilot-sdk/blob/main/docs/auth/authenticate.md)): explicit token passed to the SDK, then
`COPILOT_GITHUB_TOKEN`, `GH_TOKEN`, `GITHUB_TOKEN`, then the stored OAuth token from `copilot login`, then the GitHub CLI (`gh
auth token`) fallback. Token types: `gho_` (OAuth) yes; `github_pat_` (fine-grained PAT) yes, "Must be owned by your personal
account (not an organization) with the **Copilot Requests** account permission"; `ghu_` (GitHub App user-to-server) yes; `ghp_`
(classic PAT) no.

Headless server, three documented ways:
1. **Fine-grained PAT** ("ideal for CI/CD pipelines, containers, or headless servers"): open
   `https://github.com/settings/personal-access-tokens/new`; **Resource owner** = personal account; **Permissions** > **Account**
   > **Add permissions** > **Copilot Requests**; **Generate token**; export it as `COPILOT_GITHUB_TOKEN` (or pass it to the SDK:
   `.with_github(|g| g.with_token(token))` plus `with_use_logged_in_user(false)`).
2. **Device code with the CLI** (needs the CLI installed): `copilot login --device-code`, which prints "To authenticate, visit
   https://github.com/login/device and enter code 1234-5678"; the owner confirms in any browser. Storage: the OS keychain (Linux:
   libsecret); "on a headless Linux server without `libsecret` installed—the CLI prompts you to store the token in a plaintext
   configuration file at `~/.copilot/config.json`". The SDK then uses it: "The CLI is only one way to sign in (`copilot login`)
   and store credentials that the SDK can use".
3. **`gh auth login`** already done on the machine: lowest-priority fallback. A sign-in call in the SDK itself (device flow
   without the CLI): not stated.

**May a third-party app register its own OAuth App / GitHub App?** Yes, documented in [GitHub OAuth
setup](https://github.com/github/copilot-sdk/blob/main/docs/setup/github-oauth.md): "You create a GitHub OAuth App (or GitHub
App), users authorize it, and you pass their access token to the SDK. Copilot requests are made on behalf of each authenticated
user, using their Copilot subscription." "GitHub App vs OAuth App: Both work." The guide shows only the redirect flow
(`https://github.com/login/oauth/access_token`, callback URL). Not stated: which OAuth scopes or GitHub App permissions the user
token needs, and whether the device flow works for such an app.

Organisation-billed GitHub App installation tokens (permission **Copilot Requests** = **Read-only**, public preview) do not apply:
"Personal account and enterprise installations cannot use this authentication method."

## 3. Using the runtime as a model gateway

Possible to a large extent, but the runtime keeps the loop: it calls the model, receives tool calls, waits for results, calls the
model again. Orbite cannot do one stateless "conversation in, one completion out" call; it drives a session (`session.send(...)`).
Seeding a session with an externally built message history: not stated.

- **No built-in tools**: `ClientOptions::new(ClientMode::Empty)`. "`Empty` disables ambient tools and optional CLI behavior by
  default." "no ambient OS tools are exposed unless your application registers or allows them." Allowlist:
  `with_available_tools([...])` with names such as `custom:lookupOrder` (TypeScript sample: `availableTools: ["custom:*"]`);
  "`tools.availableTools: []` | Exposes no tools."
- **Own tools**: `Tool::new(name).with_description(..).with_parameters(json_schema)`, installed with
  `SessionToolOptions::with_definitions`; `Session::set_tools` (experimental) replaces them live. With `Tool::with_handler` the
  SDK calls the handler. Without a handler the tool is "declaration-only": the session emits `external_tool.requested`
  (`requestId`, `toolCallId`, `toolName`, `arguments`) and the caller answers with `session.tools.handlePendingToolCall` (see
  `rust/examples/manual_tool_resume.rs`). A permission handler is still required: "Without a permission handler the runtime denies
  the tool".
- **Own system prompt**: `SystemMessageConfig` with `mode` "`"append"` (default), `"replace"`, or `"customize"`". Replace: "For
  full control (removes all guardrails), use `mode: "replace"`".
- **Model**: `CreateSessionOptions::with_model("...")`, `session.set_model(..)`, list via the `models.list` RPC.
- **Streaming**: `config.events.streaming = Some(true)`, then `session.subscribe()`: `assistant.message_delta` (`delta`),
  `assistant.reasoning_delta`, final `assistant.message` (`content`, `toolRequests[]` with `toolCallId`, `name`, `arguments`),
  `assistant.turn_end`.
- **Usage**: `assistant.usage`, "emitted once for every model API call": `model`, `inputTokens`, `outputTokens`,
  `cacheReadTokens`, `cacheWriteTokens`, `reasoningTokens`, `finishReason`, `cost`, `copilotUsage`, `quotaSnapshots`. Totals:
  `session.usage.getMetrics` (`totalNanoAiu`, `totalPremiumRequestCost`; experimental). Account: `account.getQuota`.
- Extra model calls the runtime may make by itself: infinite sessions are "Enabled by default" (background compaction); disable
  with `{ enabled: false }`.

## 4. Rust

There is an official Rust SDK: `cargo add copilot-sdk` (crates.io `copilot-sdk` 2.0.1, published 2026-11-18, MIT, "Rust 1.94.0 or
later"). The GA changelog of June names `cargo add github-copilot-sdk`; that crate also exists (1.0.35). Which one is maintained
long-term: not stated.
- Default build embeds the runtime (`bundled-in-process`). "Linux runtime/forwarding builds require OS OpenSSL 3 development
  files". "Static-CRT musl targets cannot `dlopen`"; use `RuntimeConnection::SpawnedStdio` there. "In-process hosting provides
  neither crash isolation nor a sandbox."
- Wire protocol: "JSON-RPC 2.0 with `Content-Length` framing", protocol version 4 (`sdk-protocol-version.json`); a mismatched
  runtime fails at startup. Method and event names appear in the docs, and the generated SDK types are called "the source of
  truth". A standalone protocol specification, or a statement that speaking the protocol without an official SDK is supported: not
  stated. With a Rust SDK available, there is no reason to.

## 5. Billing and limits

- Current model ([individuals](https://docs.github.com/en/copilot/concepts/billing-and-usage/individuals/billing),
  [pricing](https://docs.github.com/en/copilot/reference/copilot-billing/models-and-pricing)): tokens are priced per model and
  "converted into AI credits, where 1 AI credit = $0.01 USD". "Copilot CLI" is in the list of what is billed in AI credits. SDK:
  "Billing for the Microsoft Copilot SDK is based on the same model as the Copilot CLI".
- Monthly credits: Pro $10, 1,500 (1,000 base + 500 flex); Pro+ $39, 7,000; Max $100, 20,000; Business $19/seat, 1,900 per user,
  pooled; Enterprise $39/seat, 3,900 per user, pooled. Free and Student: "an allowance of AI credits and access to models through
  auto model selection only" (amount not stated). Reset "at 00:00:00 UTC on the first day of each calendar month"; no carry-over.
- Eligibility: "All plans include Copilot CLI and the Copilot app." SDK: "available to all existing GitHub Copilot subscribers,
  including Copilot Free for personal use". An organisation can disable Copilot CLI for its members.
- When credits run out: upgrade; or "continue working by setting a budget for **additional usage**" ("additional usage **may be
  capped**"); or "wait until the next monthly cycle". To never pay more: a "**Bundled AI credits budget**" at $0 with "**Stop
  usage when budget limit is reached**". The exact error the SDK returns on exhaustion: not stated. Rate limits also apply; their
  numbers: not stated.
- Legacy premium requests and model multipliers apply only "to Copilot Pro and Copilot Pro+ subscribers on an existing annual plan
  who remained on legacy premium request-based billing after June 1, 2026". The CLI README still describes that model.

## 6. Terms

- Copilot CLI / runtime binary ([licence](https://github.com/github/copilot-cli/blob/main/LICENSE.md), proprietary): "GitHub also
  grants you the right to reproduce and redistribute unmodified copies of the Software as part of an application or service."
  Conditions include "redistributed solely as part of an application or service that provides material functionality beyond the
  Software itself" and "You include a copy of this License". "Nothing in this License restricts your choice of license for your
  application or service, including distribution under an open source license."
- Which terms govern Copilot use ([additional product
  terms](https://docs.github.com/en/site-policy/github-terms/github-terms-for-additional-products-and-features), effective
  2026-08-27): individuals, "[Section J (AI Features) of the GitHub Terms of Service]"; Business/Enterprise bought from GitHub,
  the GitHub Generative AI Services Terms. Section J has no clause on clients.
- [ToS](https://docs.github.com/en/site-policy/github-terms/github-terms-of-service) section H (effective 2026-04-27): "Abuse or
  excessively frequent requests to GitHub via the API may result in the temporary or permanent suspension of your Account's access
  to the API." "You may not share API tokens to exceed GitHub's rate limitations."
- [Acceptable Use Policies](https://docs.github.com/en/site-policy/acceptable-use-policies/github-acceptable-use-policies): "You
  will not reproduce, duplicate, copy, sell, resell or exploit any portion of the Service, use of the Service, or access to the
  Service without our express written permission."
- A sentence that forbids using Copilot tokens outside official clients, or that names account suspension for doing so: not stated
  in the SDK/CLI docs, ToS sections H and J, the additional product terms or the Acceptable Use Policies. The Generative AI
  Services Terms ("Version: March 2026") were read only via a summariser, which reported no clause on access methods, tokens or
  suspension; read the raw text before relying on that. What is documented positively: tokens are inputs to the CLI and the SDK
  runtime; no document read permits calling the Copilot backend directly.

## 7. System requirements

- With the Rust SDK default features nothing is installed on the server: the runtime is fetched at build time and embedded in the
  Orbite binary. Supported targets: "`darwin-arm64`, `darwin-x64`, `linux-x64`, `linux-arm64`, `linuxmusl-x64`, `linuxmusl-arm64`,
  `win32-x64`, and `win32-arm64`". Size added to the binary, minimum glibc, RAM/CPU: not stated.
- Runtime package for child-process or server use: `github-copilot-<version>-<platform>.tgz` from the CLI releases (v1.0.114:
  linux-x64 about 109 MB, linux-arm64 about 113 MB compressed), executable at `package/prebuilds/<platform>/copilot-runtime`. Keep
  the extracted layout intact; the version must match the SDK's protocol version.
- Full CLI: standalone archives `copilot-linux-x64.tar.gz` (about 149 MB) and `copilot-linux-arm64.tar.gz` (about 152 MB), no Node
  needed; npm install needs "Node.js 22 or later".

## What this means for Orbite (options from best to least documented)

1. **Rust SDK, runtime in process, `ClientMode::Empty`, Orbite's tools as external tools.** Add `copilot-sdk = "2"`; one session
   per Orbite conversation; system prompt in `"replace"` mode; tools declared without handlers and answered through
   `handlePendingToolCall`, or with handlers that call Orbite's own tool code; usage from `assistant.usage`. Owner: installs
   nothing; creates a fine-grained PAT with **Copilot Requests** and pastes it into Orbite. Costs to accept: the runtime owns the
   model/tool loop and the session history, the build needs OpenSSL 3 headers and a larger binary, a proprietary runtime is
   embedded in an AGPL binary (the licence allows redistribution "as part of an application or service" with its licence text
   included; have this checked), and no crash isolation.
2. **Same SDK, runtime as a child process** (`RuntimeConnection::for_spawned_stdio`, feature set `default-features = false,
   features = ["bundled-cli"]`, or an owner-supplied `copilot-runtime`). Same owner steps as 1; optionally the owner extracts the
   runtime package under e.g. `/opt/copilot`. Gains crash isolation and keeps the proprietary code out of Orbite's address space.
3. **Owner installs the Copilot CLI and signs in with `copilot login --device-code`**; Orbite (SDK) picks up the stored
   credentials. Owner: runs the install script and one login command over SSH, confirms a code in a browser on any device. No PAT
   to create or rotate, but needs shell access once, and the token may sit in plaintext in `~/.copilot/config.json`.
4. **Orbite's own OAuth App / GitHub App**, token passed to the SDK. Documented as allowed, but only with the redirect flow;
   scopes and device flow are not stated, so a headless sign-in would have to be tried before being promised.
5. **`copilot --acp` or `copilot -p --output-format=json`** as a subprocess: documented, but no documented way to pass Orbite's
   function tools; ACP is in public preview. Not a fit.
6. Not available: GitHub Models (retired); any documented Copilot HTTP completions endpoint.
