package cn.bobo.atv.nativeplayer;

import android.annotation.SuppressLint;
import android.app.Activity;
import android.graphics.Color;
import android.media.AudioManager;
import android.os.Build;
import android.os.Bundle;
import android.os.Handler;
import android.os.Looper;
import android.os.SystemClock;
import android.util.Log;
import android.view.Gravity;
import android.view.KeyEvent;
import android.view.SurfaceHolder;
import android.view.SurfaceView;
import android.view.View;
import android.view.ViewGroup;
import android.view.WindowInsets;
import android.view.WindowInsetsController;
import android.widget.Button;
import android.widget.FrameLayout;
import android.widget.LinearLayout;
import android.widget.ScrollView;
import android.widget.TextView;
import android.window.OnBackInvokedCallback;
import android.window.OnBackInvokedDispatcher;

import org.json.JSONArray;
import org.json.JSONObject;

import java.io.ByteArrayOutputStream;
import java.io.InputStream;
import java.nio.charset.StandardCharsets;
import java.util.ArrayList;
import java.util.List;

/** Native Android widgets and Surface lifecycle; all playback work is owned by Rust. */
public final class MainActivity extends Activity implements SurfaceHolder.Callback {
    private final Handler handler = new Handler(Looper.getMainLooper());
    private final List<Channel> channels = new ArrayList<>();
    private LinearLayout sidebar, channelRows, topbar;
    private FrameLayout playerArea;
    private SurfaceView surface;
    private TextView title, status, hint;
    private Button retry;
    private Channel selected;
    private String group = "yangshi";
    private long session;
    private boolean resumed, fullscreen;
    private OnBackInvokedCallback backCallback;
    private int videoWidth, videoHeight;
    private long lastStatusLog;

    private final Runnable poll = new Runnable() {
        @Override public void run() {
            if (session != 0) {
                try { showStatus(new JSONObject(NativeBridge.status(session))); }
                catch (RuntimeException | LinkageError | org.json.JSONException error) {
                    showError(getString(R.string.native_failure));
                }
            }
            if (resumed) handler.postDelayed(this, 500);
        }
    };

    @Override protected void onCreate(Bundle state) {
        super.onCreate(state);
        LinearLayout root = new LinearLayout(this);
        root.setOrientation(LinearLayout.VERTICAL);
        root.setBackgroundColor(Color.rgb(12, 13, 16));
        root.setPadding(dp(20), dp(12), dp(20), dp(12));
        setContentView(root);

        topbar = new LinearLayout(this);
        topbar.setGravity(Gravity.CENTER_VERTICAL);
        title = text("ATV · 电视直播", 22);
        topbar.addView(title, new LinearLayout.LayoutParams(0, dp(48), 1));
        Button full = button(getString(R.string.fullscreen));
        full.setId(View.generateViewId());
        full.setOnClickListener(view -> setFullscreen(!fullscreen));
        topbar.addView(full);
        root.addView(topbar);

        LinearLayout content = new LinearLayout(this);
        root.addView(content, new LinearLayout.LayoutParams(-1, 0, 1));
        sidebar = new LinearLayout(this);
        sidebar.setOrientation(LinearLayout.VERTICAL);
        sidebar.setPadding(0, dp(8), dp(16), 0);
        content.addView(sidebar, new LinearLayout.LayoutParams(dp(220), -1));
        LinearLayout groups = new LinearLayout(this);
        Button cctv = button(getString(R.string.cctv));
        Button satellite = button(getString(R.string.satellite));
        cctv.setOnClickListener(view -> setGroup("yangshi"));
        satellite.setOnClickListener(view -> setGroup("weishi"));
        groups.addView(cctv, new LinearLayout.LayoutParams(0, dp(48), 1));
        groups.addView(satellite, new LinearLayout.LayoutParams(0, dp(48), 1));
        sidebar.addView(groups);
        ScrollView scroll = new ScrollView(this);
        channelRows = new LinearLayout(this);
        channelRows.setOrientation(LinearLayout.VERTICAL);
        scroll.addView(channelRows);
        sidebar.addView(scroll, new LinearLayout.LayoutParams(-1, 0, 1));

        playerArea = new FrameLayout(this);
        playerArea.setBackgroundColor(Color.BLACK);
        content.addView(playerArea, new LinearLayout.LayoutParams(0, -1, 1));
        surface = new SurfaceView(this);
        surface.getHolder().addCallback(this);
        playerArea.addView(surface, new FrameLayout.LayoutParams(-1, -1, Gravity.CENTER));
        playerArea.addOnLayoutChangeListener((v,l,t,r,b,ol,ot,or,ob) -> fitSurface());
        LinearLayout message = new LinearLayout(this);
        message.setOrientation(LinearLayout.VERTICAL);
        message.setGravity(Gravity.CENTER);
        status = text(getString(R.string.no_surface), 20);
        status.setGravity(Gravity.CENTER);
        message.addView(status);
        retry = button(getString(R.string.retry));
        retry.setVisibility(View.GONE);
        retry.setOnClickListener(view -> select(selected));
        message.addView(retry);
        playerArea.addView(message, new FrameLayout.LayoutParams(-1, -2, Gravity.CENTER));
        hint = text(getString(R.string.remote_hint), 12);
        hint.setTextColor(Color.LTGRAY);
        hint.setGravity(Gravity.CENTER);
        root.addView(hint, new LinearLayout.LayoutParams(-1, dp(28)));

        loadChannels();
        String remembered = state == null
                ? getPreferences(MODE_PRIVATE).getString("pid", "") : state.getString("pid", "");
        for (Channel channel : channels) if (channel.pid.equals(remembered)) selected = channel;
        if (selected == null && !channels.isEmpty()) selected = channels.get(0);
        if (selected != null) { group = selected.group; title.setText(selected.name); }
        setGroup(group);
        if (!NativeBridge.available()) showError(getString(R.string.native_unavailable));
        if (Build.VERSION.SDK_INT >= 33) {
            backCallback = this::handleBack;
            getOnBackInvokedDispatcher().registerOnBackInvokedCallback(
                    OnBackInvokedDispatcher.PRIORITY_DEFAULT, backCallback);
        }
        hideSystemBars();
    }

    private void loadChannels() {
        try (InputStream input = getAssets().open("channels.json");
             ByteArrayOutputStream bytes = new ByteArrayOutputStream()) {
            byte[] buffer = new byte[4096]; int length;
            while ((length = input.read(buffer)) != -1) bytes.write(buffer, 0, length);
            JSONArray array = new JSONArray(new String(bytes.toByteArray(), StandardCharsets.UTF_8));
            for (int i = 0; i < array.length(); i++) {
                JSONObject json = array.getJSONObject(i);
                channels.add(new Channel(json.getString("pid"), json.getString("streamId"),
                        json.getString("channelName"), json.getString("channelType")));
            }
        } catch (Exception error) { showError(getString(R.string.directory_failure)); }
    }

    private void setGroup(String next) {
        group = next; channelRows.removeAllViews();
        for (Channel channel : channels) {
            if (!channel.group.equals(group)) continue;
            Button row = button(channel.name);
            row.setTag(channel.pid);
            row.setAllCaps(false);
            row.setOnClickListener(view -> select(channel));
            channelRows.addView(row, new LinearLayout.LayoutParams(-1, dp(48)));
        }
        focusSelected();
    }

    private void focusSelected() {
        if (selected == null) return;
        View row = channelRows.findViewWithTag(selected.pid);
        if (row != null) row.requestFocus();
    }

    private void select(Channel channel) {
        if (channel == null) return;
        selected = channel; title.setText(channel.name);
        getPreferences(MODE_PRIVATE).edit().putString("pid", channel.pid).apply();
        videoWidth = videoHeight = 0;
        if (!NativeBridge.available()) { showError(getString(R.string.native_unavailable)); return; }
        if (session == 0) { status.setText(R.string.no_surface); return; }
        status.setVisibility(View.VISIBLE); status.setText(R.string.loading); retry.setVisibility(View.GONE);
        try { NativeBridge.select(session, channel.pid, channel.streamId); }
        catch (RuntimeException | LinkageError error) { showError(getString(R.string.native_failure)); }
    }

    @Override public void surfaceCreated(SurfaceHolder holder) {
        if (!NativeBridge.available()) return;
        try {
            AudioManager audio = (AudioManager) getSystemService(AUDIO_SERVICE);
            int sampleRate = 48000;
            try { sampleRate = Integer.parseInt(audio.getProperty(AudioManager.PROPERTY_OUTPUT_SAMPLE_RATE)); }
            catch (RuntimeException ignored) { }
            session = NativeBridge.create(holder.getSurface(), sampleRate, getFilesDir().getAbsolutePath());
            if (session == 0) { showError(getString(R.string.native_failure)); return; }
            if (!resumed) NativeBridge.pause(session);
            select(selected);
        } catch (RuntimeException | LinkageError error) { showError(getString(R.string.native_failure)); }
    }

    @Override public void surfaceChanged(SurfaceHolder holder, int format, int width, int height) { }
    @Override public void surfaceDestroyed(SurfaceHolder holder) { releaseSession(); }

    private void showStatus(JSONObject state) {
        if (BuildConfig.DEBUG && SystemClock.elapsedRealtime() - lastStatusLog >= 5000) {
            lastStatusLog = SystemClock.elapsedRealtime();
            Log.i("ATVNative", state.toString());
        }
        videoWidth = state.optInt("videoWidth", 0); videoHeight = state.optInt("videoHeight", 0);
        fitSurface();
        String phase = state.optString("state", "idle");
        boolean playing = "playing".equals(phase) && videoWidth > 0 && videoHeight > 0;
        if (playing) {
            status.setVisibility(View.GONE); retry.setVisibility(View.GONE);
        } else if ("error".equals(phase)) {
            showError(state.optString("message", getString(R.string.native_failure)));
        } else {
            status.setVisibility(View.VISIBLE);
            status.setText("paused".equals(phase) ? R.string.paused : R.string.loading);
            retry.setVisibility(View.GONE);
        }
    }

    private void showError(String message) {
        status.setVisibility(View.VISIBLE);
        status.setText(message.isEmpty() ? getString(R.string.native_failure) : message);
        retry.setVisibility(View.VISIBLE);
    }

    private void fitSurface() {
        if (videoWidth <= 0 || videoHeight <= 0 || playerArea.getWidth() == 0 || playerArea.getHeight() == 0) return;
        float scale = Math.min((float) playerArea.getWidth() / videoWidth, (float) playerArea.getHeight() / videoHeight);
        int width = Math.round(videoWidth * scale), height = Math.round(videoHeight * scale);
        ViewGroup.LayoutParams current = surface.getLayoutParams();
        if (current.width != width || current.height != height)
            surface.setLayoutParams(new FrameLayout.LayoutParams(width, height, Gravity.CENTER));
    }

    private void setFullscreen(boolean value) {
        fullscreen = value;
        sidebar.setVisibility(value ? View.GONE : View.VISIBLE);
        topbar.setVisibility(value ? View.GONE : View.VISIBLE);
        hint.setVisibility(value ? View.GONE : View.VISIBLE);
        if (!value) focusSelected();
        hideSystemBars();
    }

    @Override public boolean onKeyDown(int keyCode, KeyEvent event) {
        if (fullscreen && (keyCode == KeyEvent.KEYCODE_DPAD_UP || keyCode == KeyEvent.KEYCODE_DPAD_DOWN)) {
            List<Channel> list = new ArrayList<>();
            for (Channel channel : channels) if (selected != null && channel.group.equals(selected.group)) list.add(channel);
            int index = list.indexOf(selected);
            if (index >= 0) select(list.get((index + (keyCode == KeyEvent.KEYCODE_DPAD_DOWN ? 1 : list.size() - 1)) % list.size()));
            return true;
        }
        if (fullscreen && (keyCode == KeyEvent.KEYCODE_DPAD_CENTER || keyCode == KeyEvent.KEYCODE_DPAD_LEFT)) {
            setFullscreen(false); return true;
        }
        return super.onKeyDown(keyCode, event);
    }

    private void handleBack() { if (fullscreen) setFullscreen(false); else finish(); }
    @SuppressLint("GestureBackNavigation") // API 33+ uses the native dispatcher above.
    @SuppressWarnings("deprecation")
    @Override public void onBackPressed() { handleBack(); }

    @Override protected void onResume() {
        super.onResume(); resumed = true;
        if (session != 0) try { NativeBridge.resume(session); }
        catch (RuntimeException | LinkageError error) { showError(getString(R.string.native_failure)); }
        handler.post(poll); hideSystemBars();
    }
    @Override protected void onPause() {
        resumed = false; handler.removeCallbacks(poll);
        if (session != 0) try { NativeBridge.pause(session); }
        catch (RuntimeException | LinkageError error) { showError(getString(R.string.native_failure)); }
        super.onPause();
    }
    @Override protected void onSaveInstanceState(Bundle state) {
        if (selected != null) state.putString("pid", selected.pid);
        super.onSaveInstanceState(state);
    }
    @Override protected void onDestroy() {
        handler.removeCallbacks(poll); releaseSession();
        if (Build.VERSION.SDK_INT >= 33 && backCallback != null)
            getOnBackInvokedDispatcher().unregisterOnBackInvokedCallback(backCallback);
        super.onDestroy();
    }
    private void releaseSession() {
        long old = session; session = 0;
        if (old != 0) try { NativeBridge.destroy(old); } catch (RuntimeException | LinkageError ignored) { }
    }

    @SuppressWarnings("deprecation") private void hideSystemBars() {
        if (Build.VERSION.SDK_INT >= 30) {
            getWindow().setDecorFitsSystemWindows(false);
            WindowInsetsController controller = getWindow().getInsetsController();
            if (controller != null) {
                controller.hide(WindowInsets.Type.systemBars());
                controller.setSystemBarsBehavior(WindowInsetsController.BEHAVIOR_SHOW_TRANSIENT_BARS_BY_SWIPE);
            }
        } else getWindow().getDecorView().setSystemUiVisibility(View.SYSTEM_UI_FLAG_FULLSCREEN
                | View.SYSTEM_UI_FLAG_HIDE_NAVIGATION | View.SYSTEM_UI_FLAG_IMMERSIVE_STICKY);
    }
    private int dp(int value) { return Math.round(value * getResources().getDisplayMetrics().density); }
    private TextView text(String value, int size) {
        TextView view = new TextView(this); view.setText(value); view.setTextSize(size); view.setTextColor(Color.WHITE); return view;
    }
    private Button button(String value) { Button view = new Button(this); view.setText(value); return view; }

    private static final class Channel {
        final String pid, streamId, name, group;
        Channel(String pid, String streamId, String name, String group) {
            this.pid = pid; this.streamId = streamId; this.name = name; this.group = group;
        }
    }
}
