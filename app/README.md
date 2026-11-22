# Orbite's app

One app for phones and computers: a window on the page your agent serves ([the web interface](../docs/interface.md)). It remembers where your agent is reached, checks that it answers, and gives way to its page. It is written in Rust alone, and holds nothing of your conversation.

- `core/`: what the app decides and shows (reading the link, its few pages), with its tests: `cargo test --workspace`.
- `src/`: the window, and the pages served to it.

Every push builds it for Linux, Windows, macOS, Android and the iPhone simulator (`.github/workflows/app.yml`); the builds are on the run's page.

## On your own iPhone, with a free Apple account

Apple lets anyone run their own app on their own phone. With a free account the app stops opening after seven days (run it again from the Mac), and Apple gives it no notifications. *These steps follow Tauri's and Apple's documentation; they have not been gone through on a Mac by the project yet.*

Once, on the Mac:

1. Install **Xcode** from the App Store and open it once. In *Xcode > Settings > Accounts*, add your Apple account.
2. In Terminal:
   ```sh
   curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
   rustup target add aarch64-apple-ios aarch64-apple-ios-sim
   brew install cocoapods xcodegen
   cargo install tauri-cli --version "^2" --locked
   git clone -b app https://github.com/Zeffut/orbite-agent
   cd orbite-agent/app
   cargo tauri ios init
   ```
3. On the iPhone: *Settings > Privacy & Security > Developer Mode*, on. Plug it into the Mac and trust the Mac.

Each time (and again after seven days):

1. `cargo tauri ios build --open` opens the project in Xcode; leave the Terminal window open.
2. In Xcode, the first time: click the project, *Signing & Capabilities*, and choose your name as **Team**.
3. Choose your iPhone at the top of the window and press **Run** (▶).
4. On the iPhone, the first time: *Settings > General > VPN & Device Management*, trust your account.

Then, in the app: ask your agent to add your phone, and paste the link it gives.
