import AVFoundation
import AudioToolbox
import CoreMedia
import Foundation

private enum MediaFailure: Error { case invalid, platform(OSStatus) }
private func checked(_ status: OSStatus) throws {
    if status != noErr { throw MediaFailure.platform(status) }
}

/// Main-thread output only. AVFoundation decodes AVC/AAC and synchronizes them.
/// This adapter never fetches a URL or interprets JavaScript/WASM.
final class AppleMedia {
    let video = AVSampleBufferDisplayLayer()
    private let audio = AVSampleBufferAudioRenderer()
    private let clock = AVSampleBufferRenderSynchronizer()
    private var videoFormat: CMVideoFormatDescription?
    private var audioFormat: CMAudioFormatDescription?
    private var sps: Data?, pps: Data?, audioCookie: Data?
    private var adts = Data()
    private var adtsPts: Int64?
    private var videoTimes: [CMTime] = []
    private var audioTimes: [CMTime] = []
    private var started = false
    private var stats = AtvOutputStats()
    private let bias: Int64 = 30_000_000

    init() {
        video.videoGravity = .resizeAspect
        clock.addRenderer(video)
        clock.addRenderer(audio)
    }

    func reset() -> Int32 {
        precondition(Thread.isMainThread)
        clock.setRate(0, time: .zero)
        video.flushAndRemoveImage()
        audio.flush()
        videoFormat = nil; audioFormat = nil
        sps = nil; pps = nil; audioCookie = nil
        adts.removeAll(keepingCapacity: false); adtsPts = nil
        videoTimes.removeAll(keepingCapacity: false)
        audioTimes.removeAll(keepingCapacity: false)
        started = false; stats = AtvOutputStats()
        return 0
    }

    private func timestamp(_ pts: Int64) throws -> CMTime {
        let (value, overflow) = pts.addingReportingOverflow(bias)
        guard !overflow, value >= 0 else { throw MediaFailure.invalid }
        return CMTime(value: value, timescale: 1_000_000)
    }
    private func start(at time: CMTime) {
        if !started {
            clock.setRate(1, time: time,
                          atHostTime: CMTimeAdd(CMClockGetTime(CMClockGetHostTimeClock()), CMTime(seconds: 0.1, preferredTimescale: 1_000_000)))
            started = true
        }
    }

    // Copy into a CoreMedia-owned allocation. Rust's callback bytes cannot be
    // retained after the callback returns.
    private func block(_ data: Data) throws -> CMBlockBuffer {
        guard !data.isEmpty else { throw MediaFailure.invalid }
        var output: CMBlockBuffer?
        try checked(CMBlockBufferCreateWithMemoryBlock(allocator: kCFAllocatorDefault,
            memoryBlock: nil, blockLength: data.count, blockAllocator: kCFAllocatorDefault,
            customBlockSource: nil, offsetToData: 0, dataLength: data.count, flags: 0,
            blockBufferOut: &output))
        guard let output else { throw MediaFailure.invalid }
        try data.withUnsafeBytes { bytes in
            try checked(CMBlockBufferReplaceDataBytes(with: bytes.baseAddress!, blockBuffer: output,
                                                     offsetIntoDestination: 0, dataLength: data.count))
        }
        return output
    }

    private func nals(_ bytes: UnsafeBufferPointer<UInt8>) -> [Data] {
        var result: [Data] = [], start: Int?, i = 0
        while i + 3 <= bytes.count {
            let prefix = i + 4 <= bytes.count && bytes[i] == 0 && bytes[i+1] == 0 && bytes[i+2] == 0 && bytes[i+3] == 1
                ? 4 : (bytes[i] == 0 && bytes[i+1] == 0 && bytes[i+2] == 1 ? 3 : 0)
            if prefix != 0 {
                if let start, start < i { result.append(Data(bytes[start..<i])) }
                start = i + prefix; i += prefix
            } else { i += 1 }
        }
        if let start, start < bytes.count { result.append(Data(bytes[start..<bytes.count])) }
        return result
    }

    func pushVideo(_ bytes: UnsafePointer<UInt8>, count: Int, pts: Int64) -> Int32 {
        precondition(Thread.isMainThread)
        guard count > 0, count <= 8 * 1024 * 1024 else { return -1 }
        if videoTimes.count >= 256 || (videoFormat != nil && !video.isReadyForMoreMediaData) { return 1 }
        do {
            let units = nals(UnsafeBufferPointer(start: bytes, count: count))
            var payload = Data(), idr = false
            for var nal in units {
                while nal.last == 0 { nal.removeLast() }
                guard let first = nal.first else { continue }
                switch first & 31 {
                case 7:
                    if sps != nal { sps = nal; pps = nil; videoFormat = nil; video.flushAndRemoveImage(); videoTimes.removeAll() }
                case 8:
                    if pps != nal { pps = nal; videoFormat = nil; video.flushAndRemoveImage(); videoTimes.removeAll() }
                case 1...5:
                    idr = idr || first & 31 == 5
                    var size = UInt32(nal.count).bigEndian
                    withUnsafeBytes(of: &size) { payload.append(contentsOf: $0) }
                    payload.append(nal)
                default: break
                }
            }
            if videoFormat == nil, let sps, let pps {
                try sps.withUnsafeBytes { sb in
                    try pps.withUnsafeBytes { pb in
                        let pointers = [sb.baseAddress!.assumingMemoryBound(to: UInt8.self), pb.baseAddress!.assumingMemoryBound(to: UInt8.self)]
                        let sizes = [sps.count, pps.count]
                        try checked(CMVideoFormatDescriptionCreateFromH264ParameterSets(allocator: kCFAllocatorDefault,
                            parameterSetCount: 2, parameterSetPointers: pointers, parameterSetSizes: sizes,
                            nalUnitHeaderLength: 4, formatDescriptionOut: &videoFormat))
                    }
                }
            }
            guard !payload.isEmpty else { return 0 }
            guard let format = videoFormat else { throw MediaFailure.invalid }
            let time = try timestamp(pts)
            var timing = CMSampleTimingInfo(duration: .invalid, presentationTimeStamp: time, decodeTimeStamp: .invalid)
            var size = payload.count, sample: CMSampleBuffer?
            try checked(CMSampleBufferCreateReady(allocator: kCFAllocatorDefault, dataBuffer: try block(payload),
                formatDescription: format, sampleCount: 1, sampleTimingEntryCount: 1, sampleTimingArray: &timing,
                sampleSizeEntryCount: 1, sampleSizeArray: &size, sampleBufferOut: &sample))
            guard let sample else { throw MediaFailure.invalid }
            if let attachments = CMSampleBufferGetSampleAttachmentsArray(sample, createIfNecessary: true),
               CFArrayGetCount(attachments) > 0 {
                let dictionary = unsafeBitCast(CFArrayGetValueAtIndex(attachments, 0), to: CFMutableDictionary.self)
                CFDictionarySetValue(dictionary, Unmanaged.passUnretained(kCMSampleAttachmentKey_NotSync).toOpaque(),
                                     Unmanaged.passUnretained(idr ? kCFBooleanFalse! : kCFBooleanTrue!).toOpaque())
            }
            let dimensions = CMVideoFormatDescriptionGetDimensions(format)
            guard dimensions.width > 0, dimensions.height > 0 else { throw MediaFailure.invalid }
            stats.width = UInt32(dimensions.width); stats.height = UInt32(dimensions.height)
            video.enqueue(sample); videoTimes.append(time); start(at: time)
            return video.status == .failed ? -1 : 0
        } catch { return -1 }
    }

    func pushAudio(_ bytes: UnsafePointer<UInt8>, count: Int, pts: Int64) -> Int32 {
        precondition(Thread.isMainThread)
        guard count >= 0, count <= 256 * 1024 else { return -1 }
        if adts.count + count > 256 * 1024 { return 1 }
        if adts.isEmpty { adtsPts = pts }
        adts.append(bytes, count: count)
        do { try feedAudio(); return 0 } catch { return -1 }
    }

    private func feedAudio() throws {
        let rates = [96000,88200,64000,48000,44100,32000,24000,22050,16000,12000,11025,8000,7350]
        while adts.count >= 7 && audio.isReadyForMoreMediaData && audioTimes.count < 256 {
            let b = [UInt8](adts.prefix(7))
            guard b[0] == 255, b[1] & 0xf6 == 0xf0, b[6] & 3 == 0 else { throw MediaFailure.invalid }
            let rateIndex = Int((b[2] >> 2) & 15), object = (b[2] >> 6) + 1
            let channelConfig = Int((b[2] & 1) << 2 | b[3] >> 6)
            guard rates.indices.contains(rateIndex), (1...7).contains(channelConfig) else { throw MediaFailure.invalid }
            let channels = channelConfig == 7 ? 8 : channelConfig
            let length = Int(b[3] & 3) << 11 | Int(b[4]) << 3 | Int(b[5] >> 5)
            let header = b[1] & 1 == 0 ? 9 : 7
            guard length > header else { throw MediaFailure.invalid }
            if adts.count < length { break }
            let cookie = Data([object << 3 | UInt8(rateIndex >> 1), UInt8(rateIndex & 1) << 7 | UInt8(channelConfig) << 3])
            if audioCookie != cookie {
                audio.flush(); audioTimes.removeAll()
                var asbd = AudioStreamBasicDescription(mSampleRate: Double(rates[rateIndex]),
                    mFormatID: kAudioFormatMPEG4AAC, mFormatFlags: UInt32(object), mBytesPerPacket: 0,
                    mFramesPerPacket: 1024, mBytesPerFrame: 0, mChannelsPerFrame: UInt32(channels), mBitsPerChannel: 0, mReserved: 0)
                try cookie.withUnsafeBytes { bytes in
                    try checked(CMAudioFormatDescriptionCreate(allocator: kCFAllocatorDefault, asbd: &asbd,
                        layoutSize: 0, layout: nil, magicCookieSize: cookie.count, magicCookie: bytes.baseAddress,
                        extensions: nil, formatDescriptionOut: &audioFormat))
                }
                audioCookie = cookie
                stats.audio_sample_rate = UInt32(rates[rateIndex]); stats.audio_channels = UInt32(channels)
            }
            guard let format = audioFormat, let pts = adtsPts else { throw MediaFailure.invalid }
            let time = try timestamp(pts), payload = Data(adts.dropFirst(header).prefix(length-header))
            var packet = AudioStreamPacketDescription(mStartOffset: 0, mVariableFramesInPacket: 1024, mDataByteSize: UInt32(payload.count))
            var sample: CMSampleBuffer?
            try checked(CMAudioSampleBufferCreateReadyWithPacketDescriptions(allocator: kCFAllocatorDefault,
                dataBuffer: try block(payload), formatDescription: format, sampleCount: 1,
                presentationTimeStamp: time, packetDescriptions: &packet, sampleBufferOut: &sample))
            guard let sample else { throw MediaFailure.invalid }
            audio.enqueue(sample)
            audioTimes.append(CMTimeAdd(time, CMTime(value: 1024, timescale: CMTimeScale(rates[rateIndex]))))
            start(at: time)
            adts.removeFirst(length); adtsPts = pts + 1_024_000_000 / Int64(rates[rateIndex])
            if audio.status == .failed { throw MediaFailure.invalid }
        }
    }

    func drain(_ output: UnsafeMutablePointer<AtvOutputStats>) -> Int32 {
        precondition(Thread.isMainThread)
        do {
            try feedAudio()
            if video.status == .failed || audio.status == .failed { return -1 }
            let time = clock.currentTime()
            // Presentation-clock accounting after AVFoundation accepted the
            // samples. This is not independent display/PCM-sink instrumentation.
            let videos = video.status == .rendering ? videoTimes.filter { CMTimeCompare($0, time) <= 0 }.count : 0
            let audios = audio.status == .rendering ? audioTimes.filter { CMTimeCompare($0, time) <= 0 }.count : 0
            stats.video_frames += UInt64(videos); stats.audio_frames += UInt64(audios * 1024)
            if video.status == .rendering { videoTimes.removeAll { CMTimeCompare($0, time) <= 0 } }
            if audio.status == .rendering { audioTimes.removeAll { CMTimeCompare($0, time) <= 0 } }
            if started { stats.position_us = max(0, CMTimeConvertScale(time, timescale: 1_000_000, method: .default).value - bias) }
            output.pointee = stats
            return 0
        } catch { return -1 }
    }
}
