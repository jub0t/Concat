import AVFoundation
import Speech

/// Auto-captions: transcribe the edit's own sound on device and turn the
/// words into short timed titles.
enum Captioner {
    struct Word { let text: String; let start: Double; let duration: Double }

    enum CaptionError: LocalizedError {
        case notAllowed, unavailable(String), noSpeech
        var errorDescription: String? {
            switch self {
            case .notAllowed: "Speech recognition is off for AlMosawiCut. Turn it on in Settings."
            case .unavailable(let lang): "Speech recognition for \(lang) isn't available on this device. Try adding it under Settings › General › Keyboard › Dictation Languages."
            case .noSpeech: "No speech was found in the video."
            }
        }
    }

    static let languages: [(id: String, name: String)] = [
        ("ar-SA", "العربية"), ("en-US", "English"), ("fr-FR", "Français"), ("es-ES", "Español"),
        ("de-DE", "Deutsch"), ("tr-TR", "Türkçe"), ("ur-PK", "اردو"), ("hi-IN", "हिन्दी"),
    ]

    static func captions(for project: Project, locale: String, wordsPerCaption: Int) async throws -> [TextOverlay] {
        let status = await withCheckedContinuation { c in SFSpeechRecognizer.requestAuthorization { c.resume(returning: $0) } }
        guard status == .authorized else { throw CaptionError.notAllowed }
        guard let recognizer = SFSpeechRecognizer(locale: Locale(identifier: locale)), recognizer.isAvailable else {
            throw CaptionError.unavailable(locale)
        }

        let audio = try await exportVoice(of: project)
        defer { try? FileManager.default.removeItem(at: audio) }

        // Prefer on-device; a device can claim support before its model is
        // downloaded, so fall back to Apple's recognizer if that fails to start.
        let words: [Word]
        do {
            if recognizer.supportsOnDeviceRecognition {
                do { words = try await recognize(audio, with: recognizer, onDevice: true) }
                catch { words = try await recognize(audio, with: recognizer, onDevice: false) }
            } else {
                words = try await recognize(audio, with: recognizer, onDevice: false)
            }
        } catch let error as NSError where error.domain == "kLSRErrorDomain" && error.code == 300
                    || error.domain == "kAFAssistantErrorDomain" && error.code == 1101 {
            // The speech service has no model for this language on this device.
            throw CaptionError.unavailable(Locale.current.localizedString(forIdentifier: locale) ?? locale)
        }
        guard !words.isEmpty else { throw CaptionError.noSpeech }
        return group(words, max: wordsPerCaption)
    }

    private static func recognize(_ audio: URL, with recognizer: SFSpeechRecognizer, onDevice: Bool) async throws -> [Word] {
        let request = SFSpeechURLRecognitionRequest(url: audio)
        request.shouldReportPartialResults = false
        request.addsPunctuation = true
        request.requiresOnDeviceRecognition = onDevice
        return try await withCheckedThrowingContinuation { c in
            var done = false
            recognizer.recognitionTask(with: request) { result, error in
                guard !done else { return }
                if let result, result.isFinal {
                    done = true
                    // A segment can hold several words; spread its time across them.
                    c.resume(returning: result.bestTranscription.segments.flatMap { segment -> [Word] in
                        let parts = segment.substring.split(whereSeparator: \.isWhitespace).map(String.init)
                        let each = segment.duration / Double(max(1, parts.count))
                        return parts.enumerated().map { i, part in
                            Word(text: part, start: segment.timestamp + Double(i) * each, duration: each)
                        }
                    })
                } else if let error {
                    done = true
                    c.resume(throwing: error)
                }
            }
        }
    }

    /// Chunks of up to `max` words, broken early on a pause or a sentence end.
    static func group(_ words: [Word], max: Int) -> [TextOverlay] {
        var out: [TextOverlay] = []
        var chunk: [Word] = []
        func flush() {
            guard let first = chunk.first, let last = chunk.last else { return }
            let end = last.start + Swift.max(last.duration, 0.25)
            out.append(TextOverlay(
                text: chunk.map(\.text).joined(separator: " "),
                start: first.start, duration: Swift.max(0.3, end - first.start),
                color: .white, accent: .black, size: 0.075, bold: true, style: .outline,
                x: 0.5, y: 0.72, isCaption: true))
            chunk.removeAll()
        }
        for word in words {
            if let last = chunk.last, word.start - (last.start + last.duration) > 0.6 { flush() }
            chunk.append(word)
            if chunk.count >= max || word.text.last.map({ ".?!،؟".contains($0) }) == true { flush() }
        }
        flush()
        // Hold each caption until the next one starts if the gap is small.
        for i in out.indices.dropLast() {
            let gap = out[i + 1].start - out[i].end
            if gap > 0, gap < 0.4 { out[i].duration += gap }
            if out[i].end > out[i + 1].start { out[i].duration = out[i + 1].start - out[i].start }
        }
        return out
    }

    private static func exportVoice(of project: Project) async throws -> URL {
        let built = try await CompositionBuilder.build(project, includeAudioItems: false)
        guard let session = AVAssetExportSession(asset: built.composition, presetName: AVAssetExportPresetAppleM4A) else {
            throw CocoaError(.featureUnsupported)
        }
        session.audioMix = built.audioMix
        let url = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString + ".m4a")
        try await session.export(to: url, as: .m4a)
        return url
    }
}
