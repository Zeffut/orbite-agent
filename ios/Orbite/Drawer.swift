// The menu that comes in from the left: the Owner's agents, one more to
// add, and the settings below. It slides over the Conversation and goes
// back with a touch beside it or a push to the left.

import SwiftUI

struct Drawer: View {
    @Environment(Agent.self) private var agent
    @Binding var open: Bool
    /// What opens the settings.
    let settings: () -> Void
    @State private var pushed: CGFloat = 0

    private let width: CGFloat = 304

    var body: some View {
        ZStack(alignment: .leading) {
            // What is behind goes dark, and takes the touch that closes.
            Color.black.opacity(open ? 0.5 : 0)
                .ignoresSafeArea()
                .allowsHitTesting(open)
                .onTapGesture { close() }
            menu
                .frame(width: width)
                .frame(maxHeight: .infinity, alignment: .top)
                .background(Color(page: "#141414").ignoresSafeArea())
                .overlay(alignment: .trailing) {
                    Rectangle().fill(.white.opacity(0.06)).frame(width: 1).ignoresSafeArea()
                }
                .offset(x: open ? min(0, pushed) : -width - 20)
                .gesture(
                    DragGesture(minimumDistance: 12)
                        .onChanged { finger in pushed = min(0, finger.translation.width) }
                        .onEnded { finger in
                            if finger.translation.width < -70 || finger.predictedEndTranslation.width < -160 {
                                close()
                            }
                            withAnimation(.snappy(duration: 0.22)) { pushed = 0 }
                        })
            // A finger coming in from the left edge opens it.
            if !open {
                Color.clear.frame(width: 18).frame(maxHeight: .infinity)
                    .contentShape(.rect)
                    .gesture(
                        DragGesture(minimumDistance: 14).onEnded { finger in
                            if finger.translation.width > 50 {
                                withAnimation(.snappy(duration: 0.28)) { open = true }
                            }
                        })
            }
        }
        .animation(.snappy(duration: 0.28), value: open)
    }

    private func close() {
        withAnimation(.snappy(duration: 0.26)) { open = false }
    }

    private var menu: some View {
        VStack(alignment: .leading, spacing: 0) {
            Text("Orbite").font(.title2.weight(.bold))
                .padding(.horizontal, 20).padding(.top, 14).padding(.bottom, 22)
            Text(agent.known.count > 1 ? say("My agents", "Mes agents") : say("My agent", "Mon agent"))
                .font(.footnote.weight(.semibold)).foregroundStyle(.secondary)
                .padding(.horizontal, 20).padding(.bottom, 8)
            ScrollView {
                VStack(spacing: 2) {
                    ForEach(agent.known) { other in
                        let here = other.address == agent.address?.absoluteString
                        Button {
                            close()
                            if !here { Task { await agent.open(other) } }
                        } label: {
                            HStack(spacing: 12) {
                                Figure(look: look(of: other, here: here), size: 38)
                                Text(here ? agent.name : other.name).font(.body.weight(.medium))
                                    .lineLimit(1)
                                Spacer()
                            }
                            .padding(.horizontal, 12).padding(.vertical, 9)
                            .background(here ? Color.white.opacity(0.08) : .clear, in: .rect(cornerRadius: 14))
                            .contentShape(.rect)
                        }
                        .buttonStyle(.plain)
                    }
                    Button {
                        close()
                        agent.adding = true
                    } label: {
                        HStack(spacing: 12) {
                            Image(systemName: "plus").font(.body.weight(.semibold))
                                .frame(width: 38, height: 38)
                                .background(.white.opacity(0.08), in: .circle)
                            Text(say("Add an agent", "Ajouter un agent")).font(.body)
                            Spacer()
                        }
                        .foregroundStyle(.secondary)
                        .padding(.horizontal, 12).padding(.vertical, 9)
                        .contentShape(.rect)
                    }
                    .buttonStyle(.plain)
                }
                .padding(.horizontal, 8)
            }
            Divider().overlay(.white.opacity(0.08))
            Button {
                close()
                settings()
            } label: {
                HStack(spacing: 12) {
                    Image(systemName: "gearshape").font(.body.weight(.medium)).frame(width: 38, height: 38)
                    Text(say("Settings", "Réglages")).font(.body.weight(.medium))
                    Spacer()
                }
                .padding(.horizontal, 20).padding(.vertical, 8)
                .contentShape(.rect)
            }
            .buttonStyle(.plain)
            .padding(.bottom, 4)
        }
    }

    /// How an agent this phone knows looks: the one open as it is now, the
    /// others as they were when last seen.
    private func look(of other: Known, here: Bool) -> Look? {
        if here { return agent.look }
        guard let outline = other.outline, let colour = other.colour else { return nil }
        return Look(outline: outline, colour: colour, eyes: other.eyes ?? 36)
    }
}
