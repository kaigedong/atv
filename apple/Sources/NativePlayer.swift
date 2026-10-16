import AVFoundation
import Foundation
import SwiftUI

struct Channel: Decodable, Identifiable {
    var id: String { pid }
    let pid: String, streamId: String, name: String, group: String
}
struct PlayerStatus: Decodable {
    var state: String = "idle", message: String = ""
    var videoWidth: UInt32 = 0, videoHeight: UInt32 = 0
}

final class NativePlayer: ObservableObject {
    let media = AppleMedia()
    @Published var channels: [Channel] = []
    @Published var selected: Channel?
    @Published var status = PlayerStatus()
    private var handle: UInt64 = 0
    private var timer: Timer?
    private var ticks = 0

    init() {
        let capacity = Int(atv_channels_json(nil, 0))
        var bytes = [CChar](repeating: 0, count: capacity)
        _ = atv_channels_json(&bytes, capacity)
        channels = (try? JSONDecoder().decode([Channel].self, from: Data(String(cString: bytes).utf8))) ?? []
        selected = channels.first { $0.pid == UserDefaults.standard.string(forKey: "channel") } ?? channels.first
    }

    func open() {
        precondition(Thread.isMainThread)
        if handle != 0 { return }
        do {
            let storage = try FileManager.default.url(for: .applicationSupportDirectory, in: .userDomainMask,
                                                     appropriateFor: nil, create: true).appendingPathComponent("ATV", isDirectory: true)
            try FileManager.default.createDirectory(at: storage, withIntermediateDirectories: true)
            #if !os(macOS)
            try AVAudioSession.sharedInstance().setCategory(.playback, mode: .moviePlayback)
            try AVAudioSession.sharedInstance().setActive(true)
            #endif
            var callbacks = AtvCallbacks()
            callbacks.abi_version = 1
            callbacks.context = Unmanaged.passUnretained(media).toOpaque()
            callbacks.reset = { context in
                guard let context else { return -1 }
                return Unmanaged<AppleMedia>.fromOpaque(context).takeUnretainedValue().reset()
            }
            callbacks.video = { context, bytes, count, pts in
                guard let context, let bytes else { return -1 }
                return Unmanaged<AppleMedia>.fromOpaque(context).takeUnretainedValue().pushVideo(bytes, count: Int(count), pts: pts)
            }
            callbacks.audio = { context, bytes, count, pts in
                guard let context, let bytes else { return -1 }
                return Unmanaged<AppleMedia>.fromOpaque(context).takeUnretainedValue().pushAudio(bytes, count: Int(count), pts: pts)
            }
            callbacks.drain = { context, output in
                guard let context, let output else { return -1 }
                return Unmanaged<AppleMedia>.fromOpaque(context).takeUnretainedValue().drain(output)
            }
            handle = storage.path.withCString { atv_player_create(&callbacks, $0) }
            guard handle != 0 else { throw CocoaError(.fileReadUnknown) }
            if let selected { select(selected) }
            let timer = Timer(timeInterval: 0.01, repeats: true) { [weak self] _ in self?.tick() }
            RunLoop.main.add(timer, forMode: .common)
            self.timer = timer
        } catch { status.state = "error"; status.message = "播放器初始化失败" }
    }
    func close() {
        precondition(Thread.isMainThread)
        timer?.invalidate(); timer = nil
        if handle != 0 { _ = atv_player_destroy(handle); handle = 0 }
        _ = media.reset()
        #if !os(macOS)
        try? AVAudioSession.sharedInstance().setActive(false, options: .notifyOthersOnDeactivation)
        #endif
    }
    deinit {
        timer?.invalidate()
        if handle != 0 {
            let active = handle, heldMedia = media
            if Thread.isMainThread { _ = atv_player_destroy(active) }
            else {
                // Keep the callback context alive until its owning thread has
                // removed the Rust player, even if a host releases us elsewhere.
                DispatchQueue.main.async {
                    _ = atv_player_destroy(active)
                    _ = heldMedia.reset()
                }
            }
        }
    }
    func select(_ channel: Channel) {
        selected = channel
        UserDefaults.standard.set(channel.pid, forKey: "channel")
        if handle != 0 {
            let result = channel.pid.withCString { pid in channel.streamId.withCString { atv_player_select(handle, pid, $0) } }
            if result != 0 { status.state = "error"; status.message = "频道选择失败" }
        }
    }
    func next(_ offset: Int) {
        guard let selected, let index = channels.firstIndex(where: { $0.id == selected.id }), !channels.isEmpty else { return }
        select(channels[(index + offset + channels.count) % channels.count])
    }
    func pause() { if handle != 0 { _ = atv_player_pause(handle) } }
    func resume() { if handle != 0 { _ = atv_player_resume(handle) } }
    private func tick() {
        guard handle != 0 else { return }
        if atv_player_tick(handle) != 0 { status.state = "error"; status.message = "播放核心异常"; return }
        ticks += 1
        if ticks % 25 != 0 { return }
        let capacity = Int(atv_player_status(handle, nil, 0))
        guard capacity > 0, capacity <= 16 * 1024 else { return }
        var output = [CChar](repeating: 0, count: capacity)
        guard atv_player_status(handle, &output, capacity) == capacity else { return }
        if let next = try? JSONDecoder().decode(PlayerStatus.self, from: Data(String(cString: output).utf8)) { status = next }
    }
}
