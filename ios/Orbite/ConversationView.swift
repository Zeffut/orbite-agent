// The Conversation: what was said, under a top that floats in the
// system's glass, and the field to say more at the bottom.

import PhotosUI
import SwiftUI
import UniformTypeIdentifiers

struct ConversationView: View {
    @Environment(Agent.self) private var agent
    @State private var text = ""
    @State private var picked: [PhotosPickerItem] = []
    @State private var choosingFile = false
    @State private var attached: [Attached] = []
    @State private var held: Date?
    @State private var away = false
    @State private var recorder = Recorder()
    @FocusState private var writing: Bool
    @State private var place = ScrollPosition(edge: .bottom)
    @Environment(\.scenePhase) private var phase
    @State private var settings = false
    @State private var looking = false
    @State private var menu = false

    struct Attached: Identifiable, Equatable {
        let id = UUID()
        let name: String
        var token: String?
    }

    var body: some View {
        ScrollView {
                LazyVStack(spacing: 12) {
                    ForEach(agent.entries) { entry in
                        EntryView(entry: entry).id(entry.id)
                    }
                    ForEach(agent.leaving) { one in
                        LeavingView(one: one).id(one.id)
                            .transition(.move(edge: .bottom).combined(with: .opacity))
                    }
                    if agent.working {
                        Thinking(look: agent.look, doing: agent.doing)
                            .transition(.opacity.combined(with: .move(edge: .bottom)))
                    }
                    Color.clear.frame(height: 1).id("end")
                }
                .padding(.horizontal, 14).padding(.top, 8)
            }
            .scrollDismissesKeyboard(.interactively)
            .defaultScrollAnchor(.bottom)
            .scrollPosition($place)
            .onChange(of: agent.entries.count) { before, now in
                // The Conversation as it comes back on opening is gone to the
                // end of at once, and again once it has taken its place; what
                // is said after that is followed gently.
                if before == 0 || now - before > 1 {
                    place.scrollTo(edge: .bottom)
                    Task {
                        for wait in [60, 250, 600] {
                            try? await Task.sleep(for: .milliseconds(wait))
                            place.scrollTo(edge: .bottom)
                        }
                    }
                } else {
                    withAnimation(.snappy) { place.scrollTo(edge: .bottom) }
                }
            }
            .onChange(of: agent.leaving.count) { withAnimation(.snappy) { place.scrollTo(edge: .bottom) } }
            .animation(.snappy, value: agent.leaving)
            .onChange(of: agent.working) { withAnimation(.snappy) { place.scrollTo(edge: .bottom) } }
            .animation(.snappy, value: agent.working)
        .background(Color(page: "#0d0d0d").ignoresSafeArea())
        // What is said fades out under the top and the bottom, rather than
        // running into the clock and the field.
        .safeAreaInset(edge: .top, spacing: 0) {
            top.background(alignment: .top) { fade(from: .top) }
        }
        .safeAreaInset(edge: .bottom, spacing: 0) {
            bottom.background(alignment: .bottom) { fade(from: .bottom) }
        }
        .sheet(isPresented: $settings) { SettingsView() }
        .sheet(isPresented: $looking) { SettingsView(part: .look) }
        .overlay { Drawer(open: $menu) { settings = true } }
        .task {
            recorder.prepare()
            // Started on its settings, when it is being tried out.
            if UserDefaults.standard.string(forKey: "show") == "settings" { settings = true }
            if let draft = UserDefaults.standard.string(forKey: "draft") { text = draft }
            // Sends a recording it is handed as if it had just been said, when it is being tried out.
            if let handed = UserDefaults.standard.string(forKey: "hear"),
               let spoken = try? Data(contentsOf: URL(fileURLWithPath: handed)) {
                let voice = Leaving()
                agent.leaving.append(voice)
                await sendVoice(spoken, URL(fileURLWithPath: handed).lastPathComponent, shown: voice.id)
            }
            // Records by itself for that many seconds, when it is being tried out.
            let alone = UserDefaults.standard.double(forKey: "record")
            if alone > 0, recorder.allowed {
                try? await Task.sleep(for: .seconds(2))
                pressed()
                try? await Task.sleep(for: .seconds(alone))
                released()
            }
        }
        .onDisappear { recorder.rest() }
        // The microphone is kept ready while the app is in front, and let go behind.
        .onChange(of: phase) {
            if phase == .active { recorder.prepare() } else if phase == .background { recorder.rest() }
        }
        .photosPicker(isPresented: .constant(false), selection: $picked)
        .onChange(of: picked) { takePhotos() }
        .fileImporter(isPresented: $choosingFile, allowedContentTypes: [.item], allowsMultipleSelection: true) { chosen in
            if case .success(let files) = chosen { files.forEach(take(file:)) }
        }
    }

    // MARK: The top

    /// The page's own dark, full at an edge of the screen and gone a
    /// little past the bar there.
    private func fade(from edge: VerticalEdge) -> some View {
        let dark = Color(page: "#0d0d0d")
        return LinearGradient(
            stops: [.init(color: dark, location: 0), .init(color: dark.opacity(0.92), location: 0.55), .init(color: dark.opacity(0), location: 1)],
            startPoint: edge == .top ? .top : .bottom, endPoint: edge == .top ? .bottom : .top)
            .padding(edge == .top ? .bottom : .top, -22)
            .ignoresSafeArea(edges: edge == .top ? .top : .bottom)
            .allowsHitTesting(false)
    }

    /// The menu on the left, and the Agent in the middle: touched, it is
    /// who the Agent is that opens, to give it a name and a look.
    private var top: some View {
        HStack {
            Button { withAnimation(.snappy(duration: 0.28)) { menu = true } } label: {
                Image(systemName: "line.3.horizontal").font(.title3.weight(.medium))
                    .frame(width: 46, height: 46)
                    .glassEffect(.regular.interactive(), in: .circle)
            }
            .accessibilityLabel(say("Menu", "Menu"))
            Spacer()
            Button { looking = true } label: {
                HStack(spacing: 9) {
                    Figure(look: agent.look, working: agent.working, size: 30)
                    Text(agent.name).font(.headline)
                    if !agent.reached {
                        Image(systemName: "wifi.slash").font(.footnote).foregroundStyle(.orange)
                    }
                }
                .padding(.leading, 10).padding(.trailing, 18).padding(.vertical, 8)
                .glassEffect(.regular.interactive(), in: .capsule)
            }
            Spacer()
            // As wide as the menu's button, so that the Agent sits in the middle.
            Color.clear.frame(width: 46, height: 46)
        }
        .tint(.primary)
        .padding(.horizontal, 12).padding(.top, 2).padding(.bottom, 6)
    }

    // MARK: The bottom

    private var bottom: some View {
        VStack(spacing: 8) {
            if let trouble = agent.trouble {
                Text(trouble).font(.footnote).foregroundStyle(.red).frame(maxWidth: .infinity, alignment: .leading)
            }
            if !attached.isEmpty {
                ScrollView(.horizontal, showsIndicators: false) {
                    HStack {
                        ForEach(attached) { file in
                            HStack(spacing: 6) {
                                if file.token == nil { ProgressView().controlSize(.mini) }
                                Text(file.name).lineLimit(1).font(.footnote)
                                Button { attached.removeAll { $0.id == file.id } } label: {
                                    Image(systemName: "xmark").font(.caption2.bold())
                                }
                            }
                            .padding(.horizontal, 12).padding(.vertical, 8)
                            .glassEffect(.regular, in: .capsule)
                        }
                    }
                }
            }
            GlassEffectContainer(spacing: 10) {
                HStack(alignment: .bottom, spacing: 10) {
                    if held == nil {
                        Menu {
                            PhotosPicker(selection: $picked, matching: .any(of: [.images, .videos])) {
                                Label(say("Photos", "Photos"), systemImage: "photo.on.rectangle")
                            }
                            Button(say("Files", "Fichiers"), systemImage: "folder") { choosingFile = true }
                        } label: {
                            Image(systemName: "plus").font(.title3.weight(.medium)).frame(width: 46, height: 46)
                                .glassEffect(.regular.interactive(), in: .circle)
                        }
                        .tint(.primary)
                        HStack(alignment: .bottom, spacing: 6) {
                            TextField(say("Message \(agent.name)", "Écrire à \(agent.name)"), text: $text, axis: .vertical)
                                .lineLimit(1...6).focused($writing).font(.body)
                                .padding(.leading, 18).padding(.vertical, 12)
                            // Sending sits in the field, the size of a letter's line.
                            if canSend {
                                Button(action: send) {
                                    Image(systemName: "arrow.up").font(.subheadline.weight(.bold))
                                        .foregroundStyle(Color(page: "#0d0d0d"))
                                        .frame(width: 32, height: 32)
                                        .background(Color(page: "#f2f2f0"), in: .circle)
                                }
                                .padding(.trailing, 7).padding(.bottom, 7)
                                .transition(.scale.combined(with: .opacity))
                            } else {
                                Color.clear.frame(width: 12, height: 1)
                            }
                        }
                        .glassEffect(.regular, in: .rect(cornerRadius: 24))
                        .animation(.spring(duration: 0.2), value: canSend)
                    } else {
                        recordingBar
                    }
                    if !(canSend && held == nil) {
                        microphone
                    }
                }
            }
        }
        .padding(.horizontal, 12).padding(.top, 6).padding(.bottom, 6)
    }

    private var canSend: Bool {
        !text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            || (!attached.isEmpty && attached.allSatisfy { $0.token != nil })
    }

    /// While the microphone is held: how long, the voice as it is heard,
    /// and how to give up.
    private var recordingBar: some View {
        HStack(spacing: 12) {
            Circle().fill(.red).frame(width: 10, height: 10)
                .opacity(away ? 0.3 : 1)
            if let held {
                Text(held, style: .timer).monospacedDigit().font(.body.weight(.medium))
                    .frame(minWidth: 44, alignment: .leading)
            }
            if away {
                Spacer()
                Text(say("Release to cancel", "Relâche pour annuler"))
                    .font(.footnote.weight(.medium)).foregroundStyle(.red)
            } else {
                HStack(spacing: 3) {
                    ForEach(Array(recorder.levels.enumerated()), id: \.offset) { _, level in
                        Capsule().fill(.primary.opacity(0.85)).frame(width: 3, height: 4 + 26 * level)
                    }
                }
                .frame(maxWidth: .infinity, alignment: .trailing)
                .animation(.linear(duration: 0.06), value: recorder.levels)
                Image(systemName: "chevron.left").font(.caption.weight(.semibold)).foregroundStyle(.secondary)
            }
        }
        .padding(.horizontal, 18).frame(height: 46)
        .glassEffect(.regular, in: .capsule)
        .transition(.opacity.combined(with: .scale(scale: 0.96, anchor: .trailing)))
    }

    /// Held, it records; let go, it sends; slid away, it gives up. It
    /// records from the instant it is touched.
    private var microphone: some View {
        Image(systemName: "mic.fill").font(.title3.weight(.medium))
            .foregroundStyle(held == nil ? Color.primary : Color.white)
            .frame(width: 46, height: 46)
            .glassEffect(held == nil ? .regular : .regular.tint(away ? .gray : .red), in: .circle)
            .scaleEffect(held == nil ? 1 : (away ? 1.1 : 1.4))
            .animation(.spring(duration: 0.16, bounce: 0.3), value: held == nil)
            .animation(.spring(duration: 0.2), value: away)
            // Touched, not gestured: a gesture waits to know what it is, and
            // recording is to start the instant the finger lands.
            .overlay {
                Touch(
                    began: { pressed() },
                    moved: { across in
                        guard held != nil else { return }
                        let now = across < -90
                        if now != away {
                            away = now
                            recorder.crossed()
                        }
                    },
                    ended: { released() })
            }
            .accessibilityLabel(say("Hold to record a voice message", "Maintenir pour enregistrer un message vocal"))
    }

    /// The finger landed on the microphone.
    private func pressed() {
        guard held == nil else { return }
        // The first touch ever asks for the microphone: nothing is recorded yet.
        guard recorder.allowed else {
            Task {
                if await !recorder.ask() {
                    agent.trouble = say(
                        "Orbite may not use the microphone: allow it in Settings.",
                        "Orbite n'a pas accès au micro : autorise-le dans Réglages.")
                }
            }
            return
        }
        held = Date()
        away = false
        recorder.begin()
        agent.trouble = nil
        writing = false
    }

    /// The finger left the microphone.
    private func released() {
        guard held != nil else { return }
        let keeping = !away
        held = nil
        away = false
        Task {
            guard let (spoken, name) = await recorder.end(keeping: keeping) else {
                if keeping {
                    agent.trouble = say("Keep it pressed to record.", "Reste appuyé pour enregistrer.")
                }
                return
            }
            // Shown as leaving at once. Written down on the phone itself, what
            // was said goes as words: the Agent reads it whatever it thinks with.
            let voice = Leaving()
            agent.leaving.append(voice)
            await sendVoice(spoken, name, shown: voice.id)
        }
    }

    /// Sends what was recorded: as words when the phone could write
    /// them down, as the recording itself when it could not; and not at
    /// all when there was nothing to make out.
    private func sendVoice(_ spoken: Data, _ name: String, shown: UUID) async {
        switch await Recorder.written(from: spoken, named: name) {
        case .said(let said):
            await agent.send(said, shown: shown)
        case .nothing:
            agent.leaving.removeAll { $0.id == shown }
            agent.trouble = say("I did not catch anything. Try again, closer to the phone.",
                                "Je n'ai rien entendu. Réessaie, plus près du téléphone.")
        case .unable:
            if let token = await agent.upload(spoken, named: name) {
                await agent.send("", files: [token], shown: shown)
            } else {
                agent.leaving.removeAll { $0.id == shown }
                agent.trouble = say("That was not sent. Try again.", "Ce n'est pas parti. Réessaie.")
            }
        }
    }

    private func send() {
        let saying = text.trimmingCharacters(in: .whitespacesAndNewlines)
        let files = attached.compactMap(\.token)
        text = ""
        attached = []
        UIImpactFeedbackGenerator(style: .light).impactOccurred()
        Task { await agent.send(saying, files: files) }
    }

    private func takePhotos() {
        let chosen = picked
        picked = []
        for item in chosen {
            let name = "photo-\(Int(Date().timeIntervalSince1970 * 1000)).\(item.supportedContentTypes.first?.preferredFilenameExtension ?? "jpg")"
            let waiting = Attached(name: name)
            attached.append(waiting)
            Task {
                if let content = try? await item.loadTransferable(type: Data.self) {
                    await uploaded(waiting, content)
                } else {
                    attached.removeAll { $0.id == waiting.id }
                }
            }
        }
    }

    private func take(file: URL) {
        let waiting = Attached(name: file.lastPathComponent)
        attached.append(waiting)
        Task {
            let allowed = file.startAccessingSecurityScopedResource()
            defer { if allowed { file.stopAccessingSecurityScopedResource() } }
            if let content = try? Data(contentsOf: file) {
                await uploaded(waiting, content)
            } else {
                attached.removeAll { $0.id == waiting.id }
            }
        }
    }

    private func uploaded(_ waiting: Attached, _ content: Data) async {
        let token = await agent.upload(content, named: waiting.name)
        if let place = attached.firstIndex(where: { $0.id == waiting.id }) {
            if let token { attached[place].token = token } else { attached.remove(at: place) }
        }
        if token == nil { agent.trouble = say("A file did not go through.", "Un fichier n'est pas passé.") }
    }
}

// MARK: What was said

/// What the Owner just said, on its way: its words, or a voice being
/// written down.
struct LeavingView: View {
    let one: Leaving

    var body: some View {
        Group {
            if let text = one.text, !text.isEmpty {
                Text(text).font(.body)
            } else {
                HStack(spacing: 4) {
                    Image(systemName: "waveform").font(.body.weight(.medium))
                        .symbolEffect(.variableColor.iterative, options: .repeating)
                    Text(say("Voice message", "Message vocal")).font(.body)
                }
                .foregroundStyle(.secondary)
            }
        }
        .padding(.horizontal, 16).padding(.vertical, 11)
        .background(Color(page: "#2b2b2b"), in: .rect(cornerRadius: 22))
        .opacity(0.75)
        .frame(maxWidth: .infinity, alignment: .trailing).padding(.leading, 44)
    }
}

struct EntryView: View {
    @Environment(Agent.self) private var agent
    let entry: Entry

    var body: some View {
        switch entry.kind {
        case .owner(let text, let files):
            VStack(alignment: .trailing, spacing: 6) {
                ForEach(files, id: \.self) { file in FileView(file: file) }
                if !text.isEmpty {
                    Text(text).font(.body).textSelection(.enabled)
                        .padding(.horizontal, 16).padding(.vertical, 11)
                        .background(Color(page: "#2b2b2b"), in: .rect(cornerRadius: 22))
                }
            }
            .frame(maxWidth: .infinity, alignment: .trailing).padding(.leading, 44)
        case .agent(let text, let choices):
            VStack(alignment: .leading, spacing: 10) {
                Written(text: text)
                ForEach(choices, id: \.self) { choice in
                    Button(choice.label) { Task { await agent.send(choice.command, tapped: true) } }
                        .buttonStyle(.glass)
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading).padding(.trailing, 20).padding(.horizontal, 4)
        case .ask(let ask):
            VStack(alignment: .leading, spacing: 10) {
                Text(ask.summary).font(.headline)
                if !ask.detail.isEmpty {
                    Text(ask.detail).font(.footnote.monospaced()).foregroundStyle(.secondary)
                }
                if ask.irreversible {
                    Text(say("This cannot be undone.", "Cela ne peut pas être annulé.")).font(.footnote).foregroundStyle(.orange)
                }
                HStack {
                    Button { answer(ask, "allow") } label: {
                        Text(say("Allow", "Autoriser")).foregroundStyle(Color(page: "#0d0d0d"))
                    }
                    .buttonStyle(.glassProminent)
                    if !ask.critical {
                        Button(say("Always", "Toujours")) { answer(ask, "always") }.buttonStyle(.glass)
                    }
                    Spacer()
                    Button(say("Deny", "Refuser"), role: .destructive) { answer(ask, "deny") }.buttonStyle(.glass)
                }
            }
            .padding(16).frame(maxWidth: .infinity, alignment: .leading)
            .glassEffect(.regular.tint(ask.critical ? .red.opacity(0.25) : .clear), in: .rect(cornerRadius: 22))
        case .settled(let summary, let answer):
            let told = [
                "allow": say("allowed", "autorisé"), "always": say("always allowed", "toujours autorisé"),
                "deny": say("denied", "refusé"), "wrote": say("answered", "répondu"),
            ][answer] ?? answer
            Text("\(summary) · \(told)").font(.footnote).foregroundStyle(.secondary)
                .frame(maxWidth: .infinity, alignment: .center)
        case .file(let file, let caption):
            VStack(alignment: .leading, spacing: 6) {
                FileView(file: file)
                if !caption.isEmpty { Text(caption).font(.footnote).foregroundStyle(.secondary) }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
        }
    }

    private func answer(_ ask: Ask, _ answer: String) {
        UIImpactFeedbackGenerator(style: .medium).impactOccurred()
        Task { await agent.answer(ask.ask, answer) }
    }
}

/// What the Agent wrote, with its lists, titles, bold and links.
struct Written: View {
    let text: String

    private enum Block: Hashable {
        case line(String)
        case item(mark: String, text: String)
        case title(String)
        case code(String)
    }

    private var blocks: [Block] {
        var blocks: [Block] = []
        for (place, part) in text.components(separatedBy: "```").enumerated() {
            if place % 2 == 1 {
                let code = part.drop { $0 != "\n" }.trimmingCharacters(in: .newlines)
                blocks.append(.code(code.isEmpty ? part : code))
                continue
            }
            var number = 0
            for line in part.components(separatedBy: "\n") {
                let trimmed = line.trimmingCharacters(in: .whitespaces)
                if trimmed.isEmpty { number = 0; continue }
                if let item = trimmed.wholeMatch(of: /[-*•]\s+(.*)/) {
                    blocks.append(.item(mark: "•", text: String(item.1)))
                } else if let item = trimmed.wholeMatch(of: /(\d+)[.)]\s+(.*)/) {
                    number += 1
                    blocks.append(.item(mark: "\(item.1).", text: String(item.2)))
                } else if let title = trimmed.wholeMatch(of: /#{1,4}\s+(.*)/) {
                    blocks.append(.title(String(title.1)))
                } else {
                    blocks.append(.line(trimmed))
                }
            }
        }
        return blocks
    }

    private func inline(_ text: String) -> AttributedString {
        (try? AttributedString(markdown: text, options: .init(interpretedSyntax: .inlineOnlyPreservingWhitespace)))
            ?? AttributedString(text)
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 9) {
            ForEach(Array(blocks.enumerated()), id: \.offset) { _, block in
                switch block {
                case .line(let line):
                    Text(inline(line))
                case .title(let title):
                    Text(inline(title)).font(.title3.bold()).padding(.top, 4)
                case .item(let mark, let text):
                    HStack(alignment: .firstTextBaseline, spacing: 10) {
                        Text(mark).foregroundStyle(.secondary).frame(minWidth: 18, alignment: .trailing)
                        Text(inline(text))
                    }
                case .code(let code):
                    ScrollView(.horizontal, showsIndicators: false) {
                        Text(code).font(.footnote.monospaced()).padding(12)
                    }
                    .background(Color(page: "#161616"), in: .rect(cornerRadius: 12))
                }
            }
        }
        .font(.body).lineSpacing(3).textSelection(.enabled).tint(Color(page: "#4c8dff"))
    }
}

/// A file of the Conversation: shown when it is a picture, named otherwise.
struct FileView: View {
    @Environment(Agent.self) private var agent
    let file: Sent
    @State private var picture: UIImage?

    var body: some View {
        Group {
            if file.image {
                if let picture {
                    Image(uiImage: picture).resizable().scaledToFit()
                        .frame(maxWidth: 260, maxHeight: 320).clipShape(.rect(cornerRadius: 16))
                } else {
                    RoundedRectangle(cornerRadius: 16).fill(Color(page: "#1a1a1a")).frame(width: 200, height: 140)
                        .overlay { ProgressView() }
                }
            } else {
                let spoken = file.name.hasPrefix("voice-")
                Label(spoken ? say("Voice message", "Message vocal") : file.name, systemImage: spoken ? "waveform" : "doc")
                    .font(.callout).padding(.horizontal, 14).padding(.vertical, 10)
                    .background(Color(page: "#1a1a1a"), in: .capsule)
            }
        }
        .task(id: file.file) {
            guard file.image, picture == nil, let content = await agent.content(of: file.file) else { return }
            picture = UIImage(data: content)
        }
    }
}

/// The Agent at work: itself, swaying, and in a few words what it is
/// doing, with a light passing over them.
struct Thinking: View {
    let look: Look?
    let doing: String?

    /// The first few words of what is being done: enough to know, at a glance.
    private var words: String {
        guard let doing, !doing.trimmingCharacters(in: .whitespaces).isEmpty else {
            return say("Thinking", "Réfléchit")
        }
        // Words only: the mark a step comes with is the Agent itself, here.
        let all = doing.split(whereSeparator: \.isWhitespace).filter { $0.contains(where: \.isLetter) || $0.contains(where: \.isNumber) }
        guard !all.isEmpty else { return say("Thinking", "Réfléchit") }
        return all.prefix(4).joined(separator: " ") + (all.count > 4 ? "…" : "")
    }

    var body: some View {
        HStack(spacing: 10) {
            Figure(look: look, working: true, size: 28)
            // One text, lit by a light that goes across it: nothing laid over it.
            TimelineView(.animation) { now in
                let along = now.date.timeIntervalSinceReferenceDate.truncatingRemainder(dividingBy: 1.8) / 1.8
                let middle = -0.3 + 1.6 * along
                Text(words).font(.callout.weight(.medium))
                    .foregroundStyle(
                        LinearGradient(
                            stops: [
                                .init(color: .white.opacity(0.45), location: 0),
                                .init(color: .white.opacity(0.45), location: max(0, min(1, middle - 0.22))),
                                .init(color: .white, location: max(0, min(1, middle))),
                                .init(color: .white.opacity(0.45), location: max(0, min(1, middle + 0.22))),
                                .init(color: .white.opacity(0.45), location: 1),
                            ], startPoint: .leading, endPoint: .trailing))
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading).padding(.horizontal, 4).padding(.vertical, 4)
    }
}
