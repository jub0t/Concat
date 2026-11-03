import AVFoundation
import CoreImage
import CoreImage.CIFilterBuiltins
import ImageIO
import UIKit

/// Where one clip or overlay sits in the built composition.
struct Segment {
    var itemID: UUID
    var isOverlay: Bool
    var trackID: CMPersistentTrackID
    var start: Double
    var duration: Double
    var orientation: CGImagePropertyOrientation
    var end: Double { start + duration }
}

/// What the frame handler reads. AVFoundation calls it on its own threads,
/// so the editor swaps whole values in under a lock instead of sharing state.
final class RenderState: @unchecked Sendable {
    private let lock = NSLock()
    private var _project: Project
    private var _segments: [Segment]
    private var _canvas: CGSize

    init(project: Project, segments: [Segment], canvas: CGSize) {
        _project = project; _segments = segments; _canvas = canvas
    }

    func update(project: Project? = nil, segments: [Segment]? = nil, canvas: CGSize? = nil) {
        lock.lock(); defer { lock.unlock() }
        if let project { _project = project }
        if let segments { _segments = segments }
        if let canvas { _canvas = canvas }
    }

    var snapshot: (Project, [Segment], CGSize) {
        lock.lock(); defer { lock.unlock() }
        return (_project, _segments, _canvas)
    }
}

/// Turns the decoded source frames at one moment into one finished canvas frame:
/// background, main track (with transitions), overlays, then titles.
enum FrameRenderer {
    static func render(sources: [CMPersistentTrackID: CIImage], time t: Double, state: RenderState) -> CIImage {
        let (project, segments, canvas) = state.snapshot
        let rect = CGRect(origin: .zero, size: canvas)
        let clips = Dictionary(project.clips.map { ($0.id, $0) }, uniquingKeysWith: { a, _ in a })
        let overlays = Dictionary(project.overlays.map { ($0.id, $0) }, uniquingKeysWith: { a, _ in a })

        let mains = segments
            .filter { !$0.isOverlay && t >= $0.start - 0.001 && t < $0.end + 0.001 }
            .sorted { $0.start < $1.start }

        func frame(_ seg: Segment) -> CIImage? {
            guard let src = sources[seg.trackID], let clip = clips[seg.itemID] else { return nil }
            let local = t - seg.start
            let bg = backdrop(project.background, source: src, seg: seg, clip: clip, canvas: canvas)
            var img = layer(src, seg: seg, clip: clip, local: local, canvas: canvas, overlay: false).composited(over: bg)
            var level = 1.0
            if clip.fadeIn > 0 { level = min(level, local / clip.fadeIn) }
            if clip.fadeOut > 0 { level = min(level, (seg.duration - local) / clip.fadeOut) }
            level = max(0, min(1, level))
            if level < 1 { img = dim(img, level) }
            return img.cropped(to: rect)
        }

        var output = solid(project.background, canvas: canvas)
        if mains.count >= 2, let a = mains.first, let b = mains.last, let fb = frame(b) {
            let fa = frame(a) ?? output
            let overlap = max(0.001, a.end - b.start)
            let kind = clips[b.itemID]?.transitionIn.kind ?? .dissolve
            output = Transitions.apply(kind, from: fa, to: fb, progress: (t - b.start) / overlap, canvas: canvas)
        } else if let seg = mains.last ?? mains.first, let f = frame(seg) {
            output = f
        }

        for item in project.overlays {
            guard t >= item.start, t < item.end,
                  let seg = segments.first(where: { $0.isOverlay && $0.itemID == item.id }),
                  let src = sources[seg.trackID], let ov = overlays[item.id] else { continue }
            let local = t - seg.start
            var img = layer(src, seg: seg, clip: ov.clip, local: local, canvas: canvas, overlay: true, mask: ov.mask, feather: ov.maskFeather)
            var level = 1.0
            if ov.clip.fadeIn > 0 { level = min(level, local / ov.clip.fadeIn) }
            if ov.clip.fadeOut > 0 { level = min(level, (seg.duration - local) / ov.clip.fadeOut) }
            if level < 1 { img = opacity(img, max(0, level)) }
            output = blend(img, over: output, mode: ov.blend)
        }

        for text in project.texts where t >= text.start && t < text.end {
            if let layer = textLayer(text, time: t, canvas: canvas) { output = layer.composited(over: output) }
        }
        return output.cropped(to: rect)
    }

    // MARK: Layers

    /// One clip, graded and placed on the canvas, transparent elsewhere.
    static func layer(_ source: CIImage, seg: Segment, clip: Clip, local: Double, canvas: CGSize, overlay: Bool,
                      mask: MaskShape = .none, feather: Double = 0) -> CIImage {
        var img = upright(source, seg: seg, clip: clip)
        img = Imaging.grade(img, clip: clip, time: local)
        img = Imaging.cutout(img, mode: clip.cutout)
        if overlay, mask != .none, let m = Imaging.shapeMask(mask, feather: feather, extent: img.extent) {
            let f = CIFilter.blendWithMask()
            f.inputImage = img; f.backgroundImage = CIImage.empty(); f.maskImage = m
            img = f.outputImage?.cropped(to: img.extent) ?? img
        }
        let size = img.extent.size
        guard size.width > 0, size.height > 0 else { return img }
        let sx = canvas.width / size.width, sy = canvas.height / size.height
        let base = (clip.fill && !overlay) ? max(sx, sy) : min(sx, sy)
        let tr = clip.transform(atLocal: local)
        let s = base * tr.scale
        var placed = img
            .transformed(by: CGAffineTransform(translationX: -size.width / 2, y: -size.height / 2))
            .transformed(by: CGAffineTransform(scaleX: s, y: s))
        if tr.rotation != 0 { placed = placed.transformed(by: CGAffineTransform(rotationAngle: -tr.rotation * .pi / 180)) }
        placed = placed.transformed(by: CGAffineTransform(
            translationX: canvas.width / 2 + tr.x * canvas.width, y: canvas.height / 2 - tr.y * canvas.height))
        if tr.opacity < 0.999 { placed = opacity(placed, tr.opacity) }
        return placed
    }

    /// Source frame turned upright, cropped and flipped, with its origin at zero.
    static func upright(_ source: CIImage, seg: Segment, clip: Clip) -> CIImage {
        var img = source.oriented(seg.orientation)
        img = img.transformed(by: CGAffineTransform(translationX: -img.extent.minX, y: -img.extent.minY))
        if !clip.crop.isFull {
            let e = img.extent, c = clip.crop
            let r = CGRect(x: e.width * c.x, y: e.height * (1 - c.y - c.h), width: e.width * c.w, height: e.height * c.h)
            img = img.cropped(to: r).transformed(by: CGAffineTransform(translationX: -r.minX, y: -r.minY))
        }
        if clip.flipped {
            img = img.transformed(by: CGAffineTransform(scaleX: -1, y: 1).translatedBy(x: -img.extent.width, y: 0))
        }
        if clip.flippedV {
            img = img.transformed(by: CGAffineTransform(scaleX: 1, y: -1).translatedBy(x: 0, y: -img.extent.height))
        }
        return img
    }

    static func solid(_ bg: CanvasBackground, canvas: CGSize) -> CIImage {
        let color: CIColor
        switch bg {
        case .black, .blur: color = .black
        case .white: color = .white
        case .teal: color = CIColor(red: 0, green: 0.42, blue: 0.4)
        case .purple: color = CIColor(red: 0.28, green: 0.16, blue: 0.5)
        case .warm: color = CIColor(red: 0.55, green: 0.3, blue: 0.18)
        }
        return CIImage(color: color).cropped(to: CGRect(origin: .zero, size: canvas))
    }

    /// The canvas behind a clip: a colour, or the clip itself blown up and blurred.
    private static func backdrop(_ bg: CanvasBackground, source: CIImage, seg: Segment, clip: Clip, canvas: CGSize) -> CIImage {
        guard bg == .blur else { return solid(bg, canvas: canvas) }
        let rect = CGRect(origin: .zero, size: canvas)
        let img = upright(source, seg: seg, clip: clip)
        let size = img.extent.size
        guard size.width > 0 else { return solid(.black, canvas: canvas) }
        let s = max(canvas.width / size.width, canvas.height / size.height) * 1.1
        let filled = img.transformed(by: CGAffineTransform(scaleX: s, y: s))
            .transformed(by: CGAffineTransform(translationX: (canvas.width - size.width * s) / 2, y: (canvas.height - size.height * s) / 2))
        let f = CIFilter.gaussianBlur(); f.inputImage = filled.clampedToExtent(); f.radius = Float(canvas.width * 0.04)
        return dim(f.outputImage?.cropped(to: rect) ?? solid(.black, canvas: canvas), 0.75)
    }

    static func dim(_ image: CIImage, _ level: Double) -> CIImage {
        let f = CIFilter.colorMatrix()
        let l = CGFloat(level)
        f.inputImage = image
        f.rVector = CIVector(x: l, y: 0, z: 0, w: 0)
        f.gVector = CIVector(x: 0, y: l, z: 0, w: 0)
        f.bVector = CIVector(x: 0, y: 0, z: l, w: 0)
        return f.outputImage ?? image
    }

    static func opacity(_ image: CIImage, _ level: Double) -> CIImage {
        let f = CIFilter.colorMatrix()
        f.inputImage = image
        f.aVector = CIVector(x: 0, y: 0, z: 0, w: CGFloat(level))
        return f.outputImage ?? image
    }

    static func blend(_ top: CIImage, over base: CIImage, mode: BlendMode) -> CIImage {
        let name: String
        switch mode {
        case .normal: return top.composited(over: base)
        case .screen: name = "CIScreenBlendMode"
        case .multiply: name = "CIMultiplyBlendMode"
        case .overlay: name = "CIOverlayBlendMode"
        case .add: name = "CIAdditionCompositing"
        case .lighten: name = "CILightenBlendMode"
        case .darken: name = "CIDarkenBlendMode"
        case .softLight: name = "CISoftLightBlendMode"
        case .hardLight: name = "CIHardLightBlendMode"
        case .difference: name = "CIDifferenceBlendMode"
        case .color: name = "CIColorBlendMode"
        case .luminosity: name = "CILuminosityBlendMode"
        }
        guard let f = CIFilter(name: name) else { return top.composited(over: base) }
        f.setValue(top, forKey: kCIInputImageKey)
        f.setValue(base, forKey: kCIInputBackgroundImageKey)
        return f.outputImage?.cropped(to: base.extent) ?? base
    }

    // MARK: Text

    private static func textLayer(_ text: TextOverlay, time t: Double, canvas: CGSize) -> CIImage? {
        let local = t - text.start
        let remaining = text.end - t
        let window = min(0.4, text.duration / 3)
        var alpha = text.opacity
        var scale = 1.0
        var dy = 0.0
        var shown = text
        func animate(_ anim: TextAnimation, _ p: Double, entering: Bool) {
            let q = max(0, min(1, p))
            switch anim {
            case .none: break
            case .fade: alpha *= q
            case .pop: scale *= 0.6 + 0.4 * q + (q < 1 ? 0.12 * sin(q * .pi) : 0); alpha *= min(1, q * 2)
            case .zoom: scale *= 0.3 + 0.7 * q; alpha *= q
            case .slideUp: dy += (entering ? 1 : -1) * (1 - q) * 0.06; alpha *= q
            case .slideDown: dy -= (entering ? 1 : -1) * (1 - q) * 0.06; alpha *= q
            case .bounce: scale *= q < 1 ? 1 + 0.25 * sin(q * .pi * 2.5) * (1 - q) : 1; alpha *= min(1, q * 3)
            case .typewriter:
                if entering, text.shape == nil {
                    let chars = Array(text.text)
                    let span = max(0.4, text.duration * 0.5)
                    let count = Int(ceil(Double(chars.count) * min(1, local / span)))
                    shown.text = String(chars.prefix(max(0, count)))
                }
            }
        }
        if text.animationIn != .none { animate(text.animationIn, text.animationIn == .typewriter ? 1 : local / window, entering: true) }
        if text.animationIn == .typewriter { animate(.typewriter, 0, entering: true) }
        if text.animationOut != .none, remaining < window { animate(text.animationOut, remaining / window, entering: false) }
        guard alpha > 0.01, var img = TextRasterizer.shared.image(for: shown, canvas: canvas) else { return nil }
        let w = img.extent.width, h = img.extent.height
        img = img.transformed(by: CGAffineTransform(translationX: -w / 2, y: -h / 2))
        if scale != 1 { img = img.transformed(by: CGAffineTransform(scaleX: scale, y: scale)) }
        if text.rotation != 0 { img = img.transformed(by: CGAffineTransform(rotationAngle: -text.rotation * .pi / 180)) }
        img = img.transformed(by: CGAffineTransform(translationX: text.x * canvas.width, y: (1 - text.y - dy) * canvas.height))
        if alpha < 0.999 { img = opacity(img, alpha) }
        return img
    }
}

/// Draws titles and shapes once and keeps the bitmap; frames reuse it until it changes.
final class TextRasterizer: @unchecked Sendable {
    static let shared = TextRasterizer()
    private let cache = NSCache<NSString, CIImage>()

    func image(for text: TextOverlay, canvas: CGSize) -> CIImage? {
        var key = text
        key.start = 0; key.duration = 0; key.x = 0; key.y = 0; key.rotation = 0; key.opacity = 1
        key.animationIn = .none; key.animationOut = .none; key.id = UUID(uuidString: "00000000-0000-0000-0000-000000000000")!
        key.isCaption = false
        let cacheKey = "\(key.hashValue)-\(Int(canvas.width))x\(Int(canvas.height))" as NSString
        if let hit = cache.object(forKey: cacheKey) { return hit }
        guard let made = text.shape != nil ? drawShape(text, canvas: canvas) : draw(text, canvas: canvas) else { return nil }
        cache.setObject(made, forKey: cacheKey)
        return made
    }

    static func font(_ font: TextFont, size: CGFloat, bold: Bool) -> UIFont {
        let weight: UIFont.Weight = bold ? .heavy : .medium
        let system = UIFont.systemFont(ofSize: size, weight: weight)
        func named(_ heavy: String, _ light: String) -> UIFont { UIFont(name: bold ? heavy : light, size: size) ?? system }
        switch font {
        case .system: return system
        case .rounded:
            return system.fontDescriptor.withDesign(.rounded).map { UIFont(descriptor: $0, size: size) } ?? system
        case .serif:
            return system.fontDescriptor.withDesign(.serif).map { UIFont(descriptor: $0, size: size) } ?? system
        case .mono: return .monospacedSystemFont(ofSize: size, weight: weight)
        case .condensed: return .systemFont(ofSize: size, weight: weight, width: .condensed)
        case .avenir: return named("AvenirNext-Heavy", "AvenirNext-Medium")
        case .futura: return named("Futura-CondensedExtraBold", "Futura-Medium")
        case .georgia: return named("Georgia-Bold", "Georgia")
        case .typewriter: return named("AmericanTypewriter-Bold", "AmericanTypewriter")
        case .marker: return named("MarkerFelt-Wide", "MarkerFelt-Thin")
        case .chalkboard: return named("ChalkboardSE-Bold", "ChalkboardSE-Regular")
        case .noteworthy: return named("Noteworthy-Bold", "Noteworthy-Light")
        case .script: return named("SnellRoundhand-Black", "SnellRoundhand")
        case .naskh: return named("DecoTypeNaskh", "DecoTypeNaskh")
        case .geeza: return named("GeezaPro-Bold", "GeezaPro")
        case .damascus: return named("DamascusBold", "Damascus")
        case .nastaliq: return named("NotoNastaliqUrdu-Bold", "NotoNastaliqUrdu")
        }
    }

    private func draw(_ text: TextOverlay, canvas: CGSize) -> CIImage? {
        var string = text.text.trimmingCharacters(in: .whitespacesAndNewlines)
        switch text.textCase {
        case .none: break
        case .upper: string = string.uppercased()
        case .lower: string = string.lowercased()
        }
        guard !string.isEmpty else { return nil }
        let fontSize = max(8, text.size * canvas.width)
        let font = Self.font(text.font, size: fontSize, bold: text.bold)
        let paragraph = NSMutableParagraphStyle()
        paragraph.alignment = .center
        paragraph.baseWritingDirection = .natural

        var attrs: [NSAttributedString.Key: Any] = [.font: font, .foregroundColor: text.color.uiColor, .paragraphStyle: paragraph]
        let maxWidth = canvas.width * 0.9
        let bounds = (string as NSString).boundingRect(
            with: CGSize(width: maxWidth, height: .greatestFiniteMagnitude),
            options: [.usesLineFragmentOrigin, .usesFontLeading], attributes: attrs, context: nil
        ).integral
        let pad = fontSize * (text.style == .box ? 0.45 : text.style == .glow ? 0.6 : 0.25)
        let size = CGSize(width: bounds.width + pad * 2, height: bounds.height + pad * 2)
        let textRect = CGRect(x: pad, y: pad, width: bounds.width, height: bounds.height)

        let format = UIGraphicsImageRendererFormat()
        format.scale = 1
        format.opaque = false
        let picture = UIGraphicsImageRenderer(size: size, format: format).image { _ in
            let ns = string as NSString
            switch text.style {
            case .plain:
                ns.draw(with: textRect, options: .usesLineFragmentOrigin, attributes: attrs, context: nil)
            case .box:
                text.accent.uiColor.withAlphaComponent(0.85 * text.accent.a).setFill()
                UIBezierPath(roundedRect: CGRect(origin: .zero, size: size), cornerRadius: fontSize * 0.3).fill()
                ns.draw(with: textRect, options: .usesLineFragmentOrigin, attributes: attrs, context: nil)
            case .outline:
                // Stamping the text round a circle gives an outline with round corners,
                // unlike a stroke, which spikes at sharp letter joins.
                var stroke = attrs
                stroke[.foregroundColor] = text.accent.uiColor
                let r = fontSize * 0.07
                for i in 0..<16 {
                    let a = Double(i) / 16 * 2 * .pi
                    ns.draw(with: textRect.offsetBy(dx: cos(a) * r, dy: sin(a) * r), options: .usesLineFragmentOrigin, attributes: stroke, context: nil)
                }
                ns.draw(with: textRect, options: .usesLineFragmentOrigin, attributes: attrs, context: nil)
            case .shadow, .glow:
                let shadow = NSShadow()
                shadow.shadowColor = text.accent.uiColor.withAlphaComponent(text.style == .glow ? 1 : 0.8)
                shadow.shadowBlurRadius = fontSize * (text.style == .glow ? 0.45 : 0.25)
                shadow.shadowOffset = text.style == .glow ? .zero : CGSize(width: 0, height: fontSize * 0.06)
                attrs[.shadow] = shadow
                ns.draw(with: textRect, options: .usesLineFragmentOrigin, attributes: attrs, context: nil)
                if text.style == .glow { ns.draw(with: textRect, options: .usesLineFragmentOrigin, attributes: attrs, context: nil) }
            }
        }
        guard let cg = picture.cgImage else { return nil }
        return CIImage(cgImage: cg)
    }

    private func drawShape(_ item: TextOverlay, canvas: CGSize) -> CIImage? {
        guard let kind = item.shape else { return nil }
        let side = max(16, item.size * canvas.width * 4)
        let width = kind == .line || kind == .arrow ? side * 1.6 : side
        let format = UIGraphicsImageRendererFormat()
        format.scale = 1
        format.opaque = false
        let stroke = item.style == .outline ? side * 0.04 : 0
        let rect = CGRect(x: stroke, y: stroke, width: width - stroke * 2, height: side - stroke * 2)
        let picture = UIGraphicsImageRenderer(size: CGSize(width: width, height: side), format: format).image { _ in
            let path = ShapePaths.path(for: kind, in: rect)
            item.color.uiColor.setFill()
            path.fill()
            if stroke > 0 {
                item.accent.uiColor.setStroke()
                path.lineWidth = stroke
                path.lineJoinStyle = .round
                path.stroke()
            }
        }
        guard let cg = picture.cgImage else { return nil }
        return CIImage(cgImage: cg)
    }
}

extension RGBA {
    var uiColor: UIColor { UIColor(red: r, green: g, blue: b, alpha: a) }
}

extension CGAffineTransform {
    /// The EXIF orientation that undoes a video track's preferredTransform.
    var videoOrientation: CGImagePropertyOrientation {
        switch (a, b, c, d) {
        case (0, 1, -1, 0): .right
        case (0, -1, 1, 0): .left
        case (-1, 0, 0, -1): .down
        case (-1, 0, 0, 1): .upMirrored
        default: .up
        }
    }
}
