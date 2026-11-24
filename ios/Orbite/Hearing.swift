// What the phone is asked about hearing and writing down a voice. The
// system answers these wherever it pleases, never promising the main
// thread: so nothing here belongs to it, and every answer is carried
// back by a continuation. Asked from a view directly, the answer landing
// elsewhere stops the app dead.

import AVFoundation
import Speech

enum Hearing {
    /// Asks for the microphone; true when it is given.
    nonisolated static func microphone() async -> Bool {
        await withCheckedContinuation { (asked: CheckedContinuation<Bool, Never>) in
            AVAudioApplication.requestRecordPermission { @Sendable given in asked.resume(returning: given) }
        }
    }

    /// Asks to write down what is said; true when it is let.
    nonisolated static func writing() async -> Bool {
        await withCheckedContinuation { (asked: CheckedContinuation<Bool, Never>) in
            SFSpeechRecognizer.requestAuthorization { @Sendable answer in
                asked.resume(returning: answer == .authorized)
            }
        }
    }

    /// What is said in the recording at `file`, in `language`, written
    /// down on this phone itself. None when the phone cannot.
    nonisolated static func written(at file: URL, in language: String) async -> String? {
        guard let recognizer = SFSpeechRecognizer(locale: Locale(identifier: language)), recognizer.isAvailable
        else { return nil }
        let request = SFSpeechURLRecognitionRequest(url: file)
        request.shouldReportPartialResults = false
        request.addsPunctuation = true
        if recognizer.supportsOnDeviceRecognition { request.requiresOnDeviceRecognition = true }
        let once = Once()
        return await withCheckedContinuation { (heard: CheckedContinuation<String?, Never>) in
            recognizer.recognitionTask(with: request) { @Sendable result, error in
                if let result, result.isFinal {
                    let text = result.bestTranscription.formattedString
                        .trimmingCharacters(in: .whitespacesAndNewlines)
                    if once.first() { heard.resume(returning: text.isEmpty ? nil : text) }
                } else if error != nil {
                    if once.first() { heard.resume(returning: nil) }
                }
            }
        }
    }
}

/// True the first time it is asked, from whatever thread, and never again.
private final class Once: @unchecked Sendable {
    private let lock = NSLock()
    private var done = false

    func first() -> Bool {
        lock.lock()
        defer { lock.unlock() }
        if done { return false }
        done = true
        return true
    }
}
