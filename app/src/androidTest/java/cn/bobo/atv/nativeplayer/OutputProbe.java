package cn.bobo.atv.nativeplayer;

import android.view.Surface;

/** Only the optional test APK contains this library and generated fixture. */
final class OutputProbe {
    static native String run(Surface surface, byte[] transport);
    static native void close();
}
