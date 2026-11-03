import AVFoundation
import Observation

/// Records a voiceover from the microphone into the project's media folder.
@MainActor
@Observable
final class VoiceRecorder {
    private(set) var isRecording = false
    private(set) var elapsed: Double = 0
    /// Input level, 0...1, for the meter.
    private(set) var level: Double = 0

    @ObservationIgnored private var recorder: AVAudioRecorder?
    @ObservationIgnored private var timer: Timer?
    @ObservationIgnored private var file: String?

    enum RecordError: LocalizedError {
        case denied, failed
        var errorDescription: String? {
            switch self {
            case .denied: "Microphone access is off for AlMosawiCut. Turn it on in Settings › Privacy & Security › Microphone."
            case .failed: "The microphone couldn't start recording."
            }
        }
    }

    func start(in media: URL) async throws {
        guard await AVAudioApplication.requestRecordPermission() else { throw RecordError.denied }
        let session = AVAudioSession.sharedInstance()
        try session.setCategory(.playAndRecord, mode: .default, options: [.defaultToSpeaker])
        try session.setActive(true)

        let name = UUID().uuidString + ".m4a"
        let settings: [String: Any] = [
            AVFormatIDKey: kAudioFormatMPEG4AAC,
            AVSampleRateKey: 48_000,
            AVNumberOfChannelsKey: 1,
            AVEncoderAudioQualityKey: AVAudioQuality.high.rawValue,
        ]
        let r = try AVAudioRecorder(url: media.appendingPathComponent(name), settings: settings)
        r.isMeteringEnabled = true
        guard r.record() else { throw RecordError.failed }
        recorder = r
        file = name
        elapsed = 0
        isRecording = true
        timer = Timer.scheduledTimer(withTimeInterval: 0.05, repeats: true) { [weak self] _ in
            MainActor.assumeIsolated { self?.tick() }
        }
    }

    private func tick() {
        guard let r = recorder else { return }
        r.updateMeters()
        elapsed = r.currentTime
        level = Double(max(0, min(1, (r.averagePower(forChannel: 0) + 50) / 50)))
    }

    /// Stops recording and hands back the file name inside the media folder.
    func stop() -> String? {
        timer?.invalidate()
        timer = nil
        recorder?.stop()
        recorder = nil
        isRecording = false
        level = 0
        AppAudio.configureForPlayback()
        defer { file = nil }
        return file
    }
}

enum AppAudio {
    /// Editing plays sound even with the ringer switch on silent.
    static func configureForPlayback() {
        try? AVAudioSession.sharedInstance().setCategory(.playback, mode: .moviePlayback)
        try? AVAudioSession.sharedInstance().setActive(true)
    }
}
