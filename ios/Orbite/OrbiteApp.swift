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
                if agent.paired && !agent.adding && agent.several && agent.chosen == nil {
                    FirstAgentView()
                } else if agent.paired && !agent.adding {
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
            // Orbite's own look: light on dark, and no colour but the Agent's.
            .tint(Color(page: "#f2f2f0"))
            .task { await agent.start() }
        }
    }
}

/// The words the app says itself, in the Owner's language.
func say(_ english: String, _ french: String) -> String {
    Locale.preferredLanguages.first?.hasPrefix("fr") == true ? french : english
}

/// A server that holds no Agent yet: the first one is a name away.
struct FirstAgentView: View {
    @Environment(Agent.self) private var agent
    @State private var name = ""
    @State private var making = false
    @State private var why: String?
    @FocusState private var writing: Bool

    var body: some View {
        VStack(spacing: 22) {
            Spacer()
            Figure(look: nil, working: making, size: 120)
            VStack(spacing: 8) {
                Text(say("Your first agent", "Ton premier agent")).font(.largeTitle.weight(.bold))
                Text(say(
                    "It gets a computer, a memory and a conversation of its own. Give it a name.",
                    "Il a son ordinateur, sa mémoire et sa conversation à lui. Donne-lui un nom."))
                    .font(.body).foregroundStyle(.secondary).multilineTextAlignment(.center)
            }
            TextField(say("Its name", "Son nom"), text: $name)
                .font(.title3).multilineTextAlignment(.center).focused($writing).submitLabel(.done)
                .padding(.vertical, 14).glassEffect(.regular, in: .capsule)
            if let why { Text(why).font(.footnote).foregroundStyle(.red) }
            Spacer()
            Button {
                let named = name.trimmingCharacters(in: .whitespaces)
                guard !named.isEmpty, !making else { return }
                making = true
                writing = false
                Task {
                    why = await agent.create(named: named)
                    making = false
                }
            } label: {
                Group {
                    if making { ProgressView() } else { Text(say("Create", "Créer")).font(.headline) }
                }
                .frame(maxWidth: .infinity).padding(.vertical, 6)
            }
            .buttonStyle(.glassProminent)
            .disabled(name.trimmingCharacters(in: .whitespaces).isEmpty || making)
        }
        .padding(.horizontal, 28).padding(.bottom, 20)
        .background(Color(page: "#0d0d0d").ignoresSafeArea())
        .onAppear { writing = true }
    }
}
