# The iPhone app is native, in Swift

Orbite is written in Rust, its apps included (ADR 0018): one language to read, one toolchain, and an app that is a window on the page the Agent serves. On an iPhone that window is not enough. The Owner asked for the best experience there is, and what an iPhone's own apps have (the system's glass, its keyboard, its haptics, its share sheet, its notifications) is given only to apps built with the system's own kit. A page in a window can imitate some of it and reach none of it.

So the iPhone app is written in Swift, with SwiftUI, and talks to the Agent the way the page does: the same pairing, the same stream of events, the same requests. Everything else stays in Rust: the Agent, the installer, the page, and the apps for computers.

## Consequences

- There are two clients of the interface's protocol now, the page and the iPhone app. What one is given, the other must be: a change to the protocol is a change to both.
- The app holds nothing of the Conversation beyond what it shows: the Agent stays where everything is kept.
- Building it takes a Mac and Xcode. The app is built from a description of its project (XcodeGen), so that no project file is kept by hand.
- An Android app is not decided here: the window on the page serves there until it is.
