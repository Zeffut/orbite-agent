# Remote MCP servers ("connectors") a self-hosted client can reach

Research note, 2026-11-19. Question: which remote MCP servers offered as connectors in Claude and
ChatGPT can a third-party, self-hosted client with no public URL (Orbite) connect to, and how?

Sources: the MCP specification, the providers' own documentation, and each server's public OAuth
metadata, fetched read-only on 2026-11-19 (`401` challenge, `/.well-known/oauth-protected-resource`,
`/.well-known/oauth-authorization-server`). No client was registered and no sign-in was completed:
"advertised" below means "present in the metadata", not "tested end to end".

## 1. What the spec requires of a client

Revision read: **2026-07-28**, marked current on the
[versioning page](https://modelcontextprotocol.io/specification/versioning). Most servers in the
wild still speak 2025-03-26 to 2025-11-25, so both shapes matter.

**Transport.**
- Streamable HTTP: every JSON-RPC message is its own `POST` to one MCP endpoint, with
  `Accept: application/json, text/event-stream`; the reply is either JSON or a request-scoped SSE
  stream, and the client must handle both
  ([Streamable HTTP](https://modelcontextprotocol.io/specification/2026-07-28/basic/transports/streamable-http)).
- 2026-07-28 removed the `initialize` handshake, protocol sessions (`Mcp-Session-Id`) and the `GET`
  stream. Each `POST` now carries `MCP-Protocol-Version`, `Mcp-Method` and, for tool calls,
  `Mcp-Name` headers that must match the body (same page).
- Older servers: try a modern request first; on `400` with no recognizable modern JSON-RPC error,
  fall back to `initialize` and the 2025-11-25 behaviour (sessions, optional `GET` stream)
  ([backward compatibility](https://modelcontextprotocol.io/specification/2026-07-28/basic/transports/streamable-http#backward-compatibility)).
- Legacy HTTP+SSE (2024-11-05) is deprecated and eligible for removal
  ([deprecated features](https://modelcontextprotocol.io/specification/2026-07-28/deprecated)).
  A client that wants it: `POST` first, and on `400/404/405` open a `GET` SSE stream and wait for
  the `endpoint` event. Several providers are retiring their `/sse` URLs (Notion on 2027-02-23,
  [Notion docs](https://developers.notion.com/guides/mcp/get-started-with-mcp)).

**Authorization** ([authorization](https://modelcontextprotocol.io/specification/2026-07-28/basic/authorization)).
Optional overall; stdio servers should not use it and take credentials from the environment.
- Discovery of the authorization server: the client MUST use Protected Resource Metadata (RFC 9728),
  from the `resource_metadata` value of the `401` `WWW-Authenticate` header, else from
  `/.well-known/oauth-protected-resource{/path}` then the root
  ([discovery](https://modelcontextprotocol.io/specification/2026-07-28/basic/authorization/authorization-server-discovery)).
- Authorization server metadata: the client MUST support both RFC 8414 and OpenID Connect Discovery,
  try the well-known URLs in a fixed order, and reject a document whose `issuer` differs (same page).
- OAuth 2.1 authorization code flow with PKCE: MUST use `S256`, and MUST refuse to proceed when
  `code_challenge_methods_supported` is absent
  ([security considerations](https://modelcontextprotocol.io/specification/2026-07-28/basic/authorization/security-considerations)).
- The `resource` parameter (RFC 8707) MUST be sent on authorization and token requests; the `iss`
  response parameter (RFC 9207) MUST be validated; `state` SHOULD be used (authorization page).
- Token goes in `Authorization: Bearer` on every request, never in the query string (same page).

**Getting a `client_id`**
([client registration](https://modelcontextprotocol.io/specification/2026-07-28/basic/authorization/client-registration)).
Priority order: (1) pre-registered credentials for that server, (2) Client ID Metadata Document
(CIMD) when the server advertises `client_id_metadata_document_supported`, (3) Dynamic Client
Registration (RFC 7591) when a `registration_endpoint` exists, (4) ask the user.
- CIMD is SHOULD: the `client_id` is an HTTPS URL, with a path, serving a JSON document with at
  least `client_id`, `client_name`, `redirect_uris`. The client must host that document publicly.
- DCR is MAY and **deprecated since 2026-07-28**, earliest removal in the first revision on or after
  2027-07-28 (deprecated features page). With DCR the client MUST send an `application_type`
  (`native` for a CLI or a locally hosted app).
- Pre-registration SHOULD be supported: a built-in client ID, or fields the user fills in.

**Redirect URIs.** "All redirect URIs MUST be either `localhost` or use HTTPS", registered in
advance and matched exactly by the authorization server (security considerations page). The spec's
own CIMD example registers `http://127.0.0.1:3000/callback` and `http://localhost:3000/callback`.

## 2. Consequence for a client with no public URL

- A loopback redirect only works when the browser runs on the Host. RFC 8252 describes it as a
  request that "never leaves the device", in the form `http://127.0.0.1:{port}/{path}`, and says
  `localhost` is NOT RECOMMENDED ([RFC 8252 §7.3, §8.3](https://datatracker.ietf.org/doc/html/rfc8252)).
  When the Owner opens the link on a phone, `127.0.0.1` is the phone: the page fails to load.
- **The MCP spec does not address this case.** It defines no device authorization grant, no
  out-of-band redirect, and no flow for a browser on another device. The only official
  authorization extensions are Enterprise-Managed Authorization and OAuth Client Credentials
  ([ext-auth](https://github.com/modelcontextprotocol/ext-auth)).

Options, none of which the spec forbids or blesses:

1. **Loopback redirect, Owner pastes the final URL.** The failed page still shows
   `http://127.0.0.1:…/callback?code=…&state=…` in the address bar; the Owner sends it to the bot,
   Orbite checks `state` and `iss` and redeems the code with the PKCE verifier that never left the
   Host. Works with every server that accepts a loopback redirect. It is a paste, so it does not
   meet "nothing to paste", and it trains the Owner to forward URLs.
2. **Device authorization grant (RFC 8628).** Outside the MCP spec. Some authorization servers
   behind MCP servers advertise a `device_authorization_endpoint` (GitHub, Vercel, Hugging Face,
   Google, Microsoft Entra; observed in metadata). Whether a device-flow token is accepted by each
   MCP server is not verified, and a `client_id` is still needed. Google limits the device flow to a
   few scopes that exclude Gmail and Calendar
   ([Google](https://developers.google.com/identity/protocols/oauth2/limited-input-device)).
3. **A relay on an HTTPS URL the project controls.** A fixed `https://…/callback` receives the
   redirect and hands the code to the Host (for example through a Telegram deep link, whose `start`
   parameter is limited to 64 characters of `A-Z a-z 0-9 _ -`,
   [Telegram](https://core.telegram.org/bots/features), so the relay must keep the code briefly).
   This is the only route to "follow a link, nothing to paste" on a phone. It adds a hosted
   component that sees authorization codes (not tokens: the PKCE verifier stays on the Host), and
   some providers allowlist non-loopback redirect domains (Intercom, below).
4. **Static token header.** No browser round trip at all; the Owner pastes a token once at setup.

Independently of the redirect, CIMD needs one public static file (the metadata document). It can
live on any static host; the Host itself does not need to be reachable.

## 3. Well-known remote servers

DCR/CIMD columns come from the live metadata; "open" claims come from the linked docs.

| Server | URL | OAuth client_id | Restricted? | Static token header |
|---|---|---|---|---|
| [GitHub](https://github.com/github/github-mcp-server/blob/main/docs/host-integration.md) | `https://api.githubcopilot.com/mcp/` | No DCR, no CIMD: "Dynamic Client Registration is NOT supported". Register your own GitHub App or OAuth App | Pre-registration only | Yes, a PAT |
| [Notion](https://developers.notion.com/guides/mcp/get-started-with-mcp) | `https://mcp.notion.com/mcp` | DCR and CIMD advertised | Docs: other clients can connect; members only, no guests | "MCP token", Business or Enterprise plan, workspace owner only |
| [Linear](https://linear.app/docs/mcp) | `https://mcp.linear.app/mcp` | DCR (documented) and CIMD advertised | None documented | Yes, API key or OAuth token |
| [Atlassian](https://support.atlassian.com/atlassian-rovo-mcp-server/docs/getting-started-with-the-atlassian-remote-mcp-server/) | `https://mcp.atlassian.com/v2/mcp` | DCR and CIMD advertised | "Any app with MCP support"; admins have domain allowlists, loopback status not verified | [Yes if the admin enables it](https://support.atlassian.com/atlassian-rovo-mcp-server/docs/configuring-authentication-via-api-token/): Basic `email:token` or service-account Bearer |
| [Sentry](https://mcp.sentry.dev/) | `https://mcp.sentry.dev/mcp` | DCR and CIMD advertised | "Any MCP-compatible client can connect" | Not verified (docs: "All connections use OAuth") |
| [Stripe](https://docs.stripe.com/mcp) | `https://mcp.stripe.com` | DCR advertised, no CIMD | Not verified | Yes, an "agent API key"; other keys refused from December 2026 |
| [Cloudflare](https://developers.cloudflare.com/agents/model-context-protocol/mcp-servers-for-cloudflare/) | `https://mcp.cloudflare.com/mcp` and product servers | DCR and CIMD advertised | None documented | Yes, user or account API token |
| [Asana](https://developers.asana.com/docs/integrating-with-asanas-mcp-server) | `https://mcp.asana.com/v2/mcp` | No DCR: "clients must still pre-register" an MCP app (client ID and secret) | Pre-registration; Enterprise admins can block clients | PAT not verified |
| [PayPal](https://developer.paypal.com/tools/mcp-server/) | `https://mcp.paypal.com/http` (`/sse` legacy) | DCR advertised, CIMD false | "Any MCP client" | Not verified (docs show a Bearer header without saying which token) |
| [Intercom](https://developers.intercom.com/docs/guides/mcp) | `https://mcp.intercom.com/mcp`, EU `mcp.eu.intercom.com` | DCR advertised | Redirect allowlist: `localhost` and `127.0.0.1` allowed on any port, anything else on request to support | Yes, access token |
| [Square](https://developer.squareup.com/docs/mcp) | `https://mcp.squareup.com/mcp` | DCR advertised | **Allowlist**: "Square maintains an allowlist of MCP clients" | No for the remote server (token is for the local one) |
| [Vercel](https://vercel.com/docs/agent-resources/vercel-mcp) | `https://mcp.vercel.com` | CIMD open to any client since [2026-11-02](https://vercel.com/changelog/vercel-mcp-now-supports-client-id-metadata-documents); DCR advertised | Without CIMD, only clients reviewed by Vercel | Not verified |
| [Supabase](https://supabase.com/docs/guides/getting-started/mcp) | `https://mcp.supabase.com/mcp` | DCR by default (documented) | None documented | Yes, a personal access token |
| [Hugging Face](https://github.com/huggingface/hf-mcp-server) | `https://huggingface.co/mcp` (`?login` to force sign-in) | DCR and CIMD advertised | None documented; anonymous access gives public-read tools | Yes, an HF token |
| [Zapier](https://docs.zapier.com/mcp/overview/how-servers-work) | `https://mcp.zapier.com/api/mcp/mcp` (URL observed, not in the page read) | DCR advertised | None documented; no SSE | "Connection token" exists, header form not verified |

Reading the table: of 15 servers, 13 advertise a registration endpoint or CIMD (one of them,
Square, then allowlists clients) and 2 require a pre-registered app (GitHub, Asana). Nine document
a static token. All 15 advertise `S256` PKCE.

## 4. Google and Microsoft

**Google** publishes first-party remote servers: `https://gmailmcp.googleapis.com/mcp/v1`,
`https://calendarmcp.googleapis.com/mcp/v1`, `https://drivemcp.googleapis.com/mcp/v1`, plus Docs,
Sheets, Slides, Chat, People and a universal search server
([Google](https://developers.google.com/workspace/guides/configure-mcp-servers)). They are in the
Workspace Developer Preview Program, not generally available. There is no DCR and no CIMD (the
metadata of `accounts.google.com` advertises neither): the developer creates a Google Cloud
project, enables the MCP APIs, and creates an OAuth client of type "Web application" with "Use
this client for an AI-powered agent" selected
([Gmail guide](https://developers.google.com/workspace/gmail/api/guides/configure-mcp-server)).
The documented redirect URIs are those of Antigravity and Claude; a loopback redirect is not
mentioned. So a third-party client can connect, but only with a client ID the Owner creates in
their own Google Cloud project. Not verified: whether personal gmail.com accounts are eligible,
whether a loopback redirect is accepted, and whether Claude's and ChatGPT's built-in Google
connectors use these servers or private ones.

**Microsoft** publishes Work IQ MCP at `https://workiq.svc.cloud.microsoft/mcp`, covering mail,
calendar, files, chats and sites through generic tools
([Microsoft](https://learn.microsoft.com/en-us/microsoft-365/copilot/extensibility/work-iq/mcp/overview)).
Authentication is Microsoft Entra ID, delegated only. The resource metadata points to the
`organizations` authority, which means work or school accounts; Entra advertises neither DCR nor
CIMD, so the client needs an Entra app registration. The older Agent 365 mail and calendar servers
are documented as legacy
([Microsoft](https://learn.microsoft.com/en-us/microsoft-agent-365/tooling-servers-overview)).
Not verified: licence requirements, admin consent, and any server for personal Outlook.com accounts
(none found in Microsoft's documentation).

## 5. What this means for Orbite

Recommended order:

1. **stdio.** No OAuth, credentials from the environment, as the spec intends. Covers the
   providers that also ship a local server (GitHub, Sentry, Square, per their docs above).
2. **Remote Streamable HTTP with a static `Authorization` header.** Documented for GitHub, Linear,
   Cloudflare, Supabase, Hugging Face, Stripe, Intercom, and conditionally Notion and Atlassian.
   One paste at setup, then nothing. Implement the 2026-07-28 request shape with the fallback to
   `initialize`; legacy SSE can wait.
3. **Remote OAuth.** Implement CIMD and DCR together: DCR is what most servers accept today, CIMD
   is what the spec now prefers and the only self-service route at Vercel. Add a "bring your own
   client ID and secret" setting, required for GitHub, Asana, Google and Microsoft.

Open risks:

- "Sign in by following a link, with nothing to paste" is not achievable from a phone with a
  loopback redirect. It needs either a relay Orbite's project hosts, or a paste. This is a product
  decision, not something the spec settles.
- DCR is deprecated and may be removed from the spec from mid-2027; CIMD requires the project to
  host a metadata document at a stable HTTPS URL, shared by every Orbite install.
- Allowlists are per provider and change without notice (Square, Intercom redirect domains, Vercel
  before November 2026). A relay domain is more exposed to them than a loopback redirect.
- A shared public client ID means any local program can imitate Orbite on a loopback redirect; the
  spec says CIMD "cannot prevent `localhost` URL impersonation".
- Tokens and refresh tokens must be stored on the Host and keyed by issuer; refresh lifetimes vary
  (Stripe ends a session 14 days after authorization, [Stripe](https://docs.stripe.com/mcp)).
- Google and Microsoft require the Owner to create cloud-console credentials: possible, but far
  from a one-link sign-in, and the Google servers are still a preview.
- Nothing here was tested by a real registration or sign-in. Before building step 3, run one
  end-to-end sign-in against Linear or Notion with a loopback redirect.
