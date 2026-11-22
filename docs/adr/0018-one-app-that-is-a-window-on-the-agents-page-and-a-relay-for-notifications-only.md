# One app, a window on the agent's own page, and a relay for notifications only

Orbite has an app for phones first, then for Windows and macOS, made once for all of them (Tauri 2, in Rust like the rest). The app holds nothing of the Conversation and has no logic of its own about it: it remembers where the Owner's agent is reached, checks that it answers, and shows the page the agent serves ([ADR 0016](0016-orbite-serves-its-own-interface.md)). What the page can do, the app can do, on every system at once, and an app installed a year ago shows today's interface.

What the app adds is what a page in a browser is refused or does badly on a phone: a place on the home screen, the microphone without a browser's conditions, and notifications.

**Notifications go through a relay the project runs**, which ADR 0016 had ruled out and this record brings in for that one purpose. A phone only wakes an app through Apple's or Google's own services, and those take the key of whoever published the app: an Orbite on its Owner's machine does not have it. So the agent tells the relay "there is something new for this device", the relay passes that on to Apple or Google, and the app, woken, fetches what is new from the agent itself. The relay never carries what was said, who said it or what it is about, and is given no address of the agent. It is the same arrangement as other self-hosted messengers have.

## Considered and not chosen

- **A separate app for each system** (Swift, Kotlin, and two for computers). Four of everything to write and keep alike, for an interface the agent already serves.
- **The interface built into the app.** An app and an agent of different versions would have to agree on every exchange; the page served by the agent never disagrees with it.
- **No relay, notifications by the page alone** (Web Push from a page added to the home screen). It needs no service of ours, and stays possible for those who want none; but it depends on the Owner going through the browser's own steps, and on a phone it is the weakest of what the systems offer.
- **Each Owner's own key with Apple and Google.** No relay, and no one but developers could use it.

## Consequences

- The project runs a service. It is small and holds little, but it has to be there: when it is not, messages still arrive when the app is opened, and nothing else is lost.
- The relay knows that a device, which it cannot name, was told something at a given time. An Owner who does not accept that turns notifications off and loses nothing else.
- Notifications on an iPhone need the app signed under a paid Apple developer account, even for one person. Without it the app works and expires after seven days.
- The app is given nothing of the system beyond what it shows: the agent's page, loaded from the agent, cannot ask the app for anything the app's own page may.
