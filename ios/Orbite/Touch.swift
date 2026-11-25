// A finger on the screen, told the instant it lands. A gesture waits to
// know what it is (a tap, a drag, something of the system's) before it
// says anything; what records a voice cannot wait.

import SwiftUI
import UIKit

struct Touch: UIViewRepresentable {
    /// The finger landed.
    let began: () -> Void
    /// The finger moved: how far across from where it landed.
    let moved: (CGFloat) -> Void
    /// The finger left, or the system took it away.
    let ended: () -> Void

    func makeUIView(context: Context) -> Surface {
        let surface = Surface()
        surface.backgroundColor = .clear
        surface.isExclusiveTouch = true
        return surface
    }

    func updateUIView(_ surface: Surface, context: Context) {
        surface.began = began
        surface.moved = moved
        surface.ended = ended
    }

    final class Surface: UIView {
        var began: () -> Void = {}
        var moved: (CGFloat) -> Void = { _ in }
        var ended: () -> Void = {}
        private var landed: CGFloat?

        // A little more than what is seen takes the finger: a thumb is not a needle.
        override func point(inside point: CGPoint, with event: UIEvent?) -> Bool {
            bounds.insetBy(dx: -8, dy: -8).contains(point)
        }

        override func touchesBegan(_ touches: Set<UITouch>, with event: UIEvent?) {
            guard landed == nil, let touch = touches.first else { return }
            landed = touch.location(in: nil).x
            began()
        }

        override func touchesMoved(_ touches: Set<UITouch>, with event: UIEvent?) {
            guard let landed, let touch = touches.first else { return }
            moved(touch.location(in: nil).x - landed)
        }

        override func touchesEnded(_ touches: Set<UITouch>, with event: UIEvent?) {
            guard landed != nil else { return }
            landed = nil
            ended()
        }

        override func touchesCancelled(_ touches: Set<UITouch>, with event: UIEvent?) {
            touchesEnded(touches, with: event)
        }
    }
}
