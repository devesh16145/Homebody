// make-icons.swift: draws the Homebody app icon and the menu-bar state glyphs.
//   swiftc -O make-icons.swift -o /tmp/make-icons && /tmp/make-icons <output dir>
// The mark is a cat peeking over a windowsill. Geometry matches ui/js/icons.js (64-unit box, y down).
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
    /// A quadratic segment, as AppKit only draws cubic ones.
    func quad(_ path: NSBezierPath, _ qx: CGFloat, _ qy: CGFloat, _ x: CGFloat, _ y: CGFloat) {
        let p0 = path.currentPoint, q = p(qx, qy), p2 = p(x, y)
        path.curve(to: p2, controlPoint1: NSPoint(x: p0.x + 2 / 3 * (q.x - p0.x), y: p0.y + 2 / 3 * (q.y - p0.y)),
                   controlPoint2: NSPoint(x: p2.x + 2 / 3 * (q.x - p2.x), y: p2.y + 2 / 3 * (q.y - p2.y)))
    }
    /// The cat's head and ears, moved down by `dy` (15 when it ducks). Separate shapes, filled one
    /// by one, so where an ear overlaps the head nothing is cut away.
    func cat(dy: CGFloat = 0) -> [NSBezierPath] {
        let path = dome(32, 42 + dy, 19)
        let left = NSBezierPath(); left.move(to: p(15.5, 33.5 + dy)); left.line(to: p(17.5, 13.5 + dy))
        quad(left, 18, 11 + dy, 20, 12.5 + dy); left.line(to: p(31, 23.5 + dy)); left.close()
        let right = NSBezierPath(); right.move(to: p(48.5, 33.5 + dy)); right.line(to: p(46.5, 13.5 + dy))
        quad(right, 46, 11 + dy, 44, 12.5 + dy); right.line(to: p(33, 23.5 + dy)); right.close()
        return [path, left, right]
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

/// Menu-bar glyph: 33×22 pt at 2x. Crop of the 64 box: x 2…62, y 10…50.
func tray(_ st: State) -> Data {
    png(66, 44) { w, h in
        let b = Box(rect: NSRect(x: 0, y: 0, width: w, height: h), x0: 2, y0: 10, w: 60, h: 40)
        let ctx = NSGraphicsContext.current!.cgContext
        // Off: the same cat, faded as a whole (one layer, so overlaps do not darken).
        if st == .off { ctx.setAlpha(0.42); ctx.beginTransparencyLayer(auxiliaryInfo: nil) }
        defer { if st == .off { ctx.endTransparencyLayer() } }
        let ink = NSColor.black
        ink.set()
        NSGraphicsContext.saveGraphicsState()
        NSBezierPath(rect: NSRect(x: 0, y: b.p(0, 42).y, width: w, height: h)).addClip()   // overlaps the sill: no seam
        b.cat(dy: st == .ducked ? 15 : 0).forEach { $0.fill() }
        NSGraphicsContext.restoreGraphicsState()
        if st != .ducked {
            let (dx, dy): (CGFloat, CGFloat) = st == .asking ? (1.8, -1.8) : st == .watching ? (-2.4, 0) : (0, 0)
            ctx.setBlendMode(.destinationOut)
            NSColor.black.set()
            b.oval(25 + dx, 32 + dy, 2.6, 3.7).fill()
            b.oval(39 + dx, 32 + dy, 2.6, 3.7).fill()
            ctx.setBlendMode(.normal)
            ink.set()
        }
        b.roundRect(6, 40, 52, 6, 3).fill()
        if st != .ducked {
            b.oval(18, 41, 3.8, 3.8).fill()
            b.oval(46, 41, 3.8, 3.8).fill()
        }
        // A small badge for "a question is waiting" and "needs attention".
        if st == .asking || st == .attention {
            ctx.setBlendMode(.destinationOut)
            b.oval(56, 15, 7.2, 7.2).fill()
            ctx.setBlendMode(.normal)
            if st == .asking { b.oval(56, 15, 4.6, 4.6).fill() }
            else {
                b.roundRect(54.6, 9.5, 2.8, 7, 1.4).fill()
                b.oval(56, 19.4, 1.5, 1.5).fill()
            }
        }
    }
}

/// App icon, 1024 px: tide tile, the cat in white, a midnight sill. Geometry from a 100 box.
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
        // The cat, from the 64 box scaled by 1.3 into the 100 box (same placement as icons.js).
        let c = Box(rect: NSRect(x: b.p(9.6, 0).x, y: b.p(0, 9.6 + 64 * 1.3 - 3.2).y, width: 64 * 1.3 * b.s, height: 64 * 1.3 * b.s), x0: 0, y0: 0, w: 64, h: 64)
        let face = NSGradient(colors: [hex(0xffffff), hex(0xd9f3f5)])!
        NSGraphicsContext.saveGraphicsState()
        NSBezierPath(rect: NSRect(x: body.minX, y: c.p(0, 40.5).y, width: body.width, height: body.maxY - c.p(0, 40.5).y)).addClip()
        // One shading across the whole cat: fill the shapes, then paint the gradient only onto them.
        let ctx = NSGraphicsContext.current!.cgContext
        ctx.beginTransparencyLayer(auxiliaryInfo: nil)
        NSColor.white.setFill()
        c.cat().forEach { $0.fill() }
        ctx.setBlendMode(.sourceIn)
        face.draw(in: NSRect(x: c.p(13, 0).x, y: c.p(0, 42).y, width: 38 * c.s, height: 32 * c.s), angle: -90)
        ctx.setBlendMode(.normal)
        ctx.endTransparencyLayer()
        NSGraphicsContext.restoreGraphicsState()
        hex(0x0b1620).setFill()
        c.oval(25, 32, 2.6, 3.7).fill()
        c.oval(39, 32, 2.6, 3.7).fill()
        hex(0x0b1620, 0.86).setFill()
        NSBezierPath(rect: NSRect(x: body.minX, y: body.minY, width: body.width, height: b.p(0, 58.4).y - body.minY)).fill()
        NSColor.white.withAlphaComponent(0.25).setFill()
        NSBezierPath(rect: NSRect(x: body.minX, y: b.p(0, 60.4).y, width: body.width, height: 2 * b.s)).fill()
        face.draw(in: b.oval(33, 59.7, 5, 5), angle: -90)
        face.draw(in: b.oval(69.4, 59.7, 5, 5), angle: -90)
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
