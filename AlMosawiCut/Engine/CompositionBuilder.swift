import AVFoundation
import CoreImage

/// A project laid out as AVFoundation objects, ready to play or export.
struct BuiltComposition {
    let composition: AVMutableComposition
    let audioMix: AVMutableAudioMix
    let segments: [Segment]
    let videoTrackIDs: [CMPersistentTrackID]

    /// A fresh video composition drawing through `state`.
    func videoComposition(state: RenderState, frameRate: Int32 = 30) -> AVVideoComposition {
        let vc = AVMutableVideoComposition()
        vc.customVideoCompositorClass = RenderCompositor.self
        vc.instructions = [RenderInstruction(
            timeRange: CMTimeRange(start: .zero, duration: composition.duration),
            trackIDs: videoTrackIDs, state: state)]
        vc.renderSize = state.snapshot.2
        vc.frameDuration = CMTime(value: 1, timescale: frameRate)
        return vc
    }
}

enum CompositionBuilder {
    static let timescale: CMTimeScale = 600

    private static func time(_ s: Double) -> CMTime { CMTime(seconds: s, preferredTimescale: timescale) }

    static func build(_ project: Project, includeAudioItems: Bool = true) async throws -> BuiltComposition {
        let composition = AVMutableComposition()
        let media = ProjectStore.mediaFolder(for: project.id)
        func newTrack(_ type: AVMediaType) throws -> AVMutableCompositionTrack {
            guard let t = composition.addMutableTrack(withMediaType: type, preferredTrackID: kCMPersistentTrackID_Invalid) else {
                throw CocoaError(.featureUnsupported)
            }
            return t
        }

        // Main clips alternate between two tracks so a transition can overlap them.
        let videoA = try newTrack(.video), videoB = try newTrack(.video)
        let audioA = try newTrack(.audio), audioB = try newTrack(.audio)
        var envelopes: [CMPersistentTrackID: [(Double, Double)]] = [:]
        var keep: [AVAsset] = []
        var segments: [Segment] = []
        var videoTrackIDs = [videoA.trackID, videoB.trackID]
        let projectEnd = project.duration

        for (i, clip) in project.clips.enumerated() {
            let start = project.start(ofClipAt: i)
            let vt = i.isMultiple(of: 2) ? videoA : videoB
            let at = i.isMultiple(of: 2) ? audioA : audioB
            let asset = AVURLAsset(url: media.appendingPathComponent(clip.file))
            guard let source = try await asset.loadTracks(withMediaType: .video).first else { continue }
            let transform = try await source.load(.preferredTransform)
            try insert(clip.pieces, of: source, into: vt, at: start)
            if !project.tracks.mainMuted, clip.hasAudio, clip.volume > 0, let sound = try await audioTrack(for: clip.file, effect: clip.audioEffect, cleanup: clip.cleanup, media: media, keep: &keep) {
                try insert(clip.pieces, of: sound, into: at, at: start)
                let inOverlap = project.overlap(at: i)
                let outOverlap = i + 1 < project.clips.count ? project.overlap(at: i + 1) : 0
                envelopes[at.trackID, default: []] += envelope(start: start, duration: clip.duration) { local in
                    var v = clip.volume * clip.transform(atLocal: local).volume
                    v *= ramp(local, over: max(clip.fadeIn, inOverlap))
                    v *= ramp(clip.duration - local, over: max(clip.fadeOut, outOverlap))
                    return v
                }
            }
            segments.append(Segment(itemID: clip.id, isOverlay: false, trackID: vt.trackID, start: start,
                                    duration: clip.duration, orientation: transform.videoOrientation))
        }

        for item in project.overlays where item.start < projectEnd {
            let clip = item.clip
            let asset = AVURLAsset(url: media.appendingPathComponent(clip.file))
            guard let source = try await asset.loadTracks(withMediaType: .video).first else { continue }
            let transform = try await source.load(.preferredTransform)
            let vt = try newTrack(.video)
            try insert(clip.pieces, of: source, into: vt, at: item.start)
            videoTrackIDs.append(vt.trackID)
            if !project.tracks.overlaysMuted, clip.hasAudio, clip.volume > 0, let sound = try await audioTrack(for: clip.file, effect: clip.audioEffect, cleanup: clip.cleanup, media: media, keep: &keep) {
                let at = try newTrack(.audio)
                try insert(clip.pieces, of: sound, into: at, at: item.start)
                envelopes[at.trackID, default: []] += envelope(start: item.start, duration: clip.duration) { local in
                    clip.volume * clip.transform(atLocal: local).volume * ramp(local, over: clip.fadeIn) * ramp(clip.duration - local, over: clip.fadeOut)
                }
            }
            segments.append(Segment(itemID: item.id, isOverlay: true, trackID: vt.trackID, start: item.start,
                                    duration: clip.duration, orientation: transform.videoOrientation))
        }

        if includeAudioItems, projectEnd > 0, !project.tracks.audioMuted {
            for item in project.audios where item.start < projectEnd {
                guard let sound = try await audioTrack(for: item.file, effect: item.effect, cleanup: item.cleanup, media: media, keep: &keep) else { continue }
                let track = try newTrack(.audio)
                let soundLength = try await sound.load(.timeRange).duration.seconds
                let trimEnd = min(item.trimEnd, soundLength)
                let piece = max(0.05, trimEnd - item.trimStart)
                var at = item.start
                var played = 0.0
                repeat {
                    let length = min(piece, projectEnd - at)
                    guard length > 0.01 else { break }
                    try track.insertTimeRange(CMTimeRange(start: time(item.trimStart), duration: time(length)), of: sound, at: time(at))
                    at += length; played += length
                } while item.loop && at < projectEnd - 0.01
                let fadeOut = item.loop ? max(item.fadeOut, min(1, played)) : item.fadeOut
                envelopes[track.trackID, default: []] += envelope(start: item.start, duration: played) { local in
                    item.volume * ramp(local, over: item.fadeIn) * ramp(played - local, over: fadeOut)
                }
            }
        }

        // An overlay can run past the last clip; the video ends where the main track does.
        if projectEnd > 0, composition.duration > time(projectEnd) {
            composition.removeTimeRange(CMTimeRange(start: time(projectEnd), end: composition.duration))
        }

        // Empty tracks (B with a single clip, silent audio) make the player fail to load.
        for track in composition.tracks where track.segments.isEmpty {
            composition.removeTrack(track)
            videoTrackIDs.removeAll { $0 == track.trackID }
        }

        let mix = AVMutableAudioMix()
        mix.inputParameters = composition.tracks(withMediaType: .audio).compactMap { track in
            guard let points = envelopes[track.trackID], !points.isEmpty else { return nil }
            let p = AVMutableAudioMixInputParameters(track: track)
            var last: (Double, Double)?
            for point in points.sorted(by: { $0.0 < $1.0 }) {
                if let prev = last, point.0 > prev.0 + 0.0005 {
                    p.setVolumeRamp(fromStartVolume: Float(prev.1), toEndVolume: Float(point.1),
                                    timeRange: CMTimeRange(start: time(prev.0), end: time(point.0)))
                }
                last = point
            }
            return p
        }
        return BuiltComposition(composition: composition, audioMix: mix, segments: segments, videoTrackIDs: videoTrackIDs)
    }

    /// Inserts speed pieces back to back, each scaled to its own speed.
    private static func insert(_ pieces: [SpeedPiece], of source: AVAssetTrack, into track: AVMutableCompositionTrack, at start: Double) throws {
        var cursor = time(start)
        let available = source.timeRange.end
        for piece in pieces {
            let s = time(piece.sourceStart)
            guard s < available else { break }
            let range = CMTimeRange(start: s, end: CMTimeMinimum(time(piece.sourceStart + piece.sourceDuration), available))
            try track.insertTimeRange(range, of: source, at: cursor)
            let target = time(range.duration.seconds / piece.speed)
            if abs(piece.speed - 1) > 0.001 {
                track.scaleTimeRange(CMTimeRange(start: cursor, duration: range.duration), toDuration: target)
            }
            cursor = cursor + target
        }
    }

    /// The audio to use for a file: its own track, or a processed copy when it has effects.
    /// The asset is kept in `keep`: a track whose asset has gone can't be inserted.
    private static func audioTrack(for file: String, effect: AudioEffect, cleanup: VoiceCleanup, media: URL,
                                   keep: inout [AVAsset]) async throws -> AVAssetTrack? {
        let url = media.appendingPathComponent(file)
        let source = effect == .none && cleanup.isOff
            ? url
            : try await AudioProcessor.processed(url, effect: effect, cleanup: cleanup, media: media)
        let asset = AVURLAsset(url: source)
        keep.append(asset)
        return try await asset.loadTracks(withMediaType: .audio).first
    }

    /// Volume samples across an item, so fades, transitions and keyframes become one set of ramps.
    private static func envelope(start: Double, duration: Double, _ volume: (Double) -> Double) -> [(Double, Double)] {
        let step = 0.1
        var points: [(Double, Double)] = []
        var t = 0.0
        while t < duration {
            points.append((start + t, max(0, volume(t))))
            t += step
        }
        points.append((start + duration, max(0, volume(duration))))
        return points
    }

    private static func ramp(_ t: Double, over length: Double) -> Double {
        guard length > 0 else { return 1 }
        return max(0, min(1, t / length))
    }
}
