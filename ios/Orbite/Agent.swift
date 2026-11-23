// The Agent as the app knows it: where it is reached, what it has said, and
// how to say something to it. Everything goes through the interface the
// Agent serves, as its own page does.

import Foundation
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
    let ask: Int
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

@Observable
@MainActor
final class Agent {
    var paired = false
    var name = "Orbite"
    var look: Look?
    var entries: [Entry] = []
    var doing: String?
    var working = false
    var reached = true
    var trouble: String?

    private var address: URL? {
        didSet { UserDefaults.standard.set(address?.absoluteString, forKey: "agent") }
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
        await enter()
        if !paired, trouble == nil {
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
        if let look = me["look"] as? [String: Any], let outline = look["outline"] as? String {
            self.look = Look(
                outline: outline, colour: look["colour"] as? String ?? "#f2f2f0",
                eyes: look["eyes"] as? Double ?? 36)
        }
        paired = true
        reached = true
        UserDefaults.standard.set(true, forKey: "paired")
        listen()
    }

    /// Forgets the Agent: the app goes back to asking for a link.
    func leave() {
        listening?.cancel()
        entries = []
        paired = false
        address = nil
        UserDefaults.standard.set(false, forKey: "paired")
    }

    /// Listens to what the Agent says, for as long as the app is open,
    /// taking the Conversation up again whenever the line drops.
    private func listen() {
        listening?.cancel()
        listening = Task { [weak self] in
            while !Task.isCancelled {
                guard let self, let asking = self.request("/api/events") else { return }
                do {
                    let (lines, answer) = try await self.session.bytes(for: asking)
                    if (answer as? HTTPURLResponse)?.statusCode == 401 {
                        self.paired = false
                        UserDefaults.standard.set(false, forKey: "paired")
                        return
                    }
                    self.reached = true
                    self.entries = []
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
        let at = (event["at"] as? Double).map { Date(timeIntervalSince1970: $0) }
        let sent: ([String: Any]) -> Sent? = { file in
            guard let token = file["file"] as? String else { return nil }
            return Sent(file: token, name: file["name"] as? String ?? "", image: file["image"] as? Bool ?? false)
        }
        switch event["kind"] as? String {
        case "owner":
            let files = (event["files"] as? [[String: Any]] ?? []).compactMap(sent)
            entries.append(Entry(id: id, kind: .owner(text: event["text"] as? String ?? "", files: files), at: at))
        case "agent":
            let choices = (event["choices"] as? [[String: Any]] ?? []).compactMap { choice -> Choice? in
                guard let label = choice["label"] as? String, let command = choice["command"] as? String else { return nil }
                return Choice(label: label, command: command)
            }
            entries.append(Entry(id: id, kind: .agent(text: event["text"] as? String ?? "", choices: choices), at: at))
            working = false
            doing = nil
        case "ask":
            let ask = Ask(
                ask: event["ask"] as? Int ?? 0, summary: event["summary"] as? String ?? "",
                detail: event["detail"] as? String ?? "", critical: event["critical"] as? Bool ?? false,
                irreversible: event["irreversible"] as? Bool ?? false)
            entries.append(Entry(id: id, kind: .ask(ask), at: at))
        case "settled":
            let settled = event["ask"] as? Int
            if let place = entries.firstIndex(where: {
                if case .ask(let ask) = $0.kind { return ask.ask == settled }
                return false
            }), case .ask(let ask) = entries[place].kind {
                entries[place].kind = .settled(summary: ask.summary, answer: event["answer"] as? String ?? "")
            }
        case "file":
            if let file = sent(event) {
                entries.append(Entry(id: id, kind: .file(file, caption: event["caption"] as? String ?? ""), at: at))
            }
        case "erase":
            let gone = Set((event["events"] as? [Int] ?? []).map(String.init))
            entries.removeAll { gone.contains($0.id) }
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

    /// Says something to the Agent, with the files sent ahead of it.
    func send(_ text: String, files: [String] = [], tapped: Bool = false) async {
        var saying: [String: Any] = ["text": text, "files": files]
        if tapped { saying["tapped"] = true }
        let body = try? JSONSerialization.data(withJSONObject: saying)
        guard let asking = request("/api/say", method: "POST", body: body),
              let (_, answer) = try? await session.data(for: asking),
              (answer as? HTTPURLResponse)?.statusCode == 200
        else {
            trouble = say("That was not sent. Try again.", "Ce n'est pas parti. Réessaie.")
            return
        }
        trouble = nil
    }

    /// Answers what the Agent asked leave for.
    func answer(_ ask: Int, _ answer: String) async {
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
}
