// The first time an Owner opens their Agent: shown round it, one thing at
// a time, and everything set as they go. Shown once, on whatever device
// comes first.

import AVFoundation
import Speech
import SwiftUI

struct WelcomeView: View {
    @Environment(Agent.self) private var agent
    @State private var step = 0
    @State private var settings: [Setting] = []
    @State private var looks = Looks()
    @State private var name = ""
    @State private var voice: Bool?

    private let steps = 6

    private func value(_ key: String) -> String { settings.first { $0.key == key }?.value ?? "" }

    private func change(_ key: String, to value: String) {
        guard let place = settings.firstIndex(where: { $0.key == key }), settings[place].value != value else { return }
        settings[place].value = value
        UISelectionFeedbackGenerator().selectionChanged()
        Task { _ = await agent.set(key, to: value) }
    }

    private var colour: String { looks.colours[value("colour")] ?? agent.look?.colour ?? "#f2f2f0" }

    private var shown: Look? {
        guard let drawn = looks.shapes[value("shape")] else { return agent.look }
        return Look(outline: drawn.outline, colour: colour, eyes: drawn.eyes)
    }

    var body: some View {
        VStack(spacing: 0) {
            // Where the Owner is in it.
            HStack(spacing: 6) {
                ForEach(0..<steps, id: \.self) { place in
                    Capsule().fill(place <= step ? Color.primary : Color.primary.opacity(0.18))
                        .frame(height: 4)
                }
            }
            .padding(.horizontal, 24).padding(.top, 14)
            .animation(.snappy, value: step)

            TabView(selection: $step) {
                hello.tag(0)
                look.tag(1)
                leave.tag(2)
                speaking.tag(3)
                services.tag(4)
                ready.tag(5)
            }
            .tabViewStyle(.page(indexDisplayMode: .never))
            .animation(.snappy, value: step)

            HStack(spacing: 12) {
                if step > 0 {
                    Button {
                        step -= 1
                    } label: {
                        Image(systemName: "chevron.left").font(.headline).frame(width: 52, height: 52)
                            .glassEffect(.regular.interactive(), in: .circle)
                    }
                    .tint(.primary)
                }
                Button {
                    UIImpactFeedbackGenerator(style: .light).impactOccurred()
                    if step == 1, !name.trimmingCharacters(in: .whitespaces).isEmpty {
                        change("name", to: name.trimmingCharacters(in: .whitespaces))
                    }
                    if step + 1 < steps {
                        step += 1
                    } else {
                        Task {
                            await agent.look_again()
                            await agent.welcomed()
                        }
                    }
                } label: {
                    Text(step + 1 < steps ? say("Continue", "Continuer") : say("Start talking", "Commencer"))
                        .font(.headline).frame(maxWidth: .infinity).frame(height: 36)
                        .foregroundStyle(Color(page: "#0d0d0d"))
                }
                .buttonStyle(.glassProminent)
            }
            .padding(.horizontal, 20).padding(.bottom, 14)
        }
        .background(Color(page: "#0d0d0d").ignoresSafeArea())
        .preferredColorScheme(.dark)
        .tint(Color(page: "#f2f2f0"))
        .interactiveDismissDisabled()
        .task {
            if let (read, drawn) = await agent.settings() {
                settings = read
                looks = drawn
                name = value("name")
            }
            // Started on one of its pages, when it is being tried out.
            step = min(max(UserDefaults.standard.integer(forKey: "page"), 0), steps - 1)
            // Asked for the microphone at once, when that is being tried out.
            if UserDefaults.standard.bool(forKey: "askvoice") {
                let heard = await AVAudioApplication.requestRecordPermission()
                let written = await withCheckedContinuation { asked in
                    SFSpeechRecognizer.requestAuthorization { asked.resume(returning: $0 == .authorized) }
                }
                voice = heard && written
            }
        }
    }

    /// One page of it: a picture, what it is about, and what there is to choose.
    private func page<Top: View, Rest: View>(
        title: String, told: String, @ViewBuilder top: () -> Top, @ViewBuilder rest: () -> Rest
    ) -> some View {
        ScrollView {
            VStack(spacing: 22) {
                top().frame(height: 170).padding(.top, 26)
                VStack(spacing: 10) {
                    Text(title).font(.largeTitle.bold()).multilineTextAlignment(.center)
                    Text(told).font(.body).foregroundStyle(.secondary).multilineTextAlignment(.center)
                }
                .padding(.horizontal, 26)
                rest().padding(.horizontal, 20)
            }
            .padding(.bottom, 20)
        }
        .scrollBounceBehavior(.basedOnSize)
    }

    private var hello: some View {
        page(
            title: say("This is \(agent.name)", "Voici \(agent.name)"),
            told: say(
                "Your own agent. It lives on your machine, remembers what matters, and works for you: here, and on every device you add.",
                "Ton agent à toi. Il vit sur ta machine, retient ce qui compte et travaille pour toi : ici, et sur chaque appareil que tu ajoutes.")
        ) {
            Figure(look: shown, working: false, size: 150)
        } rest: {
            EmptyView()
        }
    }

    private var look: some View {
        page(
            title: say("Make it yours", "À ton image"),
            told: say("Its name, its shape, its colour. You can change them whenever you like.", "Son nom, sa forme, sa couleur. Tu pourras les changer quand tu veux.")
        ) {
            Figure(look: shown, size: 130)
        } rest: {
            VStack(spacing: 18) {
                TextField(say("Name", "Nom"), text: $name)
                    .font(.title3.weight(.semibold)).multilineTextAlignment(.center)
                    .submitLabel(.done)
                    .padding(.vertical, 13).glassEffect(.regular, in: .capsule)
                LazyVGrid(columns: Array(repeating: GridItem(.flexible()), count: 6), spacing: 14) {
                    ForEach(looks.shapes.keys.sorted(), id: \.self) { shape in
                        if let drawn = looks.shapes[shape] {
                            Button { change("shape", to: shape) } label: {
                                Figure(look: Look(outline: drawn.outline, colour: colour, eyes: drawn.eyes), size: 42)
                                    .opacity(value("shape") == shape ? 1 : 0.45)
                            }
                            .buttonStyle(.plain)
                        }
                    }
                }
                HStack(spacing: 0) {
                    ForEach(looks.colours.keys.sorted(), id: \.self) { named in
                        Button { change("colour", to: named) } label: {
                            Circle().fill(Color(page: looks.colours[named] ?? "#f2f2f0")).frame(width: 24, height: 24)
                                .overlay {
                                    if value("colour") == named { Circle().stroke(.primary, lineWidth: 2).padding(-4) }
                                }
                                .frame(maxWidth: .infinity)
                        }
                        .buttonStyle(.plain)
                    }
                }
            }
        }
    }

    private var leave: some View {
        let levels: [(String, String, String, String, String)] = [
            ("careful", "Careful", "Prudent", "It asks before anything that changes something.", "Il demande avant tout ce qui change quelque chose."),
            ("balanced", "Balanced", "Équilibré", "It does the ordinary by itself, and asks for what matters.", "Il fait l'ordinaire tout seul, et demande pour ce qui compte."),
            ("free", "Free", "Libre", "It acts without asking, except for what cannot be undone.", "Il agit sans demander, sauf pour ce qui ne se rattrape pas."),
        ]
        return page(
            title: say("How much on its own?", "Jusqu'où seul ?"),
            told: say("What \(agent.name) may do without asking you first.", "Ce que \(agent.name) peut faire sans te demander d'abord.")
        ) {
            Image(systemName: "hand.raised.fill").font(.system(size: 76)).foregroundStyle(.secondary)
        } rest: {
            VStack(spacing: 10) {
                ForEach(levels, id: \.0) { level in
                    Button { change("permission", to: level.0) } label: {
                        HStack {
                            VStack(alignment: .leading, spacing: 3) {
                                Text(say(level.1, level.2)).font(.headline)
                                Text(say(level.3, level.4)).font(.footnote).foregroundStyle(.secondary)
                                    .multilineTextAlignment(.leading)
                            }
                            Spacer()
                            Image(systemName: value("permission") == level.0 ? "checkmark.circle.fill" : "circle")
                                .font(.title3).foregroundStyle(value("permission") == level.0 ? Color.primary : Color.secondary)
                        }
                        .padding(16)
                        .glassEffect(value("permission") == level.0 ? .regular.interactive() : .clear, in: .rect(cornerRadius: 18))
                    }
                    .buttonStyle(.plain)
                }
            }
        }
    }

    private var speaking: some View {
        page(
            title: say("Talk to it", "Parle-lui"),
            told: say(
                "Keep the microphone pressed and speak. What you say is written down on this phone, then sent as words.",
                "Maintiens le micro et parle. Ce que tu dis est transcrit sur ce téléphone, puis envoyé en texte.")
        ) {
            Image(systemName: "waveform").font(.system(size: 76)).foregroundStyle(.secondary)
                .symbolEffect(.variableColor.iterative, options: .repeating)
        } rest: {
            VStack(spacing: 12) {
                switch voice {
                case true?:
                    Label(say("The microphone is ready", "Le micro est prêt"), systemImage: "checkmark.circle.fill")
                        .font(.headline)
                case false?:
                    Text(say("Without the microphone you can still write. Allow it later in Settings.", "Sans le micro tu peux toujours écrire. Autorise-le plus tard dans Réglages."))
                        .font(.footnote).foregroundStyle(.secondary).multilineTextAlignment(.center)
                case nil:
                    Button {
                        Task {
                            let heard = await AVAudioApplication.requestRecordPermission()
                            let written = await withCheckedContinuation { asked in
                                SFSpeechRecognizer.requestAuthorization { asked.resume(returning: $0 == .authorized) }
                            }
                            voice = heard && written
                        }
                    } label: {
                        Label(say("Allow the microphone", "Autoriser le micro"), systemImage: "mic.fill")
                            .font(.headline).frame(maxWidth: .infinity).frame(height: 30)
                    }
                    .buttonStyle(.glass)
                }
            }
        }
    }

    private var services: some View {
        page(
            title: say("Connect your services", "Connecte tes services"),
            told: say(
                "\(agent.name) works where your things are. Sign in once; you can add or remove any of them later.",
                "\(agent.name) travaille là où sont tes affaires. Connecte-toi une fois ; tu pourras en ajouter ou en retirer plus tard.")
        ) {
            Image(systemName: "point.3.connected.trianglepath.dotted").font(.system(size: 76)).foregroundStyle(.secondary)
        } rest: {
            ConnectorsList()
        }
    }

    private var ready: some View {
        let ideas = [
            say("Remind me to call the dentist tomorrow at 9", "Rappelle-moi d'appeler le dentiste demain à 9 h"),
            say("Find a good recipe with what is in season", "Trouve une bonne recette avec ce qui est de saison"),
            say("What can you connect to, to help me?", "À quoi peux-tu te connecter pour m'aider ?"),
        ]
        return page(
            title: say("All set", "Tout est prêt"),
            told: say("Ask in your own words. A few ideas to begin with:", "Demande avec tes mots. Quelques idées pour commencer :")
        ) {
            Figure(look: shown, working: true, size: 130)
        } rest: {
            VStack(spacing: 10) {
                ForEach(ideas, id: \.self) { idea in
                    Text(idea).font(.callout).frame(maxWidth: .infinity, alignment: .leading)
                        .padding(.horizontal, 16).padding(.vertical, 13)
                        .glassEffect(.regular, in: .rect(cornerRadius: 16))
                }
                Text(say(
                    "To add a computer or another phone, ask \(agent.name) to add a device.",
                    "Pour ajouter un ordinateur ou un autre téléphone, demande à \(agent.name) d'ajouter un appareil."))
                    .font(.footnote).foregroundStyle(.secondary).multilineTextAlignment(.center).padding(.top, 6)
            }
        }
    }
}
