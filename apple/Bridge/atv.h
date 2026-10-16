#ifndef ATV_H
#define ATV_H
#include <stddef.h>
#include <stdint.h>
// ABI 1. All calls/callbacks for one handle stay on its creating thread.
// Callbacks must not reenter atv_player_*. Context survives until destroy returns.
typedef struct {
    uint64_t video_frames, audio_frames;
    int64_t position_us;
    uint32_t width, height, audio_sample_rate, audio_channels;
} AtvOutputStats;
// Push: 0 = accepted, 1 = backpressure with zero input consumed, negative = error.
// Media bytes are borrowed for the call only. Reset/drain return 0 or negative.
typedef int32_t (*AtvPush)(void *, const uint8_t *, size_t, int64_t);
typedef struct {
    uint32_t abi_version;
    void *context;
    int32_t (*reset)(void *);
    AtvPush video, audio;
    int32_t (*drain)(void *, AtvOutputStats *);
} AtvCallbacks;
uint64_t atv_player_create(const AtvCallbacks *, const char *storage_directory);
int32_t atv_player_select(uint64_t, const char *pid, const char *stream);
int32_t atv_player_tick(uint64_t);
int32_t atv_player_pause(uint64_t);
int32_t atv_player_resume(uint64_t);
int32_t atv_player_destroy(uint64_t);
// Required size includes trailing NUL; too-small buffer is untouched; 0 = error.
size_t atv_player_status(uint64_t, char *, size_t);
size_t atv_channels_json(char *, size_t);
#endif
