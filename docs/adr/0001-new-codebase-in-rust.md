# A new codebase, written in Rust

Orbite started as one person's private workspace (TypeScript, a web interface, and a third-party agent engine). The public product is written from scratch in Rust, in its own repository, instead of being extracted from that code: the private code is tied to one person and one machine in hundreds of places, and a single self-contained Rust binary is what makes a one-command native install realistic. Working pieces of the private project are ported over one at a time when they fit.

## Considered Options

- Extract the generic parts of the private TypeScript project: faster at first, rejected because the product would inherit an architecture built around a dashboard and someone else's agent engine.
