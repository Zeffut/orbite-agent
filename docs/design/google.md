# Gmail and Google Calendar: design note

Status: proposal, written before building (#29, #30). It says what the routes are, what each asks of an Owner, and what is to decide. What is marked *to verify* comes from the research note (`docs/research/mcp-connectors.md`) or from general knowledge of Google's rules, and was not tried.

## What is in the way

Everything Google offers for mail and calendars is behind a sign-in that needs a **client**: a registration with Google that says which program is asking. Unlike Notion or Linear, Google does not take a program it does not know: Orbite cannot introduce itself on the spot, as `/mcp connect` does elsewhere.

So the question is whose client it is.

## The routes

**1. Each Owner's own client, with Google's own MCP servers.** Google publishes MCP servers for Gmail, Calendar and Drive (in developer preview). The Owner creates a project in Google's console, turns those services on, and creates a client there; Orbite then signs in with it, by link and paste as for any other server.

- Asks of the Owner: about ten minutes in Google's console, once, following a guide Orbite would give step by step; then one identifier and one secret entered on the Host.
- Asks of Orbite: `/mcp connect` learning to use a client the Owner brings, instead of introducing itself. This is small, and also what GitHub's server needs.
- To verify: that personal `gmail.com` accounts may use these servers while they are a preview; that Google accepts a sign-in ending on an address of the Host itself; and how long a sign-in lasts for a client left in "testing" (Google ends those after seven days, which would mean signing in again every week unless the Owner publishes their client for themselves).

**2. Each Owner's own client, with Official Extensions.** The same client, but Orbite talks to Gmail's and Calendar's ordinary web interfaces through two Extensions it ships, instead of through Google's MCP servers.

- Same ten minutes for the Owner.
- Asks more of Orbite: the Extensions themselves, and a way for an Extension to use what a sign-in obtained, which today only MCP servers have.
- Does not depend on a preview.

**3. One client for everyone, owned by the project.** Nothing to do for an Owner but follow a link.

- Google treats reading mail as a restricted permission: a client offered to the public has to pass its verification, with a yearly security assessment by a third party (*to verify: the current terms and cost*). That is months and money, and a commitment to keep passing it.
- It also needs the relay of #45, since such a client cannot end a sign-in on an address of the Host.

## Proposed

Route 1 now, because it is the least to build and the Owner keeps everything in their own hands: Orbite learns to sign in with a client the Owner brings, and the first-day guide gains a page for Google. If the preview turns out closed to personal accounts, route 2 with the same client. Route 3 only if Orbite finds enough Owners to justify it.

## To decide

Whether asking every Owner for ten minutes in Google's console is acceptable for 1.0, knowing the alternative is a verification the project would have to pay for and keep up.
