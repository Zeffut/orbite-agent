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

    private var recording: AVAudioRecorder?
    private var ready: AVAudioRecorder?
    private var began = Date()
    private var measuring: Task<Void, Never>?

    // What the hand feels: made ready ahead, as the system asks for them to be on time.
    private let starting = UIImpactFeedbackGenerator(style: .rigid)
    private let crossing = UISelectionFeedbackGenerator()
    private let sending = UIImpactFeedbackGenerator(style: .soft)
    private let giving_up = UINotificationFeedbackGenerator()

    /// Gets everything ready before the microphone is ever touched: the
    /// sound of the phone set to record alongside whatever plays, a file
    /// to record into, and what the hand will feel.
    func prepare() {
        starting.prepare()
        crossing.prepare()
        guard allowed, ready == nil else { return }
        let audio = AVAudioSession.sharedInstance()
        try? audio.setCategory(.playAndRecord, mode: .default, options: [.mixWithOthers, .defaultToSpeaker, .allowBluetoothHFP])
        try? audio.setActive(true)
        let file = FileManager.default.temporaryDirectory
            .appendingPathComponent("voice-\(Int(Date().timeIntervalSince1970 * 1000)).m4a")
        let settings: [String: Any] = [
            AVFormatIDKey: kAudioFormatMPEG4AAC, AVSampleRateKey: 44_100, AVNumberOfChannelsKey: 1,
            AVEncoderAudioQualityKey: AVAudioQuality.high.rawValue,
        ]
        let recorder = try? AVAudioRecorder(url: file, settings: settings)
        recorder?.isMeteringEnabled = true
        recorder?.prepareToRecord()
        ready = recorder
    }

    /// Asks for the microphone, the first time it is touched.
    func ask() async -> Bool {
        allowed = await Hearing.microphone()
        if allowed { prepare() }
        return allowed
    }

    /// Starts recording, at once.
    func begin() {
        starting.impactOccurred(intensity: 1)
        if ready == nil { prepare() }
        recording = ready
        ready = nil
        began = Date()
        recording?.record()
        crossing.prepare()
        sending.prepare()
        giving_up.prepare()
        measuring = Task { [weak self] in
            while !Task.isCancelled {
                try? await Task.sleep(for: .milliseconds(60))
                guard let self, let recording = self.recording else { return }
                recording.updateMeters()
                // From decibels to something a bar can show.
                let heard = CGFloat(max(0.05, min(1, pow(10, recording.averagePower(forChannel: 0) / 28))))
                self.levels.removeFirst()
                self.levels.append(heard)
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
    func end(keeping: Bool) -> (Data, String)? {
        measuring?.cancel()
        levels = Array(repeating: 0.05, count: levels.count)
        guard let recording else { return nil }
        let file = recording.url
        let lasted = recording.currentTime
        recording.stop()
        self.recording = nil
        defer {
            try? FileManager.default.removeItem(at: file)
            prepare()
        }
        guard keeping, lasted > 0.4, let spoken = try? Data(contentsOf: file) else {
            if lasted > 0.4 { giving_up.notificationOccurred(.warning) }
            return nil
        }
        sending.impactOccurred(intensity: 0.8)
        return (spoken, file.lastPathComponent)
    }

    /// What was said in a recording, written down on this phone itself:
    /// nothing of the voice leaves it. None when the phone cannot, or was
    /// not let.
    static func written(from spoken: Data, named name: String) async -> String? {
        let language = Locale.preferredLanguages.first?.hasPrefix("fr") == true ? "fr-FR" : "en-US"
        guard await Hearing.writing() else { return nil }
        let file = FileManager.default.temporaryDirectory.appendingPathComponent("heard-" + name)
        guard (try? spoken.write(to: file)) != nil else { return nil }
        defer { try? FileManager.default.removeItem(at: file) }
        return await Hearing.written(at: file, in: language)
    }

    /// Lets the phone's sound go back to whatever else was using it.
    func rest() {
        ready = nil
        try? AVAudioSession.sharedInstance().setActive(false, options: .notifyOthersOnDeactivation)
    }
}
