// The Agent's settings, as an app shows its own: read from the Agent,
// changed with a touch, and nothing of it said in the Conversation.

import SwiftUI

struct SettingsView: View {
    @Environment(Agent.self) private var agent
    @Environment(\.dismiss) private var dismiss
    @State private var settings: [Setting] = []
    @State private var looks = Looks()
    @State private var name = ""
    @State private var loaded = false
    @State private var forgetting = false
    /// What was chosen and is not taken yet, by setting.
    @State private var wanted: [String: String] = [:]
    /// Which of it is shown: who the Agent is, or how it is set.
    var part = Part.rest

    enum Part {
        case look
        case rest
    }

    /// What a setting is called, and what it does, in the Owner's words.
    private static let said: [String: (String, String, String, String)] = [
        "permission": ("Permission", "Autorisation", "How much your agent does without asking.", "Ce que ton agent fait sans demander."),
        "quiet": ("Quiet hours", "Heures calmes", "When your agent does not write first.", "Quand ton agent ne t'écrit pas de lui-même."),
        "wakeups": ("Wake-ups", "Réveils", "How often it looks at what it has to do.", "À quelle fréquence il regarde ce qu'il a à faire."),
        "secrets": ("Passwords in messages", "Mots de passe dans les messages", "Whether a message that holds one reaches the AI.", "Si un message qui en contient un parvient à l'IA."),
        "updates": ("Automatic updates", "Mises à jour automatiques", "", ""),
        "language": ("Language", "Langue", "Of Orbite's own words.", "Des textes d'Orbite."),
        "model": ("Model", "Modèle", "", ""),
        "light": ("Light model", "Modèle léger", "For summaries and wake-ups.", "Pour les résumés et les réveils."),
        "voice": ("Voice messages", "Messages vocaux", "What writes down what you say.", "Ce qui transcrit ce que tu dis."),
        "daily_limit": ("Daily limit for this agent", "Limite par jour pour cet agent", "What it may use of your AI provider in a day.", "Ce qu'il peut utiliser de ton fournisseur d'IA en une journée."),
    ]

    private static let values: [String: (String, String)] = [
        "careful": ("Careful", "Prudent"), "balanced": ("Balanced", "Équilibré"), "free": ("Free", "Libre"),
        "off": ("Off", "Désactivé"), "on": ("On", "Activé"), "strict": ("Strict", "Strict"),
        "en": ("English", "English"), "fr": ("Français", "Français"),
        "60": ("Every hour", "Chaque heure"), "240": ("Every 4 h", "Toutes les 4 h"),
        "720": ("Every 12 h", "Toutes les 12 h"), "provider": ("AI provider", "Fournisseur d'IA"),
        "500000": ("Small", "Petite"), "2000000": ("Medium", "Moyenne"), "10000000": ("Large", "Grande"),
    ]

    private func label(_ key: String) -> String {
        Self.said[key].map { say($0.0, $0.1) } ?? key
    }

    private func worded(_ value: String) -> String {
        Self.values[value].map { say($0.0, $0.1) } ?? value
    }

    private func value(_ key: String) -> String { settings.first { $0.key == key }?.value ?? "" }

    private func change(_ key: String, to value: String) {
        guard let place = settings.firstIndex(where: { $0.key == key }), settings[place].value != value else { return }
        let before = settings[place].value
        settings[place].value = value
        wanted[key] = value
        UISelectionFeedbackGenerator().selectionChanged()
        Task {
            // Some changes make the Agent start again, and it takes nothing
            // for those few seconds: the choice is kept and given again
            // until it is taken, unless another one was made meanwhile.
            var taken = false
            for _ in 0..<25 where wanted[key] == value {
                if await agent.set(key, to: value) {
                    taken = true
                    break
                }
                try? await Task.sleep(for: .milliseconds(800))
            }
            guard wanted[key] == value else { return }
            wanted[key] = nil
            if !taken {
                if let place = settings.firstIndex(where: { $0.key == key }) { settings[place].value = before }
                return
            }
            // A name or a look takes the Agent a moment to come back with.
            if ["name", "shape", "colour"].contains(key) {
                // An Agent of before 0.9.2 starts again for these, and takes a moment.
                try? await Task.sleep(for: .milliseconds(400))
                await agent.look_again()
                try? await Task.sleep(for: .seconds(4))
                await agent.look_again()
            }
        }
    }

    var body: some View {
        NavigationStack {
            Form {
                if !loaded {
                    Section { ProgressView().frame(maxWidth: .infinity) }
                } else if settings.isEmpty {
                    // An agent older than this screen: it says so, and how to get there.
                    Section {
                        Text(say(
                            "Your agent is on an older version of Orbite, which cannot be set from here yet. Tell it: update yourself.",
                            "Ton agent est sur une version plus ancienne d'Orbite, qu'on ne peut pas encore régler d'ici. Dis-lui : mets-toi à jour."))
                            .foregroundStyle(.secondary)
                    }
                    Section {
                        Button(say("Remove this agent from this phone", "Retirer cet agent de ce téléphone"), role: .destructive) {
                            forgetting = true
                        }
                    }
                } else if part == .look {
                    Section {
                        HStack {
                            Spacer()
                            Figure(look: shown, size: 110)
                            Spacer()
                        }
                        .listRowBackground(Color.clear)
                        TextField(say("Name", "Nom"), text: $name)
                            .submitLabel(.done)
                            .onSubmit { change("name", to: name.trimmingCharacters(in: .whitespaces)) }
                    }
                    Section(say("Shape", "Forme")) {
                        LazyVGrid(columns: Array(repeating: GridItem(.flexible()), count: 6), spacing: 14) {
                            ForEach(looks.shapes.keys.sorted(), id: \.self) { shape in
                                if let drawn = looks.shapes[shape] {
                                    let look = Look(outline: drawn.outline, colour: colour, eyes: drawn.eyes)
                                    Button { change("shape", to: shape) } label: {
                                        Figure(look: look, size: 40)
                                            .padding(4)
                                            .overlay {
                                                if value("shape") == shape {
                                                    Circle().stroke(.secondary, lineWidth: 1.5).padding(-3)
                                                }
                                            }
                                    }
                                    .buttonStyle(.plain)
                                }
                            }
                        }
                        .padding(.vertical, 6)
                    }
                    Section(say("Colour", "Couleur")) {
                        LazyVGrid(columns: Array(repeating: GridItem(.flexible()), count: 6), spacing: 14) {
                            ForEach(looks.colours.keys.sorted(), id: \.self) { name in
                                Button { change("colour", to: name) } label: {
                                    Circle().fill(Color(page: looks.colours[name] ?? "#f2f2f0")).frame(width: 34, height: 34)
                                        .overlay {
                                            if value("colour") == name {
                                                Circle().stroke(.primary, lineWidth: 2).padding(-5)
                                            }
                                        }
                                }
                                .buttonStyle(.plain)
                            }
                        }
                        .padding(.vertical, 6)
                    }
                } else {
                    Section {
                        if looks.models.isEmpty {
                            LabeledContent(label("model"), value: value("model"))
                            LabeledContent(label("light"), value: worded(value("light")))
                        } else {
                            Picker(label("model"), selection: Binding(
                                get: { value("model") }, set: { change("model", to: $0) })
                            ) {
                                ForEach(looks.models) { model in
                                    Text(model.value == looks.usual
                                        ? model.label + say(" · Recommended", " · Recommandé") : model.label)
                                        .tag(model.value)
                                }
                            }
                            Picker(selection: Binding(get: { value("light") }, set: { change("light", to: $0) })) {
                                Text(worded("off")).tag("off")
                                ForEach(looks.models) { Text($0.label).tag($0.value) }
                            } label: {
                                VStack(alignment: .leading, spacing: 2) {
                                    Text(label("light"))
                                    Text(say("For summaries and wake-ups.", "Pour les résumés et les réveils."))
                                        .font(.caption).foregroundStyle(.secondary)
                                }
                            }
                        }
                    } header: {
                        Text(say("Thinks with", "Réfléchit avec"))
                    } footer: {
                        Text(say(
                            "The recommended model makes a plan last the month. A stronger one thinks further and uses it up faster.",
                            "Le modèle recommandé fait durer un abonnement tout le mois. Un modèle plus puissant va plus loin et le consomme plus vite."))
                    }
                    Section {
                        ForEach(["daily_limit", "permission", "wakeups", "quiet", "secrets", "language", "voice"], id: \.self) { key in
                            if let setting = settings.first(where: { $0.key == key }) {
                                Picker(selection: Binding(get: { setting.value }, set: { change(key, to: $0) })) {
                                    ForEach(choices(of: setting), id: \.self) { Text(worded($0)).tag($0) }
                                } label: {
                                    VStack(alignment: .leading, spacing: 2) {
                                        Text(label(key))
                                        if let told = Self.said[key], !told.2.isEmpty {
                                            Text(say(told.2, told.3)).font(.caption).foregroundStyle(.secondary)
                                                .fixedSize(horizontal: false, vertical: true)
                                        }
                                    }
                                }
                            }
                        }
                        Toggle(label("updates"), isOn: Binding(
                            get: { value("updates") == "on" }, set: { change("updates", to: $0 ? "on" : "off") }))
                            // On is green, as a switch is everywhere else on the phone.
                            .tint(.green)
                    }
                    Section {
                        NavigationLink {
                            ConnectorsScreen()
                        } label: {
                            Label(say("Services", "Services"), systemImage: "point.3.connected.trianglepath.dotted")
                        }
                    }
                    Section {
                        Button(say("Remove this agent from this phone", "Retirer cet agent de ce téléphone"), role: .destructive) {
                            forgetting = true
                        }
                    } footer: {
                        Text(say(
                            "Your agent and everything it keeps stay where they are. To come back, ask it to add this device again.",
                            "Ton agent et tout ce qu'il garde restent où ils sont. Pour revenir, demande-lui d'ajouter à nouveau cet appareil."))
                    }
                }
            }
            .navigationTitle(part == .look ? say("Your agent", "Ton agent") : say("Settings", "Réglages"))
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .confirmationAction) {
                    Button(say("Done", "OK")) { dismiss() }
                }
            }
            .confirmationDialog(
                say("Remove \(agent.name) from this phone?", "Retirer \(agent.name) de ce téléphone ?"),
                isPresented: $forgetting, titleVisibility: .visible
            ) {
                Button(say("Remove", "Retirer"), role: .destructive) {
                    let gone = agent.known.first { $0.address == agent.address?.absoluteString }
                    dismiss()
                    if let gone { Task { await agent.forget(gone) } }
                }
            }
        }
        .task {
            if let (read, drawn) = await agent.settings() {
                settings = read
                looks = drawn
                name = value("name")
            }
            loaded = true
        }
    }

    /// The colour as a page writes it, of the colour now chosen.
    private var colour: String { looks.colours[value("colour")] ?? agent.look?.colour ?? "#f2f2f0" }

    /// The Agent as the settings now say it looks.
    private var shown: Look? {
        guard let drawn = looks.shapes[value("shape")] else { return agent.look }
        return Look(outline: drawn.outline, colour: colour, eyes: drawn.eyes)
    }

    /// What a setting can be set to, the value it has among them.
    private func choices(of setting: Setting) -> [String] {
        setting.choices.contains(setting.value) ? setting.choices : [setting.value] + setting.choices
    }
}
