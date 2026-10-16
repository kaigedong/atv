#include "atv.h"
#include <assert.h>
#include <pthread.h>
#include <stdlib.h>
#include <string.h>

_Static_assert(sizeof(AtvOutputStats) == 40, "stats ABI layout");
_Static_assert(offsetof(AtvOutputStats, width) == 24, "stats member alignment");
_Static_assert(sizeof(AtvCallbacks) == 48, "64-bit callbacks ABI layout");
_Static_assert(offsetof(AtvCallbacks, context) == 8, "callback member alignment");

static int32_t reset(void *context) { (void)context; return 0; }
static int32_t push(void *context, const uint8_t *bytes, size_t count, int64_t pts) {
    (void)context; (void)bytes; (void)count; (void)pts; return 0;
}
static int32_t drain(void *context, AtvOutputStats *stats) {
    (void)context; memset(stats, 0, sizeof(*stats)); return 0;
}
static void *wrong_thread(void *value) {
    uint64_t handle = *(uint64_t *)value;
    assert(atv_player_tick(handle) == -1);
    assert(atv_player_destroy(handle) == -1);
    assert(atv_player_status(handle, NULL, 0) == 0);
    return NULL;
}
int main(int argc, char **argv) {
    assert(argc == 2);
    assert(atv_player_create(NULL, argv[1]) == 0);
    size_t count = atv_channels_json(NULL, 0);
    assert(count > 2 && count < 65536);
    char small[4] = {42, 42, 42, 42};
    assert(atv_channels_json(small, sizeof(small)) == count && small[0] == 42);
    char *channels = malloc(count);
    assert(channels);
    assert(atv_channels_json(channels, count) == count);
    assert(channels[count - 1] == 0 && channels[0] == '[');
    free(channels);
    AtvCallbacks callbacks = {1, NULL, reset, push, push, drain};
    uint64_t handle = atv_player_create(&callbacks, argv[1]);
    assert(handle != 0);
    pthread_t thread;
    assert(pthread_create(&thread, NULL, wrong_thread, &handle) == 0);
    assert(pthread_join(thread, NULL) == 0);
    assert(atv_player_select(handle, "invalid", "1") == -1);
    assert(atv_player_pause(handle) == 0);
    count = atv_player_status(handle, NULL, 0);
    assert(count > 0 && count < 4096);
    char *status = malloc(count);
    assert(status);
    assert(atv_player_status(handle, status, count) == count);
    assert(strstr(status, "paused"));
    free(status);
    assert(atv_player_destroy(handle) == 0);
    assert(atv_player_tick(handle) == -1 && atv_player_destroy(handle) == -1);
    return 0;
}
