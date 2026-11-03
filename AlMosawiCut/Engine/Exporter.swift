import AVFoundation
import Photos

enum ExportQuality: String, CaseIterable, Identifiable {
    case hd720 = "720p", hd1080 = "1080p", qhd = "2K", uhd4k = "4K"
    var id: String { rawValue }
    var scale: CGFloat {
        switch self {
        case .hd720: 2.0 / 3.0
        case .hd1080: 1
        case .qhd: 4.0 / 3.0
        case .uhd4k: 2
        }
    }
}

enum Exporter {
    static func export(
        _ project: Project, quality: ExportQuality, hevc: Bool, frameRate: Int32,
        progress: @escaping @Sendable (Double) -> Void
    ) async throws -> URL {
        let built = try await CompositionBuilder.build(project)
        let base = project.aspect.baseSize
        let canvas = CGSize(width: (base.width * quality.scale / 2).rounded() * 2,
                            height: (base.height * quality.scale / 2).rounded() * 2)
        let state = RenderState(project: project, segments: built.segments, canvas: canvas)

        let preset = hevc ? AVAssetExportPresetHEVCHighestQuality : AVAssetExportPresetHighestQuality
        guard let session = AVAssetExportSession(asset: built.composition, presetName: preset) else {
            throw CocoaError(.featureUnsupported)
        }
        session.videoComposition = built.videoComposition(state: state, frameRate: frameRate)
        session.audioMix = built.audioMix

        let safeName = project.name.replacingOccurrences(of: "/", with: "-")
        let url = FileManager.default.temporaryDirectory.appendingPathComponent("\(safeName).mp4")
        try? FileManager.default.removeItem(at: url)

        let watcher = Task {
            for await state in session.states(updateInterval: 0.2) {
                if case .exporting(let p) = state { progress(p.fractionCompleted) }
            }
        }
        defer { watcher.cancel() }
        try await session.export(to: url, as: .mp4)
        progress(1)
        return url
    }

    static func saveToPhotos(_ url: URL) async throws {
        let status = await PHPhotoLibrary.requestAuthorization(for: .addOnly)
        guard status == .authorized || status == .limited else {
            throw NSError(domain: "AlMosawiCut", code: 1, userInfo: [
                NSLocalizedDescriptionKey: "Allow AlMosawiCut to add to Photos in Settings."])
        }
        try await PHPhotoLibrary.shared().performChanges {
            PHAssetCreationRequest.creationRequestForAssetFromVideo(atFileURL: url)
        }
    }
}
