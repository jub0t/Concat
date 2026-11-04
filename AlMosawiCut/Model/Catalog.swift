import Foundation

// MARK: Transitions

enum TransitionKind: String, Codable, CaseIterable, Identifiable {
    case none
    case dissolve, fadeBlack, fadeWhite
    case blurDissolve, filmBurn
    case glitch, glitchWarp, rgbSplit
    case cube, lightLeak, flash
    case push, slide, slideUp
    case spin, swirl
    case clockWipe, iris, linearWipe, shapeWipe, whipPan
    case zoom, zoomBlur, pageCurl, ripple

    var id: String { rawValue }

    var label: String {
        switch self {
        case .none: "None"
        case .fadeBlack: "Fade to black"
        case .fadeWhite: "Fade to white"
        case .blurDissolve: "Blur dissolve"
        case .filmBurn: "Film burn"
        case .glitchWarp: "Glitch warp"
        case .rgbSplit: "RGB split"
        case .lightLeak: "Light leak"
        case .slideUp: "Slide up"
        case .clockWipe: "Clock wipe"
        case .linearWipe: "Linear wipe"
        case .shapeWipe: "Shape wipe"
        case .whipPan: "Whip pan"
        case .zoomBlur: "Zoom blur"
        case .pageCurl: "Page curl"
        default: rawValue.capitalized
        }
    }

    var symbol: String {
        switch self {
        case .none: "nosign"
        case .dissolve: "circle.lefthalf.filled"
        case .fadeBlack: "moon.fill"
        case .fadeWhite: "sun.max.fill"
        case .blurDissolve: "drop.fill"
        case .filmBurn: "flame.fill"
        case .glitch, .glitchWarp: "bolt.horizontal.fill"
        case .rgbSplit: "square.3.layers.3d"
        case .cube: "cube.fill"
        case .lightLeak: "sun.haze.fill"
        case .flash: "bolt.fill"
        case .push: "arrow.left.square.fill"
        case .slide: "rectangle.righthalf.inset.filled.arrow.right"
        case .slideUp: "arrow.up.square.fill"
        case .spin: "arrow.triangle.2.circlepath"
        case .swirl: "tornado"
        case .clockWipe: "clock.fill"
        case .iris: "smallcircle.filled.circle"
        case .linearWipe: "rectangle.lefthalf.filled"
        case .shapeWipe: "diamond.fill"
        case .whipPan: "wind"
        case .zoom: "plus.magnifyingglass"
        case .zoomBlur: "scope"
        case .pageCurl: "book.pages.fill"
        case .ripple: "water.waves"
        }
    }
}

struct Transition: Codable, Hashable {
    var kind: TransitionKind = .none
    var duration: Double = 0.5
}

// MARK: Video effects

enum VideoEffect: String, Codable, CaseIterable, Identifiable {
    case sharpen
    case gaussianBlur, motionBlur, zoomBlur
    case exposure, whiteBalance, curves, contrast, saturation
    case greenScreen, lumaKey, removeBlack
    case chromaticAberration, fisheye, pixelate, swirl, kaleidoscope
    case halation, corona
    case bloomPulse, shake, strobe
    case halftone, vhs, scanlines
    case filmGrain, glow, posterize, vignette, comic, edges, mirror

    var id: String { rawValue }

    var category: String {
        switch self {
        case .sharpen: "Basic"
        case .gaussianBlur, .motionBlur, .zoomBlur: "Blur"
        case .exposure, .whiteBalance, .curves, .contrast, .saturation: "Color"
        case .greenScreen, .lumaKey, .removeBlack: "Cutout"
        case .chromaticAberration, .fisheye, .pixelate, .swirl, .kaleidoscope: "Distort"
        case .halation, .corona: "Film"
        case .bloomPulse, .shake, .strobe: "Motion"
        case .halftone, .vhs, .scanlines: "Retro"
        case .filmGrain, .glow, .posterize, .vignette, .comic, .edges, .mirror: "Stylize"
        }
    }

    static let categories = ["Basic", "Blur", "Color", "Cutout", "Distort", "Film", "Motion", "Retro", "Stylize"]

    var label: String {
        switch self {
        case .gaussianBlur: "Gaussian blur"
        case .motionBlur: "Motion blur"
        case .zoomBlur: "Zoom blur"
        case .whiteBalance: "White balance"
        case .greenScreen: "Chroma key"
        case .lumaKey: "Luma key"
        case .removeBlack: "Remove black"
        case .chromaticAberration: "Chromatic aberration"
        case .bloomPulse: "Bloom pulse"
        case .filmGrain: "Film grain"
        case .vhs: "VHS"
        default: rawValue.capitalized
        }
    }

    var symbol: String {
        switch category {
        case "Basic": "triangle.lefthalf.filled"
        case "Blur": "drop.halffull"
        case "Color": "paintpalette.fill"
        case "Cutout": "person.crop.rectangle.badge.plus"
        case "Distort": "circle.hexagongrid.fill"
        case "Film": "film.fill"
        case "Motion": "waveform.path.ecg"
        case "Retro": "tv.fill"
        default: "sparkles"
        }
    }

    /// Effects whose slider runs both ways around a neutral middle.
    var isBipolar: Bool { [.exposure, .whiteBalance, .contrast, .saturation].contains(self) }
    var defaultAmount: Double { isBipolar ? 0.7 : 0.5 }
}

/// An effect's strength pinned to a moment of the clip (seconds from its start).
struct EffectKey: Codable, Hashable {
    var time: Double
    var amount: Double
}

struct AppliedEffect: Codable, Hashable, Identifiable {
    var id = UUID()
    var kind: VideoEffect = .sharpen
    var amount: Double = 0.5
    /// When set, the strength follows these keyframes instead of `amount`.
    var keys: [EffectKey] = []

    func amount(at t: Double) -> Double {
        let sorted = keys.sorted { $0.time < $1.time }
        guard let first = sorted.first, let last = sorted.last else { return amount }
        if t <= first.time { return first.amount }
        if t >= last.time { return last.amount }
        guard let i = sorted.firstIndex(where: { $0.time > t }) else { return last.amount }
        let a = sorted[i - 1], b = sorted[i]
        let p = (t - a.time) / max(0.0001, b.time - a.time)
        return a.amount + (b.amount - a.amount) * p * p * (3 - 2 * p)
    }
}

// MARK: Audio effects

enum AudioEffect: String, Codable, CaseIterable, Identifiable {
    case none
    case echo, room, hall, cathedral, stadium, plate, underwater
    case bass, treble, bright, distorted, lofi, radio, telephone, vinyl
    case chipmunk, deep, megaphone, robot, sweet, whisper, monster

    var id: String { rawValue }

    var category: String {
        switch self {
        case .none: ""
        case .echo, .room, .hall, .cathedral, .stadium, .plate, .underwater: "Space"
        case .bass, .treble, .bright, .distorted, .lofi, .radio, .telephone, .vinyl: "Tone"
        default: "Voice"
        }
    }

    var label: String { self == .lofi ? "Lo-fi" : rawValue.capitalized }

    var symbol: String {
        switch self {
        case .none: "nosign"
        case .echo: "dot.radiowaves.right"
        case .room, .hall, .stadium, .plate: "building.columns.fill"
        case .cathedral: "building.fill"
        case .underwater: "water.waves"
        case .bass: "speaker.wave.3.fill"
        case .treble, .bright: "sparkle"
        case .distorted: "bolt.fill"
        case .lofi, .vinyl: "opticaldisc.fill"
        case .radio: "radio.fill"
        case .telephone: "phone.fill"
        case .chipmunk: "hare.fill"
        case .deep, .monster: "tortoise.fill"
        case .megaphone: "megaphone.fill"
        case .robot: "cpu.fill"
        case .sweet: "heart.fill"
        case .whisper: "mouth.fill"
        }
    }
}

struct VoiceCleanup: Codable, Hashable, CustomStringConvertible {
    var enhance = false
    var denoise = false
    var normalize = false
    /// Pitch shift in semitones, keeping the timing.
    var pitch = 0.0
    var isOff: Bool { !enhance && !denoise && !normalize && pitch == 0 }
    var description: String {
        "\(enhance ? "e" : "")\(denoise ? "d" : "")\(normalize ? "n" : "")\(pitch != 0 ? "p\(Int((pitch * 10).rounded()))" : "")"
    }
}

// MARK: Filters

/// A named colour look, described as a handful of grading moves.
struct Look {
    var id: String
    var name: String
    var category: String
    var temperature = 0.0     // kelvin offset, + is warmer
    var tint = 0.0            // + is magenta
    var saturation = 1.0
    var contrast = 1.0
    var brightness = 0.0
    var vibrance = 0.0
    var fade = 0.0            // lift blacks
    var gain = (1.0, 1.0, 1.0)
    var lift = (0.0, 0.0, 0.0)
    var mono: String?         // a CIPhotoEffect name
    var sepia = 0.0
    var vignette = 0.0
    var grain = 0.0
    var bloom = 0.0
    var posterize = 0.0
    var chromatic = 0.0
    var shadows = 0.0
    var highlights = 1.0
}

enum LookCatalog {
    static let categories = ["Everyday", "Portrait", "Cinematic", "Film", "Retro", "Color", "Scenery", "Food", "Black & White", "Vibe", "Stylised"]

    static let all: [Look] = [
        // Everyday
        Look(id: "clean", name: "Clean", category: "Everyday", saturation: 1.05, contrast: 1.04, brightness: 0.02, shadows: 0.15),
        Look(id: "cloudlessSky", name: "Cloudless Sky", category: "Everyday", temperature: -600, saturation: 1.15, gain: (0.97, 1.0, 1.08)),
        Look(id: "fresh", name: "Fresh", category: "Everyday", temperature: -300, saturation: 1.12, brightness: 0.03, vibrance: 0.3),
        Look(id: "goldenHour", name: "Golden Hour", category: "Everyday", temperature: 1600, saturation: 1.12, contrast: 1.05, vignette: 0.5),
        Look(id: "ivory", name: "Ivory", category: "Everyday", temperature: 400, saturation: 0.85, brightness: 0.04, fade: 0.06),
        Look(id: "lagoon", name: "Lagoon", category: "Everyday", temperature: -900, tint: -10, saturation: 1.1, gain: (0.92, 1.03, 1.06)),
        Look(id: "milk", name: "Milk", category: "Everyday", saturation: 0.8, contrast: 0.9, brightness: 0.06, fade: 0.1),
        Look(id: "softLight", name: "Soft Light", category: "Everyday", contrast: 0.9, brightness: 0.03, bloom: 0.3, shadows: 0.2),
        Look(id: "umber", name: "Umber", category: "Everyday", temperature: 900, saturation: 0.9, contrast: 1.08, gain: (1.04, 0.98, 0.9)),
        Look(id: "vivid", name: "Vivid", category: "Everyday", saturation: 1.18, contrast: 1.08, vibrance: 0.6),
        // Portrait
        Look(id: "fair", name: "Fair", category: "Portrait", temperature: -150, saturation: 0.95, brightness: 0.05, shadows: 0.25, highlights: 0.9),
        Look(id: "natural", name: "Natural", category: "Portrait", temperature: 200, saturation: 1.03, contrast: 1.03, shadows: 0.1),
        Look(id: "peach", name: "Peach", category: "Portrait", temperature: 700, tint: 12, saturation: 1.05, brightness: 0.03, gain: (1.05, 0.98, 0.95)),
        Look(id: "rosy", name: "Rosy", category: "Portrait", tint: 18, saturation: 1.05, brightness: 0.02, gain: (1.06, 0.97, 1.0)),
        // Cinematic
        Look(id: "bleachBypass", name: "Bleach Bypass", category: "Cinematic", saturation: 0.45, contrast: 1.3, highlights: 0.85),
        Look(id: "coolBlue", name: "Cool Blue", category: "Cinematic", temperature: -1800, saturation: 0.9, contrast: 1.1, gain: (0.9, 0.98, 1.1)),
        Look(id: "cyberpunk", name: "Cyberpunk", category: "Cinematic", tint: 30, saturation: 1.3, contrast: 1.15, gain: (1.1, 0.8, 1.2)),
        Look(id: "daydream", name: "Daydream", category: "Cinematic", temperature: 300, saturation: 0.9, contrast: 0.9, brightness: 0.04, fade: 0.08, bloom: 0.4),
        Look(id: "film", name: "Film", category: "Cinematic", temperature: 300, saturation: 0.9, contrast: 1.1, fade: 0.05, vignette: 0.4, grain: 0.25),
        Look(id: "midnight", name: "Midnight", category: "Cinematic", temperature: -2400, saturation: 0.8, contrast: 1.15, brightness: -0.06, gain: (0.85, 0.92, 1.1)),
        Look(id: "moody", name: "Moody", category: "Cinematic", temperature: -300, saturation: 0.75, contrast: 1.15, brightness: -0.04, vignette: 0.7),
        Look(id: "night", name: "Night", category: "Cinematic", temperature: -1500, saturation: 0.7, contrast: 1.2, brightness: -0.08, lift: (0, 0.01, 0.04)),
        Look(id: "noir", name: "Noir", category: "Cinematic", contrast: 1.2, mono: "CIPhotoEffectNoir", vignette: 0.6),
        Look(id: "rain", name: "Rain", category: "Cinematic", temperature: -1000, saturation: 0.65, contrast: 0.95, fade: 0.08),
        Look(id: "sage", name: "Sage", category: "Cinematic", tint: -15, saturation: 0.8, fade: 0.05, gain: (0.96, 1.03, 0.94)),
        Look(id: "sciFi", name: "Sci-Fi", category: "Cinematic", temperature: -1200, tint: -20, saturation: 1.05, contrast: 1.2, gain: (0.88, 1.06, 1.05)),
        Look(id: "tealOrange", name: "Teal & Orange", category: "Cinematic", temperature: 500, saturation: 1.12, contrast: 1.1, gain: (1.06, 0.98, 0.95), lift: (0, 0.03, 0.05)),
        // Film
        Look(id: "classicChrome", name: "Classic Chrome", category: "Film", saturation: 0.8, contrast: 1.12, mono: nil, shadows: -0.1),
        Look(id: "retro70s", name: "Retro 70s", category: "Film", temperature: 1400, saturation: 0.85, fade: 0.12, gain: (1.05, 1.0, 0.85), grain: 0.2),
        Look(id: "super8", name: "Super 8", category: "Film", temperature: 1000, saturation: 0.9, contrast: 1.12, fade: 0.08, vignette: 0.9, grain: 0.5),
        // Retro
        Look(id: "bronze", name: "Bronze", category: "Retro", contrast: 1.1, sepia: 0.6, vignette: 0.5),
        Look(id: "digicam", name: "Digicam", category: "Retro", temperature: -200, saturation: 1.15, contrast: 1.15, brightness: 0.04, grain: 0.15, bloom: 0.3),
        Look(id: "fallingDust", name: "Falling Dust", category: "Retro", temperature: 600, saturation: 0.8, fade: 0.06, vignette: 0.5, grain: 0.6),
        Look(id: "mono", name: "Mono", category: "Retro", mono: "CIPhotoEffectMono"),
        Look(id: "polaroid", name: "Polaroid", category: "Retro", temperature: 500, saturation: 0.85, contrast: 0.92, fade: 0.1, gain: (1.02, 1.0, 0.92)),
        Look(id: "vhs", name: "VHS", category: "Retro", saturation: 1.15, contrast: 1.1, grain: 0.4, chromatic: 0.6),
        Look(id: "vintage", name: "Vintage", category: "Retro", temperature: 900, saturation: 0.7, fade: 0.1, gain: (1.03, 1.02, 0.85), vignette: 0.6),
        // Color
        Look(id: "contrastPop", name: "Contrast Pop", category: "Color", saturation: 1.1, contrast: 1.25),
        Look(id: "cool", name: "Cool", category: "Color", temperature: -1500),
        Look(id: "vibrance", name: "Vibrance", category: "Color", vibrance: 0.9),
        Look(id: "warm", name: "Warm", category: "Color", temperature: 1500),
        // Scenery
        Look(id: "autumn", name: "Autumn", category: "Scenery", temperature: 1300, saturation: 1.15, gain: (1.08, 0.98, 0.85)),
        Look(id: "tropical", name: "Tropical", category: "Scenery", temperature: 300, tint: -8, saturation: 1.3, vibrance: 0.4),
        Look(id: "vividLandscape", name: "Vivid Landscape", category: "Scenery", saturation: 1.25, contrast: 1.12, vibrance: 0.5, shadows: 0.15),
        Look(id: "winterCool", name: "Winter Cool", category: "Scenery", temperature: -2000, saturation: 0.85, brightness: 0.04, highlights: 0.95),
        // Food
        Look(id: "delicious", name: "Delicious", category: "Food", temperature: 700, saturation: 1.2, contrast: 1.08, vibrance: 0.5),
        // Black & White
        Look(id: "highContrastMono", name: "High-Contrast Mono", category: "Black & White", contrast: 1.4, mono: "CIPhotoEffectNoir"),
        Look(id: "silver", name: "Silver", category: "Black & White", contrast: 0.95, brightness: 0.04, fade: 0.05, mono: "CIPhotoEffectTonal"),
        // Vibe
        Look(id: "dreamy", name: "Dreamy", category: "Vibe", saturation: 0.9, contrast: 0.92, brightness: 0.04, bloom: 0.6),
        Look(id: "vaporwave", name: "Vaporwave", category: "Vibe", tint: 35, saturation: 1.2, gain: (1.05, 0.85, 1.15), lift: (0.04, 0, 0.06)),
        // Stylised
        Look(id: "marquee", name: "Marquee", category: "Stylised", temperature: 1200, saturation: 1.4, contrast: 1.2, bloom: 0.5),
        Look(id: "matcha", name: "Matcha", category: "Stylised", tint: -25, saturation: 0.85, fade: 0.06, gain: (0.95, 1.06, 0.9)),
        Look(id: "neon", name: "Neon", category: "Stylised", saturation: 1.5, contrast: 1.2, gain: (1.1, 0.9, 1.15), bloom: 0.6),
        Look(id: "pinkFuzz", name: "Pink Fuzz", category: "Stylised", tint: 30, saturation: 0.9, fade: 0.08, gain: (1.08, 0.92, 1.04), bloom: 0.3),
        Look(id: "retroCartoon", name: "Retro Cartoon", category: "Stylised", saturation: 1.3, contrast: 1.1, posterize: 0.6),
        Look(id: "spooky", name: "Spooky", category: "Stylised", temperature: -1000, tint: -20, saturation: 0.5, contrast: 1.25, brightness: -0.08, vignette: 1.2),
    ]

    static let byID: [String: Look] = Dictionary(uniqueKeysWithValues: all.map { ($0.id, $0) })

    static func name(_ id: String) -> String { byID[id]?.name ?? "Original" }
}
