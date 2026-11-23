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

    struct Attached: Identifiable, Equatable {
        let id = UUID()
        let name: String
        var token: String?
    }

    var body: some View {
        ScrollViewReader { scroll in
            ScrollView {
                LazyVStack(spacing: 12) {
                    ForEach(agent.entries) { entry in
                        EntryView(entry: entry).id(entry.id)
                    }
                    if let doing = agent.doing {
                        Text(doing).font(.footnote).foregroundStyle(.secondary)
                            .frame(maxWidth: .infinity, alignment: .leading).padding(.horizontal, 6)
                    }
                    Color.clear.frame(height: 1).id("end")
                }
                .padding(.horizontal, 14).padding(.top, 8)
            }
            .scrollDismissesKeyboard(.interactively)
            .defaultScrollAnchor(.bottom)
            .onChange(of: agent.entries.count) { withAnimation(.snappy) { scroll.scrollTo("end", anchor: .bottom) } }
            .onChange(of: agent.doing) { scroll.scrollTo("end", anchor: .bottom) }
        }
        .background(Color(page: "#0d0d0d").ignoresSafeArea())
        .safeAreaInset(edge: .top) { top }
        .safeAreaInset(edge: .bottom) { bottom }
        .photosPicker(isPresented: .constant(false), selection: $picked)
        .onChange(of: picked) { takePhotos() }
        .fileImporter(isPresented: $choosingFile, allowedContentTypes: [.item], allowsMultipleSelection: true) { chosen in
            if case .success(let files) = chosen { files.forEach(take(file:)) }
        }
    }

    // MARK: The top

    private var top: some View {
        HStack {
            Spacer()
            Menu {
                Button(say("Add a device", "Ajouter un appareil"), systemImage: "qrcode") {
                    Task { await agent.send("/pair", tapped: true) }
                }
                Button(say("Settings", "Réglages"), systemImage: "slider.horizontal.3") {
                    Task { await agent.send("/settings", tapped: true) }
                }
                Divider()
                Button(say("Another agent", "Un autre agent"), systemImage: "arrow.left.arrow.right", role: .destructive) {
                    agent.leave()
                }
            } label: {
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
            .tint(.primary)
            Spacer()
        }
        .padding(.top, 2).padding(.bottom, 6)
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
                            Image(systemName: "plus").font(.title3.weight(.medium)).frame(width: 48, height: 48)
                                .glassEffect(.regular.interactive(), in: .circle)
                        }
                        .tint(.primary)
                        TextField(say("Message \(agent.name)", "Écrire à \(agent.name)"), text: $text, axis: .vertical)
                            .lineLimit(1...6).focused($writing).font(.body)
                            .padding(.horizontal, 18).padding(.vertical, 13)
                            .glassEffect(.regular, in: .rect(cornerRadius: 24))
                    } else {
                        recordingBar
                    }
                    if canSend && held == nil {
                        Button(action: send) {
                            Image(systemName: "arrow.up").font(.title3.weight(.semibold)).frame(width: 48, height: 48)
                        }
                        .buttonStyle(.glassProminent).buttonBorderShape(.circle)
                    } else {
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

    /// While the microphone is held: how long, and how to give up.
    private var recordingBar: some View {
        HStack(spacing: 12) {
            Circle().fill(.red).frame(width: 10, height: 10)
            if let held {
                Text(held, style: .timer).monospacedDigit().font(.body.weight(.medium))
            }
            Spacer()
            Text(away
                ? say("Release to cancel", "Relâche pour annuler")
                : say("Slide left to cancel", "Glisse à gauche pour annuler"))
                .font(.footnote).foregroundStyle(away ? .red : .secondary)
        }
        .padding(.horizontal, 18).frame(height: 48)
        .glassEffect(.regular, in: .capsule)
    }

    /// Held, it records; let go, it sends; slid away, it gives up.
    private var microphone: some View {
        Image(systemName: "mic.fill").font(.title3.weight(.medium))
            .foregroundStyle(held == nil ? Color.primary : Color.white)
            .frame(width: 48, height: 48)
            .glassEffect(held == nil ? .regular.interactive() : .regular.tint(away ? .gray : .red), in: .circle)
            .scaleEffect(held == nil ? 1 : 1.35)
            .animation(.spring(duration: 0.25), value: held == nil)
            .gesture(
                DragGesture(minimumDistance: 0)
                    .onChanged { finger in
                        if held == nil {
                            held = Date()
                            away = false
                            UIImpactFeedbackGenerator(style: .medium).impactOccurred()
                            Task {
                                if await !recorder.begin() {
                                    held = nil
                                    agent.trouble = say(
                                        "Orbite may not use the microphone: allow it in Settings.",
                                        "Orbite n'a pas accès au micro : autorise-le dans Réglages.")
                                }
                            }
                        }
                        away = finger.translation.width < -80
                    }
                    .onEnded { _ in
                        guard let began = held else { return }
                        held = nil
                        let kept = recorder.end(keeping: !away)
                        if Date().timeIntervalSince(began) < 0.5 {
                            agent.trouble = say("Keep it pressed to record.", "Reste appuyé pour enregistrer.")
                            return
                        }
                        guard let (spoken, name) = kept else { return }
                        UIImpactFeedbackGenerator(style: .light).impactOccurred()
                        Task {
                            if let token = await agent.upload(spoken, named: name) {
                                await agent.send("", files: [token])
                            } else {
                                agent.trouble = say("That was not sent. Try again.", "Ce n'est pas parti. Réessaie.")
                            }
                        }
                    }
            )
            .accessibilityLabel(say("Hold to record a voice message", "Maintenir pour enregistrer un message vocal"))
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
                    Button(say("Allow", "Autoriser")) { answer(ask, "allow") }.buttonStyle(.glassProminent)
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
