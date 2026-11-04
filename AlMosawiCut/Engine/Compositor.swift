import AVFoundation
import CoreImage

/// Carries the render state to the compositor for the whole timeline.
final class RenderInstruction: NSObject, AVVideoCompositionInstructionProtocol, @unchecked Sendable {
    let timeRange: CMTimeRange
    let enablePostProcessing = false
    let containsTweening = true
    let requiredSourceTrackIDs: [NSValue]?
    let passthroughTrackID = kCMPersistentTrackID_Invalid
    let trackIDs: [CMPersistentTrackID]
    let state: RenderState

    init(timeRange: CMTimeRange, trackIDs: [CMPersistentTrackID], state: RenderState) {
        self.timeRange = timeRange
        self.trackIDs = trackIDs
        self.state = state
        requiredSourceTrackIDs = trackIDs.map { NSNumber(value: $0) }
    }
}

/// Draws every output frame with FrameRenderer. Owning the buffers and the
/// CIContext keeps preview and export on the same path. 8-bit sRGB by default;
/// the HDR subclass renders 16-bit float frames in HLG for a 10-bit export.
class RenderCompositor: NSObject, AVVideoCompositing, @unchecked Sendable {
    class var hdr: Bool { false }

    private let colorSpace: CGColorSpace
    private let context: CIContext
    private let queue = DispatchQueue(label: "AlMosawiCut.render")
    private let pixelFormat: OSType

    override required init() {
        let hdr = Self.hdr
        colorSpace = CGColorSpace(name: hdr ? CGColorSpace.itur_2100_HLG : CGColorSpace.sRGB) ?? CGColorSpaceCreateDeviceRGB()
        let working = CGColorSpace(name: hdr ? CGColorSpace.extendedLinearSRGB : CGColorSpace.sRGB) ?? CGColorSpaceCreateDeviceRGB()
        context = CIContext(options: [.workingColorSpace: working, .cacheIntermediates: false])
        pixelFormat = hdr ? kCVPixelFormatType_64RGBAHalf : kCVPixelFormatType_32BGRA
        super.init()
    }

    var supportsHDRSourceFrames: Bool { Self.hdr }
    var supportsWideColorSourceFrames: Bool { Self.hdr }

    var sourcePixelBufferAttributes: [String: any Sendable]? {
        [kCVPixelBufferPixelFormatTypeKey as String: pixelFormat,
         kCVPixelBufferIOSurfacePropertiesKey as String: [String: String]()]
    }

    var requiredPixelBufferAttributesForRenderContext: [String: any Sendable] {
        [kCVPixelBufferPixelFormatTypeKey as String: pixelFormat,
         kCVPixelBufferIOSurfacePropertiesKey as String: [String: String]()]
    }

    func renderContextChanged(_ newRenderContext: AVVideoCompositionRenderContext) {}

    func startRequest(_ request: AVAsynchronousVideoCompositionRequest) {
        queue.async { [context, colorSpace] in
            guard let instruction = request.videoCompositionInstruction as? RenderInstruction,
                  let output = request.renderContext.newPixelBuffer() else {
                request.finish(with: NSError(domain: "AlMosawiCut", code: 2))
                return
            }
            let size = request.renderContext.size
            var sources: [CMPersistentTrackID: CIImage] = [:]
            for id in instruction.trackIDs {
                if let buffer = request.sourceFrame(byTrackID: id) { sources[id] = CIImage(cvPixelBuffer: buffer) }
            }
            let image = FrameRenderer.render(sources: sources, time: request.compositionTime.seconds, state: instruction.state)
            context.render(image, to: output, bounds: CGRect(origin: .zero, size: size), colorSpace: colorSpace)
            request.finish(withComposedVideoFrame: output)
        }
    }

    func cancelAllPendingVideoCompositionRequests() {}
}

/// 10-bit HDR export: HLG, BT.2020.
final class HDRRenderCompositor: RenderCompositor {
    override class var hdr: Bool { true }
}
