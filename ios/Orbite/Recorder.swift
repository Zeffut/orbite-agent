// A voice message: recorded while the microphone is held, and given back
// as a file when it is let go.

import AVFoundation

@MainActor
final class Recorder {
    private var recording: AVAudioRecorder?
    private var began = Date()

    /// Starts recording; false when the microphone was not given.
    func begin() async -> Bool {
        guard await AVAudioApplication.requestRecordPermission() else { return false }
        let audio = AVAudioSession.sharedInstance()
        try? audio.setCategory(.playAndRecord, mode: .default, options: [.defaultToSpeaker])
        try? audio.setActive(true)
        let file = FileManager.default.temporaryDirectory.appendingPathComponent("voice-\(Int(Date().timeIntervalSince1970 * 1000)).m4a")
        let settings: [String: Any] = [
            AVFormatIDKey: kAudioFormatMPEG4AAC, AVSampleRateKey: 44_100, AVNumberOfChannelsKey: 1,
            AVEncoderAudioQualityKey: AVAudioQuality.high.rawValue,
        ]
        recording = try? AVAudioRecorder(url: file, settings: settings)
        began = Date()
        return recording?.record() ?? false
    }

    /// Stops; what was recorded and its name, unless it is given up or
    /// too short to hold anything.
    func end(keeping: Bool) -> (Data, String)? {
        guard let recording else { return nil }
        let file = recording.url
        recording.stop()
        self.recording = nil
        try? AVAudioSession.sharedInstance().setActive(false, options: .notifyOthersOnDeactivation)
        defer { try? FileManager.default.removeItem(at: file) }
        guard keeping, Date().timeIntervalSince(began) > 0.5, let spoken = try? Data(contentsOf: file) else { return nil }
        return (spoken, file.lastPathComponent)
    }
}
