// make-icons.swift: draws the Homebody app icon and the menu-bar state glyphs.
//   swiftc -O make-icons.swift -o /tmp/make-icons && /tmp/make-icons <output dir>
// The mark is a round face peeking over a windowsill. Geometry matches ui/js/icons.js (64-unit box, y down).
// Menu-bar glyphs are template images (black + alpha); macOS tints them for light and dark bars.

import AppKit

let outDir = CommandLine.arguments.count > 1 ? CommandLine.arguments[1] : "."

func png(_ w: Int, _ h: Int, _ draw: (CGFloat, CGFloat) -> Void) -> Data {
    let rep = NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: w, pixelsHigh: h, bitsPerSample: 8, samplesPerPixel: 4,
                               hasAlpha: true, isPlanar: false, colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0)!
    NSGraphicsContext.saveGraphicsState()
    NSGraphicsContext.current = NSGraphicsContext(bitmapImageRep: rep)
    NSGraphicsContext.current?.imageInterpolation = .high
    NSGraphicsContext.current?.shouldAntialias = true
    draw(CGFloat(w), CGFloat(h))
    NSGraphicsContext.restoreGraphicsState()
    return rep.representation(using: .png, properties: [:])!
}

func hex(_ v: UInt32, _ a: CGFloat = 1) -> NSColor {
    NSColor(srgbRed: CGFloat((v >> 16) & 255) / 255, green: CGFloat((v >> 8) & 255) / 255, blue: CGFloat(v & 255) / 255, alpha: a)
}

/// Maps a design box (x right, y down) onto a rect in AppKit space (y up).
struct Box {
    let rect: NSRect, x0: CGFloat, y0: CGFloat, w: CGFloat, h: CGFloat
    var s: CGFloat { rect.width / w }
    func p(_ x: CGFloat, _ y: CGFloat) -> NSPoint { NSPoint(x: rect.minX + (x - x0) * s, y: rect.maxY - (y - y0) * s) }
    func oval(_ cx: CGFloat, _ cy: CGFloat, _ rx: CGFloat, _ ry: CGFloat) -> NSBezierPath {
        let c = p(cx, cy)
        return NSBezierPath(ovalIn: NSRect(x: c.x - rx * s, y: c.y - ry * s, width: 2 * rx * s, height: 2 * ry * s))
    }
    func roundRect(_ x: CGFloat, _ y: CGFloat, _ w: CGFloat, _ h: CGFloat, _ r: CGFloat) -> NSBezierPath {
        let a = p(x, y + h)
        return NSBezierPath(roundedRect: NSRect(x: a.x, y: a.y, width: w * s, height: h * s), xRadius: r * s, yRadius: r * s)
    }
    /// Upper half of a circle centred at (cx, cy), closed along its diameter.
    func dome(_ cx: CGFloat, _ cy: CGFloat, _ r: CGFloat) -> NSBezierPath {
        let path = NSBezierPath()
        path.appendArc(withCenter: p(cx, cy), radius: r * s, startAngle: 0, endAngle: 180)
        path.close()
        return path
    }
}

enum State { case home, watching, asking, ducked, off, attention }

/// Menu-bar glyph: 33×22 pt at 2x. Crop of the 64 box: x 2…62, y 13…53.
func tray(_ st: State) -> Data {
    png(66, 44) { w, h in
        let b = Box(rect: NSRect(x: 0, y: 0, width: w, height: h), x0: 2, y0: 13, w: 60, h: 40)
        let ctx = NSGraphicsContext.current!.cgContext
        NSColor.black.set()
        let headY: CGFloat = st == .ducked ? 50 : 40
        let head = b.dome(32, headY, 20)
        // Everything above the sill only.
        NSGraphicsContext.saveGraphicsState()
        NSBezierPath(rect: NSRect(x: 0, y: b.p(0, 42).y, width: w, height: h)).addClip()   // overlaps the sill: no seam
        if st == .off {
            let outline = b.dome(32, 40, 18.25)
            outline.lineWidth = 3.5 * b.s
            outline.stroke()
        } else {
            head.fill()
            if st != .ducked { b.roundRect(12, 38, 40, 4, 0).fill() }   // carries the face into the sill: no seam
        }
        NSGraphicsContext.restoreGraphicsState()
        // Eyes: cut out of a solid head, drawn on an outline head.
        if st != .ducked {
            let (dx, dy): (CGFloat, CGFloat) = st == .asking ? (2.2, -2.2) : st == .watching ? (-2.6, 0) : (0, 0)
            if st != .off { ctx.setBlendMode(.destinationOut) }
            b.oval(25 + dx, 31 + dy, 2.7, 3.8).fill()
            b.oval(39 + dx, 31 + dy, 2.7, 3.8).fill()
            ctx.setBlendMode(.normal)
        }
        b.roundRect(6, 40, 52, 6, 3).fill()
        if st != .ducked {
            b.oval(17.5, 40.5, 3.6, 3.6).fill()
            b.oval(46.5, 40.5, 3.6, 3.6).fill()
        }
        // A small badge for "a question is waiting" and "needs attention".
        if st == .asking || st == .attention {
            ctx.setBlendMode(.destinationOut)
            b.oval(55, 19, 7.2, 7.2).fill()
            ctx.setBlendMode(.normal)
            if st == .asking { b.oval(55, 19, 4.6, 4.6).fill() }
            else {
                b.roundRect(53.6, 13.5, 2.8, 7, 1.4).fill()
                b.oval(55, 23.4, 1.5, 1.5).fill()
            }
        }
    }
}

/// App icon, 1024 px: tide tile, the face in white, a midnight sill. Geometry from a 100 box.
func appIcon(_ size: Int) -> Data {
    png(size, size) { s, _ in
        let inset = s * 0.0977          // Apple's grid: an 824 px body in a 1024 canvas
        let body = NSRect(x: inset, y: inset, width: s - 2 * inset, height: s - 2 * inset)
        let b = Box(rect: body, x0: 4, y0: 4, w: 92, h: 92)
        let tile = NSBezierPath(roundedRect: body, xRadius: body.width * 0.2237, yRadius: body.width * 0.2237)

        NSGraphicsContext.saveGraphicsState()
        let shadow = NSShadow()
        shadow.shadowColor = NSColor.black.withAlphaComponent(0.3)
        shadow.shadowBlurRadius = s * 0.028
        shadow.shadowOffset = NSSize(width: 0, height: -s * 0.012)
        shadow.set()
        hex(0x077a86).setFill()
        tile.fill()
        NSGraphicsContext.restoreGraphicsState()

        NSGraphicsContext.saveGraphicsState()
        tile.addClip()
        NSGradient(colors: [hex(0x5ad6de), hex(0x077a86)])!.draw(in: tile, angle: -90)
        let face = NSGradient(colors: [hex(0xffffff), hex(0xd9f3f5)])!
        let head = b.dome(50, 66, 28)
        face.draw(in: head, angle: -90)
        hex(0x0b1620).setFill()
        b.oval(40, 52, 3.8, 5.3).fill()
        b.oval(60, 52, 3.8, 5.3).fill()
        hex(0x0b1620, 0.86).setFill()
        NSBezierPath(rect: NSRect(x: body.minX, y: body.minY, width: body.width, height: b.p(0, 66).y - body.minY)).fill()
        NSColor.white.withAlphaComponent(0.25).setFill()
        NSBezierPath(rect: NSRect(x: body.minX, y: b.p(0, 68).y, width: body.width, height: 2 * b.s)).fill()
        face.draw(in: b.oval(29, 66.5, 5.2, 5.2), angle: -90)
        face.draw(in: b.oval(71, 66.5, 5.2, 5.2), angle: -90)
        NSGradient(colors: [NSColor.white.withAlphaComponent(0.28), NSColor.white.withAlphaComponent(0)])!
            .draw(in: NSRect(x: body.minX, y: body.midY, width: body.width, height: body.height / 2), angle: -90)
        NSGraphicsContext.restoreGraphicsState()

        let rim = NSBezierPath(roundedRect: body.insetBy(dx: s * 0.003, dy: s * 0.003), xRadius: body.width * 0.2237, yRadius: body.width * 0.2237)
        rim.lineWidth = s * 0.004
        NSColor.white.withAlphaComponent(0.22).setStroke()
        rim.stroke()
    }
}

func write(_ data: Data, _ name: String) {
    let url = URL(fileURLWithPath: outDir).appendingPathComponent(name)
    try! data.write(to: url)
    print("wrote \(name) (\(data.count) bytes)")
}

write(appIcon(1024), "app-icon.png")
write(tray(.home), "tray-private.png")
write(tray(.watching), "tray-watching.png")
write(tray(.off), "tray-off.png")
write(tray(.asking), "tray-asking.png")
write(tray(.ducked), "tray-blocked.png")
write(tray(.attention), "tray-attention.png")
