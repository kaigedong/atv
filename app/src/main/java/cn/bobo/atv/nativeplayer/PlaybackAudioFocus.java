package cn.bobo.atv.nativeplayer;

import android.content.Context;
import android.media.AudioAttributes;
import android.media.AudioFocusRequest;
import android.media.AudioManager;
import android.os.Build;
import android.os.Handler;

/** One Activity's music focus; stale callbacks cannot revive a released player. */
final class PlaybackAudioFocus {
    private final AudioManager manager;
    private final Handler handler;
    private final Runnable changed;
    private AudioManager.OnAudioFocusChangeListener listener;
    private Object request;
    private boolean held, requested;
    private int generation;

    PlaybackAudioFocus(Context context, Handler handler, Runnable changed) {
        manager = (AudioManager) context.getSystemService(Context.AUDIO_SERVICE);
        this.handler = handler;
        this.changed = changed;
    }

    boolean hasFocus() { return held; }
    boolean hasRequest() { return requested; }

    @SuppressWarnings("deprecation")
    void acquire() {
        if (held || requested || manager == null) return;
        abandon();
        final int token = ++generation;
        listener = change -> handler.post(() -> {
            if (!requested || token != generation) return;
            if (change == AudioManager.AUDIOFOCUS_LOSS) abandon();
            else setHeld(change == AudioManager.AUDIOFOCUS_GAIN);
        });
        int result;
        if (Build.VERSION.SDK_INT >= 26) {
            request = Modern.create(listener, handler);
            result = Modern.acquire(manager, request);
        } else {
            result = manager.requestAudioFocus(listener, AudioManager.STREAM_MUSIC, AudioManager.AUDIOFOCUS_GAIN);
        }
        // Also unregister the listener when the platform denies a request.
        requested = true;
        if (result == AudioManager.AUDIOFOCUS_REQUEST_GRANTED
                || (Build.VERSION.SDK_INT >= 26 && result == AudioManager.AUDIOFOCUS_REQUEST_DELAYED)) {
            setHeld(result == AudioManager.AUDIOFOCUS_REQUEST_GRANTED);
        } else abandon();
    }

    void acquireForPlayback() {
        if (!held && requested) abandon();
        acquire();
    }

    @SuppressWarnings("deprecation")
    void abandon() {
        ++generation;
        if (requested && manager != null) {
            if (Build.VERSION.SDK_INT >= 26) Modern.abandon(manager, request);
            else manager.abandonAudioFocus(listener);
        }
        requested = false;
        request = null;
        listener = null;
        setHeld(false);
    }

    private void setHeld(boolean value) {
        if (held != value) {
            held = value;
            changed.run();
        }
    }

    // Keep API 26 classes out of the API 19-loaded instance fields and method signatures.
    @android.annotation.TargetApi(26)
    private static final class Modern {
        static Object create(AudioManager.OnAudioFocusChangeListener listener, Handler handler) {
            return new AudioFocusRequest.Builder(AudioManager.AUDIOFOCUS_GAIN)
                    .setAudioAttributes(new AudioAttributes.Builder().setUsage(AudioAttributes.USAGE_MEDIA)
                            .setContentType(AudioAttributes.CONTENT_TYPE_MOVIE).build())
                    .setAcceptsDelayedFocusGain(true).setWillPauseWhenDucked(true)
                    .setOnAudioFocusChangeListener(listener, handler).build();
        }
        static int acquire(AudioManager manager, Object request) {
            return manager.requestAudioFocus((AudioFocusRequest) request);
        }
        static void abandon(AudioManager manager, Object request) {
            manager.abandonAudioFocusRequest((AudioFocusRequest) request);
        }
    }
}
