import SwiftUI

@main
struct AlMosawiCutApp: App {
    @State private var store = ProjectStore()

    init() {
        // Imported fonts must be registered before any title is drawn.
        _ = FontLibrary.shared
        AppAudio.configureForPlayback()
    }

    var body: some Scene {
        WindowGroup {
            HomeView()
                .environment(store)
                .preferredColorScheme(.dark)
                .tint(Theme.accent)
        }
    }
}

enum Theme {
    static let accent = Color(red: 0, green: 0.84, blue: 0.79)
    static let accentDeep = Color(red: 0, green: 0.45, blue: 0.52)
    static let background = Color(red: 0.05, green: 0.05, blue: 0.06)
    static let surface = Color(red: 0.11, green: 0.11, blue: 0.13)
    static let surfaceHigh = Color(red: 0.17, green: 0.17, blue: 0.2)
    static let secondaryText = Color.white.opacity(0.55)
    static let textTrack = Color(red: 0.86, green: 0.55, blue: 0.12)
    static let captionTrack = Color(red: 0.55, green: 0.36, blue: 0.96)
    static let shapeTrack = Color(red: 0.9, green: 0.3, blue: 0.5)
    static let overlayTrack = Color(red: 1, green: 0.78, blue: 0.2)
    static let musicTrack = Color(red: 0.12, green: 0.55, blue: 0.38)
    static let voiceTrack = Color(red: 0.2, green: 0.45, blue: 0.85)
}

extension Double {
    /// 0:07.4 style, for the timeline.
    var timecode: String {
        let total = Swift.max(0, self)
        let m = Int(total) / 60
        let s = total - Double(m * 60)
        return String(format: "%d:%04.1f", m, s)
    }
}
