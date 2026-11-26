# One server holds several Agents, each in a container of its own

Orbite was one Owner, one Agent, one Conversation on a Host (ADR 0007), installed natively with no container (ADR 0004), the Agent's computer being the machine's own desktop or one of its own (ADR 0019). The Owner now wants several Agents on one server: each with its name, its look, its memory, its Conversation and its own computer, able to talk to one another, created from an app without pairing again. They still want to see and use each Agent's screen.

Two things in the code stand in the way of doing that natively. One program runs one Agent and starts again to apply some settings, so one Agent's change would restart all of them. And every Agent would run under the same account of the machine, where nothing but the Agent's own restraint keeps one from reading another's folder, secrets and desktop.

So each Agent runs in a container of its own, and a server in front of them holds what belongs to the Owner rather than to an Agent. An Agent in its container is what an installation was: the same program, its folder, its desktop. The server lets the Owner's devices in, lists, creates and removes Agents, carries what a device says to the Agent it is for, and carries what one Agent says to another.

- **The server's**: the devices let in, where the interface is reached, the AI providers and their keys with one limit shared by all Agents, updates.
- **Each Agent's**: its name and look, its Conversation, its memory, skills, extensions, connectors, secrets, routines, and its computer.
- **Agents speak to one another by name.** What they say to each other shows in the Conversation of each, so the Owner reads it.
- **The Owner sees and uses each Agent's screen** from their devices (issue 52): the desktop is in the container, and is served from there.
- **The machine's own desktop is no longer an Agent's computer.** It could be lent to one Agent only, by a service every Agent would share; every Agent has a desktop of its own instead.

This replaces ADR 0004 and ADR 0019, and the "one Agent" of ADR 0007: the Owner is still one, and each Agent still has one Conversation.

## Consequences

- A container engine is required on the Host, and installing Orbite installs one where there is none. Where none can be had, Orbite is one Agent on the machine itself, as before: the only case left of an Agent outside a container. Small machines pay for it: each Agent is a desktop and a browser of its own.
- An Agent's image is built on the Host itself, from the program that is installed there, the first time a version needs it: nothing is published beside the program, and nothing is fetched but the system the image starts from.
- The account the server runs as asks Docker for the containers, which lets it do on the Host whatever Docker can. The server runs nothing a model asks for: every Agent, and every command of an Agent, is in a container.
- An installation of before becomes the first Agent of its server: its folder is moved into that Agent's, and what was the server's is taken out of it.
- A device is let in to a server. What it is given is always of one Agent, named in what it asks.
- What one Agent may ask of another, and what an Agent spends of the shared limit, are the Owner's to bound.
