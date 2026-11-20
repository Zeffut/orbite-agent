# At the free Permission Level nothing is asked, Critical Actions included

Until 0.3.15 a Critical Action (administrator rights, restarting the Host, Orbite's own files, what looks like a secret) needed an Approval at every Permission Level. The Owner decided otherwise on 2026-11-20: at the most permissive level an Agent must be able to keep its own machine by itself, its updates and the restart of that machine included, with nobody to tap a button.

So at `free` nothing is asked. At `careful` and `balanced` Critical Actions are asked about each time and are never allowed once and for all, as before. Raising the level stays an Approval only the Owner can give, whatever the level it is raised from.

To make "keep its own machine" real for an account that is not root, the installer lets the Agent's account run exactly these as root, and nothing else: updating Orbite, restarting Orbite's service, restarting or stopping the Host.

Considered and set aside: keeping a few Actions asked about even at `free` (which ones is arbitrary, and an Agent that stops to ask at night is what this level is chosen to avoid); a fourth level above `free` (one more thing to explain, for the same choice).

## Consequences

- At `free`, text the Agent reads on the web, in a file or in a recording can talk it into what its Owner would have refused, with no Approval in the way. The Owner is told so where the levels are explained.
- A model's own conduct is not a gate: the Agent is still told to ask before anything that pays, sends or deletes on a page, and to say what stops it, at every level.
- Sub-agents and Routines are unchanged: they only do what needs no Approval at any level, whatever the Owner's level is.
