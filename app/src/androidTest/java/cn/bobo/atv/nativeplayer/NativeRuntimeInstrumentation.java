package cn.bobo.atv.nativeplayer;

import android.app.Activity;
import android.app.Instrumentation;
import android.content.Intent;
import android.content.pm.PackageInfo;
import android.graphics.Rect;
import android.os.Build;
import android.os.Bundle;
import android.os.SystemClock;
import android.os.ParcelFileDescriptor;
import android.view.KeyEvent;
import android.view.View;
import android.view.ViewGroup;
import android.widget.Button;

import org.json.JSONArray;
import org.json.JSONObject;

import java.io.BufferedReader;
import java.io.ByteArrayOutputStream;
import java.io.InputStream;
import java.io.File;
import java.io.FileInputStream;
import java.io.FileOutputStream;
import java.io.InputStreamReader;
import java.lang.reflect.Field;
import java.nio.charset.StandardCharsets;
import java.security.MessageDigest;
import java.util.UUID;
import java.util.concurrent.atomic.AtomicReference;

/** Real native playback acceptance runner; no test player, media mocks or production hooks. */
public final class NativeRuntimeInstrumentation extends Instrumentation {
    private static final String PACKAGE = "cn.bobo.atv.nativeplayer";
    private static final String[][] CHANNELS = {
            {"CCTV1", "600001859", "yangshi"},
            {"CCTV2", "600001800", "yangshi"},
            {"CCTV13", "600001811", "yangshi"},
            {"北京卫视", "600002309", "weishi"}
    };
    private final JSONObject report = new JSONObject();
    private final JSONArray steps = new JSONArray();
    private final String runId = UUID.randomUUID().toString();
    private Bundle arguments;
    private MainActivity activity;
    private long channelTimeoutMs;
    private JSONObject currentStep;
    private JSONArray snapshots;
    private long lastSnapshot;

    @Override public void onCreate(Bundle args) {
        super.onCreate(args);
        arguments = args == null ? new Bundle() : args;
        start();
    }

    @Override public void onStart() {
        boolean passed = false;
        Throwable failure = null;
        String mode = arguments.getString("mode", "smoke");
        channelTimeoutMs = boundedArgument("channelTimeoutSeconds", 75, 10, 180) * 1000L;
        try {
            PackageInfo pkg = getTargetContext().getPackageManager().getPackageInfo(PACKAGE, 0);
            report.put("schemaVersion", 1).put("runId", runId).put("mode", mode)
                    .put("startedUnixMillis", System.currentTimeMillis())
                    .put("versionName", pkg.versionName)
                    .put("versionCode", Build.VERSION.SDK_INT >= 28 ? pkg.getLongVersionCode() : pkg.versionCode)
                    .put("targetApkSha256", digestFile(getTargetContext().getApplicationInfo().sourceDir))
                    .put("testApkSha256", digestFile(getContext().getApplicationInfo().sourceDir))
                    .put("deviceModel", Build.MODEL).put("sdk", Build.VERSION.SDK_INT)
                    .put("steps", steps)
                    .put("audioEvidence", "Native audio output counters; audible output requires the separate capture comparison");
            require(NativeBridge.available(), "native library unavailable");
            launch();
            switch (mode) {
                case "smoke": smoke(); break;
                case "all": smoke(); soak(); break;
                case "soak": selectChannel(CHANNELS[2]); awaitProgress(CHANNELS[2][1], 3); soak(); break;
                case "lifecycle": selectChannel(CHANNELS[0]); awaitProgress(CHANNELS[0][1], 3); lifecycle(); break;
                case "audio-lifecycle": audioLifecycle(); break;
                case "audio-focus-basic": audioFocusBasic(); break;
                case "retry": selectChannel(CHANNELS[0]); awaitProgress(CHANNELS[0][1], 3); retry(); break;
                case "remote": navigationAndFullscreen(); break;
                case "groups": groupNavigation(); break;
                case "channel-idle": channelIdle(); break;
                case "feedback": playbackFeedback(); break;
                case "network": network(false); break;
                case "startup-network": network(true); break;
                case "component": component(); break;
                case "settings": settings(false); break;
                case "settings-basic": settings(true); break;
                case "overlays": overlays(false); break;
                case "overlays-basic": overlays(true); break;
                default: throw new CheckFailure("unknown instrumentation mode");
            }
            passed = true;
        } catch (Throwable error) {
            failure = error;
            try {
                report.put("failureClass", error.getClass().getName());
                JSONArray frames = new JSONArray();
                StackTraceElement[] trace = error.getStackTrace();
                for (int i = 0; i < Math.min(trace.length, 12); i++) frames.put(new JSONObject()
                        .put("class", trace[i].getClassName()).put("method", trace[i].getMethodName())
                        .put("line", trace[i].getLineNumber()));
                report.put("failureFrames", frames);
                // Only our own fixed acceptance messages are suitable for the report.
                if (error instanceof CheckFailure) report.put("failure", error.getMessage());
                try { snapshot(true); } catch (Exception ignored) { }
                if (currentStep != null) currentStep.put("passed", false);
            } catch (Exception ignored) { }
        } finally {
            try { finishActivity(); }
            catch (Throwable error) {
                passed = false;
                if (failure == null) failure = error;
                try { report.put("cleanupFailureClass", error.getClass().getName()); }
                catch (Exception ignored) { }
            }
            Bundle result = new Bundle();
            try {
                report.put("passed", passed).put("finishedUnixMillis", System.currentTimeMillis());
                File external = getTargetContext().getExternalFilesDir(null);
                File directory = external == null ? null : new File(external, "runtime-native");
                if (directory == null || (!directory.isDirectory() && !directory.mkdirs()))
                    directory = new File(getTargetContext().getFilesDir(), "runtime-native");
                require(directory.isDirectory() || directory.mkdirs(), "cannot create runtime report directory");
                File destination = new File(directory, runId + ".json");
                try (FileOutputStream output = new FileOutputStream(destination)) {
                    output.write(report.toString(2).getBytes(StandardCharsets.UTF_8));
                }
                result.putString("reportPath", destination.getAbsolutePath());
            } catch (Throwable writeError) {
                passed = false;
                result.putString("reportFailureClass", writeError.getClass().getName());
            }
            result.putString("runId", runId);
            result.putString("passed", Boolean.toString(passed));
            if (failure != null) result.putString("failureClass", failure.getClass().getName());
            finish(passed ? Activity.RESULT_OK : Activity.RESULT_CANCELED, result);
        }
    }

    private int boundedArgument(String key, int fallback, int minimum, int maximum) {
        try { return Math.max(minimum, Math.min(maximum, Integer.parseInt(arguments.getString(key, "")))); }
        catch (RuntimeException ignored) { return fallback; }
    }

    private void launch() throws Exception {
        begin("launch");
        Intent intent = new Intent(Intent.ACTION_MAIN).setClassName(PACKAGE, PACKAGE + ".MainActivity")
                .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK | Intent.FLAG_ACTIVITY_CLEAR_TASK);
        activity = (MainActivity) startActivitySync(intent);
        waitForIdleSync();
        waitCondition(() -> session() != 0, 15000, "surface did not create a native session");
        snapshot(true);
        complete();
    }

    private void smoke() throws Exception {
        // Two complete rounds exercise cancellation and ordinary switching in both groups.
        for (int round = 0; round < 2; round++) {
            for (String[] channel : CHANNELS) {
                begin("switch-" + round + "-" + channel[0]);
                selectChannel(channel);
                awaitProgress(channel[1], 3);
                complete();
            }
        }
        navigationAndFullscreen();
        retry();
        lifecycle();
    }

    private void freshChannelOverlay() throws Exception {
        if (!boolField("fullscreen")) sendKeyDownUpSync(KeyEvent.KEYCODE_BACK);
        sendKeyDownUpSync(KeyEvent.KEYCODE_DPAD_CENTER);
        waitCondition(() -> !boolField("fullscreen") && channelRows().hasFocus() && windowFocused(),
                3000, "channel overlay did not acquire focus");
    }

    private void channelIdle() throws Exception {
        selectChannel(CHANNELS[0]);
        awaitProgress(CHANNELS[0][1], 2);
        long handle = session();
        begin("idle-channels-close-without-changing-playback");
        freshChannelOverlay();
        JSONObject before = status();
        SystemClock.sleep(3500);
        require(!boolField("fullscreen"), "channels closed before the idle delay");
        waitCondition(() -> boolField("fullscreen"), 4000, "idle channel list did not close");
        require(viewField("sidebar").getVisibility() == View.GONE, "idle channel sidebar remained visible");
        JSONObject after = status();
        require(session() == handle && CHANNELS[0][1].equals(selectedPid()), "idle close changed playback selection or Surface");
        require(after.optLong("videoFrames") > before.optLong("videoFrames")
                && after.optLong("audioFrames") > before.optLong("audioFrames"), "idle close interrupted audio/video output");
        complete();

        begin("remote-input-restarts-channel-idle-delay");
        freshChannelOverlay();
        SystemClock.sleep(3000);
        sendKeyDownUpSync(KeyEvent.KEYCODE_DPAD_DOWN);
        SystemClock.sleep(2500);
        require(!boolField("fullscreen"), "remote input did not restart the idle delay");
        waitCondition(() -> boolField("fullscreen"), 4000, "channels did not close after renewed inactivity");
        require(CHANNELS[0][1].equals(selectedPid()), "browsing selected a channel without confirmation");
        complete();

        begin("non-channel-focus-does-not-auto-close");
        freshChannelOverlay();
        main(() -> require(viewField("settingsButton").requestFocus(), "cannot focus top settings button"));
        SystemClock.sleep(5500);
        require(!boolField("fullscreen"), "non-channel focus was closed by the channel timer");
        main(() -> require(channelRows().findViewWithTag(CHANNELS[0][1]).requestFocus(), "cannot return focus to channels"));
        waitCondition(() -> boolField("fullscreen"), 7000, "returning channel focus did not arm idle closing");
        complete();

        begin("settings-exempt-and-return-starts-new-idle-delay");
        freshChannelOverlay();
        sendKeyDownUpSync(KeyEvent.KEYCODE_MENU);
        waitCondition(this::settingsFocused, 3000, "settings did not open for idle exemption check");
        SystemClock.sleep(5500);
        require(settingsFocused(), "channel idle timer closed settings");
        sendKeyDownUpSync(KeyEvent.KEYCODE_DPAD_LEFT);
        waitCondition(() -> !settingsShowing() && !boolField("fullscreen") && windowFocused(),
                3000, "settings LEFT did not restore channels");
        SystemClock.sleep(1500);
        require(!boolField("fullscreen"), "old idle deadline immediately closed restored channels");
        waitCondition(() -> boolField("fullscreen"), 5500, "restored channel list did not close after inactivity");
        require(session() == handle && CHANNELS[0][1].equals(selectedPid()), "idle settings navigation changed playback");
        complete();
    }

    private void groupNavigation() throws Exception {
        begin("remote-channel-groups");
        main(() -> {
            for (String name : new String[]{"cctv", "satellite"}) {
                Button heading = findButton(activity.getWindow().getDecorView(), resourceString(name));
                require(heading != null && !heading.isFocusable(), "group heading accepts remote focus");
            }
            findButton(activity.getWindow().getDecorView(), resourceString("cctv")).performClick();
            ViewGroup rows = channelRows();
            require(rows.getChildAt(rows.getChildCount() - 1).performClick(), "cannot select deep CCTV channel");
        });
        waitForIdleSync();
        sendKeyDownUpSync(KeyEvent.KEYCODE_DPAD_UP); // Exit emulator touch mode with a real remote key.
        main(() -> require(channelRows().findViewWithTag(selectedPid()).requestFocus(), "cannot focus playing CCTV"));
        String cctvPid = selectedPid();
        long handle = session();
        sendKeyDownUpSync(KeyEvent.KEYCODE_DPAD_RIGHT);
        assertFirstPageFocus("weishi", cctvPid);
        sendKeyDownUpSync(KeyEvent.KEYCODE_DPAD_RIGHT);
        waitCondition(this::settingsFocused, 3000, "satellite RIGHT did not open settings");
        require(session() == handle && cctvPid.equals(selectedPid()), "settings navigation changed playback");
        sendKeyDownUpSync(KeyEvent.KEYCODE_DPAD_LEFT);
        waitCondition(() -> !settingsShowing() && !boolField("fullscreen"), 3000, "settings LEFT did not return to satellite channels");
        waitCondition(this::windowFocused, 3000, "channel window did not regain focus after settings LEFT");
        assertFirstPageFocus("weishi", cctvPid);
        sendKeyDownUpSync(KeyEvent.KEYCODE_DPAD_LEFT);
        assertPlayingChannelFocus("yangshi", cctvPid);
        sendKeyDownUpSync(KeyEvent.KEYCODE_DPAD_LEFT);
        assertPlayingChannelFocus("yangshi", cctvPid);
        sendKeyDownUpSync(KeyEvent.KEYCODE_DPAD_RIGHT);
        assertFirstPageFocus("weishi", cctvPid);
        main(() -> {
            ViewGroup rows = channelRows();
            require(rows.getChildAt(rows.getChildCount() - 1).requestFocus(), "cannot focus deep satellite channel");
        });
        sendKeyDownUpSync(KeyEvent.KEYCODE_MENU);
        waitCondition(this::settingsFocused, 3000, "MENU did not open settings while browsing another group");
        sendKeyDownUpSync(KeyEvent.KEYCODE_BACK);
        waitCondition(() -> !settingsShowing(), 3000, "BACK did not close settings");
        main(() -> {
            ViewGroup rows = channelRows();
            require("weishi".equals(field("group")) && rows.getChildAt(rows.getChildCount() - 1).hasFocus(),
                    "settings return lost the browsed group or deep channel focus");
            require(cctvPid.equals(selectedPid()), "settings changed the playing channel");
        });
        sendKeyDownUpSync(KeyEvent.KEYCODE_DPAD_RIGHT);
        waitCondition(this::settingsFocused, 3000, "deep satellite RIGHT did not open settings");
        sendKeyDownUpSync(KeyEvent.KEYCODE_DPAD_LEFT);
        waitCondition(() -> !settingsShowing() && !boolField("fullscreen"), 3000, "settings LEFT did not restore deep satellite channel");
        waitCondition(this::windowFocused, 3000, "deep satellite window did not regain focus");
        main(() -> {
            ViewGroup rows = channelRows();
            require(rows.getChildAt(rows.getChildCount() - 1).hasFocus(), "settings LEFT lost deep satellite focus");
        });
        sendKeyDownUpSync(KeyEvent.KEYCODE_DPAD_CENTER);
        String satellitePid = selectedPid();
        require(!satellitePid.equals(cctvPid), "CENTER did not select the satellite channel");
        sendKeyDownUpSync(KeyEvent.KEYCODE_DPAD_LEFT);
        assertFirstPageFocus("yangshi", satellitePid);
        sendKeyDownUpSync(KeyEvent.KEYCODE_DPAD_RIGHT);
        assertPlayingChannelFocus("weishi", satellitePid);
        sendKeyDownUpSync(KeyEvent.KEYCODE_DPAD_LEFT);
        assertFirstPageFocus("yangshi", satellitePid);
        sendKeyDownUpSync(KeyEvent.KEYCODE_BACK);
        sendKeyDownUpSync(KeyEvent.KEYCODE_DPAD_CENTER);
        assertPlayingChannelFocus("weishi", satellitePid);
        require(session() == handle, "group browsing recreated playback session");
        sendKeyDownUpSync(KeyEvent.KEYCODE_BACK);
        sendKeyDownUpSync(KeyEvent.KEYCODE_DPAD_RIGHT);
        waitCondition(this::settingsFocused, 3000, "fullscreen RIGHT did not open settings");
        sendKeyDownUpSync(KeyEvent.KEYCODE_DPAD_LEFT);
        waitCondition(() -> !settingsShowing() && !boolField("fullscreen"), 3000, "fullscreen settings LEFT did not open satellite channels");
        waitCondition(this::windowFocused, 3000, "satellite window did not regain focus from fullscreen settings");
        assertPlayingChannelFocus("weishi", satellitePid);
        currentStep.put("deepCctvPid", cctvPid).put("deepSatellitePid", satellitePid)
                .put("satelliteRightOpensSettings", true).put("settingsLeftRestoresSatelliteFocus", true)
                .put("fullscreenSettingsLeftOpensSatellite", true);
        complete();
    }

    private void assertFirstPageFocus(String group, String playingPid) throws Exception {
        waitForIdleSync();
        main(() -> {
            require(group.equals(field("group")), "horizontal key did not switch channel group");
            require(playingPid.equals(selectedPid()), "group browsing changed the playing channel");
            ViewGroup rows = channelRows();
            View viewport = (View) rows.getParent();
            Rect viewportBounds = new Rect(); viewport.getGlobalVisibleRect(viewportBounds);
            View focus = activity.getCurrentFocus();
            require(focus != null && focus.getParent() == rows, "group switch lost channel focus");
            int fullyVisible = 0;
            for (int i = 0; i < rows.getChildCount(); i++) {
                View row = rows.getChildAt(i); int[] point = new int[2]; row.getLocationOnScreen(point);
                if (point[1] >= viewportBounds.top && point[1] + row.getHeight() <= viewportBounds.bottom) fullyVisible++;
            }
            require(viewport.getScrollY() == 0, "other group did not reset to the first page");
            require(fullyVisible > 0 && focus == rows.getChildAt(fullyVisible / 2), "focus is not in the middle of the first page");
        });
    }

    private void assertPlayingChannelFocus(String group, String pid) throws Exception {
        waitForIdleSync();
        main(() -> {
            require(group.equals(field("group")) && pid.equals(selectedPid()), "return changed playing channel or group");
            View row = channelRows().findViewWithTag(pid);
            int check = currentStep.optInt("playingFocusChecks") + 1;
            currentStep.put("playingFocusChecks", check);
            recordFocus("playingFocusCheck" + check, row);
            require(row != null && row.hasFocus(), "return did not focus the playing channel");
            Rect visible = new Rect();
            require(row.getLocalVisibleRect(visible) && visible.height() == row.getHeight(), "playing channel is outside the viewport");
        });
    }

    private void navigationAndFullscreen() throws Exception {
        begin("remote-navigation-and-fullscreen");
        selectChannel(CHANNELS[0]);
        awaitProgress(CHANNELS[0][1], 2);
        main(() -> recordFocus("channelFocusBeforeRemotePreparation",
                channelRows().findViewWithTag(CHANNELS[0][1])));
        waitCondition(this::windowFocused, 5000, "remote input target window has no focus");
        // Programmatic clicks can leave a phone emulator in touch mode. A real
        // directional key exits touch mode just as a TV remote does; do not
        // change production focusability or use a privileged touch-mode setter.
        currentStep.put("remotePreparation", "real KEYCODE_DPAD_UP");
        if (boolField("fullscreen")) sendKeyDownUpSync(KeyEvent.KEYCODE_DPAD_LEFT);
        sendKeyDownUpSync(KeyEvent.KEYCODE_DPAD_UP);
        waitForIdleSync();
        main(() -> {
            View row = channelRows().findViewWithTag(CHANNELS[0][1]);
            recordFocus("channelFocusAfterRemotePreparation", row);
            boolean focused = row != null && row.requestFocus();
            recordFocus("channelFocusAfterRequest", row);
            require(focused && row.hasFocus(), "cannot focus CCTV1 channel row");
        });
        sendKeyDownUpSync(KeyEvent.KEYCODE_DPAD_DOWN);
        waitForIdleSync();
        AtomicReference<Object> focusTag = new AtomicReference<>();
        main(() -> focusTag.set(activity.getCurrentFocus() == null ? null : activity.getCurrentFocus().getTag()));
        require(CHANNELS[1][1].equals(focusTag.get()), "DPAD_DOWN did not focus CCTV2");
        sendKeyDownUpSync(KeyEvent.KEYCODE_DPAD_CENTER);
        awaitProgress(CHANNELS[1][1], 3);
        if (boolField("fullscreen")) sendKeyDownUpSync(KeyEvent.KEYCODE_DPAD_LEFT);
        main(() -> {
            Button full = findButton(activity.getWindow().getDecorView(), resourceString("fullscreen"));
            recordFocus("fullscreenFocusBeforeRequest", full);
            boolean focused = full != null && full.requestFocus();
            recordFocus("fullscreenFocusAfterRequest", full);
            require(focused && full.hasFocus(), "cannot focus fullscreen button");
        });
        sendKeyDownUpSync(KeyEvent.KEYCODE_DPAD_CENTER);
        waitCondition(() -> boolField("fullscreen"), 3000, "DPAD_CENTER did not enter fullscreen");
        require(viewField("sidebar").getVisibility() == View.GONE, "fullscreen sidebar remains visible");
        sendKeyDownUpSync(KeyEvent.KEYCODE_DPAD_UP);
        awaitProgress(CHANNELS[0][1], 2);
        require(boolField("fullscreen") && viewField("sidebar").getVisibility() == View.GONE,
                "fullscreen DPAD_UP did not preserve fullscreen");
        sendKeyDownUpSync(KeyEvent.KEYCODE_DPAD_DOWN);
        awaitProgress(CHANNELS[1][1], 2);
        require(boolField("fullscreen") && viewField("sidebar").getVisibility() == View.GONE,
                "fullscreen DPAD_DOWN did not preserve fullscreen");
        sendKeyDownUpSync(KeyEvent.KEYCODE_DPAD_CENTER);
        waitCondition(() -> !boolField("fullscreen"), 3000, "CENTER did not open channel overlay");
        sendKeyDownUpSync(KeyEvent.KEYCODE_BACK);
        waitCondition(() -> boolField("fullscreen"), 3000, "BACK did not close channel overlay");
        require(!activity.isFinishing(), "BACK unexpectedly finished channel overlay activity");
        sendKeyDownUpSync(KeyEvent.KEYCODE_DPAD_CENTER);
        waitCondition(() -> !boolField("fullscreen"), 3000, "CENTER did not reopen channel overlay");
        awaitProgress(CHANNELS[1][1], 2);
        currentStep.put("remoteKeys", new JSONArray().put("DPAD_UP").put("DPAD_DOWN").put("DPAD_CENTER").put("DPAD_CENTER")
                .put("DPAD_UP").put("DPAD_DOWN").put("DPAD_CENTER").put("BACK").put("DPAD_CENTER"));
        complete();
    }

    private Rect bounds(String fieldName) throws Exception {
        Rect rect = new Rect();
        main(() -> {
            View view = viewField(fieldName); int[] position = new int[2]; view.getLocationOnScreen(position);
            rect.set(position[0], position[1], position[0] + view.getWidth(), position[1] + view.getHeight());
        });
        return rect;
    }
    private void assertOverlayPreservesVideo(long handle, Rect surfaceBounds, boolean basic) throws Exception {
        waitForIdleSync();
        require(session() == handle, "overlay recreated native playback session");
        currentStep.put("expectedSurfaceBounds", surfaceBounds.flattenToString())
                .put("observedSurfaceBounds", bounds("surface").flattenToString());
        require(surfaceBounds.equals(bounds("surface")), "overlay moved or resized video surface");
        main(() -> {
            View area = viewField("playerArea"); View parent = (View) area.getParent();
            require(area.getWidth() == parent.getWidth() && area.getHeight() == parent.getHeight(),
                    "video container does not fill the window");
        });
        if (!basic) awaitProgress(CHANNELS[2][1], 2);
    }
    private void playbackFeedback() throws Exception {
        begin("ui-feedback-with-controlled-status-snapshots");
        // Exercise UI decisions deterministically; these snapshots do not prove live playback.
        main(() -> ((android.os.Handler) field("handler")).removeCallbacks((Runnable) field("poll")));
        try {
            assertPlaybackFeedback("loading", 0, true, false);
            main(() -> {
                ViewGroup player = (ViewGroup) viewField("playerArea");
                for (int i = 0; i < player.getChildCount(); i++) {
                    View child = player.getChildAt(i);
                    require(!(child instanceof android.widget.TextView) || child instanceof Button,
                            "player still contains a text status overlay");
                }
                require(!viewField("loading").isFocusable(), "spinner steals remote focus");
            });
            assertPlaybackFeedback("loading", 10, false, false);
            assertPlaybackFeedback("loading", 20, false, false);
            SystemClock.sleep(2700);
            assertPlaybackFeedback("loading", 20, true, false);
            assertPlaybackFeedback("loading", 30, false, false);
            assertPlaybackFeedback("paused", 30, false, false);
            assertPlaybackFeedback("error", 30, false, true);
            main(() -> {
                android.widget.FrameLayout.LayoutParams position =
                        (android.widget.FrameLayout.LayoutParams) viewField("retry").getLayoutParams();
                require((position.gravity & android.view.Gravity.VERTICAL_GRAVITY_MASK) == android.view.Gravity.BOTTOM,
                        "retry action still covers the center of video");
            });
            assertPlaybackFeedback("loading", 0, true, false);
            assertPlaybackFeedback("playing", 10, false, false);
            currentStep.put("controlledUiCases", 9).put("noCenteredStatusText", true)
                    .put("advancingVideoHidesSpinnerDuringLoading", true).put("stalledVideoShowsSpinner", true);
            complete();
        } finally {
            main(() -> ((android.os.Handler) field("handler")).post((Runnable) field("poll")));
        }
    }

    private void assertPlaybackFeedback(String phase, long frames, boolean spinner, boolean retryVisible) throws Exception {
        JSONObject sample = new JSONObject().put("state", phase).put("pid", selectedPid())
                .put("videoFrames", frames).put("videoWidth", frames > 0 ? 1920 : 0)
                .put("videoHeight", frames > 0 ? 1080 : 0).put("message", "网络连接失败，请重试");
        main(() -> {
            java.lang.reflect.Method update = MainActivity.class.getDeclaredMethod("showStatus", JSONObject.class);
            update.setAccessible(true);
            update.invoke(activity, sample);
            require((viewField("loading").getVisibility() == View.VISIBLE) == spinner,
                    "unexpected spinner visibility for " + phase);
            require((viewField("retry").getVisibility() == View.VISIBLE) == retryVisible,
                    "unexpected retry visibility for " + phase);
        });
    }

    private void overlays(boolean basic) throws Exception {
        begin("full-size-live-video-under-native-translucent-overlays");
        if (!basic) { selectChannel(CHANNELS[2]); awaitProgress(CHANNELS[2][1], 2); }
        else {
            main(() -> NativeBridge.pause(session()));
            waitCondition(() -> "paused".equals(status().optString("state")), 10000, "cannot stabilize API 19 UI-only probe");
        }
        main(() -> {
            Button full = findButton(activity.getWindow().getDecorView(), resourceString("fullscreen"));
            require(full != null && full.performClick(), "cannot hide initial channel overlay");
        });
        waitCondition(() -> bounds("playerArea").width() > bounds("playerArea").height(), 5000, "landscape layout did not settle");
        SystemClock.sleep(300);
        waitForIdleSync();
        long handle = session(); Rect fullSize = bounds("surface");
        require(boolField("fullscreen"), "initial overlay did not close");
        sendKeyDownUpSync(KeyEvent.KEYCODE_DPAD_CENTER);
        waitCondition(() -> !boolField("fullscreen"), 3000, "CENTER did not open channel overlay");
        main(() -> {
            View panel = viewField("sidebar");
            int alpha = android.graphics.Color.alpha(((android.graphics.drawable.ColorDrawable) panel.getBackground()).getColor());
            require(alpha > 0 && alpha < 255, "channel panel is not translucent");
        });
        // Browse with real remote keys without selecting: audio/video and PID
        // must keep advancing on the old channel until explicit confirmation.
        sendKeyDownUpSync(KeyEvent.KEYCODE_DPAD_DOWN);
        main(() -> require(channelRows().findViewWithTag(selectedPid()).requestFocus(), "cannot focus playing channel"));
        sendKeyDownUpSync(KeyEvent.KEYCODE_DPAD_DOWN);
        if (!basic) require(CHANNELS[2][1].equals(selectedPid()), "browsing selected a new channel without confirmation");
        assertOverlayPreservesVideo(handle, fullSize, basic);
        sendKeyDownUpSync(KeyEvent.KEYCODE_MENU);
        waitCondition(this::settingsShowing, 3000, "MENU did not open settings overlay");
        main(() -> {
            android.app.Dialog dialog = (android.app.Dialog) field("settingsDialog");
            require((dialog.getWindow().getAttributes().flags & android.view.WindowManager.LayoutParams.FLAG_DIM_BEHIND) == 0,
                    "settings dims the entire video window");
            ViewGroup host = dialog.findViewById(android.R.id.content);
            android.widget.ScrollView scroll = (android.widget.ScrollView) host.getChildAt(0);
            int alpha = android.graphics.Color.alpha(((android.graphics.drawable.ColorDrawable) scroll.getChildAt(0).getBackground()).getColor());
            require(alpha > 0 && alpha < 255, "settings panel is not translucent");
        });
        assertOverlayPreservesVideo(handle, fullSize, basic);
        main(() -> require(viewField("qualityButton").performClick(), "cannot open quality overlay"));
        assertOverlayPreservesVideo(handle, fullSize, basic);
        sendKeyDownUpSync(KeyEvent.KEYCODE_BACK);
        waitCondition(() -> field("qualityDialog") == null, 3000, "BACK did not close quality choices");
        require(settingsShowing(), "quality BACK also closed settings");
        sendKeyDownUpSync(KeyEvent.KEYCODE_BACK);
        waitCondition(() -> !settingsShowing() && !boolField("fullscreen"), 3000, "settings did not restore channel overlay");
        assertOverlayPreservesVideo(handle, fullSize, basic);
        sendKeyDownUpSync(KeyEvent.KEYCODE_BACK);
        waitCondition(() -> boolField("fullscreen"), 3000, "BACK did not hide channel overlay");
        require(!activity.isFinishing(), "closing channel overlay finished playback");
        assertOverlayPreservesVideo(handle, fullSize, basic);
        sendKeyDownUpSync(KeyEvent.KEYCODE_DPAD_RIGHT);
        waitCondition(this::settingsShowing, 3000, "RIGHT did not open fullscreen settings");
        assertOverlayPreservesVideo(handle, fullSize, basic);
        sendKeyDownUpSync(KeyEvent.KEYCODE_BACK);
        waitCondition(() -> !settingsShowing(), 3000, "BACK did not close fullscreen settings");
        waitCondition(this::windowFocused, 3000, "playback window did not regain focus after settings");
        require(boolField("fullscreen"), "fullscreen settings unexpectedly opened channel panel");
        currentStep.put("surfaceBoundsPreserved", fullSize.flattenToString()).put("sameNativeSession", true)
                .put("translucentChannelAndSettingsPanels", true).put("liveAudioVideoAdvancedUnderOverlays", !basic)
                .put("qualityAndBackHierarchyPassed", true);
        if (!basic) {
            sendKeyDownUpSync(KeyEvent.KEYCODE_DPAD_CENTER);
            waitCondition(() -> !boolField("fullscreen"), 3000, "cannot reopen channels for switch");
            main(() -> require(channelRows().findViewWithTag(CHANNELS[1][1]).requestFocus(), "cannot focus destination channel"));
            sendKeyDownUpSync(KeyEvent.KEYCODE_DPAD_CENTER);
            awaitProgress(CHANNELS[1][1], 2);
            require(handle == session() && fullSize.equals(bounds("surface")), "confirmed switch changed video container");
            currentStep.put("confirmedChannelSwitchPassed", true);
        }
        complete();
    }

    private boolean windowFocused() throws Exception {
        AtomicReference<Boolean> focused = new AtomicReference<>(false);
        main(() -> focused.set(activity.hasWindowFocus()));
        return focused.get();
    }

    private JSONObject updateStatus() throws Exception {
        return new JSONObject(NativeBridge.updateStatus(((Number) field("updater")).longValue()));
    }
    private boolean settingsShowing() throws Exception {
        android.app.Dialog dialog = (android.app.Dialog) field("settingsDialog");
        return dialog != null && dialog.isShowing();
    }
    private boolean settingsFocused() throws Exception {
        AtomicReference<Boolean> focused = new AtomicReference<>(false);
        main(() -> {
            android.app.Dialog dialog = (android.app.Dialog) field("settingsDialog");
            focused.set(dialog != null && dialog.isShowing() && dialog.getWindow().getDecorView().hasWindowFocus());
        });
        return focused.get();
    }
    private void selectQuality(String id, String label) throws Exception {
        main(() -> require(viewField("qualityButton").performClick(), "quality button click failed"));
        main(() -> {
            android.app.AlertDialog dialog = (android.app.AlertDialog) field("qualityDialog");
            require(dialog != null && dialog.isShowing(), "quality choices did not open");
            android.widget.ListView list = dialog.getListView();
            int index = -1;
            for (int i = 0; i < list.getAdapter().getCount(); i++) if (label.equals(list.getAdapter().getItem(i).toString())) index = i;
            require(index >= 0, "server-advertised quality missing from UI");
            require(list.performItemClick(list.getAdapter().getView(index, null, list), index, list.getAdapter().getItemId(index)), "quality choice click failed");
        });
        waitCondition(() -> id.equals(status().optString("requestedQuality")), 5000, "quality preference did not reach Rust");
        awaitProgress(CHANNELS[2][1], 2);
        require(("auto".equals(id) ? "hd" : id).equals(status().optString("effectiveQuality")), "server returned a different definition");
        currentStep.put(id, new JSONObject().put("requested", id).put("effective", status().optString("effectiveQuality"))
                .put("width", status().optInt("videoWidth")).put("height", status().optInt("videoHeight")));
    }
    private void settings(boolean basic) throws Exception {
        String previousQuality = (String) field("preferredQuality");
        try {
            if (!basic) { selectChannel(CHANNELS[2]); awaitProgress(CHANNELS[2][1], 2); }
            begin("remote-settings-open-close-and-live-preservation");
            sendKeyDownUpSync(KeyEvent.KEYCODE_MENU);
            waitCondition(this::settingsShowing, 5000, "MENU did not open settings");
            main(() -> require(viewField("qualityButton").isShown(), "settings content missing"));
            sendKeyDownUpSync(KeyEvent.KEYCODE_BACK);
            waitCondition(() -> !settingsShowing(), 3000, "BACK did not close settings");
            require(!activity.isFinishing(), "settings BACK exited the app");
            main(() -> {
                Button full = findButton(activity.getWindow().getDecorView(), resourceString("fullscreen"));
                require(full != null && full.performClick(), "fullscreen entry missing");
            });
            waitCondition(() -> boolField("fullscreen"), 3000, "cannot enter fullscreen for settings test");
            long beforeSettings = session();
            sendKeyDownUpSync(KeyEvent.KEYCODE_DPAD_RIGHT);
            waitCondition(this::settingsShowing, 3000, "fullscreen RIGHT did not open settings");
            require(boolField("fullscreen") && session() == beforeSettings, "settings recreated playback surface");
            sendKeyDownUpSync(KeyEvent.KEYCODE_BACK);
            waitCondition(() -> !settingsShowing(), 3000, "settings BACK did not dismiss fullscreen overlay");
            require(boolField("fullscreen"), "closing settings unexpectedly exited fullscreen");
            sendKeyDownUpSync(KeyEvent.KEYCODE_DPAD_CENTER);
            waitCondition(() -> !boolField("fullscreen"), 3000, "CENTER did not open channel overlay");
            currentStep.put("fullscreenRightAndBackPassed", true).put("surfacePreserved", true);
            main(() -> require(viewField("settingsButton").performClick(), "visible settings button failed"));
            waitCondition(this::settingsShowing, 3000, "settings button did not open settings");
            complete();
            if (!basic) {
                begin("real-server-quality-switch-shd-fhd-auto");
                JSONArray formats = status().optJSONArray("availableQualities");
                require(formats != null && formats.toString().contains("shd") && formats.toString().contains("fhd"), "CCTV13 formats unavailable");
                selectQuality("shd", "超清");
                selectQuality("fhd", "蓝光");
                selectQuality("auto", "自动（优先流畅）");
                require("auto".equals(activity.getPreferences(Activity.MODE_PRIVATE).getString("quality", "")), "quality preference not persisted");
                complete();
            }
            begin("real-github-update-check-cancel-download-and-signature-gate");
            waitCondition(() -> !java.util.Arrays.asList("checking", "downloading", "cancelling").contains(updateStatus().optString("state")), 20000, "automatic check did not finish");
            main(() -> require(viewField("updateCheck").performClick(), "check update click failed"));
            waitCondition(() -> java.util.Arrays.asList("available", "current", "error").contains(updateStatus().optString("state")), 30000, "update check did not finish");
            currentStep.put("updateCheck", updateStatus());
            require(!"error".equals(updateStatus().optString("state")), "real GitHub release check failed");
            currentStep.put("checkState", updateStatus().optString("state"));
            if ("available".equals(updateStatus().optString("state"))) {
                main(() -> {
                    long handle = ((Number) field("updater")).longValue();
                    require(NativeBridge.updateCommand(handle, 1), "download did not start");
                    require(NativeBridge.updateCommand(handle, 2), "cancel command failed");
                });
                waitCondition(() -> "cancelled".equals(updateStatus().optString("state")), 50000, "cancelled update did not stop");
                main(() -> require(viewField("updateCheck").performClick(), "retry update check click failed"));
                waitCondition(() -> "available".equals(updateStatus().optString("state")) || "error".equals(updateStatus().optString("state")), 30000, "update retry did not finish");
                require("available".equals(updateStatus().optString("state")), "update retry failed");
                main(() -> require(viewField("updateDownload").performClick(), "download button failed"));
                waitCondition(() -> "ready".equals(updateStatus().optString("state")) || "error".equals(updateStatus().optString("state")), 60000, "APK download did not finish");
                JSONObject ready = updateStatus();
                require("ready".equals(ready.optString("state")), "real APK download or checksum verification failed");
                File root = (File) field("updateDirectory");
                File apk = new File(root, ready.getString("readyPath"));
                require(apk.isFile() && apk.length() == ready.getLong("totalBytes"), "downloaded APK missing or truncated");
                require(NativeBridge.updateVerify(((Number) field("updater")).longValue()), "cached APK reverification failed");
                currentStep.put("downloadedVersionCode", ready.getInt("versionCode")).put("downloadedBytes", apk.length())
                        .put("downloadedApkSha256", digestFile(apk.getPath())).put("cancelThenRetryPassed", true)
                        .put("legacyFileUriRequired", Build.VERSION.SDK_INT < 24);
                waitCondition(() -> viewField("updateInstall").isShown() && viewField("updateInstall").isEnabled()
                        && "ready".equals(((JSONObject) field("lastUpdate")).optString("state")), 5000, "install UI did not become ready");
                main(() -> require(viewField("updateInstall").performClick(), "install button failed"));
                waitCondition(() -> field("messageDialog") != null, 15000, "development-signature migration was not explained");
                main(() -> {
                    android.app.AlertDialog dialog = (android.app.AlertDialog) field("messageDialog");
                    android.widget.TextView message = dialog.findViewById(android.R.id.message);
                    require(message != null && message.getText().toString().contains("开发签名"), "wrong signature was not blocked");
                    dialog.dismiss();
                });
                currentStep.put("developmentSignatureBlockedBeforeInstaller", true);
            }
            if (!basic) awaitProgress(CHANNELS[2][1], 2);
            complete();
        } finally {
            main(() -> {
                android.app.Dialog dialog = (android.app.Dialog) field("settingsDialog");
                if (dialog != null) dialog.dismiss();
                Field preference = MainActivity.class.getDeclaredField("preferredQuality"); preference.setAccessible(true); preference.set(activity, previousQuality);
                activity.getPreferences(Activity.MODE_PRIVATE).edit().putString("quality", previousQuality).apply();
                NativeBridge.quality(session(), previousQuality);
            });
        }
    }

    /** UI-only diagnostics distinguish an input harness problem from product focus handling. */
    private void recordFocus(String key, View row) throws Exception {
        JSONObject value = new JSONObject().put("exists", row != null)
                .put("activityWindowFocus", activity.hasWindowFocus());
        if (row != null) {
            Rect visible = new Rect();
            boolean globallyVisible = row.getGlobalVisibleRect(visible);
            value.put("inTouchMode", row.isInTouchMode()).put("focusable", row.isFocusable())
                    .put("focusableInTouchMode", row.isFocusableInTouchMode()).put("shown", row.isShown())
                    .put("windowFocus", row.hasWindowFocus()).put("hasFocus", row.hasFocus())
                    .put("attached", row.isAttachedToWindow()).put("visibility", row.getVisibility())
                    .put("windowVisibility", row.getWindowVisibility()).put("globallyVisible", globallyVisible)
                    .put("visibleWidth", visible.width()).put("visibleHeight", visible.height());
        }
        currentStep.put(key, value);
    }

    private void retry() throws Exception {
        begin("real-server-failure-and-ui-retry");
        selectChannel(CHANNELS[0]);
        awaitProgress(CHANNELS[0][1], 2);
        long handle = session();
        // A normally formatted but nonexistent channel exercises the actual resolver
        // and service rejection. Keep the genuine selected UI channel and device GUID.
        NativeBridge.select(handle, "999999999999", "999999999999");
        waitCondition(() -> "error".equals(status().optString("state")), channelTimeoutMs,
                "nonexistent channel did not produce a real error");
        snapshot(true);
        currentStep.put("failureCode", status().optString("errorCode"));
        waitCondition(() -> viewField("retry").getVisibility() == View.VISIBLE, 3000,
                "failure did not expose retry button");
        main(() -> require(viewField("retry").performClick(), "retry button did not accept click"));
        awaitProgress(CHANNELS[0][1], 3);
        complete();
    }

    /** Explicit opt-in disruption test; never included in smoke/all modes. */
    private void network(boolean startup) throws Exception {
        begin(startup ? "startup-network-failure-and-automatic-recovery" : "real-network-loss-and-automatic-recovery");
        String fingerprint = Build.FINGERPRINT == null ? "" : Build.FINGERPRINT.toLowerCase(java.util.Locale.ROOT);
        String model = Build.MODEL == null ? "" : Build.MODEL.toLowerCase(java.util.Locale.ROOT);
        boolean emulator = fingerprint.startsWith("generic") || fingerprint.contains("sdk_gphone")
                || model.contains("emulator") || model.contains("android sdk") || model.contains("sdk_gphone")
                || model.startsWith("sdk_google_atv");
        boolean explicit = "true".equalsIgnoreCase(arguments.getString("allowNetworkChange", "false"));
        require(emulator || explicit, "network disruption requires an emulator or allowNetworkChange=true");
        selectChannel(CHANNELS[0]);
        if (!startup) awaitProgress(CHANNELS[0][1], 3);
        long handle = session();
        String wifi = radioSetting("wifi_on"), data = radioSetting("mobile_data");
        require(("0".equals(wifi) || "1".equals(wifi)) && ("0".equals(data) || "1".equals(data)),
                "radio state cannot be safely restored");
        currentStep.put("networkChangeExplicitlyAllowed", explicit).put("emulatorDetected", emulator)
                .put("initialWifiEnabled", "1".equals(wifi)).put("initialMobileDataEnabled", "1".equals(data));
        try {
            shell("svc wifi disable");
            shell("svc data disable");
            waitCondition(() -> "0".equals(radioSetting("wifi_on")) && "0".equals(radioSetting("mobile_data")),
                    10000, "radios did not disable");
            if (startup) selectChannel(CHANNELS[0]);
            waitCondition(() -> {
                JSONObject state = status();
                return state.optLong("reconnectAttempts") > 0
                        && ("network".equals(state.optString("errorCode")) || "stalled".equals(state.optString("errorCode")));
            }, channelTimeoutMs, "network outage did not trigger automatic reconnection");
            JSONObject retrying = status();
            require("loading".equals(retrying.optString("state")), "automatic retry remained in terminal error state");
            if (startup) require(retrying.optLong("videoFrames") == 0, "startup failure unexpectedly rendered video");
            currentStep.put("networkLossErrorCode", retrying.optString("errorCode"))
                    .put("reconnectAttemptsWhileOffline", retrying.optLong("reconnectAttempts"));
            waitCondition(() -> viewField("retry").getVisibility() == View.GONE, 3000,
                    "transient failure still requires a manual retry action");
            snapshot(true);
            JSONObject threads = nativeThreads();
            require(threads.optInt("network") <= 2, "automatic retries exceeded the network worker limit");
            currentStep.put("threadsAfterNetworkLoss", threads);
        } finally {
            // Both restore operations must be attempted even if the first fails.
            try { shell("svc wifi " + ("1".equals(wifi) ? "enable" : "disable")); }
            finally { shell("svc data " + ("1".equals(data) ? "enable" : "disable")); }
            waitCondition(() -> wifi.equals(radioSetting("wifi_on")) && data.equals(radioSetting("mobile_data")),
                    15000, "original radio states did not restore");
            currentStep.put("originalRadioStatesRestored", true);
        }
        // No click, channel selection, or resume command after restoring connectivity.
        awaitProgress(CHANNELS[0][1], 3);
        require(session() == handle && CHANNELS[0][1].equals(selectedPid()), "automatic recovery changed channel or native session");
        waitCondition(() -> viewField("retry").getVisibility() == View.GONE && viewField("loading").getVisibility() == View.GONE,
                3000, "recovered playback still shows retry or loading feedback");
        long reconnects = status().optLong("reconnectAttempts");
        require(reconnects > 0, "recovery did not retain automatic reconnect evidence");
        awaitProgress(CHANNELS[0][1], 5);
        require(status().optLong("reconnectAttempts") == reconnects, "healthy playback kept reconnecting");
        currentStep.put("automaticReconnectAttempts", reconnects).put("recoveredWithoutUserAction", true)
                .put("healthyPlaybackDoesNotReconnect", true).put("sameNativeSession", true);
        complete();
    }

    private String radioSetting(String key) throws Exception {
        // Keys are internal fixed literals; no shell input is accepted from test arguments.
        require("wifi_on".equals(key) || "mobile_data".equals(key), "unknown radio setting");
        return shell("settings get global " + key).trim();
    }

    private String shell(String command) throws Exception {
        ParcelFileDescriptor descriptor = getUiAutomation().executeShellCommand(command);
        try (InputStream input = new ParcelFileDescriptor.AutoCloseInputStream(descriptor);
             ByteArrayOutputStream output = new ByteArrayOutputStream()) {
            byte[] bytes = new byte[256]; int length;
            while ((length = input.read(bytes)) != -1) {
                require(output.size() + length <= 4096, "radio command returned excessive output");
                output.write(bytes, 0, length);
            }
            return output.toString(StandardCharsets.UTF_8.name());
        }
    }

    private PlaybackAudioFocus playbackAudioFocus() throws Exception {
        return (PlaybackAudioFocus) field("audioFocus");
    }

    private void audioProbe(String stage, boolean audible) throws Exception {
        int seconds = boundedArgument("audioProbeSeconds", 0, 0, 30);
        if (seconds == 0) return;
        Bundle signal = new Bundle();
        signal.putString("audioProbeStage", stage);
        signal.putString("audioProbeExpectation", audible ? "audible" : "silent");
        sendStatus(2, signal);
        SystemClock.sleep(seconds * 1000L);
    }

    @SuppressWarnings("deprecation")
    private void audioLifecycle() throws Exception {
        begin("initial-audio-focus-and-output");
        selectChannel(CHANNELS[0]);
        awaitProgress(CHANNELS[0][1], 3);
        require(playbackAudioFocus().hasFocus(), "initial playback did not acquire audio focus");
        audioProbe("initial", true);
        complete();
        for (int index = 1; index <= 3; index++) {
            begin("finish-before-audio-reopen-" + index);
            MainActivity oldActivity = activity;
            PlaybackAudioFocus oldFocus = playbackAudioFocus();
            long oldHandle = session();
            main(() -> oldActivity.finish());
            waitCondition(oldActivity::isDestroyed, 10000, "finished Activity was not destroyed");
            require(!oldFocus.hasFocus() && !oldFocus.hasRequest(), "finished Activity retained audio focus");
            complete();
            // Reopen as soon as the old Activity is destroyed, without selecting any channel.
            launch();
            begin("remembered-channel-audio-reopen-" + index);
            require(session() != oldHandle, "reopened Activity reused a destroyed native session");
            require(CHANNELS[0][1].equals(selectedPid()), "reopen did not restore the remembered channel");
            awaitProgress(CHANNELS[0][1], 3);
            require(playbackAudioFocus().hasFocus(), "reopened playback did not reacquire audio focus");
            require(status().optLong("reconnectAttempts") == 0, "reopen required an automatic reconnect workaround");
            waitCondition(() -> nativeThreads().optInt("render") == 1, 5000, "old renderer survived the audio reopen");
            currentStep.put("reopenedWithoutChannelSelection", true).put("audioFocusReacquired", true);
            audioProbe("reopened-" + index, true);
            complete();
        }
        begin("temporary-audio-focus-loss-and-regain");
        android.media.AudioManager manager = (android.media.AudioManager) activity.getSystemService(Activity.AUDIO_SERVICE);
        android.media.AudioManager.OnAudioFocusChangeListener competing = change -> { };
        long handle = session();
        try {
            main(() -> require(manager.requestAudioFocus(competing, android.media.AudioManager.STREAM_MUSIC,
                    android.media.AudioManager.AUDIOFOCUS_GAIN_TRANSIENT) == android.media.AudioManager.AUDIOFOCUS_REQUEST_GRANTED,
                    "competing temporary focus request was denied"));
            waitCondition(() -> !playbackAudioFocus().hasFocus() && "paused".equals(status().optString("state")),
                    5000, "temporary focus loss did not pause native output");
            SystemClock.sleep(1500);
            audioProbe("focus-lost", false);
            main(() -> manager.abandonAudioFocus(competing));
            waitCondition(() -> playbackAudioFocus().hasFocus(), 5000, "released competing focus did not restore player focus");
            awaitProgress(CHANNELS[0][1], 3);
            require(session() == handle, "focus regain replaced the Surface session");
            audioProbe("focus-regained", true);
            currentStep.put("resumedWithoutChannelSelection", true).put("sameNativeSession", true);
            complete();
        } finally {
            main(() -> manager.abandonAudioFocus(competing));
        }
        begin("stale-focus-callback-after-release");
        java.lang.reflect.Field listenerField = PlaybackAudioFocus.class.getDeclaredField("listener");
        listenerField.setAccessible(true);
        android.media.AudioManager.OnAudioFocusChangeListener stale =
                (android.media.AudioManager.OnAudioFocusChangeListener) listenerField.get(playbackAudioFocus());
        main(() -> {
            playbackAudioFocus().abandon();
            stale.onAudioFocusChange(android.media.AudioManager.AUDIOFOCUS_GAIN);
        });
        waitForIdleSync();
        require(!playbackAudioFocus().hasFocus() && !playbackAudioFocus().hasRequest(), "stale callback revived abandoned audio focus");
        waitCondition(() -> "paused".equals(status().optString("state")), 3000, "stale callback resumed native output");
        main(() -> playbackAudioFocus().acquire());
        awaitProgress(CHANNELS[0][1], 2);
        complete();
        audioFocusTransitions(true);
    }

    private void audioFocusBasic() throws Exception {
        // Focus/API compatibility only: the API 19 emulator does not decode the live AVC profile.
        for (int index = 0; index < 3; index++) {
            begin("focus-only-reopen-" + index);
            waitCondition(() -> playbackAudioFocus().hasFocus(), 5000, "legacy startup did not acquire focus");
            main(() -> NativeBridge.pause(session()));
            complete();
            if (index < 2) {
                MainActivity old = activity;
                PlaybackAudioFocus focus = playbackAudioFocus();
                main(old::finish);
                waitCondition(old::isDestroyed, 10000, "legacy Activity did not finish");
                require(!focus.hasFocus() && !focus.hasRequest(), "legacy Activity retained focus after finish");
                launch();
            }
        }
        audioFocusTransitions(false);
    }

    @SuppressWarnings("deprecation")
    private void audioFocusTransitions(boolean live) throws Exception {
        begin("surface-recreation-while-waiting-for-audio-focus");
        android.media.AudioManager manager = (android.media.AudioManager) activity.getSystemService(Activity.AUDIO_SERVICE);
        android.media.AudioManager.OnAudioFocusChangeListener competing = change -> { };
        try {
            main(() -> require(manager.requestAudioFocus(competing, android.media.AudioManager.STREAM_MUSIC,
                    android.media.AudioManager.AUDIOFOCUS_GAIN_TRANSIENT) == android.media.AudioManager.AUDIOFOCUS_REQUEST_GRANTED,
                    "transient focus request was denied"));
            waitCondition(() -> !playbackAudioFocus().hasFocus(), 5000, "transient focus loss was not delivered");
            main(() -> playbackAudioFocus().acquire());
            require(!playbackAudioFocus().hasFocus() && playbackAudioFocus().hasRequest(),
                    "ordinary acquisition stole transient focus");
            long old = session();
            main(() -> viewField("surface").setVisibility(View.GONE));
            waitCondition(() -> session() == 0, 5000, "focus test Surface did not release its session");
            main(() -> viewField("surface").setVisibility(View.VISIBLE));
            waitCondition(() -> session() != 0 && session() != old && "paused".equals(status().optString("state")),
                    10000, "Surface recreation played before regaining focus");
            require(!playbackAudioFocus().hasFocus() && playbackAudioFocus().hasRequest(),
                    "automatic Surface selection stole transient focus");
            main(() -> manager.abandonAudioFocus(competing));
            waitCondition(() -> playbackAudioFocus().hasFocus(), 5000, "transient focus was not restored");
            if (live) awaitProgress(selectedPid(), 2);
            else main(() -> NativeBridge.pause(session()));
            complete();

            begin("permanent-audio-focus-loss-and-explicit-recovery");
            java.lang.reflect.Field listenerField = PlaybackAudioFocus.class.getDeclaredField("listener");
            listenerField.setAccessible(true);
            android.media.AudioManager.OnAudioFocusChangeListener stale =
                    (android.media.AudioManager.OnAudioFocusChangeListener) listenerField.get(playbackAudioFocus());
            main(() -> require(manager.requestAudioFocus(competing, android.media.AudioManager.STREAM_MUSIC,
                    android.media.AudioManager.AUDIOFOCUS_GAIN) == android.media.AudioManager.AUDIOFOCUS_REQUEST_GRANTED,
                    "permanent competing focus request was denied"));
            waitCondition(() -> !playbackAudioFocus().hasFocus() && !playbackAudioFocus().hasRequest()
                            && "paused".equals(status().optString("state")),
                    5000, "permanent focus loss did not clear its request and pause native output");
            main(() -> {
                manager.abandonAudioFocus(competing);
                stale.onAudioFocusChange(android.media.AudioManager.AUDIOFOCUS_GAIN);
            });
            waitForIdleSync();
            require(!playbackAudioFocus().hasFocus(), "old gain callback revived permanently lost focus");
            main(() -> playbackAudioFocus().acquireForPlayback());
            waitCondition(() -> playbackAudioFocus().hasFocus(), 5000, "explicit playback did not regain focus");
            if (live) awaitProgress(selectedPid(), 2);
            else main(() -> NativeBridge.pause(session()));
            complete();
        } finally {
            main(() -> manager.abandonAudioFocus(competing));
        }
    }

    private void lifecycle() throws Exception {
        begin("background-pause-and-resume");
        String pid = selectedPid();
        long beforeBackground = session();
        main(() -> activity.startActivity(new Intent(Intent.ACTION_MAIN).addCategory(Intent.CATEGORY_HOME)
                .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)));
        waitCondition(() -> !boolField("resumed"), 5000, "activity did not enter background");
        SystemClock.sleep(1500);
        long pausedHandle = session();
        if (pausedHandle != 0) {
            JSONObject before = status();
            require("paused".equals(before.optString("state")), "background renderer did not report paused");
            SystemClock.sleep(1500);
            JSONObject after = status();
            require(after.optLong("videoFrames") == before.optLong("videoFrames")
                    && after.optLong("audioFrames") == before.optLong("audioFrames"),
                    "background output counters continued advancing");
            snapshot(true);
            waitCondition(() -> nativeThreads().optInt("network") == 0, 15000,
                    "paused native network worker remained live");
            currentStep.put("backgroundNativeThreads", nativeThreads());
        } else {
            currentStep.put("backgroundSurfaceReleased", true);
            awaitReleased(beforeBackground);
        }
        main(() -> activity.startActivity(new Intent(Intent.ACTION_MAIN).setClassName(PACKAGE, PACKAGE + ".MainActivity")
                .addFlags(Intent.FLAG_ACTIVITY_REORDER_TO_FRONT | Intent.FLAG_ACTIVITY_SINGLE_TOP)));
        waitCondition(() -> boolField("resumed") && session() != 0, 15000, "activity did not resume with a surface");
        awaitProgress(pid, 3);
        complete();

        begin("surface-destroy-and-recreate");
        long old = session();
        main(() -> viewField("surface").setVisibility(View.GONE));
        waitCondition(() -> session() == 0, 10000, "Surface destruction did not clear the Java handle");
        awaitReleased(old);
        main(() -> viewField("surface").setVisibility(View.VISIBLE));
        waitCondition(() -> session() != 0 && session() != old, 15000, "Surface recreation did not create a new native handle");
        awaitProgress(pid, 3);
        complete();
    }

    private void soak() throws Exception {
        begin("continuous-native-live");
        int durationSeconds = boundedArgument("durationSeconds", 600, 10, 7200);
        String pid = selectedPid();
        JSONObject initial = awaitPlaying(pid);
        long targetPosition = initial.optLong("positionMs") + durationSeconds * 1000L;
        long initialVideo = initial.optLong("videoFrames"), initialAudio = initial.optLong("audioFrames");
        long started = SystemClock.elapsedRealtime();
        long deadline = started + durationSeconds * 1000L + 120000L;
        long lastPosition = initial.optLong("positionMs"), lastAdvance = SystemClock.elapsedRealtime();
        long video = initialVideo, audio = initialAudio, lastVideo = started, lastAudio = started;
        while (SystemClock.elapsedRealtime() < deadline) {
            JSONObject state = status();
            require(!"error".equals(state.optString("state")), "live playback reported an error during soak");
            require(pid.equals(state.optString("pid")), "soak switched to an unexpected channel");
            snapshot(false);
            long position = state.optLong("positionMs");
            if (position > lastPosition) { lastPosition = position; lastAdvance = SystemClock.elapsedRealtime(); }
            if (state.optLong("videoFrames") > video) { video = state.optLong("videoFrames"); lastVideo = SystemClock.elapsedRealtime(); }
            if (state.optLong("audioFrames") > audio) { audio = state.optLong("audioFrames"); lastAudio = SystemClock.elapsedRealtime(); }
            require(SystemClock.elapsedRealtime() - lastVideo < 15000 && SystemClock.elapsedRealtime() - lastAudio < 15000,
                    "one native media track stopped advancing during soak");
            require(SystemClock.elapsedRealtime() - lastAdvance < 30000, "media position stalled during soak");
            if (SystemClock.elapsedRealtime() - started >= durationSeconds * 1000L && position >= targetPosition && state.optLong("videoFrames") > initialVideo
                    && state.optLong("audioFrames") > initialAudio && "playing".equals(state.optString("state"))) {
                currentStep.put("elapsedWallMs", SystemClock.elapsedRealtime() - started).put("requestedSeconds", durationSeconds)
                        .put("mediaPositionAdvancedMs", position - initial.optLong("positionMs"))
                        .put("videoFramesAdvanced", state.optLong("videoFrames") - initialVideo)
                        .put("audioFramesAdvanced", state.optLong("audioFrames") - initialAudio)
                        .put("tenMinuteMediaGatePassed", durationSeconds >= 600 && position - initial.optLong("positionMs") >= 600000);
                snapshot(true); complete(); return;
            }
            SystemClock.sleep(1000);
        }
        throw new CheckFailure("soak did not reach the required advancing media position");
    }

    private void component() throws Exception {
        begin("generated-baseline-AVC-AAC-platform-output");
        main(() -> {
            NativeBridge.pause(session());
            Button full = findButton(activity.getWindow().getDecorView(), resourceString("fullscreen"));
            if (!boolField("fullscreen") && full != null) full.performClick();
        });
        waitCondition(() -> "paused".equals(status().optString("state")), 15000, "cannot pause live output before isolated component probe");
        System.load(new File(getContext().getApplicationInfo().nativeLibraryDir, "libatv_output_probe.so").getAbsolutePath());
        byte[] transport;
        try (InputStream input = getContext().getAssets().open("baseline-avc-aac.ts");
             ByteArrayOutputStream output = new ByteArrayOutputStream()) {
            byte[] buffer = new byte[4096]; int count;
            while ((count = input.read(buffer)) >= 0) output.write(buffer, 0, count);
            transport = output.toByteArray();
        }
        try {
            android.view.SurfaceHolder holder = ((android.view.SurfaceView) viewField("surface")).getHolder();
            JSONObject probe = new JSONObject(OutputProbe.run(holder.getSurface(), transport));
            currentStep.put("outputProbe", probe);
            require(!probe.has("failure"), "generated fixture failed native AV output");
            require(probe.optLong("videoFrames") >= 110 && probe.optLong("audioFrames") >= 198000
                    && probe.optLong("nonzeroPcmSamples") > 10000, "generated fixture did not produce advancing video and nonzero PCM");
            android.graphics.Bitmap screenshot = getUiAutomation().takeScreenshot();
            require(screenshot != null, "cannot capture generated video presentation");
            final Rect bounds = new Rect(); main(() -> viewField("surface").getGlobalVisibleRect(bounds));
            int colorful = 0;
            for (int y = Math.max(0, bounds.top); y < Math.min(screenshot.getHeight(), bounds.bottom); y += 8)
                for (int x = Math.max(0, bounds.left); x < Math.min(screenshot.getWidth(), bounds.right); x += 8) {
                    int pixel = screenshot.getPixel(x, y), r = android.graphics.Color.red(pixel),
                            g = android.graphics.Color.green(pixel), b = android.graphics.Color.blue(pixel);
                    if (Math.max(r, Math.max(g, b)) - Math.min(r, Math.min(g, b)) > 80) colorful++;
                }
            screenshot.recycle(); currentStep.put("colorfulSurfaceSamples", colorful);
            require(colorful > 20, "generated color pattern was not visible on the output Surface");
            complete();
        } finally { OutputProbe.close(); }
    }

    private void finishActivity() throws Exception {
        if (activity == null) return;
        begin("finish-and-native-resource-release");
        long old = session();
        main(() -> activity.finish());
        waitCondition(() -> activity.isDestroyed() && session() == 0, 10000, "activity finish did not release its surface handle");
        awaitReleased(old);
        complete();
        activity = null;
    }

    private void awaitReleased(long old) throws Exception {
        waitCondition(() -> (old == 0 || "idle".equals(new JSONObject(NativeBridge.status(old)).optString("state")))
                        && nativeThreads().optInt("render") == 0 && nativeThreads().optInt("network") == 0,
                15000, "destroyed native handle or native worker threads remained live");
        currentStep.put("releasedHandleState", old == 0 ? "already-released" : new JSONObject(NativeBridge.status(old)).optString("state"))
                .put("nativeThreadsAfterRelease", nativeThreads());
    }

    private void selectChannel(String[] channel) throws Exception {
        JSONObject previous = status();
        main(() -> {
            if (boolField("fullscreen")) throw new CheckFailure("channel UI selection requires sidebar visible");
            Button group = findButton(activity.getWindow().getDecorView(), resourceString("weishi".equals(channel[2]) ? "satellite" : "cctv"));
            require(group != null && group.performClick(), "cannot select channel group");
            View row = channelRows().findViewWithTag(channel[1]);
            require(row != null && row.performClick(), "cannot select requested channel row");
        });
        waitForIdleSync();
        long deadline = SystemClock.elapsedRealtime() + 5000;
        boolean acknowledged = false;
        do {
            JSONObject state = status();
            acknowledged = channel[1].equals(state.optString("pid"))
                    && (!channel[1].equals(previous.optString("pid"))
                    || "loading".equals(state.optString("state"))
                    || state.optLong("videoFrames") < previous.optLong("videoFrames")
                    || state.optLong("audioFrames") < previous.optLong("audioFrames")
                    || state.optLong("videoFrames") == 0);
            if (acknowledged) break;
            SystemClock.sleep(20);
        } while (SystemClock.elapsedRealtime() < deadline);
        require(acknowledged, "native renderer did not acknowledge channel selection");
    }

    private JSONObject awaitPlaying(String pid) throws Exception {
        final AtomicReference<JSONObject> ready = new AtomicReference<>();
        waitCondition(() -> {
            JSONObject state = status(); snapshot(false);
            require(!pid.equals(state.optString("pid")) || !"error".equals(state.optString("state")),
                    "native playback entered error while loading");
            boolean playing = pid.equals(state.optString("pid")) && "playing".equals(state.optString("state"))
                    && state.optLong("videoFrames") > 0 && state.optLong("audioFrames") > 0
                    && state.optInt("videoWidth") > 0 && state.optInt("videoHeight") > 0;
            if (playing) ready.set(state);
            return playing;
        }, channelTimeoutMs, "live channel did not produce native audio and video");
        return ready.get();
    }

    private void awaitProgress(String pid, int seconds) throws Exception {
        JSONObject first = awaitPlaying(pid);
        long until = SystemClock.elapsedRealtime() + seconds * 1000L;
        while (SystemClock.elapsedRealtime() < until) { snapshot(false); SystemClock.sleep(250); }
        JSONObject next = status();
        require(pid.equals(next.optString("pid")) && "playing".equals(next.optString("state")), "channel did not remain playing");
        require(next.optLong("videoFrames") > first.optLong("videoFrames")
                && next.optLong("audioFrames") > first.optLong("audioFrames")
                && next.optLong("positionMs") > first.optLong("positionMs"), "native audio/video counters or media time did not advance");
        snapshot(true);
    }

    private JSONObject status() throws Exception {
        long handle = session();
        return handle == 0 ? new JSONObject().put("state", "idle") : new JSONObject(NativeBridge.status(handle));
    }
    private long session() throws Exception { return ((Number) field("session")).longValue(); }
    private String selectedPid() throws Exception {
        Object selected = field("selected"); Field pid = selected.getClass().getDeclaredField("pid"); pid.setAccessible(true); return (String) pid.get(selected);
    }
    private ViewGroup channelRows() throws Exception { return (ViewGroup) field("channelRows"); }
    private View viewField(String name) throws Exception { return (View) field(name); }
    private boolean boolField(String name) throws Exception { return (Boolean) field(name); }
    private Object field(String name) throws Exception {
        AtomicReference<Object> value = new AtomicReference<>(); AtomicReference<Exception> error = new AtomicReference<>();
        Runnable read = () -> { try { Field f = MainActivity.class.getDeclaredField(name); f.setAccessible(true); value.set(f.get(activity)); } catch (Exception e) { error.set(e); } };
        if (android.os.Looper.myLooper() == android.os.Looper.getMainLooper()) read.run(); else runOnMainSync(read);
        if (error.get() != null) throw error.get(); return value.get();
    }
    private String resourceString(String name) { return getTargetContext().getString(getTargetContext().getResources().getIdentifier(name, "string", PACKAGE)); }
    private Button findButton(View view, String text) {
        if (view instanceof Button && text.contentEquals(((Button) view).getText())) return (Button) view;
        if (view instanceof ViewGroup) for (int i = 0; i < ((ViewGroup) view).getChildCount(); i++) {
            Button found = findButton(((ViewGroup) view).getChildAt(i), text); if (found != null) return found;
        }
        return null;
    }
    private JSONObject nativeThreads() throws Exception {
        int render = 0, network = 0; File[] tasks = new File("/proc/self/task").listFiles();
        require(tasks != null, "cannot inspect native worker threads");
        for (File task : tasks) {
            try (BufferedReader input = new BufferedReader(new InputStreamReader(new FileInputStream(new File(task, "comm")), StandardCharsets.UTF_8))) {
                String name = input.readLine(); if ("atv-render".equals(name)) render++; if ("atv-network".equals(name)) network++;
            } catch (java.io.FileNotFoundException ignored) { /* A thread can exit between listing and reading. */ }
        }
        return new JSONObject().put("render", render).put("network", network);
    }
    private void begin(String name) throws Exception {
        currentStep = new JSONObject().put("name", name).put("startedElapsedMs", SystemClock.elapsedRealtime());
        snapshots = new JSONArray(); currentStep.put("snapshots", snapshots); steps.put(currentStep); lastSnapshot = 0;
    }
    private void complete() throws Exception { snapshot(true); currentStep.put("passed", true).put("finishedElapsedMs", SystemClock.elapsedRealtime()); }
    private void snapshot(boolean force) throws Exception {
        long now = SystemClock.elapsedRealtime(); if (snapshots == null || (!force && now - lastSnapshot < 5000) || snapshots.length() >= 1500) return;
        JSONObject raw = status(), safe = new JSONObject().put("elapsedMs", now);
        for (String key : new String[]{"state","pid","positionMs","videoWidth","videoHeight","videoFrames","audioFrames","errorCode","reconnectAttempts"}) if (raw.has(key)) safe.put(key, raw.get(key));
        if (raw.has("buffers")) safe.put("buffers", raw.getJSONObject("buffers"));
        android.os.Debug.MemoryInfo memory = new android.os.Debug.MemoryInfo();
        android.os.Debug.getMemoryInfo(memory);
        Runtime runtime = Runtime.getRuntime();
        safe.put("processCpuMs", android.os.Process.getElapsedCpuTime())
                .put("totalPssKiB", memory.getTotalPss())
                .put("privateDirtyKiB", memory.getTotalPrivateDirty())
                .put("nativePssKiB", memory.nativePss)
                .put("dalvikPssKiB", memory.dalvikPss)
                .put("nativeHeapAllocatedBytes", android.os.Debug.getNativeHeapAllocatedSize())
                .put("javaHeapUsedBytes", runtime.totalMemory() - runtime.freeMemory());
        safe.put("nativeThreads", nativeThreads()); snapshots.put(safe); lastSnapshot = now;
    }
    private interface Operation { void run() throws Exception; }
    private interface Condition { boolean ready() throws Exception; }
    private void main(Operation action) throws Exception {
        AtomicReference<Exception> error = new AtomicReference<>(); runOnMainSync(() -> { try { action.run(); } catch (Exception e) { error.set(e); } });
        if (error.get() != null) throw error.get();
    }
    private void waitCondition(Condition condition, long timeoutMs, String failure) throws Exception {
        long deadline = SystemClock.elapsedRealtime() + timeoutMs;
        do { if (condition.ready()) return; SystemClock.sleep(250); } while (SystemClock.elapsedRealtime() < deadline);
        throw new CheckFailure(failure);
    }
    private static void require(boolean condition, String message) throws CheckFailure { if (!condition) throw new CheckFailure(message); }
    private static final class CheckFailure extends Exception { CheckFailure(String message) { super(message); } }
    private static String digestFile(String path) throws Exception {
        MessageDigest digest = MessageDigest.getInstance("SHA-256"); byte[] buffer = new byte[32768];
        try (FileInputStream input = new FileInputStream(path)) { int n; while ((n = input.read(buffer)) != -1) digest.update(buffer, 0, n); }
        StringBuilder result = new StringBuilder(); for (byte b : digest.digest()) result.append(String.format(java.util.Locale.ROOT, "%02x", b & 255)); return result.toString();
    }
}
