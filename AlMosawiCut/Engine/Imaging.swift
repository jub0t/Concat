import AVFoundation
import CoreImage
import CoreImage.CIFilterBuiltins
import UIKit
import Vision

/// Per-clip picture work: adjustments, the filter look, effects and cutouts.
/// Everything here keeps the image's extent.
enum Imaging {
    // MARK: Grade

    static func grade(_ image: CIImage, clip: Clip, time: Double) -> CIImage {
        let extent = image.extent
        var x = image
        if clip.hasAdjustments { x = adjust(x, clip: clip) }
        if clip.filter != "none", let look = LookCatalog.byID[clip.filter] {
            let looked = apply(look, to: x, time: time).cropped(to: extent)
            x = clip.filterIntensity >= 0.999 ? looked : dissolve(x, looked, clip.filterIntensity)
        }
        for effect in clip.effects { x = apply(effect, to: x, time: time).cropped(to: extent) }
        return x.cropped(to: extent)
    }

    private static func adjust(_ image: CIImage, clip: Clip) -> CIImage {
        var x = image
        let extent = image.extent
        if clip.colorRange != .auto {
            // Video levels run 16-235; re-read them as full range or squeeze full range into them.
            let f = CIFilter.colorMatrix()
            f.inputImage = x
            let (gain, bias): (CGFloat, CGFloat) = clip.colorRange == .limited ? (255.0 / 219.0, -16.0 / 219.0) : (219.0 / 255.0, 16.0 / 255.0)
            f.rVector = CIVector(x: gain, y: 0, z: 0, w: 0)
            f.gVector = CIVector(x: 0, y: gain, z: 0, w: 0)
            f.bVector = CIVector(x: 0, y: 0, z: gain, w: 0)
            f.biasVector = CIVector(x: bias, y: bias, z: bias, w: 0)
            x = f.outputImage ?? x
        }
        if clip.exposure != 0 {
            let f = CIFilter.exposureAdjust(); f.inputImage = x; f.ev = Float(clip.exposure * 2); x = f.outputImage ?? x
        }
        if clip.temperature != 0 || clip.tint != 0 {
            x = temperature(x, kelvin: clip.temperature * 2500, tint: clip.tint * 40)
        }
        if clip.brightness != 0 || clip.contrast != 1 || clip.saturation != 1 {
            x = controls(x, saturation: clip.saturation, contrast: clip.contrast, brightness: clip.brightness)
        }
        if clip.highlights != 1 || clip.shadows != 0 {
            let f = CIFilter.highlightShadowAdjust()
            f.inputImage = x; f.highlightAmount = Float(clip.highlights); f.shadowAmount = Float(clip.shadows)
            x = f.outputImage ?? x
        }
        if clip.sharpness != 0 {
            let f = CIFilter.sharpenLuminance(); f.inputImage = x; f.sharpness = Float(clip.sharpness * 2); x = f.outputImage ?? x
        }
        if clip.vignette != 0 { x = vignette(x, amount: clip.vignette * 2, extent: extent) }
        if !clip.wheels.isNeutral { x = colorWheels(x, clip.wheels) }
        if clip.enhance > 0 {
            // Clean up noise, then bring back edge detail.
            let n = CIFilter.noiseReduction()
            n.inputImage = x; n.noiseLevel = Float(0.01 + clip.enhance * 0.03); n.sharpness = Float(0.4 + clip.enhance * 0.6)
            x = n.outputImage ?? x
            let u = CIFilter.unsharpMask()
            u.inputImage = x; u.radius = Float(max(2, max(extent.width, extent.height) / 600)); u.intensity = Float(clip.enhance * 0.8)
            x = u.outputImage ?? x
        }
        return x.cropped(to: extent)
    }

    /// Pushes shadows, midtones and highlights toward their wheel's colour, weighted by brightness.
    private static func colorWheels(_ image: CIImage, _ w: ColorWheels) -> CIImage {
        let size = 24
        let key = "wheels-\(w.hashValue)"
        let data: Data = cubeCache.withLock { cache in
            if let hit = cache[key] { return hit }
            func push(_ c: RGBA) -> (Double, Double, Double) { ((c.r - 0.5) * c.a * 0.5, (c.g - 0.5) * c.a * 0.5, (c.b - 0.5) * c.a * 0.5) }
            let s = push(w.shadows), m = push(w.midtones), h = push(w.highlights)
            var cube = [Float](repeating: 0, count: size * size * size * 4)
            var i = 0
            for b in 0..<size {
                for g in 0..<size {
                    for r in 0..<size {
                        let rf = Double(r) / Double(size - 1), gf = Double(g) / Double(size - 1), bf = Double(b) / Double(size - 1)
                        let l = 0.299 * rf + 0.587 * gf + 0.114 * bf
                        let ws = (1 - l) * (1 - l), wh = l * l, wm = max(0, 1 - ws - wh)
                        cube[i] = Float(min(1, max(0, rf + s.0 * ws + m.0 * wm + h.0 * wh)))
                        cube[i + 1] = Float(min(1, max(0, gf + s.1 * ws + m.1 * wm + h.1 * wh)))
                        cube[i + 2] = Float(min(1, max(0, bf + s.2 * ws + m.2 * wm + h.2 * wh)))
                        cube[i + 3] = 1
                        i += 4
                    }
                }
            }
            let d = cube.withUnsafeBufferPointer { Data(buffer: $0) }
            cache[key] = d
            return d
        }
        let f = CIFilter.colorCube()
        f.inputImage = image; f.cubeDimension = Float(size); f.cubeData = data
        return f.outputImage ?? image
    }

    // MARK: Looks

    static func apply(_ look: Look, to image: CIImage, time: Double) -> CIImage {
        let extent = image.extent
        var x = image
        if look.temperature != 0 || look.tint != 0 { x = temperature(x, kelvin: look.temperature, tint: look.tint) }
        if look.vibrance != 0 {
            let f = CIFilter.vibrance(); f.inputImage = x; f.amount = Float(look.vibrance); x = f.outputImage ?? x
        }
        if look.saturation != 1 || look.contrast != 1 || look.brightness != 0 {
            x = controls(x, saturation: look.saturation, contrast: look.contrast, brightness: look.brightness)
        }
        if look.highlights != 1 || look.shadows != 0 {
            let f = CIFilter.highlightShadowAdjust()
            f.inputImage = x; f.highlightAmount = Float(look.highlights); f.shadowAmount = Float(look.shadows)
            x = f.outputImage ?? x
        }
        if look.gain != (1, 1, 1) || look.lift != (0, 0, 0) || look.fade != 0 {
            let k = 1 - look.fade
            let f = CIFilter.colorMatrix()
            f.inputImage = x
            f.rVector = CIVector(x: look.gain.0 * k, y: 0, z: 0, w: 0)
            f.gVector = CIVector(x: 0, y: look.gain.1 * k, z: 0, w: 0)
            f.bVector = CIVector(x: 0, y: 0, z: look.gain.2 * k, w: 0)
            f.biasVector = CIVector(x: look.lift.0 + look.fade, y: look.lift.1 + look.fade, z: look.lift.2 + look.fade, w: 0)
            x = f.outputImage ?? x
        }
        if let mono = look.mono, let f = CIFilter(name: mono) {
            f.setValue(x, forKey: kCIInputImageKey); x = f.outputImage ?? x
        }
        if look.sepia > 0 {
            let f = CIFilter.sepiaTone(); f.inputImage = x; f.intensity = Float(look.sepia); x = f.outputImage ?? x
        }
        if look.bloom > 0 { x = bloom(x, radius: extent.width * 0.015, intensity: look.bloom, extent: extent) }
        if look.posterize > 0 {
            let f = CIFilter.colorPosterize(); f.inputImage = x; f.levels = Float(4 + (1 - look.posterize) * 10); x = f.outputImage ?? x
        }
        if look.chromatic > 0 { x = chromatic(x, offset: look.chromatic * extent.width * 0.004) }
        if look.vignette > 0 { x = vignette(x, amount: look.vignette, extent: extent) }
        if look.grain > 0 { x = grain(x, amount: look.grain * 0.12, time: time, extent: extent) }
        return x.cropped(to: extent)
    }

    // MARK: Effects

    static func apply(_ effect: AppliedEffect, to image: CIImage, time: Double) -> CIImage {
        let extent = image.extent
        let a = effect.amount(at: time)
        let bi = (a - 0.5) * 2
        let center = CIVector(x: extent.midX, y: extent.midY)
        let short = min(extent.width, extent.height)
        switch effect.kind {
        case .sharpen:
            let f = CIFilter.sharpenLuminance(); f.inputImage = image; f.sharpness = Float(a * 2); f.radius = 2.5
            return f.outputImage ?? image
        case .gaussianBlur:
            let f = CIFilter.gaussianBlur(); f.inputImage = image.clampedToExtent(); f.radius = Float(a * short * 0.04)
            return f.outputImage?.cropped(to: extent) ?? image
        case .motionBlur:
            let f = CIFilter.motionBlur(); f.inputImage = image.clampedToExtent(); f.radius = Float(a * short * 0.05)
            return f.outputImage?.cropped(to: extent) ?? image
        case .zoomBlur:
            let f = CIFilter.zoomBlur(); f.inputImage = image.clampedToExtent(); f.center = CGPoint(x: extent.midX, y: extent.midY)
            f.amount = Float(a * 40)
            return f.outputImage?.cropped(to: extent) ?? image
        case .exposure:
            let f = CIFilter.exposureAdjust(); f.inputImage = image; f.ev = Float(bi * 2)
            return f.outputImage ?? image
        case .whiteBalance:
            return temperature(image, kelvin: bi * 3000, tint: 0)
        case .curves:
            let f = CIFilter.toneCurve()
            f.inputImage = image
            f.point0 = CGPoint(x: 0, y: 0)
            f.point1 = CGPoint(x: 0.25, y: 0.25 - 0.12 * a)
            f.point2 = CGPoint(x: 0.5, y: 0.5)
            f.point3 = CGPoint(x: 0.75, y: 0.75 + 0.12 * a)
            f.point4 = CGPoint(x: 1, y: 1)
            return f.outputImage ?? image
        case .contrast:
            return controls(image, saturation: 1, contrast: 1 + bi * 0.6, brightness: 0)
        case .saturation:
            return controls(image, saturation: 1 + bi, contrast: 1, brightness: 0)
        case .greenScreen:
            return chromaKey(image, hue: 0.33, tolerance: 0.06 + a * 0.12)
        case .lumaKey:
            return lumaKey(image, threshold: a * 0.8, invert: false)
        case .removeBlack:
            return lumaKey(image, threshold: 0.05 + a * 0.3, invert: false)
        case .chromaticAberration:
            return chromatic(image, offset: a * extent.width * 0.015)
        case .fisheye:
            let f = CIFilter.bumpDistortion(); f.inputImage = image.clampedToExtent(); f.center = CGPoint(x: extent.midX, y: extent.midY)
            f.radius = Float(short * 0.7); f.scale = Float(a * 1.2)
            return f.outputImage?.cropped(to: extent) ?? image
        case .pixelate:
            let f = CIFilter.pixellate(); f.inputImage = image.clampedToExtent(); f.center = CGPoint(x: extent.midX, y: extent.midY)
            f.scale = Float(4 + a * short * 0.06)
            return f.outputImage?.cropped(to: extent) ?? image
        case .swirl:
            let f = CIFilter.twirlDistortion(); f.inputImage = image.clampedToExtent(); f.center = CGPoint(x: extent.midX, y: extent.midY)
            f.radius = Float(short * 0.6); f.angle = Float(a * .pi * 2)
            return f.outputImage?.cropped(to: extent) ?? image
        case .kaleidoscope:
            let f = CIFilter.kaleidoscope(); f.inputImage = image; f.count = Int(3 + a * 9); f.center = CGPoint(x: extent.midX, y: extent.midY)
            f.angle = Float(time * 0.3)
            return f.outputImage?.cropped(to: extent) ?? image
        case .halation:
            return halation(image, amount: a, extent: extent)
        case .corona:
            let glow = bloom(image, radius: short * 0.03, intensity: a, extent: extent)
            let g = CIFilter.radialGradient()
            g.center = CGPoint(x: extent.midX, y: extent.midY); g.radius0 = Float(short * 0.2); g.radius1 = Float(short * 0.8)
            g.color0 = CIColor(red: 1, green: 0.7, blue: 0.4, alpha: 0); g.color1 = CIColor(red: 1, green: 0.6, blue: 0.3, alpha: a * 0.35)
            return add(g.outputImage?.cropped(to: extent), over: glow)
        case .bloomPulse:
            let pulse = 0.5 + 0.5 * sin(time * .pi * 3)
            return bloom(image, radius: short * 0.03, intensity: a * 1.5 * pulse, extent: extent)
        case .shake:
            let d = a * short * 0.02
            let dx = sin(time * 37) * d, dy = cos(time * 29) * d
            let scaled = image.transformed(by: CGAffineTransform(translationX: -extent.midX, y: -extent.midY)
                .concatenating(CGAffineTransform(scaleX: 1.06, y: 1.06))
                .concatenating(CGAffineTransform(translationX: extent.midX + dx, y: extent.midY + dy)))
            return scaled.cropped(to: extent)
        case .strobe:
            let on = (time * 6).truncatingRemainder(dividingBy: 1) < 0.18
            return on ? controls(image, saturation: 1, contrast: 1, brightness: a * 0.6) : image
        case .halftone:
            let f = CIFilter.dotScreen(); f.inputImage = image; f.center = CGPoint(x: extent.midX, y: extent.midY)
            f.width = Float(4 + a * short * 0.02); f.sharpness = 0.7
            return f.outputImage?.cropped(to: extent) ?? image
        case .vhs:
            let shifted = chromatic(image, offset: a * extent.width * 0.008)
            return grain(scanlines(shifted, strength: a * 0.35, extent: extent), amount: a * 0.1, time: time, extent: extent)
        case .scanlines:
            return scanlines(image, strength: a * 0.6, extent: extent)
        case .filmGrain:
            return grain(image, amount: a * 0.25, time: time, extent: extent)
        case .glow:
            return bloom(image, radius: short * (0.01 + a * 0.03), intensity: a * 1.2, extent: extent)
        case .posterize:
            let f = CIFilter.colorPosterize(); f.inputImage = image; f.levels = Float(2 + (1 - a) * 14)
            return f.outputImage ?? image
        case .vignette:
            return vignette(image, amount: a * 2.5, extent: extent)
        case .comic:
            let f = CIFilter.comicEffect(); f.inputImage = image
            return dissolve(image, f.outputImage?.cropped(to: extent) ?? image, a)
        case .edges:
            let f = CIFilter.edges(); f.inputImage = image; f.intensity = Float(1 + a * 9)
            return f.outputImage?.cropped(to: extent) ?? image
        case .mirror:
            let half = CGRect(x: extent.minX, y: extent.minY, width: extent.width / 2, height: extent.height)
            let left = image.cropped(to: half)
            let flipped = left.transformed(by: CGAffineTransform(scaleX: -1, y: 1)
                .concatenating(CGAffineTransform(translationX: extent.minX * 2 + extent.width, y: 0)))
            return flipped.composited(over: left).cropped(to: extent)
        }
    }

    // MARK: Cutout

    /// The cut-out, then the painted strokes: Keep restores, Erase removes.
    static func cutout(_ image: CIImage, clip: Clip) -> CIImage {
        let cut = cutout(image, mode: clip.cutout)
        guard !clip.maskStrokes.isEmpty else { return cut }
        let masks = strokeMasks(clip.maskStrokes, extent: image.extent)
        var out = cut
        if let keep = masks.keep {
            let f = CIFilter.blendWithMask()
            f.inputImage = image; f.backgroundImage = out; f.maskImage = keep
            out = f.outputImage ?? out
        }
        if let erase = masks.erase {
            let f = CIFilter.blendWithMask()
            f.inputImage = CIImage.empty(); f.backgroundImage = out; f.maskImage = erase
            out = f.outputImage ?? out
        }
        return out.cropped(to: image.extent)
    }

    private static let strokeCache = Locked<[String: (CIImage?, CIImage?)]>([:])

    /// White where the strokes painted, black elsewhere, the size of `extent`.
    static func strokeMasks(_ strokes: [MaskStroke], extent: CGRect) -> (keep: CIImage?, erase: CIImage?) {
        let key = "\(strokes.hashValue)-\(Int(extent.width))x\(Int(extent.height))"
        if let hit = strokeCache.withLock({ $0[key] }) { return (hit.0, hit.1) }
        let scale = 512 / max(1, max(extent.width, extent.height))
        let size = CGSize(width: max(1, (extent.width * scale).rounded()), height: max(1, (extent.height * scale).rounded()))
        func render(_ list: [MaskStroke]) -> CIImage? {
            guard !list.isEmpty else { return nil }
            let format = UIGraphicsImageRendererFormat()
            format.scale = 1
            format.opaque = true
            let picture = UIGraphicsImageRenderer(size: size, format: format).image { ctx in
                UIColor.black.setFill()
                ctx.fill(CGRect(origin: .zero, size: size))
                UIColor.white.setFill()
                UIColor.white.setStroke()
                let short = min(size.width, size.height)
                for stroke in list {
                    let r = max(1, CGFloat(stroke.radius) * short)
                    let pts = stride(from: 0, to: stroke.points.count - 1, by: 2).map {
                        CGPoint(x: CGFloat(stroke.points[$0]) * size.width, y: CGFloat(stroke.points[$0 + 1]) * size.height)
                    }
                    guard let first = pts.first else { continue }
                    if pts.count == 1 {
                        UIBezierPath(ovalIn: CGRect(x: first.x - r, y: first.y - r, width: r * 2, height: r * 2)).fill()
                        continue
                    }
                    let path = UIBezierPath()
                    path.lineWidth = r * 2
                    path.lineCapStyle = .round
                    path.lineJoinStyle = .round
                    path.move(to: first)
                    for p in pts.dropFirst() { path.addLine(to: p) }
                    path.stroke()
                }
            }
            guard let cg = picture.cgImage else { return nil }
            return CIImage(cgImage: cg)
                .transformed(by: CGAffineTransform(scaleX: extent.width / size.width, y: extent.height / size.height))
                .transformed(by: CGAffineTransform(translationX: extent.minX, y: extent.minY))
        }
        let result = (render(strokes.filter(\.keep)), render(strokes.filter { !$0.keep }))
        strokeCache.withLock { cache in
            if cache.count > 40 { cache.removeAll() }
            cache[key] = result
        }
        return (result.0, result.1)
    }

    /// The picture moved against the measured shake, zoomed a little so no edge shows.
    static func stabilized(_ image: CIImage, clip: Clip, sourceTime: Double) -> CIImage {
        let path = clip.stabilizePath
        guard clip.stabilize > 0, path.count >= 4 else { return image }
        let n = path.count / 2
        let f = max(0, min(Double(n - 1), sourceTime * 10))
        let i = Int(f), j = min(n - 1, i + 1), t = f - Double(i)
        var dx = path[2 * i] + (path[2 * j] - path[2 * i]) * t
        var dy = path[2 * i + 1] + (path[2 * j + 1] - path[2 * i + 1]) * t
        if clip.flipped { dx = -dx }
        if clip.flippedV { dy = -dy }
        let e = image.extent
        let s = clip.stabilize
        let zoom = 1 + 0.12 * s
        let moved = image
            .transformed(by: CGAffineTransform(translationX: -e.midX, y: -e.midY))
            .transformed(by: CGAffineTransform(scaleX: zoom, y: zoom))
            .transformed(by: CGAffineTransform(translationX: e.midX + dx * e.width * s, y: e.midY + dy * e.height * s))
        return moved.clampedToExtent().cropped(to: e)
    }

    static func cutout(_ image: CIImage, mode: CutoutMode) -> CIImage {
        switch mode {
        case .none: return image
        case .person: return visionCutout(image, subject: false)
        case .subject: return visionCutout(image, subject: true)
        case .greenScreen: return chromaKey(image, hue: 0.33, tolerance: 0.12)
        case .blueScreen: return chromaKey(image, hue: 0.62, tolerance: 0.1)
        case .lumaKey: return lumaKey(image, threshold: 0.35, invert: false)
        case .removeBlack: return lumaKey(image, threshold: 0.12, invert: false)
        }
    }

    private static func visionCutout(_ image: CIImage, subject: Bool) -> CIImage {
        let extent = image.extent
        let handler = VNImageRequestHandler(ciImage: image, options: [:])
        var mask: CVPixelBuffer?
        if subject {
            let request = VNGenerateForegroundInstanceMaskRequest()
            if (try? handler.perform([request])) != nil, let result = request.results?.first {
                mask = try? result.generateScaledMaskForImage(forInstances: result.allInstances, from: handler)
            }
        } else {
            let request = VNGeneratePersonSegmentationRequest()
            request.qualityLevel = .balanced
            request.outputPixelFormat = kCVPixelFormatType_OneComponent8
            if (try? handler.perform([request])) != nil { mask = request.results?.first?.pixelBuffer }
        }
        guard let mask else { return image }
        var m = CIImage(cvPixelBuffer: mask)
        m = m.transformed(by: CGAffineTransform(scaleX: extent.width / m.extent.width, y: extent.height / m.extent.height))
            .transformed(by: CGAffineTransform(translationX: extent.minX, y: extent.minY))
        let f = CIFilter.blendWithMask()
        f.inputImage = image; f.backgroundImage = CIImage.empty(); f.maskImage = m
        return f.outputImage?.cropped(to: extent) ?? image
    }

    private static func chromaKey(_ image: CIImage, hue target: Double, tolerance: Double) -> CIImage {
        let size = 32
        let key = "\(target)-\(tolerance)"
        let data: Data = cubeCache.withLock { cache in
            if let hit = cache[key] { return hit }
            var cube = [Float](repeating: 0, count: size * size * size * 4)
            var i = 0
            for b in 0..<size {
                for g in 0..<size {
                    for r in 0..<size {
                        let rf = Double(r) / Double(size - 1), gf = Double(g) / Double(size - 1), bf = Double(b) / Double(size - 1)
                        let (h, s, v) = hsv(rf, gf, bf)
                        var d = abs(h - target); d = min(d, 1 - d)
                        let keyed = d < tolerance && s > 0.3 && v > 0.15
                        let alpha: Float = keyed ? 0 : 1
                        cube[i] = Float(rf) * alpha; cube[i + 1] = Float(gf) * alpha; cube[i + 2] = Float(bf) * alpha; cube[i + 3] = alpha
                        i += 4
                    }
                }
            }
            let d = cube.withUnsafeBufferPointer { Data(buffer: $0) }
            cache[key] = d
            return d
        }
        let f = CIFilter.colorCube()
        f.inputImage = image; f.cubeDimension = Float(size); f.cubeData = data
        return f.outputImage ?? image
    }

    private static let cubeCache = Locked<[String: Data]>([:])

    private static func hsv(_ r: Double, _ g: Double, _ b: Double) -> (Double, Double, Double) {
        let mx = max(r, g, b), mn = min(r, g, b), d = mx - mn
        var h = 0.0
        if d > 0 {
            if mx == r { h = ((g - b) / d).truncatingRemainder(dividingBy: 6) }
            else if mx == g { h = (b - r) / d + 2 }
            else { h = (r - g) / d + 4 }
            h /= 6; if h < 0 { h += 1 }
        }
        return (h, mx == 0 ? 0 : d / mx, mx)
    }

    private static func lumaKey(_ image: CIImage, threshold: Double, invert: Bool) -> CIImage {
        let k = 8.0
        let m = CIFilter.colorMatrix()
        m.inputImage = image
        let row = CIVector(x: 0.3 * k, y: 0.59 * k, z: 0.11 * k, w: 0)
        m.rVector = row; m.gVector = row; m.bVector = row
        m.aVector = CIVector(x: 0, y: 0, z: 0, w: 1)
        m.biasVector = CIVector(x: -threshold * k, y: -threshold * k, z: -threshold * k, w: 0)
        let clamp = CIFilter.colorClamp()
        clamp.inputImage = m.outputImage
        clamp.minComponents = CIVector(x: 0, y: 0, z: 0, w: 1); clamp.maxComponents = CIVector(x: 1, y: 1, z: 1, w: 1)
        let f = CIFilter.blendWithMask()
        f.inputImage = image; f.backgroundImage = CIImage.empty(); f.maskImage = clamp.outputImage
        return f.outputImage?.cropped(to: image.extent) ?? image
    }

    // MARK: Building blocks

    static func controls(_ image: CIImage, saturation: Double, contrast: Double, brightness: Double) -> CIImage {
        let f = CIFilter.colorControls()
        f.inputImage = image; f.saturation = Float(saturation); f.contrast = Float(contrast); f.brightness = Float(brightness)
        return f.outputImage ?? image
    }

    /// Positive kelvin warms the picture.
    static func temperature(_ image: CIImage, kelvin: Double, tint: Double) -> CIImage {
        let f = CIFilter.temperatureAndTint()
        f.inputImage = image
        f.neutral = CIVector(x: 6500 + kelvin, y: tint)
        f.targetNeutral = CIVector(x: 6500, y: 0)
        return f.outputImage ?? image
    }

    static func dissolve(_ a: CIImage, _ b: CIImage, _ amount: Double) -> CIImage {
        let f = CIFilter.dissolveTransition()
        f.inputImage = a; f.targetImage = b; f.time = Float(max(0, min(1, amount)))
        return f.outputImage?.cropped(to: a.extent) ?? b
    }

    static func vignette(_ image: CIImage, amount: Double, extent: CGRect) -> CIImage {
        let f = CIFilter.vignette()
        f.inputImage = image; f.intensity = Float(amount); f.radius = Float(max(extent.width, extent.height) / 900)
        return f.outputImage?.cropped(to: extent) ?? image
    }

    static func bloom(_ image: CIImage, radius: CGFloat, intensity: Double, extent: CGRect) -> CIImage {
        let f = CIFilter.bloom()
        f.inputImage = image.clampedToExtent(); f.radius = Float(radius); f.intensity = Float(intensity)
        return f.outputImage?.cropped(to: extent) ?? image
    }

    static func chromatic(_ image: CIImage, offset: CGFloat) -> CIImage {
        func channel(_ r: CGFloat, _ g: CGFloat, _ b: CGFloat) -> CIImage {
            let f = CIFilter.colorMatrix()
            f.inputImage = image
            f.rVector = CIVector(x: r, y: 0, z: 0, w: 0); f.gVector = CIVector(x: 0, y: g, z: 0, w: 0)
            f.bVector = CIVector(x: 0, y: 0, z: b, w: 0); f.aVector = CIVector(x: 0, y: 0, z: 0, w: 1)
            return f.outputImage ?? image
        }
        let red = channel(1, 0, 0).transformed(by: CGAffineTransform(translationX: offset, y: 0))
        let green = channel(0, 1, 0)
        let blue = channel(0, 0, 1).transformed(by: CGAffineTransform(translationX: -offset, y: 0))
        let rg = add(red, over: green)
        return add(blue, over: rg).cropped(to: image.extent)
    }

    static func add(_ top: CIImage?, over base: CIImage) -> CIImage {
        guard let top else { return base }
        let f = CIFilter.additionCompositing()
        f.inputImage = top; f.backgroundImage = base
        return f.outputImage?.cropped(to: base.extent) ?? base
    }

    static func grain(_ image: CIImage, amount: Double, time: Double, extent: CGRect) -> CIImage {
        guard let noise = CIFilter.randomGenerator().outputImage else { return image }
        let shift = CGFloat(Int(time * 24) % 97) * 37
        let m = CIFilter.colorMatrix()
        m.inputImage = noise.transformed(by: CGAffineTransform(translationX: shift, y: shift * 0.7))
        let v = CIVector(x: 0, y: amount, z: 0, w: 0)
        m.rVector = v; m.gVector = v; m.bVector = v
        m.aVector = CIVector(x: 0, y: 0, z: 0, w: 0)
        m.biasVector = CIVector(x: -amount / 2, y: -amount / 2, z: -amount / 2, w: 0)
        return add(m.outputImage?.cropped(to: extent), over: image)
    }

    static func scanlines(_ image: CIImage, strength: Double, extent: CGRect) -> CIImage {
        let s = CIFilter.stripesGenerator()
        s.color0 = CIColor(red: 0, green: 0, blue: 0, alpha: strength)
        s.color1 = CIColor(red: 0, green: 0, blue: 0, alpha: 0)
        s.width = Float(max(1.5, extent.height / 360)); s.sharpness = 0.6
        guard let stripes = s.outputImage?.transformed(by: CGAffineTransform(rotationAngle: .pi / 2)).cropped(to: extent) else { return image }
        return stripes.composited(over: image)
    }

    private static func halation(_ image: CIImage, amount: Double, extent: CGRect) -> CIImage {
        let m = CIFilter.colorMatrix()
        m.inputImage = image
        let row = CIVector(x: 0.3 * 3, y: 0.59 * 3, z: 0.11 * 3, w: 0)
        m.rVector = row; m.gVector = CIVector(x: 0.1, y: 0.2, z: 0.05, w: 0); m.bVector = CIVector(x: 0, y: 0, z: 0, w: 0)
        m.biasVector = CIVector(x: -2, y: -0.2, z: 0, w: 0)
        let blur = CIFilter.gaussianBlur()
        blur.inputImage = m.outputImage?.clampedToExtent(); blur.radius = Float(min(extent.width, extent.height) * 0.02)
        let glow = CIFilter.colorMatrix()
        glow.inputImage = blur.outputImage?.cropped(to: extent)
        glow.rVector = CIVector(x: amount, y: 0, z: 0, w: 0); glow.gVector = CIVector(x: 0, y: amount, z: 0, w: 0)
        glow.bVector = CIVector(x: 0, y: 0, z: amount, w: 0); glow.aVector = CIVector(x: 0, y: 0, z: 0, w: 0)
        return add(glow.outputImage, over: image)
    }

    /// Alpha mask for an overlay's shape, the size of `extent`.
    static func shapeMask(_ shape: MaskShape, feather: Double, extent: CGRect) -> CIImage? {
        let short = min(extent.width, extent.height)
        var mask: CIImage?
        switch shape {
        case .none: return nil
        case .circle:
            let g = CIFilter.radialGradient()
            g.center = CGPoint(x: extent.midX, y: extent.midY)
            g.radius0 = Float(short / 2 * (1 - feather * 0.6)); g.radius1 = Float(short / 2)
            g.color0 = .white; g.color1 = CIColor(red: 0, green: 0, blue: 0, alpha: 0)
            mask = g.outputImage
        case .rectangle, .rounded:
            let g = CIFilter.roundedRectangleGenerator()
            let inset = short * 0.02
            g.extent = extent.insetBy(dx: inset, dy: inset)
            g.radius = Float(shape == .rounded ? short * 0.12 : 0)
            g.color = .white
            mask = g.outputImage
        case .heart, .star:
            mask = pathMask(shape, size: extent.size)?.transformed(by: CGAffineTransform(translationX: extent.minX, y: extent.minY))
        }
        if feather > 0, shape != .circle, let m = mask {
            let b = CIFilter.gaussianBlur(); b.inputImage = m; b.radius = Float(feather * short * 0.08)
            mask = b.outputImage
        }
        return mask?.cropped(to: extent)
    }

    private static func pathMask(_ shape: MaskShape, size: CGSize) -> CIImage? {
        let side: CGFloat = 256
        let picture = UIGraphicsImageRenderer(size: CGSize(width: side, height: side)).image { _ in
            UIColor.white.setFill()
            ShapePaths.path(for: shape == .heart ? .heart : .star, in: CGRect(x: 8, y: 8, width: side - 16, height: side - 16)).fill()
        }
        guard let cg = picture.cgImage else { return nil }
        let img = CIImage(cgImage: cg)
        return img.transformed(by: CGAffineTransform(scaleX: size.width / img.extent.width, y: size.height / img.extent.height))
    }
}

/// Measures a clip's shake: how far each frame moved from the last, the camera's
/// path smoothed, and the difference kept as a correction for every 1/10 s.
enum Stabilizer {
    static func analyse(_ url: URL, duration: Double, progress: @escaping @Sendable (Double) -> Void) async throws -> [Double] {
        let generator = AVAssetImageGenerator(asset: AVURLAsset(url: url))
        generator.appliesPreferredTrackTransform = true
        generator.maximumSize = CGSize(width: 320, height: 320)
        generator.requestedTimeToleranceBefore = CMTime(value: 1, timescale: 60)
        generator.requestedTimeToleranceAfter = CMTime(value: 1, timescale: 60)
        let count = max(2, Int(duration * 10))
        let times = (0..<count).map { CMTime(seconds: Double($0) / 10, preferredTimescale: 600) }
        var previous: CGImage?
        var x = 0.0, y = 0.0
        var path: [(Double, Double)] = []
        for await result in generator.images(for: times) {
            try Task.checkCancellation()
            guard let frame = try? result.image else {
                path.append((x, y))
                continue
            }
            if let previous {
                let request = VNTranslationalImageRegistrationRequest(targetedCGImage: frame)
                try? VNImageRequestHandler(cgImage: previous).perform([request])
                if let shift = request.results?.first?.alignmentTransform {
                    // The shift that lines this frame up with the last is the motion, reversed.
                    x -= Double(shift.tx) / Double(max(1, frame.width))
                    y -= Double(shift.ty) / Double(max(1, frame.height))
                }
            }
            previous = frame
            path.append((x, y))
            progress(Double(path.count) / Double(count))
        }
        // Follow a smoothed path (about a second either side), so pans stay but shake goes.
        let radius = 10
        var out: [Double] = []
        for i in path.indices {
            let lo = max(0, i - radius), hi = min(path.count - 1, i + radius)
            let span = path[lo...hi]
            let sx = span.reduce(0) { $0 + $1.0 } / Double(span.count)
            let sy = span.reduce(0) { $0 + $1.1 } / Double(span.count)
            out.append(sx - path[i].0)
            out.append(sy - path[i].1)
        }
        return out
    }
}

/// Paths for shape stickers and masks, drawn into a rect.
enum ShapePaths {
    static func path(for kind: ShapeKind, in r: CGRect) -> UIBezierPath {
        switch kind {
        case .square: return UIBezierPath(roundedRect: r, cornerRadius: r.width * 0.06)
        case .circle: return UIBezierPath(ovalIn: r)
        case .triangle:
            let p = UIBezierPath()
            p.move(to: CGPoint(x: r.midX, y: r.minY)); p.addLine(to: CGPoint(x: r.maxX, y: r.maxY)); p.addLine(to: CGPoint(x: r.minX, y: r.maxY)); p.close()
            return p
        case .parallelogram:
            let p = UIBezierPath(); let s = r.width * 0.25
            p.move(to: CGPoint(x: r.minX + s, y: r.minY + r.height * 0.2)); p.addLine(to: CGPoint(x: r.maxX, y: r.minY + r.height * 0.2))
            p.addLine(to: CGPoint(x: r.maxX - s, y: r.maxY - r.height * 0.2)); p.addLine(to: CGPoint(x: r.minX, y: r.maxY - r.height * 0.2)); p.close()
            return p
        case .star:
            let p = UIBezierPath()
            let c = CGPoint(x: r.midX, y: r.midY), outer = r.width / 2, inner = outer * 0.42
            for i in 0..<10 {
                let a = CGFloat(i) * .pi / 5 - .pi / 2
                let rad = i.isMultiple(of: 2) ? outer : inner
                let pt = CGPoint(x: c.x + cos(a) * rad, y: c.y + sin(a) * rad)
                i == 0 ? p.move(to: pt) : p.addLine(to: pt)
            }
            p.close(); return p
        case .heart:
            let p = UIBezierPath(); let w = r.width, h = r.height, x = r.minX, y = r.minY
            p.move(to: CGPoint(x: x + w / 2, y: y + h))
            p.addCurve(to: CGPoint(x: x, y: y + h * 0.3), controlPoint1: CGPoint(x: x + w * 0.1, y: y + h * 0.75), controlPoint2: CGPoint(x: x, y: y + h * 0.5))
            p.addArc(withCenter: CGPoint(x: x + w * 0.25, y: y + h * 0.28), radius: w * 0.25, startAngle: .pi, endAngle: 0, clockwise: true)
            p.addArc(withCenter: CGPoint(x: x + w * 0.75, y: y + h * 0.28), radius: w * 0.25, startAngle: .pi, endAngle: 0, clockwise: true)
            p.addCurve(to: CGPoint(x: x + w / 2, y: y + h), controlPoint1: CGPoint(x: x + w, y: y + h * 0.5), controlPoint2: CGPoint(x: x + w * 0.9, y: y + h * 0.75))
            p.close(); return p
        case .arrow:
            let p = UIBezierPath(); let mid = r.midY, t = r.height * 0.18
            p.move(to: CGPoint(x: r.minX, y: mid - t)); p.addLine(to: CGPoint(x: r.maxX - r.width * 0.35, y: mid - t))
            p.addLine(to: CGPoint(x: r.maxX - r.width * 0.35, y: r.minY + r.height * 0.15)); p.addLine(to: CGPoint(x: r.maxX, y: mid))
            p.addLine(to: CGPoint(x: r.maxX - r.width * 0.35, y: r.maxY - r.height * 0.15)); p.addLine(to: CGPoint(x: r.maxX - r.width * 0.35, y: mid + t))
            p.addLine(to: CGPoint(x: r.minX, y: mid + t)); p.close(); return p
        case .line:
            return UIBezierPath(roundedRect: CGRect(x: r.minX, y: r.midY - r.height * 0.05, width: r.width, height: r.height * 0.1), cornerRadius: r.height * 0.05)
        }
    }
}

/// A value shared across render threads.
final class Locked<Value>: @unchecked Sendable {
    private var value: Value
    private let lock = NSLock()
    init(_ value: Value) { self.value = value }
    func withLock<T>(_ body: (inout Value) -> T) -> T {
        lock.lock(); defer { lock.unlock() }
        return body(&value)
    }
}
