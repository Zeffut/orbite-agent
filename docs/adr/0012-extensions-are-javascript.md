# Extensions are written in JavaScript

Extensions are scripts the Core runs in a sandbox (ADR 0003), and most of them will be written by the Agent itself, on the Host, with whatever model the Owner chose. The language is therefore picked for the model's sake first: Extensions are JavaScript, run by an embedded QuickJS engine.

A throwaway prototype ran the same Extension contract (`describe()` and `run(action, input)`, with the Core lending only `http_get`, `store_get` and `store_set`) on three embedded engines, and had two small models write the same currency-converter Extension from one brief, five times each per language, checked by eight calls:

| | Rhai | Lua (Luau) | JavaScript (QuickJS) |
|---|---|---|---|
| Written right by the models | 0 of 10 | 10 of 10 | 10 of 10 |
| Sandboxed without effort (no file, system or network access unless lent) | yes | yes | yes |
| Cold load and run of a small Extension | 0.4 ms | 0.2 ms | 0.3 ms |
| Stops a runaway script | built in | by a hook | by a hook |

All three engines together added about 9 MB to a program; speed and isolation do not tell them apart.

## Considered Options

- **Rhai**: the natural choice for a Rust program, pure Rust and sandboxed by design, rejected because the models did not know its syntax: not one Extension out of ten even parsed.
- **Lua**: as well written by the models as JavaScript on this task, and the lightest. Rejected because the language brings almost nothing with it: no JSON, no regular expressions, no dates, no URL encoding, each of which the Core would have to lend and describe, and each a new thing for a model to get wrong. JavaScript has them built in, Connectors deal in JSON all day, and promises give asynchronous calls to the Core a familiar shape.
- **Python**: the other language models write best, rejected for lack of an embeddable engine that is both small and complete.
- **Compiled plugins (WebAssembly)**: rejected with ADR 0003, since they need a compiler on every Host.

## Consequences

- The engine is C code built into the program; it cross-compiles with the rest, as SQLite already does.
- An Extension has no `require`, no `fetch`, no file system and no timers of its own: everything it can reach is lent by the Core, which is what makes an Extension safe to let the Agent write.
- The brief given to a model must say plainly that the two functions are global and what the Core lends: in the first runs, the failures in Lua and JavaScript all came from an ambiguous brief, none from the language.
