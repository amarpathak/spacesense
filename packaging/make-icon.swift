// Renders packaging/AppIcon.icns: a blue squircle with a white disk-usage ring.
// Run: swift packaging/make-icon.swift  (needs only Command Line Tools)
import AppKit

let size: CGFloat = 1024
let img = NSImage(size: NSSize(width: size, height: size))
img.lockFocus()
let ctx = NSGraphicsContext.current!.cgContext

// macOS icon grid: 824 px body centred in 1024, corner radius ~185.
let body = NSRect(x: 100, y: 100, width: 824, height: 824)
let path = NSBezierPath(roundedRect: body, xRadius: 185, yRadius: 185)
ctx.saveGState()
let shadow = NSShadow()
shadow.shadowColor = NSColor.black.withAlphaComponent(0.3)
shadow.shadowOffset = NSSize(width: 0, height: -12)
shadow.shadowBlurRadius = 28
shadow.set()
NSColor.black.setFill()
path.fill()
ctx.restoreGState()
path.addClip()
NSGradient(colors: [NSColor(calibratedRed: 0.16, green: 0.52, blue: 1.0, alpha: 1),
                    NSColor(calibratedRed: 0.05, green: 0.25, blue: 0.62, alpha: 1)])!
    .draw(in: body, angle: -90)

// Usage ring: segments of different lengths, like the app's disk bar.
let c = NSPoint(x: size / 2, y: size / 2)
let r: CGFloat = 270
let segments: [(CGFloat, NSColor)] = [
    (0.42, .white),
    (0.20, NSColor.white.withAlphaComponent(0.75)),
    (0.16, NSColor(calibratedRed: 0.55, green: 0.95, blue: 0.65, alpha: 1)),
    (0.22, NSColor.white.withAlphaComponent(0.22)),
]
var start: CGFloat = 90
for (frac, color) in segments {
    let end = start - frac * 360
    let arc = NSBezierPath()
    arc.appendArc(withCenter: c, radius: r, startAngle: start - 2, endAngle: end + 2, clockwise: true)
    arc.lineWidth = 110
    arc.lineCapStyle = .butt
    color.setStroke()
    arc.stroke()
    start = end
}
// Sparkle in the middle: "cleaned".
let s = NSBezierPath()
let k: CGFloat = 95
s.move(to: NSPoint(x: c.x, y: c.y + k))
s.curve(to: NSPoint(x: c.x + k, y: c.y), controlPoint1: NSPoint(x: c.x + 12, y: c.y + 12), controlPoint2: NSPoint(x: c.x + 12, y: c.y + 12))
s.curve(to: NSPoint(x: c.x, y: c.y - k), controlPoint1: NSPoint(x: c.x + 12, y: c.y - 12), controlPoint2: NSPoint(x: c.x + 12, y: c.y - 12))
s.curve(to: NSPoint(x: c.x - k, y: c.y), controlPoint1: NSPoint(x: c.x - 12, y: c.y - 12), controlPoint2: NSPoint(x: c.x - 12, y: c.y - 12))
s.curve(to: NSPoint(x: c.x, y: c.y + k), controlPoint1: NSPoint(x: c.x - 12, y: c.y + 12), controlPoint2: NSPoint(x: c.x - 12, y: c.y + 12))
NSColor.white.setFill()
s.fill()
img.unlockFocus()

let dir = URL(fileURLWithPath: CommandLine.arguments.count > 1 ? CommandLine.arguments[1] : "packaging")
let iconset = dir.appendingPathComponent("AppIcon.iconset")
try? FileManager.default.removeItem(at: iconset)
try FileManager.default.createDirectory(at: iconset, withIntermediateDirectories: true)
for base in [16, 32, 128, 256, 512] {
    for scale in [1, 2] {
        let px = base * scale
        let rep = NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: px, pixelsHigh: px, bitsPerSample: 8,
                                   samplesPerPixel: 4, hasAlpha: true, isPlanar: false,
                                   colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0)!
        NSGraphicsContext.saveGraphicsState()
        NSGraphicsContext.current = NSGraphicsContext(bitmapImageRep: rep)
        img.draw(in: NSRect(x: 0, y: 0, width: px, height: px))
        NSGraphicsContext.restoreGraphicsState()
        let name = scale == 1 ? "icon_\(base)x\(base).png" : "icon_\(base)x\(base)@2x.png"
        try rep.representation(using: .png, properties: [:])!.write(to: iconset.appendingPathComponent(name))
    }
}
print("wrote \(iconset.path)")
