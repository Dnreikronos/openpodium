import AppKit

guard CommandLine.arguments.count == 2 else {
    fatalError("Usage: generate-icon <output.iconset>")
}
let output = URL(fileURLWithPath: CommandLine.arguments[1], isDirectory: true)
try FileManager.default.createDirectory(at: output, withIntermediateDirectories: true)

func drawIcon(pixels: Int, destination: URL) throws {
    let bitmap = NSBitmapImageRep(
        bitmapDataPlanes: nil, pixelsWide: pixels, pixelsHigh: pixels,
        bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true, isPlanar: false,
        colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0
    )!
    let context = NSGraphicsContext(bitmapImageRep: bitmap)!
    NSGraphicsContext.saveGraphicsState()
    NSGraphicsContext.current = context
    context.imageInterpolation = .high
    let scale = CGFloat(pixels) / 1024
    let transform = AffineTransform(scale: scale)
    (transform as NSAffineTransform).concat()

    let tile = NSBezierPath(roundedRect: NSRect(x: 104, y: 104, width: 816, height: 816), xRadius: 184, yRadius: 184)
    NSGraphicsContext.saveGraphicsState()
    let shadow = NSShadow()
    shadow.shadowColor = NSColor.black.withAlphaComponent(0.18)
    shadow.shadowBlurRadius = 30
    shadow.shadowOffset = NSSize(width: 0, height: -14)
    shadow.set()
    NSColor(calibratedRed: 0.02, green: 0.43, blue: 0.95, alpha: 1).setFill()
    tile.fill()
    NSGraphicsContext.restoreGraphicsState()

    NSGradient(colors: [
        NSColor(calibratedRed: 0.00, green: 0.40, blue: 0.93, alpha: 1),
        NSColor(calibratedRed: 0.06, green: 0.61, blue: 1.00, alpha: 1)
    ])!.draw(in: tile, angle: 90)
    NSColor.white.withAlphaComponent(0.19).setStroke()
    tile.lineWidth = 2
    tile.stroke()

    let mark = NSBezierPath(roundedRect: NSRect(x: 318, y: 294, width: 388, height: 436), xRadius: 145, yRadius: 145)
    mark.lineWidth = 66
    NSColor.white.setStroke()
    mark.stroke()

    NSGraphicsContext.restoreGraphicsState()
    try bitmap.representation(using: .png, properties: [:])!.write(to: destination)
}

for size in [16, 32, 128, 256, 512] {
    try drawIcon(pixels: size, destination: output.appendingPathComponent("icon_\(size)x\(size).png"))
    try drawIcon(pixels: size * 2, destination: output.appendingPathComponent("icon_\(size)x\(size)@2x.png"))
}
