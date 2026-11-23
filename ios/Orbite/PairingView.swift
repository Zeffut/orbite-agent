// The first screen: this device is let in by the link the Agent gives,
// read with the camera from the code it shows, or pasted.

import SwiftUI
import VisionKit

struct PairingView: View {
    @Environment(Agent.self) private var agent
    @State private var link = ""
    @State private var scanning = false
    @State private var busy = false

    var body: some View {
        VStack(spacing: 22) {
            Spacer()
            Figure(look: nil, size: 120)
            VStack(spacing: 8) {
                Text(say("Open your agent", "Ouvre ton agent")).font(.largeTitle.bold())
                Text(say(
                    "Ask your agent to add this device, then scan the code it shows.",
                    "Demande à ton agent d'ajouter cet appareil, puis scanne le code qu'il affiche."))
                    .font(.body).foregroundStyle(.secondary).multilineTextAlignment(.center)
            }
            .padding(.horizontal, 28)
            Spacer()
            if let trouble = agent.trouble {
                Text(trouble).font(.callout).foregroundStyle(.red).multilineTextAlignment(.center).padding(.horizontal, 28)
            }
            VStack(spacing: 12) {
                Button {
                    scanning = true
                } label: {
                    Label(say("Scan the code", "Scanner le code"), systemImage: "qrcode.viewfinder")
                        .font(.headline).frame(maxWidth: .infinity).padding(.vertical, 8)
                }
                .buttonStyle(.glassProminent)
                .disabled(!DataScannerViewController.isSupported)

                HStack(spacing: 10) {
                    TextField(say("or paste its link", "ou colle son lien"), text: $link)
                        .textInputAutocapitalization(.never).autocorrectionDisabled().keyboardType(.URL)
                        .submitLabel(.go).onSubmit(open)
                        .padding(.horizontal, 16).padding(.vertical, 13)
                        .glassEffect(.regular, in: .capsule)
                    if !link.isEmpty {
                        Button(action: open) {
                            Image(systemName: "arrow.up").font(.headline).frame(width: 30, height: 30)
                        }
                        .buttonStyle(.glassProminent).buttonBorderShape(.circle)
                    }
                }
            }
            .padding(.horizontal, 20).padding(.bottom, 12)
            .disabled(busy)
        }
        .background(Color(page: "#0d0d0d").ignoresSafeArea())
        .sheet(isPresented: $scanning) {
            Scanner { read in
                scanning = false
                link = read
                open()
            }
            .ignoresSafeArea()
        }
    }

    private func open() {
        guard !link.isEmpty, !busy else { return }
        busy = true
        Task {
            await agent.pair(with: link)
            busy = false
        }
    }
}

/// The camera, reading the first code it is pointed at.
struct Scanner: UIViewControllerRepresentable {
    let read: (String) -> Void

    func makeCoordinator() -> Reader { Reader(read: read) }

    func makeUIViewController(context: Context) -> DataScannerViewController {
        let scanner = DataScannerViewController(
            recognizedDataTypes: [.barcode(symbologies: [.qr])], qualityLevel: .balanced,
            recognizesMultipleItems: false, isHighlightingEnabled: true)
        scanner.delegate = context.coordinator
        try? scanner.startScanning()
        return scanner
    }

    func updateUIViewController(_ scanner: DataScannerViewController, context: Context) {}

    final class Reader: NSObject, DataScannerViewControllerDelegate {
        let read: (String) -> Void
        private var done = false
        init(read: @escaping (String) -> Void) { self.read = read }

        func dataScanner(_ scanner: DataScannerViewController, didAdd added: [RecognizedItem], allItems: [RecognizedItem]) {
            guard !done, case .barcode(let code)? = added.first, let written = code.payloadStringValue else { return }
            done = true
            UINotificationFeedbackGenerator().notificationOccurred(.success)
            read(written)
        }
    }
}
