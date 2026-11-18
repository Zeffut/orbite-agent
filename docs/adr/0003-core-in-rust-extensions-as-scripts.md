# The Core is Rust; Extensions are scripts

The Agent writes, tests and repairs Extensions on the Host, often several times in a row. Extensions are therefore scripts run in a sandbox by the Core, not compiled Rust: a Rust toolchain on every Host would add more than a gigabyte to the install and minutes to every correction on a small machine. The Agent may write Extensions but never the Core, which only changes through releases; this is what lets every installation update safely while each one grows its own personal Extensions.

Official Extensions (mail, calendar, tasks) use the same mechanism as the Agent's own, so the mechanism is proven and the Agent has real examples to imitate. The scripting language is chosen by a prototype.
