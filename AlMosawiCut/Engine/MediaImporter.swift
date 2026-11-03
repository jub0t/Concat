import AVFoundation
import CoreTransferable
import UIKit
import UniformTypeIdentifiers

/// A movie handed over by PhotosPicker, copied out of the picker's sandbox.
struct PickedMovie: Transferable {
    let url: URL

    static var transferRepresentation: some TransferRepresentation {
        FileRepresentation(contentType: .movie) { movie in
            SentTransferredFile(movie.url)
        } importing: { received in
            let ext = received.file.pathExtension.isEmpty ? "mov" : received.file.pathExtension
            let copy = FileManager.default.temporaryDirectory
                .appendingPathComponent(UUID().uuidString).appendingPathExtension(ext)
            try FileManager.default.copyItem(at: received.file, to: copy)
            return PickedMovie(url: copy)
        }
    }
}

enum MediaImporter {
    enum ImportError: LocalizedError {
        case noVideo, noAudio, writerFailed
        var errorDescription: String? {
            switch self {
            case .noVideo: "That file has no video track."
            case .noAudio: "That file has no sound."
            case .writerFailed: "Couldn't turn the photo into a clip."
            }
        }
    }

    /// Seconds of footage a photo becomes; the clip starts trimmed to 3.
    static let photoSourceLength = 10.0

    /// Moves a video into the project and describes it as a clip.
    static func importVideo(at url: URL, into project: UUID) async throws -> Clip {
        let name = UUID().uuidString + "." + (url.pathExtension.isEmpty ? "mov" : url.pathExtension)
        let dest = ProjectStore.mediaFolder(for: project).appendingPathComponent(name)
        try FileManager.default.moveItem(at: url, to: dest)
        let asset = AVURLAsset(url: dest)
        guard try await !asset.loadTracks(withMediaType: .video).isEmpty else {
            try? FileManager.default.removeItem(at: dest)
            throw ImportError.noVideo
        }
        let duration = try await asset.load(.duration).seconds
        let hasAudio = try await !asset.loadTracks(withMediaType: .audio).isEmpty
        return Clip(file: name, hasAudio: hasAudio, sourceDuration: duration, trimEnd: duration)
    }

    /// Writes a still as a short H.264 file so it behaves like any clip.
    static func importPhoto(_ image: UIImage, into project: UUID) async throws -> Clip {
        let name = UUID().uuidString + ".mp4"
        let dest = ProjectStore.mediaFolder(for: project).appendingPathComponent(name)
        try await writeStill(image, to: dest, seconds: photoSourceLength)
        return Clip(file: name, isPhoto: true, hasAudio: false, sourceDuration: photoSourceLength, trimEnd: 3)
    }

    /// Copies a sound (or a movie's sound) from Files. Music loops under the edit
    /// and sits lower in the mix; a sound plays once at full volume.
    static func importAudio(at url: URL, into project: UUID, asMusic: Bool = true) async throws -> AudioItem {
        let scoped = url.startAccessingSecurityScopedResource()
        defer { if scoped { url.stopAccessingSecurityScopedResource() } }
        let name = UUID().uuidString + "." + (url.pathExtension.isEmpty ? "m4a" : url.pathExtension)
        let dest = ProjectStore.mediaFolder(for: project).appendingPathComponent(name)
        try FileManager.default.copyItem(at: url, to: dest)
        let duration = try await audioDuration(of: dest)
        let title = url.deletingPathExtension().lastPathComponent
        return asMusic
            ? AudioItem(file: name, name: title, kind: .music, sourceDuration: duration, trimEnd: duration,
                        volume: 0.6, fadeOut: 1, loop: true)
            : AudioItem(file: name, name: title, kind: .sound, sourceDuration: duration, trimEnd: duration)
    }

    /// Uses the sound of a video picked from Photos as its own audio item.
    static func importAudio(fromMovie url: URL, into project: UUID) async throws -> AudioItem {
        let name = UUID().uuidString + "." + (url.pathExtension.isEmpty ? "mov" : url.pathExtension)
        let dest = ProjectStore.mediaFolder(for: project).appendingPathComponent(name)
        try FileManager.default.moveItem(at: url, to: dest)
        let duration = try await audioDuration(of: dest)
        return AudioItem(file: name, name: "Sound from video", kind: .extracted, sourceDuration: duration, trimEnd: duration)
    }

    /// Length of a file's sound; throws (and deletes the file) if it has none.
    private static func audioDuration(of url: URL) async throws -> Double {
        let asset = AVURLAsset(url: url)
        guard let track = try await asset.loadTracks(withMediaType: .audio).first else {
            try? FileManager.default.removeItem(at: url)
            throw ImportError.noAudio
        }
        let range = try await track.load(.timeRange)
        if range.duration.seconds > 0 { return range.end.seconds }
        return try await asset.load(.duration).seconds
    }

    private static func writeStill(_ image: UIImage, to url: URL, seconds: Double) async throws {
        // Fit inside 1920 on the long side, even dimensions for the encoder.
        let longSide = max(image.size.width, image.size.height)
        let scale = min(1, 1920 / longSide)
        let width = Int(image.size.width * scale) / 2 * 2
        let height = Int(image.size.height * scale) / 2 * 2

        let writer = try AVAssetWriter(outputURL: url, fileType: .mp4)
        let input = AVAssetWriterInput(mediaType: .video, outputSettings: [
            AVVideoCodecKey: AVVideoCodecType.h264,
            AVVideoWidthKey: width,
            AVVideoHeightKey: height,
        ])
        let adaptor = AVAssetWriterInputPixelBufferAdaptor(assetWriterInput: input, sourcePixelBufferAttributes: [
            kCVPixelBufferPixelFormatTypeKey as String: kCVPixelFormatType_32BGRA,
            kCVPixelBufferWidthKey as String: width,
            kCVPixelBufferHeightKey as String: height,
        ])
        writer.add(input)
        guard writer.startWriting() else { throw ImportError.writerFailed }
        writer.startSession(atSourceTime: .zero)

        guard let buffer = pixelBuffer(from: image, width: width, height: height) else { throw ImportError.writerFailed }
        // One frame a second keeps the file tiny and every seek lands on a real frame.
        for second in 0..<Int(seconds) {
            while !input.isReadyForMoreMediaData { try await Task.sleep(nanoseconds: 2_000_000) }
            adaptor.append(buffer, withPresentationTime: CMTime(value: CMTimeValue(second), timescale: 1))
        }
        input.markAsFinished()
        writer.endSession(atSourceTime: CMTime(seconds: seconds, preferredTimescale: 600))
        await writer.finishWriting()
        if writer.status != .completed { throw writer.error ?? ImportError.writerFailed }
    }

    private static func pixelBuffer(from image: UIImage, width: Int, height: Int) -> CVPixelBuffer? {
        var buffer: CVPixelBuffer?
        CVPixelBufferCreate(kCFAllocatorDefault, width, height, kCVPixelFormatType_32BGRA,
                            [kCVPixelBufferCGImageCompatibilityKey: true,
                             kCVPixelBufferCGBitmapContextCompatibilityKey: true] as CFDictionary, &buffer)
        guard let buffer else { return nil }
        CVPixelBufferLockBaseAddress(buffer, [])
        defer { CVPixelBufferUnlockBaseAddress(buffer, []) }
        guard let ctx = CGContext(
            data: CVPixelBufferGetBaseAddress(buffer), width: width, height: height, bitsPerComponent: 8,
            bytesPerRow: CVPixelBufferGetBytesPerRow(buffer), space: CGColorSpaceCreateDeviceRGB(),
            bitmapInfo: CGImageAlphaInfo.premultipliedFirst.rawValue | CGBitmapInfo.byteOrder32Little.rawValue
        ) else { return nil }
        // UIKit drawing applies the photo's orientation for us.
        UIGraphicsPushContext(ctx)
        ctx.translateBy(x: 0, y: CGFloat(height))
        ctx.scaleBy(x: 1, y: -1)
        image.draw(in: CGRect(x: 0, y: 0, width: width, height: height))
        UIGraphicsPopContext()
        return buffer
    }
}
