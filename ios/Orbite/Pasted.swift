// What is on the phone's clipboard, taken as files to send: a picture, a
// PDF, a document copied from another app. The phone hands each over
// wherever it pleases, never promising the main thread, so nothing here
// belongs to it.

import UIKit
import UniformTypeIdentifiers

enum Pasted {
    /// What of a copied thing is a file to send, as the phone calls its
    /// kind; none for plain words or a link, which belong in the field.
    nonisolated static func kind(of provider: NSItemProvider) -> UTType? {
        provider.registeredTypeIdentifiers.compactMap { UTType($0) }.first { type in
            type.conforms(to: .image) || type.conforms(to: .pdf) || type.conforms(to: .movie)
                || type.conforms(to: .audio) || type.conforms(to: .archive)
                || type.conforms(to: .spreadsheet) || type.conforms(to: .presentation)
                || (type.conforms(to: .data) && !type.conforms(to: .text) && !type.conforms(to: .url))
        }
    }

    /// The content of a copied thing, as `kind`.
    nonisolated static func content(of provider: NSItemProvider, as kind: UTType) async -> Data? {
        await withCheckedContinuation { (read: CheckedContinuation<Data?, Never>) in
            _ = provider.loadDataRepresentation(forTypeIdentifier: kind.identifier) { @Sendable content, _ in
                read.resume(returning: content)
            }
        }
    }
}
