import AVFoundation
import Speech

/// Auto-captions: transcribe the edit's own sound on device and turn the
/// words into short timed titles.
enum Captioner {
    struct Word: Sendable { let text: String; let start: Double; let duration: Double }

    enum CaptionError: LocalizedError {
        case notAllowed, unavailable(String), noSpeech, noSound, timedOut
        var errorDescription: String? {
            switch self {
            case .timedOut: "Apple's speech service didn't answer. Check the internet connection, or install the language for on-device use under Settings › General › Keyboard › Dictation Languages, then try again."
            case .noSound: "This edit has no sound to caption. Unmute a clip, or add a voiceover."
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

    /// Server recognition stops at about a minute, so longer edits are heard in pieces.
    private static let chunkLength = 50.0

    static func captions(for project: Project, locale: String, wordsPerCaption: Int,
                         progress: @escaping @MainActor (String) -> Void = { _ in }) async throws -> [TextOverlay] {
        await progress("Checking speech recognition…")
        let status = await withCheckedContinuation { c in SFSpeechRecognizer.requestAuthorization { c.resume(returning: $0) } }
        guard status == .authorized else { throw CaptionError.notAllowed }
        guard let recognizer = SFSpeechRecognizer(locale: Locale(identifier: locale)), recognizer.isAvailable else {
            throw CaptionError.unavailable(locale)
        }

        await progress("Getting the sound from your edit…")
        let audio = try await exportVoice(of: project)
        defer { try? FileManager.default.removeItem(at: audio) }
        let length = try await AVURLAsset(url: audio).load(.duration).seconds

        // Prefer on-device, which has no length limit; a device can claim support
        // before its model is downloaded, so fall back to Apple's service, in pieces.
        let words: [Word]
        do {
            if recognizer.supportsOnDeviceRecognition {
                await progress("Listening on this device…")
                // The on-device model can be missing or still downloading and never answer.
                do { words = try await recognize(audio, with: recognizer, onDevice: true, timeout: max(25, length * 1.5 + 10)) }
                catch { words = try await recognizeInPieces(audio, length: length, with: recognizer, progress: progress) }
            } else {
                words = try await recognizeInPieces(audio, length: length, with: recognizer, progress: progress)
            }
        } catch let error as NSError where error.domain == "kLSRErrorDomain" && error.code == 300
                    || error.domain == "kAFAssistantErrorDomain" && error.code == 1101 {
            // The speech service has no model for this language on this device.
            throw CaptionError.unavailable(Locale.current.localizedString(forIdentifier: locale) ?? locale)
        }
        guard !words.isEmpty else { throw CaptionError.noSpeech }
        return group(words, max: wordsPerCaption)
    }

    /// Sends the sound to Apple's recognizer a piece at a time and puts the words back on one clock.
    private static func recognizeInPieces(_ audio: URL, length: Double, with recognizer: SFSpeechRecognizer,
                                          progress: @MainActor (String) -> Void) async throws -> [Word] {
        guard length > chunkLength + 5 else {
            await progress("Listening…")
            do { return try await recognize(audio, with: recognizer, onDevice: false, timeout: 90) }
            catch let error as NSError where error.domain == "kAFAssistantErrorDomain" && error.code == 1110 { return [] }
        }
        let asset = AVURLAsset(url: audio)
        let count = Int(ceil(length / chunkLength))
        var words: [Word] = []
        for i in 0..<count {
            await progress("Listening… part \(i + 1) of \(count)")
            let start = Double(i) * chunkLength
            let piece = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString + ".m4a")
            defer { try? FileManager.default.removeItem(at: piece) }
            guard let session = AVAssetExportSession(asset: asset, presetName: AVAssetExportPresetAppleM4A) else {
                throw CocoaError(.featureUnsupported)
            }
            session.timeRange = CMTimeRange(start: CMTime(seconds: start, preferredTimescale: 600),
                                            duration: CMTime(seconds: min(chunkLength, length - start), preferredTimescale: 600))
            try await session.export(to: piece, as: .m4a)
            do {
                let heard = try await recognize(piece, with: recognizer, onDevice: false, timeout: 90)
                words += heard.map { Word(text: $0.text, start: $0.start + start, duration: $0.duration) }
            } catch let error as NSError where error.domain == "kAFAssistantErrorDomain" && (error.code == 1110 || error.code == 203) {
                continue  // nothing spoken in this piece
            }
        }
        return words
    }

    /// Recognizes one file, giving up after `timeout` seconds instead of waiting forever.
    private static func recognize(_ audio: URL, with recognizer: SFSpeechRecognizer, onDevice: Bool,
                                  timeout: Double) async throws -> [Word] {
        try await withThrowingTaskGroup(of: [Word].self) { group in
            group.addTask { try await recognizeOnce(audio, with: recognizer, onDevice: onDevice) }
            group.addTask {
                try await Task.sleep(for: .seconds(timeout))
                throw CaptionError.timedOut
            }
            defer { group.cancelAll() }
            guard let words = try await group.next() else { throw CaptionError.timedOut }
            return words
        }
    }

    /// Holds a running recognition so a timeout can stop it and resume its waiter exactly once.
    private final class Recognition: @unchecked Sendable {
        private let lock = NSLock()
        private var continuation: CheckedContinuation<[Word], Error>?
        private var task: SFSpeechRecognitionTask?
        private var finished = false

        func start(_ c: CheckedContinuation<[Word], Error>, _ make: () -> SFSpeechRecognitionTask) {
            lock.lock()
            if finished { lock.unlock(); c.resume(throwing: CancellationError()); return }
            continuation = c
            lock.unlock()
            let t = make()
            lock.lock(); task = t; lock.unlock()
        }

        func finish(_ result: Result<[Word], Error>) {
            lock.lock()
            let c = continuation
            continuation = nil
            finished = true
            lock.unlock()
            c?.resume(with: result)
        }

        func cancel() {
            lock.lock(); let t = task; lock.unlock()
            t?.cancel()
            finish(.failure(CancellationError()))
        }
    }

    private static func recognizeOnce(_ audio: URL, with recognizer: SFSpeechRecognizer, onDevice: Bool) async throws -> [Word] {
        let request = SFSpeechURLRecognitionRequest(url: audio)
        request.shouldReportPartialResults = false
        request.addsPunctuation = true
        request.requiresOnDeviceRecognition = onDevice
        let recognition = Recognition()
        return try await withTaskCancellationHandler {
            try await withCheckedThrowingContinuation { c in
                recognition.start(c) {
                    recognizer.recognitionTask(with: request) { result, error in
                        if let result, result.isFinal {
                            // A segment can hold several words; spread its time across them.
                            recognition.finish(.success(result.bestTranscription.segments.flatMap { segment -> [Word] in
                                let parts = segment.substring.split(whereSeparator: \.isWhitespace).map(String.init)
                                let each = segment.duration / Double(max(1, parts.count))
                                return parts.enumerated().map { i, part in
                                    Word(text: part, start: segment.timestamp + Double(i) * each, duration: each)
                                }
                            }))
                        } else if let error {
                            recognition.finish(.failure(error))
                        }
                    }
                }
            }
        } onCancel: {
            recognition.cancel()
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

    /// The edit's spoken sound on the timeline's clock: clips, plus voiceovers and
    /// audio pulled out of clips. Music and sound effects are left out, and voice
    /// effects are dropped so the words are clear.
    private static func exportVoice(of project: Project) async throws -> URL {
        var voice = project
        for i in voice.clips.indices { voice.clips[i].audioEffect = .none }
        for i in voice.overlays.indices { voice.overlays[i].clip.audioEffect = .none }
        voice.audios = voice.audios
            .filter { $0.kind == .voiceover || $0.kind == .extracted }
            .map { var a = $0; a.effect = .none; return a }
        let built = try await CompositionBuilder.build(voice, includeAudioItems: true)
        guard !built.composition.tracks(withMediaType: .audio).isEmpty else { throw CaptionError.noSound }
        guard let session = AVAssetExportSession(asset: built.composition, presetName: AVAssetExportPresetAppleM4A) else {
            throw CocoaError(.featureUnsupported)
        }
        session.audioMix = built.audioMix
        let url = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString + ".m4a")
        try await session.export(to: url, as: .m4a)
        return url
    }
}
