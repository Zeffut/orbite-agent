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
        // Told as it goes: a long recording is written down a stretch at a
        // time, each stretch starting afresh, and only the telling as it
        // goes lets them all be kept.
        request.shouldReportPartialResults = true
        request.addsPunctuation = true
        if recognizer.supportsOnDeviceRecognition { request.requiresOnDeviceRecognition = true }
        let listening = Listening()
        return await withCheckedContinuation { (heard: CheckedContinuation<String?, Never>) in
            listening.heard = heard
            listening.task = recognizer.recognitionTask(with: request, delegate: listening)
        }
    }
}

/// Follows the writing down of one recording to its end, keeping every
/// stretch of it: the phone writes a long recording down in stretches,
/// each one a text of its own that starts afresh after a pause, and what
/// it gives at the end is only the last of them.
private final class Listening: NSObject, SFSpeechRecognitionTaskDelegate, @unchecked Sendable {
    private let lock = NSLock()
    /// The stretches written down so far, in the order they were said.
    private var stretches: [String] = []
    private var answered = false
    var heard: CheckedContinuation<String?, Never>?
    var task: SFSpeechRecognitionTask?

    /// The words of a text, as they compare: without capitals or marks.
    private static func words(_ text: String) -> [String] {
        text.lowercased().split(whereSeparator: { !$0.isLetter && !$0.isNumber }).map(String.init)
    }

    /// Keeps what was just written down: more of the stretch under way,
    /// said better, or the start of a new one. The phone does not say
    /// which, and the times it gives start again with each stretch: a
    /// stretch under way keeps its first word and only grows, so a text
    /// that starts otherwise, or is suddenly much shorter, is a new one.
    private func keep(_ written: SFTranscription) {
        let said = written.formattedString.trimmingCharacters(in: .whitespacesAndNewlines)
        let now = Self.words(said)
        guard !now.isEmpty else { return }
        lock.lock()
        defer { lock.unlock() }
        guard let under_way = stretches.last else {
            stretches.append(said)
            return
        }
        let before = Self.words(under_way)
        let starts_alike = before.first == now.first
        let shrank = before.count >= 6 && now.count * 2 < before.count
        if starts_alike && !shrank {
            stretches[stretches.count - 1] = said
        } else if !(before.count >= now.count && Array(before.suffix(now.count)) == now) {
            // Not the end of what is already kept, said once more.
            stretches.append(said)
        }
    }

    private func answer() {
        lock.lock()
        let all = stretches.joined(separator: " ")
        let first = !answered
        answered = true
        lock.unlock()
        if first { heard?.resume(returning: all.isEmpty ? nil : all) }
    }

    func speechRecognitionTask(_ task: SFSpeechRecognitionTask, didHypothesizeTranscription transcription: SFTranscription) {
        keep(transcription)
    }

    func speechRecognitionTask(_ task: SFSpeechRecognitionTask, didFinishRecognition result: SFSpeechRecognitionResult) {
        keep(result.bestTranscription)
    }

    func speechRecognitionTask(_ task: SFSpeechRecognitionTask, didFinishSuccessfully successfully: Bool) {
        answer()
    }

    func speechRecognitionTaskWasCancelled(_ task: SFSpeechRecognitionTask) {
        answer()
    }
}
