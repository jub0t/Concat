import PhotosUI
import SwiftUI

struct HomeView: View {
    @Environment(ProjectStore.self) private var store
    @State private var openRequest: OpenRequest?
    /// A home-screen tool waiting for the user to pick which project to use it in.
    @State private var toolChoice: HomeTool?
    /// The tool to open once a new project has been made from Photos.
    @State private var newProjectTool: HomeTool?
    /// The template the next Photos pick fills.
    @State private var templateToFill: Project?
    @State private var pickerItems: [PhotosPickerItem] = []
    @State private var showPicker = false
    @State private var importing = false
    @State private var renaming: Project?
    @State private var newName = ""
    @State private var errorMessage: String?

    private let columns = [GridItem(.flexible(), spacing: 12), GridItem(.flexible(), spacing: 12)]

    var body: some View {
        NavigationStack {
            ScrollView {
                VStack(alignment: .leading, spacing: 24) {
                    header
                    newProjectCard
                    features
                    if !store.templates.isEmpty { templatesRow }
                    if !store.projects.isEmpty {
                        HStack {
                            Text("Projects").font(.title3.weight(.bold))
                            Spacer()
                            Text("\(store.projects.count)").font(.subheadline.weight(.semibold)).foregroundStyle(Theme.secondaryText)
                        }
                        LazyVGrid(columns: columns, spacing: 16) {
                            ForEach(store.projects) { project in
                                ProjectCard(project: project)
                                    .onTapGesture { openRequest = OpenRequest(project: project) }
                                    .contextMenu {
                                        Button("Rename", systemImage: "pencil") { newName = project.name; renaming = project }
                                        Button("Duplicate", systemImage: "plus.square.on.square") { store.duplicate(project) }
                                        Button("Delete", systemImage: "trash", role: .destructive) { store.delete(project) }
                                    }
                            }
                        }
                    }
                }
                .padding(.horizontal, 18)
                .padding(.bottom, 30)
            }
            .background(Theme.background.ignoresSafeArea())
            .toolbar(.hidden, for: .navigationBar)
            .photosPicker(isPresented: $showPicker, selection: $pickerItems, maxSelectionCount: 50,
                          matching: .any(of: [.videos, .images]), preferredItemEncoding: .current)
            .onChange(of: pickerItems) { _, items in
                guard !items.isEmpty else { return }
                Task { await createProject(from: items) }
            }
            #if DEBUG
            .task {
                // `-open` jumps straight into the newest project when launching from Xcode.
                if ProcessInfo.processInfo.arguments.contains("-open") { openRequest = store.projects.first.map { OpenRequest(project: $0) } }
            }
            #endif
            .fullScreenCover(item: $openRequest, onDismiss: { store.reload() }) { request in
                EditorHost(project: request.project, startTool: request.tool)
            }
            .confirmationDialog(toolChoice?.title ?? "", isPresented: Binding(get: { toolChoice != nil }, set: { if !$0 { toolChoice = nil } }),
                                titleVisibility: .visible) {
                if let tool = toolChoice {
                    if let latest = store.projects.first {
                        Button("Use in “\(latest.name)”") { afterDialog { openRequest = OpenRequest(project: latest, tool: tool) } }
                    }
                    Button("New project from Photos") { newProjectTool = tool; templateToFill = nil; afterDialog { showPicker = true } }
                    Button("Cancel", role: .cancel) {}
                }
            }
            .alert("Rename project", isPresented: Binding(get: { renaming != nil }, set: { if !$0 { renaming = nil } })) {
                TextField("Name", text: $newName)
                Button("Save") { if let p = renaming { store.rename(p, to: newName) } }
                Button("Cancel", role: .cancel) {}
            }
            .alert("Import failed", isPresented: Binding(get: { errorMessage != nil }, set: { if !$0 { errorMessage = nil } })) {
                Button("OK") {}
            } message: { Text(errorMessage ?? "") }
            .overlay {
                if importing {
                    VStack(spacing: 12) {
                        ProgressView().controlSize(.large)
                        Text("Importing…").font(.subheadline.weight(.medium))
                    }
                    .padding(28)
                    .background(.ultraThinMaterial, in: RoundedRectangle(cornerRadius: 20))
                }
            }
        }
    }

    private var header: some View {
        HStack(alignment: .center) {
            VStack(alignment: .leading, spacing: 2) {
                Text("AlMosawiCut").font(.system(size: 30, weight: .heavy, design: .rounded))
                Text("Edit anywhere. No watermark.").font(.subheadline).foregroundStyle(Theme.secondaryText)
            }
            Spacer()
            Image(systemName: "scissors")
                .font(.system(size: 18, weight: .bold))
                .frame(width: 42, height: 42)
                .background(Theme.surface, in: Circle())
        }
        .padding(.top, 8)
    }

    private var newProjectCard: some View {
        Button { newProjectTool = nil; templateToFill = nil; showPicker = true } label: {
            HStack(spacing: 16) {
                Image(systemName: "plus")
                    .font(.system(size: 26, weight: .bold))
                    .frame(width: 58, height: 58)
                    .background(.white.opacity(0.22), in: RoundedRectangle(cornerRadius: 16))
                VStack(alignment: .leading, spacing: 4) {
                    Text("New project").font(.title3.weight(.bold))
                    Text("Pick videos and photos to start cutting").font(.subheadline).opacity(0.85)
                }
                Spacer()
                Image(systemName: "arrow.right").font(.headline)
            }
            .foregroundStyle(.black)
            .padding(18)
            .background(
                LinearGradient(colors: [Theme.accent, Color(red: 0.35, green: 0.9, blue: 0.75)],
                               startPoint: .topLeading, endPoint: .bottomTrailing),
                in: RoundedRectangle(cornerRadius: 22))
        }
        .buttonStyle(PressStyle())
    }

    private var features: some View {
        ScrollView(.horizontal, showsIndicators: false) {
            HStack(spacing: 8) {
                ForEach(HomeTool.allCases) { tool in
                    Button { pick(tool) } label: {
                        Label(tool.title, systemImage: tool.icon)
                            .font(.caption.weight(.semibold))
                            .padding(.horizontal, 12).frame(height: 32)
                            .background(Theme.surface, in: Capsule())
                            .contentShape(Capsule())
                    }
                    .buttonStyle(PressStyle())
                    .foregroundStyle(.white)
                }
            }
        }
    }

    /// Presenting straight from a dialog's button can be dropped while it animates away.
    private func afterDialog(_ action: @escaping @MainActor () -> Void) {
        Task { @MainActor in
            try? await Task.sleep(for: .milliseconds(350))
            action()
        }
    }

    /// With no projects yet, a tool starts a new one; otherwise ask which to use.
    private func pick(_ tool: HomeTool) {
        templateToFill = nil
        if store.projects.isEmpty {
            newProjectTool = tool
            showPicker = true
        } else {
            toolChoice = tool
        }
    }

    private var templatesRow: some View {
        VStack(alignment: .leading, spacing: 10) {
            Text("Templates").font(.title3.weight(.bold))
            ScrollView(.horizontal, showsIndicators: false) {
                HStack(spacing: 10) {
                    ForEach(store.templates) { t in
                        Button {
                            templateToFill = t
                            newProjectTool = nil
                            showPicker = true
                        } label: {
                            VStack(alignment: .leading, spacing: 6) {
                                Image(systemName: "rectangle.stack.badge.play.fill").font(.title2).foregroundStyle(Theme.accent)
                                Text(t.name).font(.subheadline.weight(.semibold)).lineLimit(1)
                                Text("\(t.clips.count) slot\(t.clips.count == 1 ? "" : "s") · \(t.aspect.rawValue)")
                                    .font(.caption).foregroundStyle(Theme.secondaryText)
                            }
                            .padding(14)
                            .frame(width: 150, alignment: .leading)
                            .background(Theme.surface, in: RoundedRectangle(cornerRadius: 16))
                        }
                        .buttonStyle(PressStyle())
                        .contextMenu {
                            Button("Delete template", systemImage: "trash", role: .destructive) { store.deleteTemplate(t) }
                        }
                    }
                }
            }
            Text("Tap a template and pick your videos: they fill its slots in order, with its edits, titles and music.")
                .font(.caption).foregroundStyle(Theme.secondaryText)
        }
    }

    /// A project from a template, its slots filled with the picked footage in order.
    private func fillTemplate(_ template: Project, with items: [PhotosPickerItem]) async {
        var project = store.project(fromTemplate: template)
        let (clips, failures) = await PickerImport.clips(from: items, projectID: project.id)
        for (i, clip) in clips.enumerated() {
            if i < project.clips.count { project.clips[i].replaceMedia(with: clip) } else { project.clips.append(clip) }
        }
        let formatter = DateFormatter()
        formatter.dateFormat = "MMM d, HH:mm"
        project.name = template.name + " " + formatter.string(from: Date())
        store.save(project)
        if !failures.isEmpty { errorMessage = failures.joined(separator: "\n") }
        openRequest = OpenRequest(project: project)
    }

    private func createProject(from items: [PhotosPickerItem]) async {
        importing = true
        let tool = newProjectTool
        let template = templateToFill
        defer { importing = false; pickerItems = []; newProjectTool = nil; templateToFill = nil }
        if let template {
            await fillTemplate(template, with: items)
            return
        }
        let formatter = DateFormatter()
        formatter.dateFormat = "MMM d, HH:mm"
        var project = store.create(named: "Project " + formatter.string(from: Date()))
        let (clips, failures) = await PickerImport.clips(from: items, projectID: project.id)
        project.clips = clips
        store.save(project)
        if !failures.isEmpty { errorMessage = failures.joined(separator: "\n") }
        if !clips.isEmpty { openRequest = OpenRequest(project: project, tool: tool) }
    }
}

/// A project to open, and optionally a tool to open it at.
struct OpenRequest: Identifiable {
    let id = UUID()
    var project: Project
    var tool: HomeTool?
}

/// Shortcuts on the home screen into the editor's main tools.
enum HomeTool: String, CaseIterable, Identifiable {
    case captions, removeBG, tts, record, music, fonts, effects, transitions, keyframes, voiceFX
    var id: String { rawValue }

    var title: String {
        switch self {
        case .captions: "Auto captions"
        case .removeBG: "Remove BG"
        case .tts: "Text to speech"
        case .record: "Voiceover"
        case .music: "Add audio"
        case .fonts: "My fonts"
        case .effects: "Effects"
        case .transitions: "Transitions"
        case .keyframes: "Keyframes"
        case .voiceFX: "Voice FX"
        }
    }

    var icon: String {
        switch self {
        case .captions: "captions.bubble.fill"
        case .removeBG: "person.crop.rectangle"
        case .tts: "text.bubble.fill"
        case .record: "mic.fill"
        case .music: "music.note.list"
        case .fonts: "textformat"
        case .effects: "sparkles"
        case .transitions: "square.split.diagonal.2x2"
        case .keyframes: "diamond.fill"
        case .voiceFX: "waveform"
        }
    }
}

enum PickerImport {
    @MainActor
    static func clips(from items: [PhotosPickerItem], projectID: UUID) async -> ([Clip], [String]) {
        var clips: [Clip] = []
        var failures: [String] = []
        for item in items {
            do {
                if item.supportedContentTypes.contains(where: { $0.conforms(to: .movie) }),
                   let movie = try await item.loadTransferable(type: PickedMovie.self) {
                    clips.append(try await MediaImporter.importVideo(at: movie.url, into: projectID))
                } else if let data = try await item.loadTransferable(type: Data.self), let image = UIImage(data: data) {
                    clips.append(try await MediaImporter.importPhoto(image, into: projectID))
                } else {
                    failures.append("One item couldn't be read.")
                }
            } catch {
                failures.append(error.localizedDescription)
            }
        }
        return (clips, failures)
    }
}

private struct ProjectCard: View {
    let project: Project
    @State private var thumbs = Thumbnails.shared

    private static let relative: RelativeDateTimeFormatter = {
        let f = RelativeDateTimeFormatter()
        f.unitsStyle = .short
        return f
    }()

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            ZStack {
                Rectangle().fill(Theme.surface)
                if let clip = project.clips.first {
                    let strip = thumbs.strip(for: clip, projectID: project.id)
                    if let image = strip.first {
                        Image(uiImage: image).resizable().scaledToFill()
                    }
                } else {
                    Image(systemName: "film").font(.largeTitle).foregroundStyle(.tertiary)
                }
            }
            .frame(height: 200)
            .frame(maxWidth: .infinity)
            .clipShape(RoundedRectangle(cornerRadius: 16))
            .overlay(alignment: .bottomTrailing) {
                Text(project.duration.timecode)
                    .font(.caption2.monospacedDigit().weight(.bold))
                    .padding(.horizontal, 7).padding(.vertical, 4)
                    .background(.black.opacity(0.6), in: Capsule())
                    .padding(8)
            }
            .overlay(alignment: .topLeading) {
                Text(project.aspect.rawValue)
                    .font(.system(size: 9.5, weight: .bold))
                    .padding(.horizontal, 6).padding(.vertical, 3)
                    .background(.black.opacity(0.55), in: Capsule())
                    .padding(8)
            }
            VStack(alignment: .leading, spacing: 2) {
                Text(project.name).font(.subheadline.weight(.semibold)).lineLimit(1)
                Text("Edited " + Self.relative.localizedString(for: project.modifiedAt, relativeTo: Date()))
                    .font(.caption).foregroundStyle(Theme.secondaryText)
            }
        }
        .contentShape(Rectangle())
    }
}
