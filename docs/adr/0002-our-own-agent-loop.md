# We write our own agent loop

The loop that turns a model's output into Actions is written by this project, with no third-party agent framework underneath. It costs months that a library would save, and we accept that cost: memory, Skills, Routines, Sub-agents and token economy all live inside that loop, and it is the part of the product we most need to shape freely. Both comparable open-source agents (OpenClaw, Hermes Agent) ended up owning theirs.

## Considered Options

- Build on a multi-provider library behind our own interface (pi was the candidate): rejected in favour of a loop made for exactly this product.
