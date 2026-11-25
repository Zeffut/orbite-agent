// The services an Agent is connected to with a touch: the Owner signs in
// in a browser that opens over the app, and comes back connected.

import SafariServices
import SwiftUI

struct ConnectorsList: View {
    @Environment(Agent.self) private var agent
    @State private var connectors: [Connector] = []
    @State private var with_a_touch = true
    @State private var loaded = false
    @State private var signing: Signing?
    @State private var busy: String?
    @State private var failed: String?

    struct Signing: Identifiable {
        let name: String
        let link: URL
        var id: String { name }
    }

    var body: some View {
        VStack(spacing: 10) {
            if !loaded {
                ProgressView().padding(.vertical, 20)
            } else if connectors.isEmpty {
                Text(say(
                    "Your agent is on an older version of Orbite, which cannot connect services from here yet.",
                    "Ton agent est sur une version plus ancienne d'Orbite, qui ne sait pas encore connecter des services d'ici."))
                    .font(.footnote).foregroundStyle(.secondary).multilineTextAlignment(.center)
            } else {
                if !with_a_touch {
                    Text(say(
                        "Your agent's page is not reached encrypted yet: ask it in words to connect a service.",
                        "La page de ton agent n'est pas encore joignable en chiffré : demande-lui avec tes mots de connecter un service."))
                        .font(.footnote).foregroundStyle(.secondary).multilineTextAlignment(.center)
                }
                if let failed {
                    Text(failed).font(.footnote).foregroundStyle(.red).multilineTextAlignment(.center)
                }
                ForEach(connectors) { connector in
                    HStack(spacing: 12) {
                        Mark(of: connector)
                        VStack(alignment: .leading, spacing: 2) {
                            Text(connector.label).font(.headline)
                            Text(connector.does).font(.footnote).foregroundStyle(.secondary)
                        }
                        Spacer(minLength: 4)
                        if connector.connected {
                            Image(systemName: "checkmark.circle.fill").font(.title3).foregroundStyle(.green)
                        } else if busy == connector.name {
                            ProgressView()
                        } else {
                            Button(say("Connect", "Connecter")) { begin(connector) }
                                .buttonStyle(.glass).disabled(!with_a_touch || busy != nil)
                        }
                    }
                    .padding(.horizontal, 14).padding(.vertical, 12)
                    .glassEffect(.regular, in: .rect(cornerRadius: 18))
                }
            }
        }
        .task { await look() }
        .sheet(item: $signing, onDismiss: { Task { await look() } }) { signing in
            Browser(link: signing.link).ignoresSafeArea()
        }
        // Signed in, the browser is closed for the Owner as soon as it shows.
        .task(id: signing?.name) {
            guard let name = signing?.name else { return }
            while !Task.isCancelled, signing != nil {
                try? await Task.sleep(for: .seconds(2))
                if let (now, _) = await agent.connectors(), now.first(where: { $0.name == name })?.connected == true {
                    connectors = now
                    UINotificationFeedbackGenerator().notificationOccurred(.success)
                    signing = nil
                }
            }
        }
    }

    private func look() async {
        if let (all, touch) = await agent.connectors() {
            connectors = all
            with_a_touch = touch
        }
        loaded = true
    }

    private func begin(_ connector: Connector) {
        failed = nil
        busy = connector.name
        Task {
            if let link = await agent.connect(connector.name) {
                signing = Signing(name: connector.name, link: link)
            } else {
                failed = say(
                    "\(connector.label) could not be reached just now. Try again in a moment.",
                    "\(connector.label) est injoignable pour l'instant. Réessaie dans un moment.")
            }
            busy = nil
        }
    }
}

/// A page of the web, over the app, for the time of a sign-in.
struct Browser: UIViewControllerRepresentable {
    let link: URL
    func makeUIViewController(context: Context) -> SFSafariViewController { SFSafariViewController(url: link) }
    func updateUIViewController(_ browser: SFSafariViewController, context: Context) {}
}

/// The services, as a screen of the settings.
struct ConnectorsScreen: View {
    var body: some View {
        ScrollView {
            ConnectorsList().padding(.horizontal, 16).padding(.top, 8)
        }
        .background(Color(page: "#0d0d0d").ignoresSafeArea())
        .navigationTitle(say("Services", "Services"))
        .navigationBarTitleDisplayMode(.inline)
    }
}

/// The mark of a service, in its own colour on a light tile; its first
/// letter where this app does not carry its mark.
struct Mark: View {
    let name: String
    let label: String

    init(of connector: Connector) {
        name = connector.name
        label = connector.label
    }

    /// The colour each service is known by.
    private static let colours: [String: String] = [
        "notion": "#111111", "calendly": "#006bff", "trello": "#0052cc", "linear": "#5e6ad2",
        "atlassian": "#0052cc", "clickup": "#7b68ee", "airtable": "#18bfff", "stripe": "#635bff",
        "paypal": "#003087", "sentry": "#362d59", "supabase": "#2fae76", "cloudflare": "#f38020",
    ]

    var body: some View {
        Group {
            if UIImage(named: "logo-" + name) != nil {
                Image("logo-" + name).resizable().scaledToFit().padding(9)
                    .foregroundStyle(Color(page: Self.colours[name] ?? "#111111"))
                    .background(Color(page: "#f2f2f0"), in: .rect(cornerRadius: 11))
            } else {
                Text(String(label.prefix(1))).font(.headline)
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
                    .background(Color.primary.opacity(0.1), in: .rect(cornerRadius: 11))
            }
        }
        .frame(width: 42, height: 42)
    }
}
