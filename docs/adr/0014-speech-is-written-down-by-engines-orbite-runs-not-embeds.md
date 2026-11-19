# Speech is written down by engines Orbite runs, not one it embeds

Owners speak to their Agent in Telegram, and want their words written down either by a Provider or on their own Host, where nothing leaves the machine. No single local engine suits every Host: they differ in what they need, how they are installed and which languages they hear well, and the Owner asked for as much choice as possible.

Orbite therefore embeds no speech engine. It knows three ways to reach one on the Host, chosen with `orbite transcription`:

- **whisper.cpp, run as its own program** (`whisper-cli`), once for each recording. It is the engine with a per-file command, single-file models whose checksums its project publishes, and a package in Ubuntu 26.04. Orbite downloads and checks the model, converts recordings with ffmpeg when it is there, and warns before a model is chosen that is too small to hear well or too heavy for the Host, from the project's own memory table and from timing it on the Host.
- **A local server speaking OpenAI's transcription protocol.** This is how faster-whisper is used (through speaches), and any other engine that offers that protocol.
- **A command of the Owner's**, given the recording and printing what was said: sherpa-onnx, Vosk, a script.

Considered and set aside: linking whisper.cpp into Orbite (a C++ build inside a static Rust binary for two architectures, and one engine for everybody); having the installer fetch whisper.cpp's build-tag archives where there is no package (they exist but its project does not document them); installing a Python environment for faster-whisper (not something an installer should own).

## Consequences

- On a Host without a package for whisper.cpp (Debian 12 and 13, Ubuntu 22.04 and 24.04 today), the Owner builds it or chooses another way; Orbite says so rather than fetching something undocumented.
- Quality warnings rest on what the projects publish, which is little for French: Orbite says a model is small, not how often it will be wrong.
- A Provider remains the default, since it needs nothing on the Host.
