# Subscriptions only through each Provider's official path

Owners should be able to run their Agent on Subscriptions they already pay for, but Providers differ sharply on what third-party tools may do, and paying accounts have reportedly been banned over it. Orbite uses a Subscription only through a path its Provider documents for third parties: an official sign-in programme or SDK where one exists, and for Claude by running the Provider's own unmodified program, signed in by the Owner, without Orbite ever reading or storing its tokens. Where no such path exists (Gemini consumer plans at the time of writing), Orbite offers API Keys only. API Keys are supported for every known Provider.

## Consequences

- Each Subscription path can be closed by its Provider without notice; the Owner is told so when connecting one.
- We never reuse another product's client identity or impersonate an official client, even when it would work.
- Decided on 2026-11-19: for Claude, Orbite offers an API Key only, which includes a key drawing on the API credit that comes with an Owner's plan. Driving Anthropic's own program is not built: Anthropic does not let other products offer its sign-in, and a key is simpler for the Owner and for Orbite.
