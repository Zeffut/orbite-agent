// Orbite on an iPhone: the Owner's Conversation with their Agent, drawn
// with the system's own kit. The app keeps nothing of the Conversation: it
// remembers where the Agent is, and shows what the Agent says.

import SwiftUI

@main
struct OrbiteApp: App {
    @State private var agent = Agent()

    var body: some Scene {
        WindowGroup {
            Group {
                if agent.paired && !agent.adding {
                    ConversationView()
                        .fullScreenCover(isPresented: Binding(get: { agent.welcoming }, set: { _ in })) {
                            WelcomeView().environment(agent)
                        }
                } else {
                    PairingView()
                }
            }
            .environment(agent)
            .preferredColorScheme(.dark)
            .task { await agent.start() }
        }
    }
}

/// The words the app says itself, in the Owner's language.
func say(_ english: String, _ french: String) -> String {
    Locale.preferredLanguages.first?.hasPrefix("fr") == true ? french : english
}
