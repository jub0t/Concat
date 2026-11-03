import CoreImage
import CoreImage.CIFilterBuiltins
import UIKit

/// Blends the outgoing frame `a` into the incoming frame `b` at progress `p` (0...1).
/// Both are full canvas frames.
enum Transitions {
    static func apply(_ kind: TransitionKind, from a: CIImage, to b: CIImage, progress raw: Double, canvas: CGSize) -> CIImage {
        let rect = CGRect(origin: .zero, size: canvas)
        let p = max(0, min(1, raw))
        let e = p * p * (3 - 2 * p)
        let bump = sin(p * .pi)
        let w = canvas.width, h = canvas.height
        let center = CGPoint(x: w / 2, y: h / 2)
        func moved(_ img: CIImage, _ dx: CGFloat, _ dy: CGFloat) -> CIImage {
            img.transformed(by: CGAffineTransform(translationX: dx, y: dy))
        }
        func scaled(_ img: CIImage, _ s: CGFloat, rotate: CGFloat = 0) -> CIImage {
            img.transformed(by: CGAffineTransform(translationX: -center.x, y: -center.y)
                .concatenating(CGAffineTransform(scaleX: s, y: s))
                .concatenating(CGAffineTransform(rotationAngle: rotate))
                .concatenating(CGAffineTransform(translationX: center.x, y: center.y)))
        }
        let black = CIImage(color: .black).cropped(to: rect)
        let white = CIImage(color: .white).cropped(to: rect)

        let out: CIImage
        switch kind {
        case .none:
            out = p < 0.5 ? a : b
        case .dissolve:
            out = Imaging.dissolve(a, b, e)
        case .fadeBlack:
            out = p < 0.5 ? Imaging.dissolve(a, black, p * 2) : Imaging.dissolve(black, b, (p - 0.5) * 2)
        case .fadeWhite:
            out = p < 0.5 ? Imaging.dissolve(a, white, p * 2) : Imaging.dissolve(white, b, (p - 0.5) * 2)
        case .blurDissolve:
            out = Imaging.dissolve(blur(a, bump * 40, rect), blur(b, bump * 40, rect), e)
        case .filmBurn:
            let base = Imaging.dissolve(a, b, e)
            let g = CIFilter.radialGradient()
            g.center = CGPoint(x: w * (0.2 + 0.6 * p), y: h * 0.6); g.radius0 = 0; g.radius1 = Float(max(w, h) * 0.8)
            g.color0 = CIColor(red: 1, green: 0.55, blue: 0.15, alpha: bump); g.color1 = CIColor(red: 0.9, green: 0.2, blue: 0, alpha: 0)
            out = Imaging.add(g.outputImage?.cropped(to: rect), over: base)
        case .glitch, .glitchWarp, .rgbSplit:
            let src = p < 0.5 ? a : b
            var img = Imaging.chromatic(src, offset: bump * w * (kind == .rgbSplit ? 0.05 : 0.025))
            if kind != .rgbSplit {
                let jitter = (p * 37).truncatingRemainder(dividingBy: 1) > 0.5 ? bump * w * 0.04 : -bump * w * 0.03
                img = moved(img, jitter, 0)
            }
            if kind == .glitchWarp {
                let f = CIFilter.twirlDistortion(); f.inputImage = img.clampedToExtent(); f.center = center
                f.radius = Float(min(w, h) * 0.6); f.angle = Float(bump * 1.5)
                img = f.outputImage ?? img
            }
            out = img
        case .cube:
            let pa = perspective(a, rect: rect, side: e, leaving: true)
            let pb = perspective(b, rect: rect, side: e, leaving: false)
            out = pb.composited(over: pa).composited(over: black)
        case .lightLeak:
            let base = Imaging.dissolve(a, b, e)
            let g = CIFilter.linearGradient()
            g.point0 = CGPoint(x: w * (p * 1.5 - 0.3), y: 0); g.point1 = CGPoint(x: w * (p * 1.5 + 0.3), y: h)
            g.color0 = CIColor(red: 1, green: 0.4, blue: 0.5, alpha: 0); g.color1 = CIColor(red: 1, green: 0.75, blue: 0.3, alpha: bump * 0.9)
            out = Imaging.add(g.outputImage?.cropped(to: rect), over: base)
        case .flash:
            let base = Imaging.dissolve(a, b, e)
            out = Imaging.dissolve(base, white, pow(bump, 2) * 0.95)
        case .push:
            out = moved(b, w * (1 - e), 0).composited(over: moved(a, -w * e, 0))
        case .slide:
            out = moved(b, w * (1 - e), 0).composited(over: a)
        case .slideUp:
            out = moved(b, 0, -h * (1 - e)).composited(over: a)
        case .spin:
            if p < 0.5 {
                out = scaled(a, 1 - 0.6 * e, rotate: CGFloat(e) * .pi).composited(over: black)
            } else {
                out = scaled(b, 0.4 + 0.6 * e, rotate: -CGFloat(1 - e) * .pi).composited(over: black)
            }
        case .swirl:
            let src = p < 0.5 ? a : b
            let f = CIFilter.twirlDistortion(); f.inputImage = src.clampedToExtent(); f.center = center
            f.radius = Float(max(w, h)); f.angle = Float(bump * .pi * 3)
            out = f.outputImage ?? src
        case .clockWipe:
            out = masked(b, over: a, mask: clockMask(progress: e, rect: rect))
        case .iris:
            let g = CIFilter.radialGradient()
            let r = Float(hypot(w, h) / 2 * e)
            g.center = center; g.radius0 = max(0, r - 6); g.radius1 = r + 6
            g.color0 = .white; g.color1 = CIColor(red: 0, green: 0, blue: 0, alpha: 0)
            out = masked(b, over: a, mask: g.outputImage)
        case .linearWipe:
            let g = CIFilter.linearGradient()
            let x = w * (1.2 * e - 0.1)
            g.point0 = CGPoint(x: x - w * 0.05, y: 0); g.point1 = CGPoint(x: x + w * 0.05, y: 0)
            g.color0 = .white; g.color1 = CIColor(red: 0, green: 0, blue: 0, alpha: 0)
            out = masked(b, over: a, mask: g.outputImage)
        case .shapeWipe:
            let side = max(w, h) * 1.5 * e
            let square = CIImage(color: .white).cropped(to: CGRect(x: -side / 2, y: -side / 2, width: side, height: side))
                .transformed(by: CGAffineTransform(rotationAngle: .pi / 4))
                .transformed(by: CGAffineTransform(translationX: center.x, y: center.y))
            out = masked(b, over: a, mask: square)
        case .whipPan:
            let pushed = moved(b, w * (1 - e), 0).composited(over: moved(a, -w * e, 0))
            let f = CIFilter.motionBlur(); f.inputImage = pushed.clampedToExtent(); f.radius = Float(bump * w * 0.08); f.angle = 0
            out = f.outputImage ?? pushed
        case .zoom:
            out = p < 0.5
                ? Imaging.dissolve(scaled(a, 1 + 1.5 * e), scaled(b, 2.2 - e), e)
                : Imaging.dissolve(scaled(a, 1 + 1.5 * e), scaled(b, 2 - e), e)
        case .zoomBlur:
            let src = p < 0.5 ? a : b
            let f = CIFilter.zoomBlur(); f.inputImage = src.clampedToExtent(); f.center = center; f.amount = Float(bump * 60)
            out = f.outputImage ?? src
        case .pageCurl:
            let f = CIFilter.pageCurlTransition()
            f.inputImage = a; f.targetImage = b; f.backsideImage = a; f.extent = rect
            f.time = Float(e); f.angle = Float.pi * 0.8; f.radius = Float(min(w, h) * 0.2)
            out = f.outputImage ?? b
        case .ripple:
            let f = CIFilter.rippleTransition()
            f.inputImage = a; f.targetImage = b; f.center = center; f.extent = rect
            f.time = Float(e); f.width = Float(min(w, h) * 0.15); f.scale = 40
            f.shadingImage = CIImage(color: CIColor(red: 0.5, green: 0.5, blue: 0.5)).cropped(to: rect)
            out = f.outputImage ?? b
        }
        return out.cropped(to: rect)
    }

    private static func blur(_ img: CIImage, _ radius: Double, _ rect: CGRect) -> CIImage {
        guard radius > 0.5 else { return img }
        let f = CIFilter.gaussianBlur(); f.inputImage = img.clampedToExtent(); f.radius = Float(radius)
        return f.outputImage?.cropped(to: rect) ?? img
    }

    private static func masked(_ top: CIImage, over base: CIImage, mask: CIImage?) -> CIImage {
        guard let mask else { return top }
        let f = CIFilter.blendWithMask()
        f.inputImage = top; f.backgroundImage = base; f.maskImage = mask
        return f.outputImage ?? top
    }

    /// One face of a turning cube.
    private static func perspective(_ img: CIImage, rect: CGRect, side p: Double, leaving: Bool) -> CIImage {
        let w = rect.width, h = rect.height
        let f = CIFilter.perspectiveTransform()
        f.inputImage = img
        let shrink = h * 0.12
        if leaving {
            let x = w * (1 - p)  // right edge moves left
            f.topLeft = CGPoint(x: 0, y: h); f.bottomLeft = CGPoint(x: 0, y: 0)
            f.topRight = CGPoint(x: x, y: h - shrink * p); f.bottomRight = CGPoint(x: x, y: shrink * p)
            let out = f.outputImage ?? img
            return out.transformed(by: CGAffineTransform(translationX: 0, y: 0)).cropped(to: CGRect(x: 0, y: 0, width: max(1, x), height: h))
        } else {
            let x = w * (1 - p)
            f.topLeft = CGPoint(x: x, y: h - shrink * (1 - p)); f.bottomLeft = CGPoint(x: x, y: shrink * (1 - p))
            f.topRight = CGPoint(x: w, y: h); f.bottomRight = CGPoint(x: w, y: 0)
            return f.outputImage ?? img
        }
    }

    private static func clockMask(progress: Double, rect: CGRect) -> CIImage? {
        let side: CGFloat = 256
        let picture = UIGraphicsImageRenderer(size: CGSize(width: side, height: side)).image { ctx in
            UIColor.black.setFill(); ctx.fill(CGRect(x: 0, y: 0, width: side, height: side))
            let c = CGPoint(x: side / 2, y: side / 2)
            let path = UIBezierPath()
            path.move(to: c)
            path.addArc(withCenter: c, radius: side, startAngle: -.pi / 2, endAngle: -.pi / 2 + CGFloat(progress) * 2 * .pi, clockwise: true)
            path.close()
            UIColor.white.setFill(); path.fill()
        }
        guard let cg = picture.cgImage else { return nil }
        let img = CIImage(cgImage: cg)
        return img.transformed(by: CGAffineTransform(scaleX: rect.width / side, y: rect.height / side))
    }
}
