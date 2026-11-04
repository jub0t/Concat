import CoreGraphics
import Foundation

/// The canvas a project renders to.
enum AspectRatio: String, Codable, CaseIterable, Identifiable {
    case portrait = "9:16"
    case landscape = "16:9"
    case square = "1:1"
    case social = "4:5"
    case classic = "4:3"
    case tall = "3:4"
    case cinema = "21:9"

    var id: String { rawValue }

    /// 1080p-class base size; exports scale it.
    var baseSize: CGSize {
        switch self {
        case .portrait: CGSize(width: 1080, height: 1920)
        case .landscape: CGSize(width: 1920, height: 1080)
        case .square: CGSize(width: 1080, height: 1080)
        case .social: CGSize(width: 1080, height: 1350)
        case .classic: CGSize(width: 1440, height: 1080)
        case .tall: CGSize(width: 1080, height: 1440)
        case .cinema: CGSize(width: 2520, height: 1080)
        }
    }

    var ratio: CGFloat { baseSize.width / baseSize.height }
}

struct RGBA: Codable, Hashable {
    var r: Double, g: Double, b: Double, a: Double = 1

    static let white = RGBA(r: 1, g: 1, b: 1)
    static let black = RGBA(r: 0, g: 0, b: 0)
    static let yellow = RGBA(r: 1, g: 0.86, b: 0.1)
    static let clear = RGBA(r: 0, g: 0, b: 0, a: 0)
    static let swatches: [RGBA] = [
        .white, .black, .yellow,
        RGBA(r: 1, g: 0.25, b: 0.3), RGBA(r: 0, g: 0.82, b: 0.78),
        RGBA(r: 0.3, g: 0.55, b: 1), RGBA(r: 0.65, g: 0.4, b: 1), RGBA(r: 1, g: 0.55, b: 0.15),
        RGBA(r: 0.2, g: 0.8, b: 0.35), RGBA(r: 1, g: 0.45, b: 0.7),
    ]
}

/// Where a layer sits and how solid it is. Offsets are fractions of the canvas.
struct Transform: Codable, Hashable {
    var scale = 1.0
    var x = 0.0
    var y = 0.0
    /// Degrees, clockwise.
    var rotation = 0.0
    var opacity = 1.0
    var volume = 1.0

    static func mix(_ a: Transform, _ b: Transform, _ t: Double) -> Transform {
        func m(_ p: Double, _ q: Double) -> Double { p + (q - p) * t }
        return Transform(scale: m(a.scale, b.scale), x: m(a.x, b.x), y: m(a.y, b.y),
                         rotation: m(a.rotation, b.rotation), opacity: m(a.opacity, b.opacity), volume: m(a.volume, b.volume))
    }
}

/// How a value travels from one keyframe to the next.
enum KeyframeEasing: String, Codable, CaseIterable, Identifiable {
    case smooth, linear, easeIn, easeOut, hold
    var id: String { rawValue }
    var label: String {
        switch self {
        case .smooth: "Smooth"
        case .linear: "Linear"
        case .easeIn: "Ease in"
        case .easeOut: "Ease out"
        case .hold: "Hold"
        }
    }
    func apply(_ t: Double) -> Double {
        switch self {
        case .smooth: t * t * (3 - 2 * t)
        case .linear: t
        case .easeIn: t * t * t
        case .easeOut: 1 - pow(1 - t, 3)
        case .hold: 0
        }
    }
}

/// A transform pinned to a moment of the clip. Time is seconds from the clip's start on the timeline.
struct Keyframe: Codable, Hashable, Identifiable {
    var id = UUID()
    var time: Double
    var value: Transform
    /// The curve from this keyframe to the next.
    var easing: KeyframeEasing = .smooth
}

/// Lift/gamma/gain-style tints: a colour and strength (`a`) for each tonal range.
struct ColorWheels: Codable, Hashable {
    static let neutral = RGBA(r: 0.5, g: 0.5, b: 0.5, a: 0)
    var shadows = ColorWheels.neutral
    var midtones = ColorWheels.neutral
    var highlights = ColorWheels.neutral
    var isNeutral: Bool { shadows.a == 0 && midtones.a == 0 && highlights.a == 0 }
}

/// How a clip's pixel levels are read, for footage that looks washed out or crushed.
enum ColorRangeMode: String, Codable, CaseIterable, Identifiable {
    case auto, limited, full
    var id: String { rawValue }
    var label: String {
        switch self {
        case .auto: "Auto"
        case .limited: "Expand (washed out)"
        case .full: "Compress (crushed)"
        }
    }
}

/// Hide, mute and lock for each kind of track.
struct TrackFlags: Codable, Hashable {
    var mainHidden = false, mainMuted = false, mainLocked = false
    var overlaysHidden = false, overlaysMuted = false, overlaysLocked = false
    var textHidden = false, textLocked = false
    var audioMuted = false, audioLocked = false
}

/// One word of a title drawn in its own colour; `index` counts words from 0.
struct WordHighlight: Codable, Hashable {
    var index: Int
    var color: RGBA
}

/// Normalised crop, origin top-left of the upright source.
struct CropRect: Codable, Hashable {
    var x = 0.0, y = 0.0, w = 1.0, h = 1.0
    var isFull: Bool { x <= 0.001 && y <= 0.001 && w >= 0.999 && h >= 0.999 }
}

enum CanvasBackground: String, Codable, CaseIterable, Identifiable {
    case black, white, blur, teal, purple, warm
    var id: String { rawValue }
    var label: String { rawValue.capitalized }
}

enum BlendMode: String, Codable, CaseIterable, Identifiable {
    case normal, screen, multiply, overlay, add, lighten, darken, softLight, hardLight, difference, color, luminosity
    var id: String { rawValue }
    var label: String {
        switch self {
        case .softLight: "Soft light"
        case .hardLight: "Hard light"
        default: rawValue.capitalized
        }
    }
}

enum MaskShape: String, Codable, CaseIterable, Identifiable {
    case none, circle, rectangle, rounded, heart, star
    var id: String { rawValue }
    var label: String { rawValue == "none" ? "None" : rawValue.capitalized }
}

/// Ways to cut the subject out of its background.
enum CutoutMode: String, Codable, CaseIterable, Identifiable {
    case none, person, subject, greenScreen, blueScreen, lumaKey, removeBlack
    var id: String { rawValue }
    var label: String {
        switch self {
        case .none: "Off"
        case .person: "Remove background (people)"
        case .subject: "Remove background (any subject)"
        case .greenScreen: "Green screen"
        case .blueScreen: "Blue screen"
        case .lumaKey: "Luma key"
        case .removeBlack: "Remove black"
        }
    }
    var short: String {
        switch self {
        case .none: "Off"
        case .person: "People"
        case .subject: "Subject"
        case .greenScreen: "Green"
        case .blueScreen: "Blue"
        case .lumaKey: "Luma"
        case .removeBlack: "Black"
        }
    }
}

/// Speed shapes over a clip's length: (position 0...1, speed).
enum SpeedCurve: String, Codable, CaseIterable, Identifiable {
    case none, montage, hero, bullet, jumpCut, flashIn, flashOut, custom
    var id: String { rawValue }
    var label: String {
        switch self {
        case .none: "Off"
        case .jumpCut: "Jump cut"
        case .flashIn: "Flash in"
        case .flashOut: "Flash out"
        default: rawValue.capitalized
        }
    }

    var points: [(Double, Double)] {
        switch self {
        case .none: [(0, 1), (1, 1)]
        case .montage: [(0, 0.7), (0.2, 0.7), (0.32, 3), (0.48, 0.7), (0.65, 0.7), (0.78, 3), (1, 0.7)]
        case .hero: [(0, 2.5), (0.28, 2.5), (0.4, 0.4), (0.6, 0.4), (0.72, 2.5), (1, 2.5)]
        case .bullet: [(0, 3), (0.35, 3), (0.45, 0.2), (0.55, 0.2), (0.65, 3), (1, 3)]
        case .jumpCut: [(0, 1), (0.3, 1), (0.35, 5), (0.45, 5), (0.5, 1), (0.8, 1), (0.85, 5), (1, 5)]
        case .flashIn: [(0, 4), (0.3, 4), (0.5, 1), (1, 1)]
        case .flashOut: [(0, 1), (0.5, 1), (0.7, 4), (1, 4)]
        case .custom: [(0, 1), (1, 1)]
        }
    }

    func speed(at p: Double) -> Double {
        let pts = points
        guard let after = pts.firstIndex(where: { $0.0 >= p }), after > 0 else { return pts.first?.1 ?? 1 }
        let a = pts[after - 1], b = pts[after]
        let t = b.0 == a.0 ? 0 : (p - a.0) / (b.0 - a.0)
        return a.1 + (b.1 - a.1) * t
    }
}

/// One stretch of source played at one speed; a curved clip is several.
struct SpeedPiece {
    var sourceStart: Double
    var sourceDuration: Double
    var speed: Double
    var duration: Double { sourceDuration / speed }
}

/// One shot on the main track, or the media inside an overlay.
struct Clip: Codable, Identifiable, Hashable {
    var id = UUID()
    /// File name inside the project's media folder.
    var file: String = ""
    var isPhoto = false
    var hasAudio = true
    var sourceDuration: Double = 0
    var trimStart: Double = 0
    var trimEnd: Double = 0
    var speed: Double = 1
    var speedCurve: SpeedCurve = .none
    /// The custom curve: speeds at six evenly spaced points across the clip.
    var speedPoints: [Double] = [1, 1, 1, 1, 1, 1]
    var volume: Double = 1
    var filter: String = "none"
    var filterIntensity: Double = 1
    var effects: [AppliedEffect] = []
    var brightness: Double = 0
    var contrast: Double = 1
    var saturation: Double = 1
    var exposure: Double = 0
    var temperature: Double = 0
    var tint: Double = 0
    var highlights: Double = 1
    var shadows: Double = 0
    var sharpness: Double = 0
    var vignette: Double = 0
    var wheels = ColorWheels()
    var colorRange: ColorRangeMode = .auto
    var fill = false
    var flipped = false
    var flippedV = false
    var crop = CropRect()
    var transform = Transform()
    var keyframes: [Keyframe] = []
    var cutout: CutoutMode = .none
    var fadeIn: Double = 0
    var fadeOut: Double = 0
    /// How this clip arrives from the one before it.
    var transitionIn = Transition()
    var audioEffect: AudioEffect = .none
    var cleanup = VoiceCleanup()

    /// Source stretches with their speeds, in play order.
    var pieces: [SpeedPiece] {
        let length = max(0.05, trimEnd - trimStart)
        guard speedCurve != .none else {
            return [SpeedPiece(sourceStart: trimStart, sourceDuration: length, speed: max(0.05, speed))]
        }
        let n = 24
        let step = length / Double(n)
        return (0..<n).map { i in
            let p = (Double(i) + 0.5) / Double(n)
            let factor = speedCurve == .custom ? Self.interpolate(speedPoints, at: p) : speedCurve.speed(at: p)
            return SpeedPiece(sourceStart: trimStart + Double(i) * step, sourceDuration: step,
                              speed: max(0.05, speed * factor))
        }
    }

    /// A value between evenly spaced points, eased so the speed changes smoothly.
    static func interpolate(_ points: [Double], at p: Double) -> Double {
        guard points.count > 1 else { return points.first ?? 1 }
        let x = max(0, min(1, p)) * Double(points.count - 1)
        let i = min(points.count - 2, Int(x))
        let t = x - Double(i)
        let e = t * t * (3 - 2 * t)
        return points[i] + (points[i + 1] - points[i]) * e
    }

    /// Seconds this clip takes on the timeline.
    var duration: Double { max(0.05, pieces.reduce(0) { $0 + $1.duration }) }

    /// Source second shown at `local` timeline seconds into the clip.
    func sourceTime(atLocal local: Double) -> Double {
        var t = max(0, local)
        for piece in pieces {
            if t <= piece.duration { return piece.sourceStart + t * piece.speed }
            t -= piece.duration
        }
        return trimEnd
    }

    /// The transform at `local` seconds, eased between keyframes.
    func transform(atLocal local: Double) -> Transform {
        let keys = keyframes.sorted { $0.time < $1.time }
        guard let first = keys.first, let last = keys.last else { return transform }
        if local <= first.time { return first.value }
        if local >= last.time { return last.value }
        guard let i = keys.firstIndex(where: { $0.time > local }) else { return last.value }
        let a = keys[i - 1], b = keys[i]
        let t = (local - a.time) / max(0.0001, b.time - a.time)
        return Transform.mix(a.value, b.value, a.easing.apply(t))
    }

    var hasAdjustments: Bool {
        brightness != 0 || contrast != 1 || saturation != 1 || exposure != 0 || temperature != 0 || tint != 0
            || highlights != 1 || shadows != 0 || sharpness != 0 || vignette != 0 || !wheels.isNeutral
            || colorRange != .auto
    }
}

/// A clip floating above the main track: picture-in-picture, stickers, B-roll.
struct OverlayItem: Codable, Identifiable, Hashable {
    var id = UUID()
    var clip = Clip()
    var start: Double = 0
    var blend: BlendMode = .normal
    var mask: MaskShape = .none
    var maskFeather: Double = 0
    var duration: Double { clip.duration }
    var end: Double { start + duration }
}

enum TextStyle: String, Codable, CaseIterable, Identifiable {
    case plain, outline, box, shadow, glow
    var id: String { rawValue }
    var label: String { rawValue.capitalized }
}

enum TextFont: String, Codable, CaseIterable, Identifiable {
    case system, rounded, serif, mono, condensed, avenir, futura, georgia, typewriter, marker, chalkboard,
         noteworthy, script, naskh, geeza, damascus, nastaliq
    var id: String { rawValue }
    var label: String {
        switch self {
        case .system: "Default"
        case .naskh: "نسخ"
        case .geeza: "جيزة"
        case .damascus: "دمشق"
        case .nastaliq: "نستعليق"
        default: rawValue.capitalized
        }
    }
}

enum TextCase: String, Codable, CaseIterable, Identifiable {
    case none, upper, lower
    var id: String { rawValue }
    var label: String { self == .none ? "Aa" : self == .upper ? "AA" : "aa" }
}

enum TextAnimation: String, Codable, CaseIterable, Identifiable {
    case none, fade, pop, slideUp, slideDown, zoom, typewriter, bounce
    var id: String { rawValue }
    var label: String {
        switch self {
        case .none: "None"
        case .slideUp: "Slide up"
        case .slideDown: "Slide down"
        default: rawValue.capitalized
        }
    }
}

enum ShapeKind: String, Codable, CaseIterable, Identifiable {
    case square, circle, triangle, parallelogram, star, heart, arrow, line
    var id: String { rawValue }
    var symbol: String {
        switch self {
        case .square: "square.fill"
        case .circle: "circle.fill"
        case .triangle: "triangle.fill"
        case .parallelogram: "rectangle.portrait.fill"
        case .star: "star.fill"
        case .heart: "heart.fill"
        case .arrow: "arrow.right"
        case .line: "minus"
        }
    }
}

/// A title, caption or shape floating over the video.
struct TextOverlay: Codable, Identifiable, Hashable {
    var id = UUID()
    var text: String = ""
    var start: Double = 0
    var duration: Double = 3
    var color: RGBA = .white
    var accent: RGBA = .black
    /// Font size as a fraction of the canvas width.
    var size: Double = 0.07
    var bold = true
    var style: TextStyle = .outline
    var font: TextFont = .system
    /// PostScript name of a font imported from Files; when set it wins over `font`.
    var customFont: String?
    /// Extra letter spacing, as a fraction of the font size.
    var tracking: Double = 0
    var textCase: TextCase = .none
    var animationIn: TextAnimation = .none
    var animationOut: TextAnimation = .none
    /// Centre, normalised, y measured from the top.
    var x: Double = 0.5
    var y: Double = 0.5
    var rotation: Double = 0
    var opacity: Double = 1
    var isCaption = false
    /// Words drawn in their own colour.
    var highlights: [WordHighlight] = []
    /// When each word is spoken, seconds from the start (captions made from speech).
    var wordStarts: [Double] = []
    /// Light up each word as it is spoken.
    var karaoke = false
    var karaokeColor: RGBA = .yellow
    /// When set, this item draws a shape instead of text.
    var shape: ShapeKind?

    var end: Double { start + duration }
}

enum AudioKind: String, Codable {
    case music, voiceover, extracted, sound
    var symbol: String {
        switch self {
        case .music: "music.note"
        case .voiceover: "waveform"
        case .extracted: "waveform.path"
        case .sound: "speaker.wave.2"
        }
    }
}

/// A sound on its own track: music, a voiceover, or audio pulled from a clip.
struct AudioItem: Codable, Identifiable, Hashable {
    var id = UUID()
    var file: String = ""
    var name: String = ""
    var kind: AudioKind = .music
    var start: Double = 0
    var sourceDuration: Double = 0
    var trimStart: Double = 0
    var trimEnd: Double = 0
    var volume: Double = 1
    var fadeIn: Double = 0
    var fadeOut: Double = 0
    /// Repeat to the end of the video.
    var loop = false
    var effect: AudioEffect = .none
    var cleanup = VoiceCleanup()
    /// The clip this sound was extracted from, so it can be put back.
    var sourceClip: UUID?

    var length: Double { max(0.05, trimEnd - trimStart) }
}

struct Project: Codable, Identifiable, Hashable {
    var id = UUID()
    var name: String = "Project"
    var createdAt = Date()
    var modifiedAt = Date()
    var aspect: AspectRatio = .portrait
    var background: CanvasBackground = .black
    var clips: [Clip] = []
    var overlays: [OverlayItem] = []
    var texts: [TextOverlay] = []
    var audios: [AudioItem] = []
    var tracks = TrackFlags()

    /// How much clip `index` overlaps the one before it.
    func overlap(at index: Int) -> Double {
        guard index > 0, index < clips.count, clips[index].transitionIn.kind != .none else { return 0 }
        let limit = min(clips[index - 1].duration, clips[index].duration) / 2
        return min(clips[index].transitionIn.duration, limit)
    }

    /// Timeline start of the clip at `index`.
    func start(ofClipAt index: Int) -> Double {
        var t = 0.0
        for i in 0..<index { t += clips[i].duration - overlap(at: i + 1) }
        return t
    }

    var duration: Double {
        guard !clips.isEmpty else { return 0 }
        return start(ofClipAt: clips.count - 1) + clips[clips.count - 1].duration
    }

    /// The clip under timeline time `t`, with its timeline start. During a
    /// transition this is the incoming clip.
    func clip(at t: Double) -> (index: Int, start: Double)? {
        guard !clips.isEmpty else { return nil }
        var found = (0, 0.0)
        for i in clips.indices {
            let s = start(ofClipAt: i)
            if t >= s - 0.0001 { found = (i, s) } else { break }
        }
        return (found.0, found.1)
    }

    /// Properties that change what AVFoundation has to load and lay out.
    /// Anything else only changes how frames are drawn.
    var structureKey: String {
        let c = clips.map {
            "\($0.id)|\($0.file)|\($0.trimStart)|\($0.trimEnd)|\($0.speed)|\($0.speedCurve)|\($0.speedPoints)|\($0.volume)|\($0.transitionIn.kind)|\($0.transitionIn.duration)|\($0.audioEffect)|\($0.cleanup)|\($0.keyframes.map { "\($0.time):\($0.value.volume)" })|\($0.fadeIn)|\($0.fadeOut)"
        }
        let o = overlays.map { "\($0.id)|\($0.clip.file)|\($0.start)|\($0.clip.trimStart)|\($0.clip.trimEnd)|\($0.clip.speed)|\($0.clip.speedCurve)|\($0.clip.speedPoints)|\($0.clip.volume)|\($0.clip.audioEffect)|\($0.clip.cleanup)" }
        let a = audios.map { "\($0.id)|\($0.file)|\($0.start)|\($0.trimStart)|\($0.trimEnd)|\($0.volume)|\($0.loop)|\($0.fadeIn)|\($0.fadeOut)|\($0.effect)|\($0.cleanup)" }
        return c.joined(separator: ",") + "#" + o.joined(separator: ",") + "#" + a.joined(separator: ",") + "#" + aspect.rawValue
            + "#\(tracks.mainMuted)\(tracks.overlaysMuted)\(tracks.audioMuted)"
    }
}
