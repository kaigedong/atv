import SwiftUI
import AVFoundation

#if os(macOS)
import AppKit
private final class MediaView: NSView {
    private let display: AVSampleBufferDisplayLayer
    init(_ display: AVSampleBufferDisplayLayer) {
        self.display = display; super.init(frame: .zero)
        wantsLayer = true; layer = CALayer(); layer?.backgroundColor = NSColor.black.cgColor
        layer?.addSublayer(display)
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) unsupported") }
    override func layout() { super.layout(); display.frame = bounds }
}
private struct VideoSurface: NSViewRepresentable {
    let media: AppleMedia
    func makeNSView(context: Context) -> NSView { MediaView(media.video) }
    func updateNSView(_ view: NSView, context: Context) { }
}
#else
import UIKit
private final class MediaView: UIView {
    private let display: AVSampleBufferDisplayLayer
    init(_ display: AVSampleBufferDisplayLayer) {
        self.display = display; super.init(frame: .zero)
        backgroundColor = .black; layer.addSublayer(display)
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) unsupported") }
    override func layoutSubviews() { super.layoutSubviews(); display.frame = bounds }
}
private struct VideoSurface: UIViewRepresentable {
    let media: AppleMedia
    func makeUIView(context: Context) -> UIView { MediaView(media.video) }
    func updateUIView(_ view: UIView, context: Context) { }
}
#endif

private struct ContentView: View {
    @StateObject private var player = NativePlayer()
    @State private var fullscreen = false
    @Environment(\.scenePhase) private var phase
    var body: some View {
        HStack(spacing: 0) {
            if !fullscreen {
                List {
                    ForEach(["yangshi", "weishi"], id: \.self) { group in
                        Section(group == "yangshi" ? "央视" : "卫视") {
                            ForEach(player.channels.filter { $0.group == group }) { channel in
                                Button { player.select(channel) } label: {
                                    HStack { Text(channel.name); if player.selected?.id == channel.id { Image(systemName: "play.fill") } }
                                }
                            }
                        }
                    }
                }.frame(minWidth: 180, idealWidth: 240, maxWidth: 320)
            }
            VStack {
                HStack {
                    Text(player.selected?.name ?? "ATV").font(.headline)
                    Spacer()
                    Button(fullscreen ? "退出全屏" : "全屏") { fullscreen.toggle() }
                }
                ZStack {
                    VideoSurface(media: player.media)
                    if player.status.state != "playing" {
                        VStack(spacing: 16) {
                            Text(player.status.message.isEmpty ? "正在连接直播…" : player.status.message)
                            if player.status.state == "error", let channel = player.selected {
                                Button("重试") { player.select(channel) }
                            }
                        }.padding().background(.black.opacity(0.8))
                    }
                }
                HStack {
                    Button("上一台") { player.next(-1) }
                    #if !os(tvOS)
                        .keyboardShortcut(.upArrow, modifiers: [])
                    #endif
                    Button("下一台") { player.next(1) }
                    #if !os(tvOS)
                        .keyboardShortcut(.downArrow, modifiers: [])
                    #endif
                    Spacer()
                }
            }.padding()
        }
        .background(.black).preferredColorScheme(.dark)
        .onAppear { player.open() }.onDisappear { player.close() }
        .onChange(of: phase) { next in if next == .active { player.resume() } else { player.pause() } }
        #if os(tvOS)
        .onExitCommand { fullscreen = false }
        .onMoveCommand { direction in
            if fullscreen {
                if direction == .up { player.next(-1) }
                if direction == .down { player.next(1) }
            }
        }
        #endif
    }
}

@main
struct ATVApp: App {
    var body: some Scene {
        #if os(macOS)
        Window("ATV · 电视直播", id: "main") { ContentView().frame(minWidth: 800, minHeight: 450) }
        #else
        WindowGroup { ContentView() }
        #endif
    }
}
