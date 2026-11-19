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

Using a flat-rate plan (ChatGPT, GitHub Copilot, Claude) instead of an API key is **not available yet**. When it is, it will go only through each provider's official sign-in for third-party programs, never through a path a provider forbids (see [ADR 0006](adr/0006-subscriptions-through-official-paths-only.md)), and each one will have its page here stating that path and its risk.

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

Summaries and wake-ups do not need your best model. Setting `light_model` in `/var/lib/orbite/config.json` to the name of a smaller model of the same provider makes Orbite use it for those.
