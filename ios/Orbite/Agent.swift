// The Agent as the app knows it: where it is reached, what it has said, and
// how to say something to it. Everything goes through the interface the
// Agent serves, as its own page does.

import Foundation
import UIKit
import Observation
import SwiftUI

/// One thing shown in the Conversation.
struct Entry: Identifiable, Equatable {
    enum Kind: Equatable {
        case owner(text: String, files: [Sent])
        case agent(text: String, choices: [Choice])
        case ask(Ask)
        case settled(summary: String, answer: String)
        case file(Sent, caption: String)
    }
    let id: String
    var kind: Kind
    var at: Date?
}

/// A sign-in the Agent opened: the service, what it is called, and where.
struct SignIn: Identifiable, Equatable {
    let name: String
    let label: String
    let link: URL
    var id: String { link.absoluteString }
}

/// Something the Owner said that is on its way: its words, or none yet
/// while a voice is being written down.
struct Leaving: Identifiable, Equatable {
    let id = UUID()
    var text: String?
}

struct Sent: Equatable, Hashable {
    let file: String
    let name: String
    let image: Bool
}

struct Choice: Equatable, Hashable {
    let label: String
    let command: String
}

struct Ask: Equatable {
    let ask: String
    let summary: String
    let detail: String
    let critical: Bool
    let irreversible: Bool
}

/// How the Agent looks: its outline in a square of 100, its colour, and
/// how far down its eyes sit.
struct Look: Equatable {
    let outline: String
    let colour: String
    let eyes: Double
}

/// An Agent this phone was let in by: where it is, and how it looked last.
struct Known: Codable, Equatable, Identifiable {
    var address: String
    var name: String
    var outline: String?
    var colour: String?
    var eyes: Double?
    var id: String { address }
    var look: Look? {
        outline.map { Look(outline: $0, colour: colour ?? "#f2f2f0", eyes: eyes ?? 36) }
    }
}

/// One setting of the Agent: what it is set to, and what it can be set to.
struct Setting: Identifiable, Equatable {
    let key: String
    var value: String
    let choices: [String]
    var id: String { key }
}

/// How each shape and colour is drawn, as the Agent says.
struct Looks: Equatable {
    var shapes: [String: (outline: String, eyes: Double)] = [:]
    var colours: [String: String] = [:]
    /// The models the Agent can think with: what to set, and what to show.
    var models: [Model] = []
    /// The model Orbite takes unasked, among them.
    var usual: String?
    static func == (one: Looks, other: Looks) -> Bool {
        one.colours == other.colours && one.shapes.keys.sorted() == other.shapes.keys.sorted()
            && one.models == other.models
    }

    struct Model: Identifiable, Equatable {
        let value: String
        let label: String
        var id: String { value }
    }
}

@Observable
@MainActor
final class Agent {
    /// Every Agent this phone knows, and whether one more is being added.
    var known: [Known] = []
    var adding = false
    /// True until the Owner has been shown round this Agent, on any device.
    var welcoming = false
    var paired = false
    var name = "Orbite"
    var look: Look?
    var entries: [Entry] = []
    /// A service the Agent asks its Owner to sign in to, now: opened at
    /// once, over the Conversation.
    var signing: SignIn?
    /// True once the Conversation as it was kept has been read.
    var loaded = false
    /// What is being read back of the Conversation, before it is shown:
    /// shown entry by entry, it would be seen going by.
    @ObservationIgnored private var read: [Entry] = []
    @ObservationIgnored private var reading = false
    /// True while entries are taken that are not news: nothing is felt for them.
    @ObservationIgnored private var quiet = false
    /// What an entry is added to: what is shown, or what is being read back.
    private var shown: [Entry] {
        get { reading ? read : entries }
        set { if reading { read = newValue } else { entries = newValue } }
    }
    /// True while there may be more of the Conversation further back.
    var further = true
    /// The last entry read, as the Agent numbers them: where to take the
    /// Conversation up again, rather than read it all anew.
    private var seen: Int?
    /// What the Owner just said, shown at once, until the Agent has it.
    var leaving: [Leaving] = []
    var doing: String?
    var working = false
    var reached = true
    var trouble: String? {
        didSet { if trouble != nil, trouble != oldValue { Haptic.wrong() } }
    }

    private(set) var address: URL? {
        didSet { UserDefaults.standard.set(address?.absoluteString, forKey: "agent") }
    }

    private func keep() {
        if let written = try? JSONEncoder().encode(known) {
            UserDefaults.standard.set(written, forKey: "agents")
        }
    }

    /// Writes down the Agent now open among those this phone knows.
    private func remember() {
        guard let address else { return }
        let now = Known(
            address: address.absoluteString, name: name, outline: look?.outline, colour: look?.colour,
            eyes: look?.eyes)
        if let place = known.firstIndex(where: { $0.address == now.address }) {
            known[place] = now
        } else {
            known.append(now)
        }
        keep()
    }

    /// Goes over to another Agent this phone knows.
    func open(_ other: Known) async {
        guard let to = URL(string: other.address), to != address else { return }
        listening?.cancel()
        entries = []
        loaded = false
        seen = nil
        further = true
        doing = nil
        working = false
        name = other.name
        look = other.look
        address = to
        await enter()
    }

    /// Forgets one Agent, on this phone alone; another it knows is opened.
    func forget(_ gone: Known) async {
        known.removeAll { $0.address == gone.address }
        keep()
        guard gone.address == address?.absoluteString else { return }
        listening?.cancel()
        entries = []
        loaded = false
        seen = nil
        further = true
        if let next = known.first, let to = URL(string: next.address) {
            name = next.name
            look = next.look
            address = to
            await enter()
        } else {
            paired = false
            address = nil
            UserDefaults.standard.set(false, forKey: "paired")
        }
    }
    private var listening: Task<Void, Never>?
    private let session: URLSession = {
        let settings = URLSessionConfiguration.default
        settings.httpCookieStorage = .shared
        settings.timeoutIntervalForRequest = 30
        settings.timeoutIntervalForResource = 60 * 60 * 24
        return URLSession(configuration: settings)
    }()

    /// Opens the Conversation again where the app was pointed last.
    func start() async {
        if let written = UserDefaults.standard.data(forKey: "agents"),
           let kept = try? JSONDecoder().decode([Known].self, from: written) {
            known = kept
        }
        // Given a link as it is started (by a link opened, or when tried out).
        if let link = UserDefaults.standard.string(forKey: "link") {
            await pair(with: link)
            return
        }
        guard let kept = UserDefaults.standard.string(forKey: "agent"), let kept = URL(string: kept)
        else { return }
        address = kept
        await enter()
    }

    /// A request to the Agent's interface, made as its own page makes them.
    private func request(_ path: String, method: String = "GET", body: Data? = nil, kind: String? = "application/json") -> URLRequest? {
        guard let address, let to = URL(string: path, relativeTo: address) else { return nil }
        var request = URLRequest(url: to)
        request.httpMethod = method
        request.setValue("1", forHTTPHeaderField: "x-orbite")
        var origin = "\(address.scheme ?? "https")://\(address.host ?? "")"
        if let port = address.port { origin += ":\(port)" }
        request.setValue(origin, forHTTPHeaderField: "Origin")
        if let body {
            request.httpBody = body
            if let kind { request.setValue(kind, forHTTPHeaderField: "content-type") }
        }
        return request
    }

    /// Lets this device in with the link the Agent gave: its address, and
    /// after the `#` the code that works once.
    func pair(with link: String) async {
        trouble = nil
        let link = link.trimmingCharacters(in: .whitespacesAndNewlines)
        guard let read = URLComponents(string: link), let host = read.host, !host.isEmpty,
              let scheme = read.scheme, scheme == "https" || scheme == "http"
        else {
            trouble = say("That is not a link from your agent.", "Ce n'est pas un lien de ton agent.")
            return
        }
        var base = URLComponents()
        base.scheme = scheme
        base.host = host
        base.port = read.port
        base.path = "/"
        // Should this one not let the phone in, the one open stays open.
        let before = (address, name, look, paired)
        defer {
            if !paired, before.3 {
                (address, name, look, paired) = before
                listen()
            }
        }
        listening?.cancel()
        address = base.url
        let code = read.fragment ?? ""
        if !code.isEmpty {
            let named = UIDevice.current.name
            let body = try? JSONSerialization.data(withJSONObject: ["code": code, "name": named])
            guard let asking = request("/api/pair", method: "POST", body: body),
                  let (_, answer) = try? await session.data(for: asking),
                  (answer as? HTTPURLResponse)?.statusCode == 200
            else {
                trouble = say(
                    "That link no longer works: ask your agent for a new one.",
                    "Ce lien ne marche plus : demande-en un nouveau à ton agent.")
                return
            }
        }
        paired = false
        await enter()
        if paired {
            entries = []
        loaded = false
        seen = nil
        further = true
            adding = false
        } else if trouble == nil {
            trouble = say("Your agent did not let this device in.", "Ton agent n'a pas laissé entrer cet appareil.")
        }
    }

    /// Finds out who the Agent is, then listens to it.
    private func enter() async {
        guard let asking = request("/api/me"), let (told, answer) = try? await session.data(for: asking) else {
            // Kept, and not reached right now: the Conversation opens when it is.
            paired = address != nil && UserDefaults.standard.bool(forKey: "paired")
            reached = false
            if paired { listen() }
            return
        }
        guard (answer as? HTTPURLResponse)?.statusCode == 200,
              let me = try? JSONSerialization.jsonObject(with: told) as? [String: Any]
        else {
            paired = false
            UserDefaults.standard.set(false, forKey: "paired")
            return
        }
        name = me["agent"] as? String ?? "Orbite"
        // An Agent that says nothing of it is older than being shown round.
        welcoming = (me["welcomed"] as? Bool) == false
        if let look = me["look"] as? [String: Any], let outline = look["outline"] as? String {
            self.look = Look(
                outline: outline, colour: look["colour"] as? String ?? "#f2f2f0",
                eyes: look["eyes"] as? Double ?? 36)
        }
        paired = true
        reached = true
        UserDefaults.standard.set(true, forKey: "paired")
        remember()
        listen()
    }

    /// Listens to what the Agent says, for as long as the app is open,
    /// taking the Conversation up again whenever the line drops.
    private func listen() {
        listening?.cancel()
        listening = Task { [weak self] in
            while !Task.isCancelled {
                // Taken up where it was left: only what was missed is read.
                guard let self,
                      let asking = self.request(self.seen.map { "/api/events?after=\($0)" } ?? "/api/events")
                else { return }
                do {
                    let (lines, answer) = try await self.session.bytes(for: asking)
                    if (answer as? HTTPURLResponse)?.statusCode == 401 {
                        self.paired = false
                        UserDefaults.standard.set(false, forKey: "paired")
                        return
                    }
                    self.reached = true
                    // Read from its start, the Conversation is shown once it is
                    // all there; taken up where it was left, as it comes.
                    if self.seen == nil {
                        self.read = []
                        self.reading = true
                    }
                    for try await line in lines.lines {
                        guard line.hasPrefix("data:") else { continue }
                        let said = line.dropFirst(5).trimmingCharacters(in: .whitespaces)
                        if let event = try? JSONSerialization.jsonObject(with: Data(said.utf8)) as? [String: Any] {
                            self.took(event)
                        }
                    }
                } catch {}
                if Task.isCancelled { return }
                self.reached = false
                try? await Task.sleep(for: .seconds(2))
            }
        }
    }

    /// Takes one event of the Conversation.
    private func took(_ event: [String: Any]) {
        let id = (event["id"] as? Int).map(String.init) ?? UUID().uuidString
        if let number = event["id"] as? Int, number > (seen ?? 0) { seen = number }
        let at = (event["at"] as? Double).map { Date(timeIntervalSince1970: $0) }
        let sent: ([String: Any]) -> Sent? = { file in
            guard let token = file["file"] as? String else { return nil }
            return Sent(file: token, name: file["name"] as? String ?? "", image: file["image"] as? Bool ?? false)
        }
        switch event["kind"] as? String {
        case "sign-in":
            // Asked for in the Conversation: opened here and now, not kept.
            if !reading, !quiet, let written = event["link"] as? String, let link = URL(string: written),
               link.scheme == "https" {
                let name = event["name"] as? String ?? ""
                signing = SignIn(name: name, label: event["label"] as? String ?? name, link: link)
                Haptic.asked()
            }
        case "caught-up":
            // All that was kept has come: it is shown in one go, at its end.
            if reading {
                reading = false
                entries = read
                read = []
            }
            loaded = true
        case "owner":
            let files = (event["files"] as? [[String: Any]] ?? []).compactMap(sent)
            // It has it: what was shown as leaving is this.
            let said = event["text"] as? String ?? ""
            if let place = leaving.firstIndex(where: { $0.text == said || ($0.text == nil && said.isEmpty) }) {
                leaving.remove(at: place)
            }
            shown.append(Entry(id: id, kind: .owner(text: event["text"] as? String ?? "", files: files), at: at))
        case "agent":
            let choices = (event["choices"] as? [[String: Any]] ?? []).compactMap { choice -> Choice? in
                guard let label = choice["label"] as? String, let command = choice["command"] as? String else { return nil }
                return Choice(label: label, command: command)
            }
            shown.append(Entry(id: id, kind: .agent(text: event["text"] as? String ?? "", choices: choices), at: at))
            if !reading, !quiet { Haptic.arrived() }
            working = false
            doing = nil
        case "ask":
            let ask = Ask(
                ask: event["ask"] as? String ?? "", summary: event["summary"] as? String ?? "",
                detail: event["detail"] as? String ?? "", critical: event["critical"] as? Bool ?? false,
                irreversible: event["irreversible"] as? Bool ?? false)
            shown.append(Entry(id: id, kind: .ask(ask), at: at))
            if !reading, !quiet { Haptic.asked() }
        case "settled":
            let settled = event["ask"] as? String
            if let place = shown.firstIndex(where: {
                if case .ask(let ask) = $0.kind { return ask.ask == settled }
                return false
            }), case .ask(let ask) = shown[place].kind {
                shown[place].kind = .settled(summary: ask.summary, answer: event["answer"] as? String ?? "")
            }
        case "file":
            if let file = sent(event) {
                shown.append(Entry(id: id, kind: .file(file, caption: event["caption"] as? String ?? ""), at: at))
            }
        case "erase":
            let gone = Set((event["events"] as? [Int] ?? []).map(String.init))
            shown.removeAll { gone.contains($0.id) }
        case "step":
            doing = "\(event["icon"] as? String ?? "") \(event["text"] as? String ?? "")"
            working = true
        case "working":
            working = true
        case "idle":
            working = false
            doing = nil
        default:
            break
        }
    }

    /// Reads what was said before what is shown: a screenful more, put
    /// before it. The entry that was first, to keep the eye where it was.
    func earlier() async -> String? {
        guard further, let first = entries.first(where: { Int($0.id) != nil }), let before = Int(first.id),
              let asking = request("/api/earlier?before=\(before)"),
              let (told, answer) = try? await session.data(for: asking),
              (answer as? HTTPURLResponse)?.statusCode == 200,
              let read = try? JSONSerialization.jsonObject(with: told) as? [String: Any]
        else {
            further = false
            return nil
        }
        // Taken as any entry is, aside from what is shown, then put before it.
        let (shown, was_working, was_doing, was_leaving, was_seen) = (entries, working, doing, leaving, seen)
        entries = []
        quiet = true
        for event in read["entries"] as? [[String: Any]] ?? [] { took(event) }
        quiet = false
        let older = entries
        (working, doing, leaving, seen) = (was_working, was_doing, was_leaving, was_seen)
        entries = older + shown
        further = (read["more"] as? Bool ?? false) && !older.isEmpty
        return first.id
    }

    /// Says something to the Agent, with the files sent ahead of it.
    func send(_ text: String, files: [String] = [], tapped: Bool = false, shown: UUID? = nil) async {
        // Shown at once, and at work at once: neither waits for the way there and back.
        var showing = shown
        if let shown, let place = leaving.firstIndex(where: { $0.id == shown }) {
            leaving[place].text = text
        } else if !tapped, !text.hasPrefix("/") {
            let one = Leaving(text: text)
            leaving.append(one)
            showing = one.id
        }
        if !text.hasPrefix("/") { working = true }
        var saying: [String: Any] = ["text": text, "files": files]
        if tapped { saying["tapped"] = true }
        let body = try? JSONSerialization.data(withJSONObject: saying)
        guard let asking = request("/api/say", method: "POST", body: body),
              let (_, answer) = try? await session.data(for: asking),
              (answer as? HTTPURLResponse)?.statusCode == 200
        else {
            leaving.removeAll { $0.id == showing }
            working = false
            trouble = say("That was not sent. Try again.", "Ce n'est pas parti. Réessaie.")
            return
        }
        trouble = nil
        // Never left showing for good, should the Agent not say it back.
        if let showing {
            Task { [weak self] in
                try? await Task.sleep(for: .seconds(20))
                self?.leaving.removeAll { $0.id == showing }
            }
        }
    }

    /// Answers what the Agent asked leave for.
    func answer(_ ask: String, _ answer: String) async {
        let body = try? JSONSerialization.data(withJSONObject: ["ask": ask, "answer": answer])
        if let asking = request("/api/answer", method: "POST", body: body) {
            _ = try? await session.data(for: asking)
        }
    }

    /// Sends a file ahead of the message it goes with; what it is known by.
    func upload(_ content: Data, named name: String) async -> String? {
        let named = name.addingPercentEncoding(withAllowedCharacters: .alphanumerics) ?? "file"
        guard let asking = request("/api/upload?name=\(named)", method: "POST", body: content, kind: "application/octet-stream"),
              let (told, answer) = try? await session.data(for: asking),
              (answer as? HTTPURLResponse)?.statusCode == 200,
              let taken = try? JSONSerialization.jsonObject(with: told) as? [String: Any]
        else { return nil }
        return taken["file"] as? String
    }

    /// What a file of the Conversation holds.
    func content(of file: String) async -> Data? {
        let named = file.addingPercentEncoding(withAllowedCharacters: .alphanumerics) ?? ""
        guard let asking = request("/api/file?id=\(named)"), let (content, answer) = try? await session.data(for: asking),
              (answer as? HTTPURLResponse)?.statusCode == 200
        else { return nil }
        return content
    }

    /// The Agent's settings, and how shapes and colours are drawn.
    func settings() async -> ([Setting], Looks)? {
        guard let asking = request("/api/settings"), let (told, answer) = try? await session.data(for: asking),
              (answer as? HTTPURLResponse)?.statusCode == 200,
              let all = try? JSONSerialization.jsonObject(with: told) as? [String: Any]
        else { return nil }
        let settings = (all["settings"] as? [[String: Any]] ?? []).compactMap { one -> Setting? in
            guard let key = one["key"] as? String else { return nil }
            return Setting(key: key, value: one["value"] as? String ?? "", choices: one["choices"] as? [String] ?? [])
        }
        var looks = Looks()
        for (shape, drawn) in all["shapes"] as? [String: [String: Any]] ?? [:] {
            if let outline = drawn["outline"] as? String {
                looks.shapes[shape] = (outline, drawn["eyes"] as? Double ?? 36)
            }
        }
        looks.colours = all["colours"] as? [String: String] ?? [:]
        looks.models = (all["models"] as? [[String: Any]] ?? []).compactMap { one in
            guard let value = one["value"] as? String else { return nil }
            return Looks.Model(value: value, label: one["label"] as? String ?? value)
        }
        looks.usual = all["usual"] as? String
        return (settings, looks)
    }

    /// Changes one setting; true when the Agent took it.
    func set(_ key: String, to value: String) async -> Bool {
        let body = try? JSONSerialization.data(withJSONObject: ["key": key, "value": value])
        guard let asking = request("/api/settings", method: "POST", body: body),
              let (_, answer) = try? await session.data(for: asking)
        else { return false }
        return (answer as? HTTPURLResponse)?.statusCode == 200
    }

    /// Looks again at who the Agent is, after its name or its look changed.
    func look_again() async {
        guard let asking = request("/api/me"), let (told, _) = try? await session.data(for: asking),
              let me = try? JSONSerialization.jsonObject(with: told) as? [String: Any]
        else { return }
        name = me["agent"] as? String ?? name
        if let look = me["look"] as? [String: Any], let outline = look["outline"] as? String {
            self.look = Look(
                outline: outline, colour: look["colour"] as? String ?? "#f2f2f0",
                eyes: look["eyes"] as? Double ?? 36)
        }
        remember()
    }

    /// The Owner was shown round: it is not shown again, here or elsewhere.
    func welcomed() async {
        welcoming = false
        if let asking = request("/api/welcomed", method: "POST", body: Data("{}".utf8)) {
            _ = try? await session.data(for: asking)
        }
    }

    /// The services connected with a touch, and whether that is possible
    /// from here (the Agent's page reached encrypted).
    func connectors() async -> ([Connector], Bool)? {
        guard let asking = request("/api/connectors"), let (told, answer) = try? await session.data(for: asking),
              (answer as? HTTPURLResponse)?.statusCode == 200,
              let all = try? JSONSerialization.jsonObject(with: told) as? [String: Any]
        else { return nil }
        let connectors = (all["connectors"] as? [[String: Any]] ?? []).compactMap { one -> Connector? in
            guard let name = one["name"] as? String else { return nil }
            return Connector(
                name: name, label: one["label"] as? String ?? name, does: one["does"] as? String ?? "",
                connected: one["connected"] as? Bool ?? false)
        }
        return (connectors, all["with_a_touch"] as? Bool ?? false)
    }

    /// Begins connecting a service: the link to sign in with.
    func connect(_ name: String) async -> URL? {
        let body = try? JSONSerialization.data(withJSONObject: ["name": name])
        guard let asking = request("/api/connectors", method: "POST", body: body),
              let (told, answer) = try? await session.data(for: asking),
              (answer as? HTTPURLResponse)?.statusCode == 200,
              let begun = try? JSONSerialization.jsonObject(with: told) as? [String: Any],
              let link = begun["link"] as? String
        else { return nil }
        return URL(string: link)
    }
}

/// A service the Agent can be connected to.
struct Connector: Identifiable, Equatable {
    let name: String
    let label: String
    let does: String
    var connected: Bool
    var id: String { name }
}

/// What the hand feels, kept to what tells something: an answer came, the
/// Agent asks leave, something went wrong. Nothing for what the eye sees
/// well enough.
@MainActor
enum Haptic {
    static func arrived() { UIImpactFeedbackGenerator(style: .soft).impactOccurred(intensity: 0.7) }
    static func asked() { UINotificationFeedbackGenerator().notificationOccurred(.warning) }
    static func done() { UINotificationFeedbackGenerator().notificationOccurred(.success) }
    static func refused() { UIImpactFeedbackGenerator(style: .rigid).impactOccurred(intensity: 0.6) }
    static func touched() { UIImpactFeedbackGenerator(style: .light).impactOccurred(intensity: 0.6) }
    static func wrong() { UINotificationFeedbackGenerator().notificationOccurred(.error) }
}
