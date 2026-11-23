// Who the Owner is talking to: the Agent in its own shape and colour, or
// Orbite's planet until it has one. It blinks at rest, and sways while the
// Agent works.

import SwiftUI

extension Color {
    /// A colour written as a page writes it, such as #1fbfa3.
    init(page: String) {
        let digits = page.trimmingCharacters(in: CharacterSet(charactersIn: "#"))
        let value = UInt64(digits, radix: 16) ?? 0xf2f2f0
        self.init(
            red: Double((value >> 16) & 0xff) / 255, green: Double((value >> 8) & 0xff) / 255,
            blue: Double(value & 0xff) / 255)
    }
}

/// An outline given as the points of a path in a square of 100.
struct Outline: Shape {
    let points: [CGPoint]

    init(_ written: String) {
        var points: [CGPoint] = []
        for part in written.split(whereSeparator: { "MLZ".contains($0) }) {
            let numbers = part.split(separator: " ").compactMap { Double($0) }
            if numbers.count == 2 { points.append(CGPoint(x: numbers[0], y: numbers[1])) }
        }
        self.points = points
    }

    func path(in rect: CGRect) -> Path {
        var path = Path()
        let scale = min(rect.width, rect.height) / 100
        for (place, point) in points.enumerated() {
            let at = CGPoint(x: rect.minX + point.x * scale, y: rect.minY + point.y * scale)
            if place == 0 { path.move(to: at) } else { path.addLine(to: at) }
        }
        path.closeSubpath()
        return path
    }
}

struct Figure: View {
    let look: Look?
    var working = false
    var size: CGFloat = 30
    @State private var shut = false
    @State private var swaying = false

    var body: some View {
        ZStack {
            if let look {
                Outline(look.outline)
                    .fill(Color(page: look.colour))
                    .rotationEffect(.degrees(working ? (swaying ? 9 : -9) : 0))
                    .scaleEffect(working ? 1.04 : 1)
                eyes(top: look.eyes)
            } else {
                // Orbite's own mark: a planet and its ring.
                Ellipse()
                    .stroke(Color(page: "#f2f2f0"), lineWidth: size * 0.05)
                    .frame(width: size * 0.88, height: size * 0.26)
                    .rotationEffect(.degrees(-24))
                Circle().fill(Color(page: "#f2f2f0")).frame(width: size * 0.54, height: size * 0.54)
                eyes(top: 38)
            }
        }
        .frame(width: size, height: size)
        .animation(.easeInOut(duration: 0.75).repeatForever(autoreverses: true), value: swaying)
        .onChange(of: working, initial: true) { _, now in swaying = now }
        .task {
            // A blink now and then.
            while !Task.isCancelled {
                try? await Task.sleep(for: .seconds(Double.random(in: 3...6)))
                withAnimation(.easeInOut(duration: 0.09)) { shut = true }
                try? await Task.sleep(for: .milliseconds(130))
                withAnimation(.easeInOut(duration: 0.09)) { shut = false }
            }
        }
    }

    private func eyes(top: Double) -> some View {
        let unit = size / 100
        return HStack(spacing: 10 * unit) {
            ForEach(0..<2, id: \.self) { _ in
                Capsule().fill(Color(page: "#0d0d0d"))
                    .frame(width: 7 * unit, height: (shut ? 2 : 15) * unit)
            }
        }
        .offset(y: (top + 7.5 - 50) * unit)
    }
}
