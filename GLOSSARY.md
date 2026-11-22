# Orbite

An open-source personal AI agent that lives on a machine its owner controls, uses the AI its owner already pays for, and adapts to its owner over time.

## Language

### Product

**Orbite**:
The product: the program installed on a Host that runs one Agent for one Owner.
_Avoid_: bot, assistant, platform, app

**Agent**:
The single persistent AI worker of an installation, named by its Owner, with its own Memory, Skills and Computer.
_Avoid_: bot, assistant, AI

**Owner**:
The one person an installation belongs to and the only person its Agent obeys.
_Avoid_: user, account, admin, customer

**Host**:
The machine Orbite is installed on.
_Avoid_: server, box, node

**Core**:
Everything shipped in an Orbite release; identical on every installation and never modified by the Agent.
_Avoid_: engine, kernel, base

### Talking to the Agent

**Conversation**:
The one continuous exchange between an Owner and their Agent; there is exactly one per Agent.
_Avoid_: chat, thread, session

**Channel**:
A place where the Conversation happens: the Interface, Telegram, a terminal.
_Avoid_: client, platform, frontend

**Interface**:
The Channel Orbite serves itself: the Conversation in a browser, on the Owner's Devices.
_Avoid_: dashboard, web app, UI

**Device**:
A browser the Owner let into the Interface, on a computer or a phone.
_Avoid_: session, client, login

**Pairing**:
Letting a Device in, with a code used once.
_Avoid_: login, sign-in, authentication

**Approval**:
The Owner's answer to an Agent asking permission before an Action.
_Avoid_: confirmation, consent, prompt

**Permission Level**:
The Owner's setting, from most restrictive to most permissive, that decides which Actions need an Approval.
_Avoid_: mode, autonomy level, trust level

**Critical Action**:
An Action that needs an Approval at every Permission Level but the most permissive, and that is never allowed once and for all: taking administrator rights, restarting the Host, changing Orbite's own files, reading what looks like a secret.
_Avoid_: dangerous action, sensitive action

### What the Agent thinks with

**Provider**:
A company or local program that runs AI models for the Agent.
_Avoid_: vendor, backend, LLM

**Subscription**:
A flat-rate consumer plan with a Provider, used by the Agent through that Provider's official sign-in.
_Avoid_: plan, account, OAuth

**API Key**:
Pay-per-use access to a Provider.
_Avoid_: token, credential

**Sub-agent**:
A temporary worker the Agent starts for a long or parallel piece of work, which reports back and disappears.
_Avoid_: agent (reserved for the Owner's Agent), worker, task

### What the Agent acts with

**Action**:
Anything the Agent does that has an effect outside its own reasoning.
_Avoid_: tool call, operation, step

**Computer**:
The Agent's own desktop on the Host, with a screen and a browser, that it sees and controls like a person would.
_Avoid_: sandbox, VM, desktop, browser

**Connector**:
What links the Agent to one outside service so it can read from it or act on it.
_Avoid_: integration, plugin, app

**Extension**:
A unit added to an installation on top of the Core, written by the Orbite project or by the Agent itself; Connectors and Routines are delivered as Extensions.
_Avoid_: plugin, module, add-on

**Official Extension**:
An Extension written and maintained by the Orbite project and shipped with it.
_Avoid_: built-in, core extension

### What the Agent keeps

**Memory**:
What the Agent knows about its Owner and their context; every entry is visible to the Owner and can be edited or removed.
_Avoid_: context, knowledge, notes

**Skill**:
A reusable way of doing something that the Agent has learned or been given.
_Avoid_: recipe, playbook, prompt

### When the Agent acts on its own

**Routine**:
Work the Agent does without being asked, started by a schedule or a Trigger.
_Avoid_: cron, job, automation, task

**Trigger**:
An outside event that starts a Routine, such as an incoming email.
_Avoid_: webhook, hook, event

**Wake-up**:
A periodic moment when the Agent reviews what is pending and decides whether to act or to tell its Owner.
_Avoid_: heartbeat, tick, poll
