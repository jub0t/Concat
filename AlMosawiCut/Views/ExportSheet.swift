import SwiftUI

struct ExportSheet: View {
    let project: Project
    @Environment(\.dismiss) private var dismiss
    @AppStorage("exportQuality") private var quality: ExportQuality = .hd1080
    @AppStorage("exportFPS") private var fps = 30
    @AppStorage("exportHEVC") private var hevc = false
    @AppStorage("export10bit") private var tenBit = false
    @State private var progress: Double?
    @State private var result: URL?
    @State private var saved = false
    @State private var error: String?
    @State private var task: Task<Void, Never>?

    var body: some View {
        NavigationStack {
            ScrollView {
                VStack(alignment: .leading, spacing: 18) {
                    if let result {
                        VStack(spacing: 14) {
                            Image(systemName: saved ? "checkmark.circle.fill" : "film.fill")
                                .font(.system(size: 46)).foregroundStyle(Theme.accent)
                            Text(saved ? "Saved to Photos" : "Your video is ready").font(.title3.weight(.bold))
                            HStack(spacing: 10) {
                                if !saved {
                                    Button {
                                        Task {
                                            do { try await Exporter.saveToPhotos(result); saved = true }
                                            catch { self.error = error.localizedDescription }
                                        }
                                    } label: { big("Save to Photos", "photo.on.rectangle", primary: true) }
                                }
                                ShareLink(item: result) { big("Share", "square.and.arrow.up", primary: saved) }
                            }
                        }
                        .frame(maxWidth: .infinity)
                        .padding(.top, 10)
                    } else if let progress {
                        VStack(spacing: 14) {
                            ZStack {
                                Circle().stroke(Theme.surfaceHigh, lineWidth: 8)
                                Circle().trim(from: 0, to: progress).stroke(Theme.accent, style: StrokeStyle(lineWidth: 8, lineCap: .round))
                                    .rotationEffect(.degrees(-90))
                                Text("\(Int(progress * 100))%").font(.title2.weight(.bold).monospacedDigit())
                            }
                            .frame(width: 120, height: 120)
                            Text("Rendering on this device…").font(.subheadline).foregroundStyle(Theme.secondaryText)
                            Button("Cancel", role: .destructive) { task?.cancel(); self.progress = nil }
                        }
                        .frame(maxWidth: .infinity).padding(.top, 10)
                    } else {
                        option("Resolution") {
                            Picker("", selection: $quality) { ForEach(ExportQuality.allCases) { Text($0.rawValue).tag($0) } }
                                .pickerStyle(.segmented)
                        }
                        option("Frame rate") {
                            Picker("", selection: $fps) { ForEach([24, 30, 60], id: \.self) { Text("\($0) fps").tag($0) } }
                                .pickerStyle(.segmented)
                        }
                        Toggle("HEVC (smaller files)", isOn: $hevc).font(.subheadline)
                        if hevc {
                            Toggle("10-bit HDR (HLG)", isOn: $tenBit).font(.subheadline)
                            if tenBit {
                                Text("Brighter highlights and smoother gradients on HDR screens. Plays on recent iPhones, Macs and HDR TVs.")
                                    .font(.caption).foregroundStyle(Theme.secondaryText)
                            }
                        }
                        HStack {
                            info("Canvas", project.aspect.rawValue)
                            info("Length", project.duration.timecode)
                            info("Size", sizeLabel)
                        }
                        Button { start() } label: { big("Export video", "square.and.arrow.up", primary: true) }
                        Text("No watermark. Everything renders on this device.").font(.caption).foregroundStyle(Theme.secondaryText)
                            .frame(maxWidth: .infinity)
                    }
                    if let error { Text(error).font(.caption).foregroundStyle(.red) }
                }
                .padding(20)
            }
            .background(Theme.surface)
            .navigationTitle("Export")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar { ToolbarItem(placement: .topBarTrailing) { Button("Done") { task?.cancel(); dismiss() } } }
        }
        .presentationDetents([.medium, .large])
        .presentationBackground(Theme.surface)
    }

    private var sizeLabel: String {
        let s = project.aspect.baseSize
        return "\(Int(s.width * quality.scale))×\(Int(s.height * quality.scale))"
    }

    private func option<Content: View>(_ title: String, @ViewBuilder _ content: () -> Content) -> some View {
        VStack(alignment: .leading, spacing: 8) { SectionLabel(title); content() }
    }

    private func info(_ title: String, _ value: String) -> some View {
        VStack(spacing: 3) {
            Text(value).font(.subheadline.weight(.bold).monospacedDigit())
            Text(title).font(.caption2).foregroundStyle(Theme.secondaryText)
        }
        .frame(maxWidth: .infinity).padding(.vertical, 10)
        .background(Theme.surfaceHigh, in: RoundedRectangle(cornerRadius: 12))
    }

    private func big(_ title: String, _ icon: String, primary: Bool) -> some View {
        Label(title, systemImage: icon)
            .font(.headline)
            .frame(maxWidth: .infinity, minHeight: 50)
            .background(primary ? Theme.accent : Theme.surfaceHigh, in: RoundedRectangle(cornerRadius: 14))
            .foregroundStyle(primary ? .black : .white)
    }

    private func start() {
        error = nil
        progress = 0
        task = Task {
            do {
                let url = try await Exporter.export(project, quality: quality, hevc: hevc, frameRate: Int32(fps), tenBit: hevc && tenBit) { p in
                    Task { @MainActor in if progress != nil { progress = p } }
                }
                result = url
                progress = nil
            } catch {
                if !Task.isCancelled { self.error = error.localizedDescription }
                progress = nil
            }
        }
    }
}
