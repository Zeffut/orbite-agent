// The devices let in to the server that holds the Owner's agents: listed,
// turned away, and one more let in with a code this phone shows. Asked of
// the server itself, by a device that is let in: never of an agent.

import CoreImage.CIFilterBuiltins
import SwiftUI

struct Device: Identifiable, Equatable {
    let name: String
    let seen: Date
    let this: Bool
    var id: String { name }
}

struct NewDevice: Equatable {
    let code: String
    let link: String?
}

struct DevicesScreen: View {
    @Environment(Agent.self) private var agent
    @State private var devices: [Device]?
    @State private var adding = false
    @State private var given: NewDevice?
    @State private var leaving: Device?

    var body: some View {
        List {
            if let devices {
                Section {
                    ForEach(devices) { device in
                        HStack {
                            VStack(alignment: .leading, spacing: 2) {
                                Text(device.name)
                                Text(say("Last seen ", "Vu le ") + device.seen.formatted(date: .abbreviated, time: .omitted))
                                    .font(.caption).foregroundStyle(.secondary)
                            }
                            Spacer()
                            if device.this {
                                Text(say("This phone", "Ce téléphone")).font(.footnote).foregroundStyle(.secondary)
                            } else {
                                Button(say("Remove", "Retirer"), role: .destructive) { leaving = device }
                                    .buttonStyle(.borderless)
                            }
                        }
                    }
                }
                Section {
                    Button {
                        Haptic.touched()
                        adding = true
                        Task {
                            given = await agent.oneMoreDevice()
                            adding = false
                            if given == nil { Haptic.wrong() }
                        }
                    } label: {
                        HStack {
                            Label(say("Add a device", "Ajouter un appareil"), systemImage: "plus")
                            if adding { Spacer(); ProgressView() }
                        }
                    }
                    .disabled(adding)
                    if let given {
                        VStack(spacing: 12) {
                            if let link = given.link, let drawn = Self.picture(of: link) {
                                Image(uiImage: drawn).interpolation(.none).resizable()
                                    .frame(width: 190, height: 190)
                                    .padding(8).background(.white, in: .rect(cornerRadius: 14))
                            }
                            Text(given.code).font(.title.monospaced().weight(.semibold))
                                .textSelection(.enabled)
                            if let link = given.link, let address = URL(string: link) {
                                ShareLink(item: address) {
                                    Label(say("Send the link", "Envoyer le lien"), systemImage: "square.and.arrow.up")
                                }
                            }
                        }
                        .frame(maxWidth: .infinity)
                        .padding(.vertical, 8)
                    }
                } footer: {
                    Text(say(
                        "On the new device, scan this code or open the link. It works once, for ten minutes, and lets it open every agent here.",
                        "Sur le nouvel appareil, scanne ce code ou ouvre le lien. Il sert une fois, pendant dix minutes, et lui ouvre tous les agents d'ici."))
                }
            } else {
                Section { ProgressView().frame(maxWidth: .infinity) }
            }
        }
        .navigationTitle(say("Devices", "Appareils"))
        .navigationBarTitleDisplayMode(.inline)
        .task { devices = await agent.devices() }
        .confirmationDialog(
            say("Remove \(leaving?.name ?? "")?", "Retirer \(leaving?.name ?? "") ?"),
            isPresented: Binding(get: { leaving != nil }, set: { if !$0 { leaving = nil } }),
            titleVisibility: .visible
        ) {
            Button(say("Remove", "Retirer"), role: .destructive) {
                guard let gone = leaving else { return }
                Task {
                    _ = await agent.turnAway(gone.name)
                    devices = await agent.devices()
                }
            }
        } message: {
            Text(say("It will no longer open your agents.", "Il ne pourra plus ouvrir tes agents."))
        }
    }

    /// The link as a picture a phone's camera reads.
    private static func picture(of link: String) -> UIImage? {
        let making = CIFilter.qrCodeGenerator()
        making.message = Data(link.utf8)
        making.correctionLevel = "M"
        guard let drawn = making.outputImage?.transformed(by: CGAffineTransform(scaleX: 8, y: 8)),
              let made = CIContext().createCGImage(drawn, from: drawn.extent)
        else { return nil }
        return UIImage(cgImage: made)
    }
}
