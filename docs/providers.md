# Providers

A provider runs the AI model your agent thinks with. You choose one during setup, with `orbite setup provider` later, and Orbite talks to it directly from your machine: nothing passes through anyone else.

## With an API key

Pay-per-use access. You create a key in the provider's console and type it during setup; it is sealed on the machine and never shown again.

| Name in setup | Provider |
|---|---|
| `anthropic` | Anthropic (Claude) |
| `openai` | OpenAI |
| `google` | Google (Gemini) |
| `xai` | xAI (Grok) |
| `mistral` | Mistral |
| `deepseek` | DeepSeek |
| `groq` | Groq |
| `cerebras` | Cerebras |
| `together` | Together AI |
| `fireworks` | Fireworks AI |
| `openrouter` | OpenRouter (many models behind one key) |
| `custom` | any server that speaks OpenAI's protocol, at the address you give |

Anthropic is spoken to in its own protocol; every other provider through the OpenAI-compatible protocol it offers. So far Orbite has been run for real against Groq only: the others are implemented from each provider's documentation and tested against stand-ins, and reports are welcome.

**The risk** of an API key is cost: an agent that works a lot spends a lot. `/usage` shows what was used, and most providers let you cap spending in their console; do it.

## On your own machine

`ollama` and `lmstudio` use a model running on the machine itself, with no key and no cost per use. Nothing leaves the machine at all. Small local models often cannot use actions reliably: let the tryout tell you (below).

## With a subscription

### ChatGPT (Plus or Pro)

Choose `chatgpt` in `orbite setup` to have your agent think with the usage included in your ChatGPT plan, with no API key. This goes through the sign-in OpenAI opens to open-source programs that run on their user's machine ("Sign in with ChatGPT", ChatGPT plan usage), and through nothing else.

1. Setup shows a link under **Continue with ChatGPT**. Open it in a browser on any device and sign in.
2. Your browser ends on a page that does not load, at an address starting with `http://127.0.0.1:1455`. That is expected: nothing listens there. Copy the whole address and paste it into setup.
3. Setup confirms **You're using your ChatGPT plan**, lists the models your plan offers, and tries the one you choose.

What lets your agent in is sealed with its other secrets and renewed by itself. `/usage` says when your plan is being used; you see and limit what Orbite may use at [chatgpt.com/settings/usage](https://chatgpt.com/settings/usage), and you can disconnect it there at any time. If the sign-in ends (it does after thirty days without use, or when you disconnect it), your agent says so, and you sign in again from the conversation with `/providers connect chatgpt` (the same link and paste), or with `orbite setup provider` on the machine.

**The risks.** OpenAI calls this programme a preview: it may change it or end it, and Orbite would then need an API key again. On a Plus plan the usage limits are shared with everything else you do with ChatGPT; when they are reached your agent says so and waits. OpenAI's documentation describes signing in on the machine that runs the program; pasting the final address from another device follows the same exchange but is not a procedure OpenAI describes. Written from OpenAI's documentation and tested against a stand-in: **not yet tried with a real plan**.

### Others

GitHub Copilot is **not available yet**. For Claude, Orbite takes an API key only: Anthropic does not let other programs offer its sign-in. If your Claude plan comes with API credit, a key created in Anthropic's console draws on it. A plan is only ever used through its provider's official sign-in for third-party programs, never through a path a provider forbids (see [ADR 0006](adr/0006-subscriptions-through-official-paths-only.md)).

## Several providers, in order

Your agent can think with several providers. The first one answers while it can; the next takes over when the one before is out of usage, over the limit you gave it, refuses its key or cannot be reached, and your agent tells you when someone else than the usual one answers. A typical line-up is a plan you already pay for first, then a second plan or an API key behind it.

On the machine:

- `orbite providers` shows them in order, with their limits.
- `orbite providers add` sets one more up, last in line: its key or sign-in, its model, a tryout.
- `orbite providers first NAME` puts one first.
- `orbite providers limit NAME 200000` lets it use at most that many tokens a day, read and written together; `none` lifts the limit. A provider over its limit is not asked again until the next day.
- `orbite providers remove NAME` removes one, with its key.

In the conversation, `/providers` shows the same list with what each used today, and takes `first`, `limit` and `remove` the same way. Adding one is done on the machine, since it takes a key or a sign-in. Changes apply when Orbite starts again; as a service it restarts by itself when you change them from the conversation.

A plan that says it is out of usage (a ChatGPT plan at its limit, for instance) is left alone for an hour before it is asked again. A failure of another kind, for two minutes.

## Choosing a model

Models differ in what matters to an agent, and Orbite shows you before you rely on one. `orbite tryout MODEL` gives a model a few real, short tasks and reports what they showed:

- **Actions**: does it ask for the right action with the right input, use the result, and refrain when none is needed. A model that answers in words where an action was needed **cannot act**: an agent on it can only talk. Setup asks you to insist before choosing such a model; nothing is ever blocked.
- **Images**: does it see them. A model that does not will not see the photos you send.
- **Conversation**: does it keep a detail given a few thousand tokens earlier.
- **Strictness**: given two ordinary requests on its owner's own accounts (signing up to a service, sending a message) with everything needed to do them, does it carry them out. When it does not, you are shown what it said instead. Some models are markedly stricter than others. Orbite reports this as it is and does not try to get a model past a refusal: if you want a less strict agent, choose another model.

A tryout costs a few thousand tokens and about a minute. Setup runs one by itself for a model nobody has tried, and shows what is already known beside the others. What is known comes from real tryouts only: those kept in [`catalogue/models.json`](../catalogue/models.json), built into each release, and those run on your own machine.

The tryout does not tell how fast a model uses up a budget: that depends on your provider's prices. As a rule, the largest models cost many times more per answer than the small ones.

To change model later: `/settings model NAME` in Telegram, or `orbite settings model NAME` on the machine.

## A lighter model for simple work

Summaries and wake-ups do not need your best model. Setup asks for a lighter model of the same provider once the main one is chosen; pass over the question to use the main model for everything. `sudo orbite setup provider` asks it again.
