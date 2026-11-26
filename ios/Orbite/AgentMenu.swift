// The Agent's own menu, opened by touching its name: who it is, to give it
// a name and a look, and its screen, to see what it does on its computer
// or to take over.

import SwiftUI

struct AgentMenu: View {
    @Environment(Agent.self) private var agent
    @Environment(\.dismiss) private var dismiss
    @State private var customizing = false
    @State private var watching = false

    var body: some View {
        NavigationStack {
            List {
                Section {
                    VStack(spacing: 8) {
                        Figure(look: agent.look, working: agent.working, size: 92)
                        Text(agent.name).font(.title2.weight(.semibold))
                        Text(say("My agent", "Mon agent")).font(.subheadline).foregroundStyle(.secondary)
                    }
                    .frame(maxWidth: .infinity)
                    .padding(.vertical, 6)
                    .listRowBackground(Color.clear)
                }
                Section {
                    row("paintbrush.pointed", say("Customize", "Personnaliser"),
                        say("Its name, its shape, its colour.", "Son nom, sa forme, sa couleur.")) { customizing = true }
                    row("display", say("Screen", "Écran"),
                        say("See what it does on its computer, or take over.", "Voir ce qu'il fait sur son ordinateur, ou prendre la main.")) { watching = true }
                }
            }
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .topBarTrailing) {
                    Button(say("Done", "OK")) { dismiss() }
                }
            }
        }
        .presentationDetents([.medium, .large])
        .sheet(isPresented: $customizing) { SettingsView(part: .look) }
        .fullScreenCover(isPresented: $watching) { ScreenView() }
        .onAppear {
            if UserDefaults.standard.string(forKey: "show") == "screen" { watching = true }
        }
    }

    private func row(_ symbol: String, _ title: String, _ about: String, go: @escaping () -> Void) -> some View {
        Button {
            Haptic.touched()
            go()
        } label: {
            HStack(spacing: 14) {
                Image(systemName: symbol).font(.body.weight(.medium))
                    .frame(width: 36, height: 36)
                    .background(.quaternary, in: .rect(cornerRadius: 10))
                VStack(alignment: .leading, spacing: 2) {
                    Text(title)
                    Text(about).font(.footnote).foregroundStyle(.secondary)
                }
                Spacer()
                Image(systemName: "chevron.right").font(.footnote.weight(.semibold)).foregroundStyle(.tertiary)
            }
            .contentShape(.rect)
        }
        .tint(.primary)
    }
}

/// The Agent's computer as it is this moment, and the Owner's hand on it.
struct ScreenView: View {
    @Environment(Agent.self) private var agent
    @Environment(\.dismiss) private var dismiss
    @Environment(\.scenePhase) private var phase
    @State private var picture: UIImage?
    /// Nil until it was looked at once; then whether its computer is started.
    @State private var on: Bool?
    /// What taking it gave this phone, while it has it.
    @State private var holder: String?
    @State private var starting = false
    @State private var large = false
    @State private var text = ""
    @FocusState private var writing: Bool

    private var taken: Bool { holder != nil }

    var body: some View {
        NavigationStack {
            VStack(spacing: 16) {
                stage
                Text(taken
                    ? say("You have its mouse and keyboard: touch the screen to click, write below. \(agent.name) waits.",
                          "Tu as sa souris et son clavier : touche l'écran pour cliquer, écris en dessous. \(agent.name) attend.")
                    : say("You see what \(agent.name) sees. Take over to do something yourself, such as signing in.",
                          "Tu vois ce que voit \(agent.name). Prends la main pour faire quelque chose toi-même, comme te connecter à un compte."))
                    .font(.footnote).foregroundStyle(.secondary)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(.horizontal, 20)
                if taken { keyboard }
                Spacer(minLength: 0)
                Button {
                    Task { await taken ? give() : take() }
                } label: {
                    Group {
                        if starting { ProgressView() } else {
                            Text(taken ? say("Give it back to \(agent.name)", "Rendre la main à \(agent.name)") : say("Take over", "Prendre le contrôle"))
                                .font(.headline)
                        }
                    }
                    .frame(maxWidth: .infinity).padding(.vertical, 6)
                }
                .buttonStyle(.glassProminent)
                .tint(taken ? .secondary : .accentColor)
                .disabled(starting)
                .padding(.horizontal, 20).padding(.bottom, 12)
            }
            .navigationTitle(say("Screen", "Écran"))
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .topBarLeading) {
                    Button {
                        Task {
                            await give()
                            dismiss()
                        }
                    } label: { Image(systemName: "xmark") }
                }
                ToolbarItem(placement: .topBarTrailing) {
                    Button { withAnimation(.snappy) { large.toggle() } } label: {
                        Image(systemName: large ? "minus.magnifyingglass" : "plus.magnifyingglass")
                    }
                    .disabled(picture == nil)
                }
            }
        }
        .task { await follow() }
        .onChange(of: phase) { _, now in
            // Left for another app: its computer goes back to the Agent.
            if now == .background { Task { await give() } }
        }
    }

    /// The picture, whole across the phone or large enough to read and aim.
    private var stage: some View {
        GeometryReader { room in
            let across = large ? room.size.width * 2.4 : room.size.width - 24
            ScrollView(large ? [.horizontal, .vertical] : []) {
                Group {
                    if let picture {
                        let down = across * picture.size.height / picture.size.width
                        Image(uiImage: picture).resizable().interpolation(.high)
                            .frame(width: across, height: down)
                            .clipShape(.rect(cornerRadius: large ? 0 : 14))
                            .overlay {
                                RoundedRectangle(cornerRadius: large ? 0 : 14)
                                    .stroke(taken ? Color.accentColor : Color.secondary.opacity(0.3), lineWidth: taken ? 2 : 1)
                            }
                            .onTapGesture(coordinateSpace: .local) { at in
                                guard taken else { return }
                                Haptic.touched()
                                let x = Int(at.x / across * picture.size.width), y = Int(at.y / down * picture.size.height)
                                Task { await act(["do": "click", "x": max(0, x), "y": max(0, y)]) }
                            }
                    } else {
                        RoundedRectangle(cornerRadius: 14).fill(.quaternary)
                            .frame(width: across, height: across * 10 / 16)
                            .overlay {
                                if on == false {
                                    Text(say("Its computer is off: it starts when your agent needs it, or when you take over.",
                                             "Son ordinateur est éteint : il démarre quand ton agent en a besoin, ou quand tu prends la main."))
                                        .font(.footnote).foregroundStyle(.secondary)
                                        .multilineTextAlignment(.center).padding(24)
                                } else {
                                    ProgressView()
                                }
                            }
                    }
                }
                .padding(.horizontal, large ? 0 : 12)
            }
        }
        .frame(height: large ? 420 : (UIScreen.main.bounds.width - 24) * 10 / 16)
        .padding(.top, 8)
    }

    /// What a phone has no keys for: a field whose text goes to its screen,
    /// and the keys that are not letters.
    private var keyboard: some View {
        VStack(spacing: 10) {
            HStack(spacing: 8) {
                TextField(say("Type on its screen…", "Écrire sur son écran…"), text: $text)
                    .textInputAutocapitalization(.never).autocorrectionDisabled()
                    .focused($writing)
                    .submitLabel(.send)
                    .onSubmit { write(then: nil) }
                    .padding(.horizontal, 14).padding(.vertical, 10)
                    .glassEffect(.regular, in: .capsule)
                Button { write(then: nil) } label: {
                    Image(systemName: "arrow.up").font(.body.weight(.semibold)).frame(width: 40, height: 40)
                }
                .buttonStyle(.glassProminent).buttonBorderShape(.circle)
                .disabled(text.isEmpty)
            }
            HStack(spacing: 8) {
                key(say("Enter", "Entrée"), "return", "Return")
                key(say("Delete", "Effacer"), "delete.left", "BackSpace")
                key("Tab", "arrow.right.to.line", "Tab")
                key(say("Up", "Haut"), "chevron.up", "Page_Up")
                key(say("Down", "Bas"), "chevron.down", "Page_Down")
            }
        }
        .padding(.horizontal, 20)
    }

    private func key(_ label: String, _ symbol: String, _ name: String) -> some View {
        Button {
            Haptic.touched()
            write(then: name)
        } label: {
            Image(systemName: symbol).font(.callout.weight(.medium)).frame(maxWidth: .infinity).frame(height: 38)
        }
        .buttonStyle(.glass)
        .accessibilityLabel(label)
    }

    /// Types what is in the field, then presses `key` when one is given.
    private func write(then key: String?) {
        let typed = text
        text = ""
        Task {
            if !typed.isEmpty { await act(["do": "type", "text": typed]) }
            if let key { await act(["do": "key", "keys": key]) }
        }
    }

    private func take() async {
        starting = true
        let (status, given) = await agent.hand(["do": "take"])
        starting = false
        if status == 200, let given {
            holder = given
            Haptic.done()
            await look()
        } else {
            Haptic.wrong()
        }
    }

    private func give() async {
        guard let had = holder else { return }
        holder = nil
        writing = false
        _ = await agent.hand(["do": "give", "holder": had])
    }

    private func act(_ gesture: [String: Any]) async {
        guard let holder else { return }
        var gesture = gesture
        gesture["holder"] = holder
        let (status, _) = await agent.hand(gesture)
        // No longer this phone's: taken elsewhere, or left too long.
        if status == 409 { self.holder = nil }
        await look()
    }

    private func look() async {
        guard let seen = await agent.screenNow(held: holder) else { return }
        if seen.isEmpty {
            on = false
            picture = nil
        } else if let shown = UIImage(data: seen) {
            on = true
            picture = shown
        }
    }

    /// Follows the screen for as long as it is shown.
    private func follow() async {
        while !Task.isCancelled {
            if phase == .active { await look() }
            try? await Task.sleep(for: .milliseconds(taken ? 500 : 1300))
        }
    }
}
