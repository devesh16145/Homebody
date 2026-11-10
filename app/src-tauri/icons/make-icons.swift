// make-icons.swift — draws the app icon and the menu-bar state glyphs.
//   swiftc -O make-icons.swift -o /tmp/make-icons && /tmp/make-icons <output dir>
// Menu-bar glyphs are template images (black + alpha); macOS tints them for light and dark bars.

import AppKit

let outDir = CommandLine.arguments.count > 1 ? CommandLine.arguments[1] : "."

func png(_ size: Int, _ draw: (CGFloat) -> Void) -> Data {
    let rep = NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: size, pixelsHigh: size, bitsPerSample: 8, samplesPerPixel: 4,
                               hasAlpha: true, isPlanar: false, colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0)!
    NSGraphicsContext.saveGraphicsState()
    NSGraphicsContext.current = NSGraphicsContext(bitmapImageRep: rep)
    NSGraphicsContext.current?.imageInterpolation = .high
    draw(CGFloat(size))
    NSGraphicsContext.restoreGraphicsState()
    return rep.representation(using: .png, properties: [:])!
}

/// A shield in a 100×100 design space, scaled into `rect`.
func shield(in rect: NSRect) -> NSBezierPath {
    let s = rect.width / 100
    func p(_ x: CGFloat, _ y: CGFloat) -> NSPoint { NSPoint(x: rect.minX + x * s, y: rect.minY + y * s) }
    let path = NSBezierPath()
    path.move(to: p(50, 96))
    path.curve(to: p(88, 80), controlPoint1: p(62, 89), controlPoint2: p(76, 83))
    path.line(to: p(88, 52))
    path.curve(to: p(50, 4), controlPoint1: p(88, 28), controlPoint2: p(72, 12))
    path.curve(to: p(12, 52), controlPoint1: p(28, 12), controlPoint2: p(12, 28))
    path.line(to: p(12, 80))
    path.curve(to: p(50, 96), controlPoint1: p(24, 83), controlPoint2: p(38, 89))
    path.close()
    return path
}

func stroke(_ pts: [(CGFloat, CGFloat)], in rect: NSRect, width: CGFloat) -> NSBezierPath {
    let s = rect.width / 100
    let path = NSBezierPath()
    path.lineWidth = width * s
    path.lineCapStyle = .round
    path.lineJoinStyle = .round
    for (i, pt) in pts.enumerated() {
        let q = NSPoint(x: rect.minX + pt.0 * s, y: rect.minY + pt.1 * s)
        if i == 0 { path.move(to: q) } else { path.line(to: q) }
    }
    return path
}

func dot(_ x: CGFloat, _ y: CGFloat, _ r: CGFloat, in rect: NSRect) -> NSBezierPath {
    let s = rect.width / 100
    return NSBezierPath(ovalIn: NSRect(x: rect.minX + (x - r) * s, y: rect.minY + (y - r) * s, width: 2 * r * s, height: 2 * r * s))
}

enum Glyph { case check, eye, slash, question, cross, bang }

/// Menu-bar glyph. Solid shield with the mark cut out, or an outline shield with the mark drawn.
func tray(_ glyph: Glyph, solid: Bool) -> Data {
    png(44) { size in
        let rect = NSRect(x: 3, y: 2, width: size - 6, height: size - 6).offsetBy(dx: 0, dy: 1)
        let ctx = NSGraphicsContext.current!.cgContext
        NSColor.black.set()
        let sh = shield(in: rect)
        if solid {
            sh.fill()
            ctx.setBlendMode(.destinationOut)
        } else {
            sh.lineWidth = 3.2
            sh.stroke()
        }
        let w: CGFloat = solid ? 10 : 9
        switch glyph {
        case .check: stroke([(31, 50), (45, 36), (70, 63)], in: rect, width: w).stroke()
        case .cross: stroke([(35, 36), (65, 66)], in: rect, width: w).stroke(); stroke([(65, 36), (35, 66)], in: rect, width: w).stroke()
        case .slash: stroke([(30, 30), (70, 72)], in: rect, width: w).stroke()
        case .eye: dot(50, 52, 11, in: rect).fill()
        case .bang: stroke([(50, 72), (50, 46)], in: rect, width: w).stroke(); dot(50, 28, 6, in: rect).fill()
        case .question:
            let q = NSBezierPath()
            let s = rect.width / 100
            func p(_ x: CGFloat, _ y: CGFloat) -> NSPoint { NSPoint(x: rect.minX + x * s, y: rect.minY + y * s) }
            q.lineWidth = w * s; q.lineCapStyle = .round; q.lineJoinStyle = .round
            q.move(to: p(37, 64))
            q.curve(to: p(50, 76), controlPoint1: p(37, 71), controlPoint2: p(43, 76))
            q.curve(to: p(63, 64), controlPoint1: p(57, 76), controlPoint2: p(63, 71))
            q.curve(to: p(50, 44), controlPoint1: p(63, 54), controlPoint2: p(50, 54))
            q.stroke()
            dot(50, 28, 6, in: rect).fill()
        }
        ctx.setBlendMode(.normal)
    }
}

/// App icon: a deep green rounded square with a white shield and a green check.
func appIcon(_ size: Int) -> Data {
    png(size) { s in
        let inset = s * 0.085
        let body = NSRect(x: inset, y: inset, width: s - 2 * inset, height: s - 2 * inset)
        let radius = body.width * 0.2237
        let bg = NSBezierPath(roundedRect: body, xRadius: radius, yRadius: radius)
        NSGraphicsContext.saveGraphicsState()
        let shadow = NSShadow()
        shadow.shadowColor = NSColor.black.withAlphaComponent(0.28)
        shadow.shadowBlurRadius = s * 0.025
        shadow.shadowOffset = NSSize(width: 0, height: -s * 0.012)
        shadow.set()
        NSColor(calibratedRed: 0.07, green: 0.42, blue: 0.30, alpha: 1).setFill()
        bg.fill()
        NSGraphicsContext.restoreGraphicsState()
        NSGradient(colors: [NSColor(calibratedRed: 0.16, green: 0.66, blue: 0.47, alpha: 1), NSColor(calibratedRed: 0.04, green: 0.33, blue: 0.25, alpha: 1)])!.draw(in: bg, angle: -90)
        // Soft top highlight.
        NSGraphicsContext.saveGraphicsState()
        bg.addClip()
        NSGradient(colors: [NSColor.white.withAlphaComponent(0.16), NSColor.white.withAlphaComponent(0)])!.draw(in: NSRect(x: body.minX, y: body.midY, width: body.width, height: body.height / 2), angle: -90)
        NSGraphicsContext.restoreGraphicsState()

        let sr = body.insetBy(dx: body.width * 0.2, dy: body.width * 0.2).offsetBy(dx: 0, dy: -body.width * 0.005)
        NSGraphicsContext.saveGraphicsState()
        let ss = NSShadow()
        ss.shadowColor = NSColor.black.withAlphaComponent(0.22)
        ss.shadowBlurRadius = s * 0.02
        ss.shadowOffset = NSSize(width: 0, height: -s * 0.008)
        ss.set()
        NSColor.white.setFill()
        shield(in: sr).fill()
        NSGraphicsContext.restoreGraphicsState()
        NSColor(calibratedRed: 0.07, green: 0.47, blue: 0.33, alpha: 1).setStroke()
        stroke([(31, 50), (45, 36), (70, 63)], in: sr, width: 10).stroke()
    }
}

func write(_ data: Data, _ name: String) {
    let url = URL(fileURLWithPath: outDir).appendingPathComponent(name)
    try! data.write(to: url)
    print("wrote \(name) (\(data.count) bytes)")
}

write(appIcon(1024), "app-icon.png")
write(tray(.check, solid: true), "tray-private.png")
write(tray(.eye, solid: false), "tray-watching.png")
write(tray(.slash, solid: false), "tray-off.png")
write(tray(.question, solid: true), "tray-asking.png")
write(tray(.cross, solid: true), "tray-blocked.png")
write(tray(.bang, solid: false), "tray-attention.png")
