import PhotosUI
import SwiftUI

struct HomeView: View {
    @Environment(ProjectStore.self) private var store
    @State private var openProject: Project?
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
                    if !store.projects.isEmpty {
                        HStack {
                            Text("Projects").font(.title3.weight(.bold))
                            Spacer()
                            Text("\(store.projects.count)").font(.subheadline.weight(.semibold)).foregroundStyle(Theme.secondaryText)
                        }
                        LazyVGrid(columns: columns, spacing: 16) {
                            ForEach(store.projects) { project in
                                ProjectCard(project: project)
                                    .onTapGesture { openProject = project }
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
                if ProcessInfo.processInfo.arguments.contains("-open") { openProject = store.projects.first }
            }
            #endif
            .fullScreenCover(item: $openProject, onDismiss: { store.reload() }) { project in
                EditorHost(project: project)
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
        Button { showPicker = true } label: {
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
                feature("captions.bubble.fill", "Auto captions")
                feature("person.crop.rectangle", "Remove BG")
                feature("text.bubble.fill", "Text to speech")
                feature("sparkles", "Effects")
                feature("square.split.diagonal.2x2", "Transitions")
                feature("diamond.fill", "Keyframes")
                feature("waveform", "Voice FX")
            }
        }
    }

    private func feature(_ icon: String, _ title: String) -> some View {
        Label(title, systemImage: icon)
            .font(.caption.weight(.semibold))
            .padding(.horizontal, 12).frame(height: 32)
            .background(Theme.surface, in: Capsule())
    }

    private func createProject(from items: [PhotosPickerItem]) async {
        importing = true
        defer { importing = false; pickerItems = [] }
        let formatter = DateFormatter()
        formatter.dateFormat = "MMM d, HH:mm"
        var project = store.create(named: "Project " + formatter.string(from: Date()))
        let (clips, failures) = await PickerImport.clips(from: items, projectID: project.id)
        project.clips = clips
        store.save(project)
        if !failures.isEmpty { errorMessage = failures.joined(separator: "\n") }
        if !clips.isEmpty { openProject = project }
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
