import Foundation
import Observation
import WhisperKit

/// Whisper running on the iPhone (WhisperKit, on Core ML), the way Concat captions
/// with local Whisper: the words come straight from the edit's own sound, with no
/// microphone, Speech Recognition permission, Dictation or network once the model is here.
@MainActor
@Observable
final class WhisperEngine {
    static let shared = WhisperEngine()

    /// The multilingual Whisper sizes, smallest first.
    enum Size: String, CaseIterable, Identifiable {
        case tiny, base, small
        var id: String { rawValue }
        var variant: String { "openai_whisper-\(rawValue)" }
        var label: String {
            switch self {
            case .tiny: "Tiny"
            case .base: "Base"
            case .small: "Small"
            }
        }
        var detail: String {
            switch self {
            case .tiny: "About 75 MB. Fastest, rough on names."
            case .base: "About 145 MB. Good for clear speech."
            case .small: "About 480 MB. Best quality; recommended for Arabic."
            }
        }
    }

    enum EngineError: LocalizedError {
        case noModel
        var errorDescription: String? {
            "The speech model couldn't be downloaded. Connect to the internet once to download it; after that captions work offline."
        }
    }

    /// 0...1 while a model downloads.
    private(set) var downloadProgress: Double?
    /// Bumped when a model arrives or is removed, so views re-read what's downloaded.
    private(set) var revision = 0

    @ObservationIgnored private var kit: WhisperKit?
    @ObservationIgnored private var loaded: Size?

    /// Models live in Application Support, out of iCloud backups.
    nonisolated static let folder: URL = {
        let base = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
        var url = base.appendingPathComponent("Whisper", isDirectory: true)
        try? FileManager.default.createDirectory(at: url, withIntermediateDirectories: true)
        var values = URLResourceValues()
        values.isExcludedFromBackup = true
        try? url.setResourceValues(values)
        return url
    }()

    private static func key(_ size: Size) -> String { "whisperModel.\(size.rawValue)" }

    /// Where a downloaded model sits; stored relative, since the app's folder moves between installs.
    private func modelURL(_ size: Size) -> URL? {
        guard let relative = UserDefaults.standard.string(forKey: Self.key(size)) else { return nil }
        let url = Self.folder.appendingPathComponent(relative, isDirectory: true)
        return FileManager.default.fileExists(atPath: url.path) ? url : nil
    }

    func isDownloaded(_ size: Size) -> Bool {
        _ = revision
        return modelURL(size) != nil
    }

    func delete(_ size: Size) {
        if let url = modelURL(size) { try? FileManager.default.removeItem(at: url) }
        UserDefaults.standard.removeObject(forKey: Self.key(size))
        if loaded == size { kit = nil; loaded = nil }
        revision += 1
    }

    /// The loaded pipeline for `size`, downloading the model the first time.
    private func pipeline(_ size: Size, progress: @MainActor (String) -> Void) async throws -> WhisperKit {
        if let kit, loaded == size { return kit }
        kit = nil
        loaded = nil
        let folder: URL
        if let existing = modelURL(size) {
            folder = existing
        } else {
            progress("Downloading the \(size.label) speech model, once…")
            downloadProgress = 0
            defer { downloadProgress = nil }
            do {
                folder = try await WhisperKit.download(variant: size.variant, downloadBase: Self.folder) { p in
                    let fraction = p.fractionCompleted
                    Task { @MainActor in WhisperEngine.shared.downloadProgress = fraction }
                }
            } catch {
                throw EngineError.noModel
            }
            let base = Self.folder.standardizedFileURL.path
            let full = folder.standardizedFileURL.path
            let relative = full.hasPrefix(base) ? String(full.dropFirst(base.count)).trimmingCharacters(in: CharacterSet(charactersIn: "/")) : full
            UserDefaults.standard.set(relative, forKey: Self.key(size))
            revision += 1
        }
        progress("Loading the speech model…")
        let config = WhisperKitConfig(model: size.variant, downloadBase: Self.folder, modelFolder: folder.path,
                                      tokenizerFolder: Self.folder, verbose: false, prewarm: false, load: true, download: false)
        let made = try await WhisperKit(config)
        kit = made
        loaded = size
        return made
    }

    /// Words with their times in `audio`, in `language` (a Whisper code such as "ar"; nil hears it).
    func transcribe(_ audio: URL, language: String?, size: Size, progress: @MainActor (String) -> Void) async throws -> [Captioner.Word] {
        let kit = try await pipeline(size, progress: progress)
        progress("Listening on this iPhone…")
        var options = DecodingOptions()
        options.task = .transcribe
        options.language = language
        options.detectLanguage = language == nil
        options.temperature = 0
        options.skipSpecialTokens = true
        options.wordTimestamps = true
        options.chunkingStrategy = .vad
        let results = try await kit.transcribe(audioPath: audio.path, decodeOptions: options)
        var words: [Captioner.Word] = []
        for segment in results.flatMap({ $0.segments }) {
            if let timed = segment.words, !timed.isEmpty {
                for w in timed {
                    let text = Self.clean(w.word)
                    guard !text.isEmpty else { continue }
                    words.append(Captioner.Word(text: text, start: Double(w.start), duration: Double(max(0.02, w.end - w.start))))
                }
            } else {
                // No word times: spread the segment's words over its span.
                let parts = Self.clean(segment.text).split(whereSeparator: \.isWhitespace).map(String.init)
                guard !parts.isEmpty else { continue }
                let span = Double(max(0.1, segment.end - segment.start)) / Double(parts.count)
                for (i, part) in parts.enumerated() {
                    words.append(Captioner.Word(text: part, start: Double(segment.start) + Double(i) * span, duration: span))
                }
            }
        }
        return words.sorted { $0.start < $1.start }
    }

    /// Drops Whisper's markers such as <|en|> and [BLANK_AUDIO].
    private static func clean(_ s: String) -> String {
        s.replacingOccurrences(of: "<\\|[^|]*\\|>", with: "", options: .regularExpression)
            .replacingOccurrences(of: "\\[[A-Z_ ]+\\]", with: "", options: .regularExpression)
            .trimmingCharacters(in: .whitespacesAndNewlines)
    }
}
