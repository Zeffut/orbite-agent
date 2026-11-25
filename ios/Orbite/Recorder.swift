// A voice message: recorded while the microphone is held, and given back
// as a file when it is let go. Everything slow is done ahead, so that
// recording starts the instant the finger lands.

import AVFoundation
import Observation
import Speech
import UIKit

@Observable
@MainActor
final class Recorder {
    /// How loud the voice is right now, from 0 to 1, a few times a second.
    var levels: [CGFloat] = Array(repeating: 0.05, count: 28)
    /// True once the Owner has given the microphone.
    private(set) var allowed = AVAudioApplication.shared.recordPermission == .granted

    private let tape = Tape()
    private var measuring: Task<Void, Never>?

    // What the hand feels: made ready ahead, as the system asks for them to be on time.
    private let starting = UIImpactFeedbackGenerator(style: .rigid)
    private let crossing = UISelectionFeedbackGenerator()
    private let sending = UIImpactFeedbackGenerator(style: .soft)
    private let giving_up = UINotificationFeedbackGenerator()

    /// Gets everything ready before the microphone is ever touched: the
    /// sound of the phone set to record, a file to record into, and what
    /// the hand will feel. None of it holds the screen up.
    func prepare() {
        starting.prepare()
        crossing.prepare()
        if allowed { tape.ready() }
    }

    /// Asks for the microphone, the first time it is touched.
    func ask() async -> Bool {
        allowed = await Hearing.microphone()
        prepare()
        return allowed
    }

    /// Starts recording. The hand feels it and the screen shows it at
    /// once; the phone starts hearing on its own, a breath later, without
    /// either waiting for it.
    func begin() {
        starting.impactOccurred(intensity: 1)
        tape.begin()
        crossing.prepare()
        sending.prepare()
        giving_up.prepare()
        measuring = Task { [weak self] in
            while !Task.isCancelled {
                try? await Task.sleep(for: .milliseconds(50))
                guard let self else { return }
                self.levels.removeFirst()
                self.levels.append(self.tape.loudness)
            }
        }
    }

    /// The finger went into, or came back from, where letting go gives up.
    func crossed() {
        crossing.selectionChanged()
        crossing.prepare()
    }

    /// Stops; what was recorded and its name, unless it is given up or
    /// too short to hold anything.
    func end(keeping: Bool) async -> (Data, String)? {
        measuring?.cancel()
        levels = Array(repeating: 0.05, count: levels.count)
        if keeping { sending.impactOccurred(intensity: 0.8) }
        let (spoken, lasted) = await tape.end()
        guard keeping, lasted > 0.4, let spoken else {
            if !keeping, lasted > 0.4 { giving_up.notificationOccurred(.warning) }
            return nil
        }
        return spoken
    }

    /// What was said in a recording, written down on this phone itself:
    /// nothing of the voice leaves it. None when the phone cannot, or was
    /// not let.
    static func written(from spoken: Data, named name: String) async -> Heard {
        let language = Locale.preferredLanguages.first?.hasPrefix("fr") == true ? "fr-FR" : "en-US"
        guard await Hearing.writing() else { return .unable }
        let file = FileManager.default.temporaryDirectory.appendingPathComponent("heard-" + name)
        guard (try? spoken.write(to: file)) != nil else { return .unable }
        defer { try? FileManager.default.removeItem(at: file) }
        return await Hearing.written(at: file, in: language).map(Heard.said) ?? .nothing
    }

    /// What came of writing a recording down: what was said, nothing
    /// that could be made out, or the phone not being able or let to.
    enum Heard {
        case said(String)
        case nothing
        case unable
    }

    /// Lets the phone's sound go back to whatever else was using it.
    func rest() {
        tape.rest()
    }
}

/// The recording itself, kept off the screen's thread: starting the
/// phone's microphone takes it a moment, and nothing on the screen is to
/// wait for that. Everything is done in turn on a queue of its own.
private final class Tape: @unchecked Sendable {
    private let turn = DispatchQueue(label: "orbite.voice", qos: .userInteractive)
    private let lock = NSLock()
    private var heard: CGFloat = 0.05
    // Touched only in turn.
    private var ready: AVAudioRecorder?
    private var recording: AVAudioRecorder?
    private var meter: DispatchSourceTimer?
    private var awake = false

    /// How loud the voice is right now, from 0 to 1.
    var loudness: CGFloat {
        lock.lock()
        defer { lock.unlock() }
        return heard
    }

    func ready(_ then: (@Sendable () -> Void)? = nil) {
        turn.async {
            self.wake()
            if self.ready == nil { self.ready = Self.blank() }
            then?()
        }
    }

    func begin() {
        let asked = ProcessInfo.processInfo.systemUptime
        turn.async {
            let before = ProcessInfo.processInfo.systemUptime
            self.wake()
            let recorder = self.ready ?? Self.blank()
            self.ready = nil
            self.recording = recorder
            let started = recorder?.record() ?? false
            let hearing = ProcessInfo.processInfo.systemUptime
            let meter = DispatchSource.makeTimerSource(queue: self.turn)
            meter.schedule(deadline: .now(), repeating: .milliseconds(50))
            meter.setEventHandler { [weak self] in
                guard let self, let recording = self.recording else { return }
                recording.updateMeters()
                // From decibels to something a bar can show.
                // A voice at arm's length sits around -35 to -15: that span is
                // what fills the bars, the peaks lifting them a little more.
                let loud = 0.7 * recording.averagePower(forChannel: 0) + 0.3 * recording.peakPower(forChannel: 0)
                let span = max(0, min(1, (Double(loud) + 50) / 38))
                let level = CGFloat(max(0.05, pow(span, 1.3)))
                self.lock.lock()
                self.heard = level
                self.lock.unlock()
            }
            meter.resume()
            self.meter = meter
            Self.note(String(
                format: "begin started=%@ waited-its-turn=%.0fms starting=%.0fms",
                started ? "yes" : "no", (before - asked) * 1000, (hearing - before) * 1000))
        }
    }

    /// Stops; what was recorded with its name, and how long it lasted.
    func end() async -> ((Data, String)?, TimeInterval) {
        await withCheckedContinuation { (told: CheckedContinuation<((Data, String)?, TimeInterval), Never>) in
            turn.async {
                self.meter?.cancel()
                self.meter = nil
                self.lock.lock()
                self.heard = 0.05
                self.lock.unlock()
                guard let recording = self.recording else {
                    told.resume(returning: (nil, 0))
                    return
                }
                let file = recording.url
                let lasted = recording.currentTime
                recording.stop()
                self.recording = nil
                let spoken = try? Data(contentsOf: file)
                try? FileManager.default.removeItem(at: file)
                Self.note(String(format: "end lasted=%.2fs bytes=%d", lasted, spoken?.count ?? 0))
                told.resume(returning: (spoken.map { ($0, file.lastPathComponent) }, lasted))
                // Ready for the next one before it is asked.
                self.ready = Self.blank()
            }
        }
    }

    func rest() {
        turn.async {
            guard self.recording == nil else { return }
            if let ready = self.ready { try? FileManager.default.removeItem(at: ready.url) }
            self.ready = nil
            self.awake = false
            try? AVAudioSession.sharedInstance().setActive(false, options: .notifyOthersOnDeactivation)
        }
    }

    /// The phone's sound set to record alongside whatever plays. The
    /// phone's own microphone is the one heard: asking a headset for its
    /// own takes it a second or more and spoils what it was playing.
    private func wake() {
        guard !awake else { return }
        let before = ProcessInfo.processInfo.systemUptime
        let audio = AVAudioSession.sharedInstance()
        try? audio.setCategory(.playAndRecord, mode: .default, options: [.mixWithOthers, .defaultToSpeaker])
        awake = (try? audio.setActive(true)) != nil
        Self.note(String(format: "wake ok=%@ took=%.0fms", awake ? "yes" : "no",
                         (ProcessInfo.processInfo.systemUptime - before) * 1000))
    }

    /// A recorder with a file to record into, ready to start.
    private static func blank() -> AVAudioRecorder? {
        let file = FileManager.default.temporaryDirectory
            .appendingPathComponent("voice-\(Int(Date().timeIntervalSince1970 * 1000)).m4a")
        let settings: [String: Any] = [
            AVFormatIDKey: kAudioFormatMPEG4AAC, AVSampleRateKey: 44_100, AVNumberOfChannelsKey: 1,
            AVEncoderAudioQualityKey: AVAudioQuality.high.rawValue,
        ]
        let recorder = try? AVAudioRecorder(url: file, settings: settings)
        recorder?.isMeteringEnabled = true
        recorder?.prepareToRecord()
        return recorder
    }

    /// Keeps how long each step took, on the phone only, to find what is
    /// slow when something is: the last few hundred lines, no more.
    private static func note(_ line: String) {
        guard let kept = FileManager.default.urls(for: .cachesDirectory, in: .userDomainMask).first?
            .appendingPathComponent("voice.log")
        else { return }
        var all = (try? String(contentsOf: kept, encoding: .utf8))?.split(separator: "\n").suffix(300).map(String.init) ?? []
        all.append("\(Date().ISO8601Format()) \(line)")
        try? all.joined(separator: "\n").write(to: kept, atomically: true, encoding: .utf8)
    }
}
