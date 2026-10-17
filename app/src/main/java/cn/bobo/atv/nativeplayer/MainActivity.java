package cn.bobo.atv.nativeplayer;

import android.annotation.SuppressLint;
import android.app.Activity;
import android.app.AlertDialog;
import android.app.Dialog;
import android.graphics.Color;
import android.graphics.PorterDuff;
import android.graphics.drawable.ColorDrawable;
import android.graphics.drawable.StateListDrawable;
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
import android.widget.ProgressBar;
import android.widget.ScrollView;
import android.widget.TextView;
import android.widget.Toast;
import android.window.OnBackInvokedCallback;
import android.window.OnBackInvokedDispatcher;

import org.json.JSONArray;
import org.json.JSONObject;

import java.io.ByteArrayOutputStream;
import java.io.InputStream;
import java.io.File;
import java.nio.charset.StandardCharsets;
import java.util.ArrayList;
import java.util.List;

/** Native Android widgets and Surface lifecycle; all playback work is owned by Rust. */
public final class MainActivity extends Activity implements SurfaceHolder.Callback {
    private final Handler handler = new Handler(Looper.getMainLooper());
    private final List<Channel> channels = new ArrayList<>();
    private LinearLayout sidebar, channelRows, topbar;
    private ScrollView channelScroll;
    private Button cctvGroup, satelliteGroup;
    private boolean pendingChannelFocus;
    private FrameLayout playerArea;
    private SurfaceView surface;
    private TextView title, hint;
    private ProgressBar loading;
    private Button retry;
    private long renderedVideoFrames, lastVideoProgressAt;
    private String playbackError = "";
    private Channel selected;
    private String group = "yangshi";
    private long session;
    private boolean resumed, fullscreen;
    private OnBackInvokedCallback backCallback;
    private int videoWidth, videoHeight;
    private long lastStatusLog;
    private Button settingsButton, qualityButton, updateCheck, updateDownload, updateInstall, updateCancel;
    private TextView qualitySummary, updateSummary;
    private Dialog settingsDialog;
    private AlertDialog qualityDialog, messageDialog;
    private JSONObject lastPlayback = new JSONObject(), lastUpdate = new JSONObject();
    private String preferredQuality = "auto";
    private long updater;
    private File updateDirectory;
    private boolean installing, automaticCheckStarted;
    private final Runnable automaticCheck = () -> {
        if (updater != 0 && resumed && !automaticCheckStarted) {
            automaticCheckStarted = true;
            NativeBridge.updateCommand(updater, 3);
        }
    };

    private final Runnable poll = new Runnable() {
        @Override public void run() {
            if (session != 0) {
                try { showStatus(new JSONObject(NativeBridge.status(session))); }
                catch (RuntimeException | LinkageError | org.json.JSONException error) {
                    showError(getString(R.string.native_failure));
                }
            }
            if (updater != 0) {
                try { lastUpdate = new JSONObject(NativeBridge.updateStatus(updater)); refreshSettings(); }
                catch (RuntimeException | LinkageError | org.json.JSONException error) { }
            }
            if (resumed) handler.postDelayed(this, 500);
        }
    };

    @Override protected void onCreate(Bundle state) {
        super.onCreate(state);
        FrameLayout root = new FrameLayout(this);
        root.setBackgroundColor(Color.BLACK);
        setContentView(root);

        // Keep the video in one full-window container. Menus are sibling
        // overlays, so showing them never resizes or replaces the Surface.
        playerArea = new FrameLayout(this);
        playerArea.setBackgroundColor(Color.BLACK);
        root.addView(playerArea, new FrameLayout.LayoutParams(-1, -1));
        surface = new SurfaceView(this);
        surface.getHolder().addCallback(this);
        surface.setOnClickListener(view -> setFullscreen(false));
        surface.setFocusable(false);
        playerArea.addView(surface, new FrameLayout.LayoutParams(-1, -1, Gravity.CENTER));
        playerArea.addOnLayoutChangeListener((v,l,t,r,b,ol,ot,or,ob) -> fitSurface());
        loading = new ProgressBar(this);
        loading.setIndeterminate(true);
        loading.setFocusable(false);
        loading.setContentDescription(getString(R.string.loading));
        loading.getIndeterminateDrawable().mutate().setColorFilter(Color.rgb(104, 224, 191), PorterDuff.Mode.SRC_IN);
        playerArea.addView(loading, new FrameLayout.LayoutParams(dp(40), dp(40), Gravity.CENTER));
        retry = button(getString(R.string.retry));
        retry.setVisibility(View.GONE);
        retry.setOnClickListener(view -> select(selected));
        FrameLayout.LayoutParams retryPosition = new FrameLayout.LayoutParams(dp(144), dp(48), Gravity.BOTTOM | Gravity.CENTER_HORIZONTAL);
        retryPosition.bottomMargin = dp(52);
        playerArea.addView(retry, retryPosition);

        topbar = new LinearLayout(this);
        topbar.setGravity(Gravity.CENTER_VERTICAL);
        topbar.setPadding(dp(20), dp(4), dp(20), dp(4));
        topbar.setBackgroundColor(Color.argb(180, 12, 13, 16));
        title = text("ATV · 电视直播", 22);
        topbar.addView(title, new LinearLayout.LayoutParams(0, dp(48), 1));
        settingsButton = button(getString(R.string.settings));
        settingsButton.setOnClickListener(view -> openSettings());
        topbar.addView(settingsButton);
        Button full = button(getString(R.string.fullscreen));
        full.setId(View.generateViewId());
        full.setOnClickListener(view -> setFullscreen(!fullscreen));
        topbar.addView(full);
        root.addView(topbar, new FrameLayout.LayoutParams(-1, dp(56), Gravity.TOP));

        sidebar = new LinearLayout(this);
        sidebar.setOrientation(LinearLayout.VERTICAL);
        sidebar.setPadding(dp(12), dp(8), dp(12), dp(8));
        sidebar.setBackgroundColor(Color.argb(190, 12, 13, 16));
        FrameLayout.LayoutParams channelPanel = new FrameLayout.LayoutParams(
                Math.min(dp(280), getResources().getDisplayMetrics().widthPixels / 2), -1, Gravity.LEFT);
        channelPanel.setMargins(dp(16), dp(68), 0, dp(48));
        root.addView(sidebar, channelPanel);
        LinearLayout groups = new LinearLayout(this);
        cctvGroup = button(getString(R.string.cctv));
        satelliteGroup = button(getString(R.string.satellite));
        cctvGroup.setOnClickListener(view -> setGroup("yangshi"));
        satelliteGroup.setOnClickListener(view -> setGroup("weishi"));
        // Group headings remain tappable, but remote focus belongs to channels.
        cctvGroup.setFocusable(false);
        satelliteGroup.setFocusable(false);
        groups.addView(cctvGroup, new LinearLayout.LayoutParams(0, dp(48), 1));
        groups.addView(satelliteGroup, new LinearLayout.LayoutParams(0, dp(48), 1));
        sidebar.addView(groups);
        channelScroll = new ScrollView(this);
        channelRows = new LinearLayout(this);
        channelRows.setOrientation(LinearLayout.VERTICAL);
        channelRows.addOnLayoutChangeListener((v,l,t,r,b,ol,ot,or,ob) -> {
            if (pendingChannelFocus) {
                pendingChannelFocus = false;
                focusGroup();
            }
        });
        channelScroll.addView(channelRows);
        sidebar.addView(channelScroll, new LinearLayout.LayoutParams(-1, 0, 1));

        hint = text(getString(R.string.remote_hint), 12);
        hint.setTextColor(Color.LTGRAY);
        hint.setGravity(Gravity.CENTER);
        hint.setBackgroundColor(Color.argb(180, 12, 13, 16));
        root.addView(hint, new FrameLayout.LayoutParams(-1, dp(36), Gravity.BOTTOM));

        loadChannels();
        preferredQuality = getPreferences(MODE_PRIVATE).getString("quality", "auto");
        if (!java.util.Arrays.asList("auto", "sd", "hd", "shd", "fhd").contains(preferredQuality)) preferredQuality = "auto";
        if (NativeBridge.available()) {
            File privateUpdates = new File(getFilesDir(), "updates");
            updateDirectory = Build.VERSION.SDK_INT < 24 ? getExternalFilesDir("updates") : privateUpdates;
            if (updateDirectory == null) updateDirectory = getDir("update_staging", MODE_PRIVATE);
            try { updater = NativeBridge.updateCreate(privateUpdates.getAbsolutePath(), updateDirectory.getAbsolutePath(),
                    BuildConfig.VERSION_CODE, Build.VERSION.SDK_INT); }
            catch (RuntimeException | LinkageError error) { updater = 0; }
        }
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
        group = next;
        cctvGroup.setTextColor("yangshi".equals(group) ? Color.rgb(104, 224, 191) : Color.WHITE);
        satelliteGroup.setTextColor("weishi".equals(group) ? Color.rgb(104, 224, 191) : Color.WHITE);
        channelRows.removeAllViews();
        for (Channel channel : channels) {
            if (!channel.group.equals(group)) continue;
            Button row = button(channel.name);
            row.setTag(channel.pid);
            row.setAllCaps(false);
            row.setOnClickListener(view -> select(channel));
            channelRows.addView(row, new LinearLayout.LayoutParams(-1, dp(48)));
        }
        channelScroll.scrollTo(0, 0);
        // Row and viewport heights must be measured before choosing the first page's middle.
        pendingChannelFocus = true;
        channelRows.requestLayout();
    }

    private void focusGroup() {
        int count = channelRows.getChildCount();
        if (count == 0 || fullscreen) return;
        View playing = selected == null ? null : channelRows.findViewWithTag(selected.pid);
        if (playing != null) {
            playing.requestFocus();
            channelScroll.scrollTo(0, Math.max(0, playing.getTop()
                    - (channelScroll.getHeight() - playing.getHeight()) / 2));
        } else {
            int rowHeight = Math.max(1, channelRows.getChildAt(0).getHeight());
            int visibleRows = Math.min(count, Math.max(1, channelScroll.getHeight() / rowHeight));
            channelRows.getChildAt(visibleRows / 2).requestFocus();
            channelScroll.scrollTo(0, 0);
        }
    }

    private void focusSelected() {
        if (selected == null) return;
        if (!selected.group.equals(group)) setGroup(selected.group);
        else if (channelRows.isLayoutRequested()) pendingChannelFocus = true;
        else focusGroup();
    }

    private void select(Channel channel) {
        if (channel == null) return;
        selected = channel; title.setText(channel.name);
        getPreferences(MODE_PRIVATE).edit().putString("pid", channel.pid).apply();
        videoWidth = videoHeight = 0;
        renderedVideoFrames = lastVideoProgressAt = 0;
        playbackError = "";
        loading.setVisibility(View.VISIBLE);
        retry.setVisibility(View.GONE);
        if (!NativeBridge.available()) { showError(getString(R.string.native_unavailable)); return; }
        if (session == 0) return;
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
            NativeBridge.quality(session, preferredQuality);
            select(selected);
        } catch (RuntimeException | LinkageError error) { showError(getString(R.string.native_failure)); }
    }

    @Override public void surfaceChanged(SurfaceHolder holder, int format, int width, int height) { }
    @Override public void surfaceDestroyed(SurfaceHolder holder) { releaseSession(); }

    private void showStatus(JSONObject state) {
        lastPlayback = state;
        if (BuildConfig.DEBUG && SystemClock.elapsedRealtime() - lastStatusLog >= 5000) {
            lastStatusLog = SystemClock.elapsedRealtime();
            Log.i("ATVNative", state.toString());
        }
        videoWidth = state.optInt("videoWidth", 0); videoHeight = state.optInt("videoHeight", 0);
        fitSurface();
        long now = SystemClock.elapsedRealtime();
        long frames = state.optLong("videoFrames", 0);
        if (frames < renderedVideoFrames) lastVideoProgressAt = 0;
        if (frames > renderedVideoFrames && videoWidth > 0 && videoHeight > 0) lastVideoProgressAt = now;
        renderedVideoFrames = frames;
        String phase = state.optString("state", "idle");
        if ("error".equals(phase)) {
            showError(state.optString("message", getString(R.string.native_failure)));
        } else {
            playbackError = state.optString("errorCode").isEmpty() ? "" : state.optString("message");
            // Network/audio state can briefly say loading while decoded video keeps advancing.
            boolean recentVideo = lastVideoProgressAt > 0 && now - lastVideoProgressAt < 2500;
            boolean playing = "playing".equals(phase) && videoWidth > 0 && videoHeight > 0;
            loading.setVisibility(!"paused".equals(phase) && !playing && !recentVideo ? View.VISIBLE : View.GONE);
            retry.setVisibility(View.GONE);
            refreshSettings();
        }
    }

    private void showError(String message) {
        playbackError = message.isEmpty() ? getString(R.string.native_failure) : message;
        loading.setVisibility(View.GONE);
        retry.setContentDescription(getString(R.string.retry) + "：" + playbackError);
        retry.setVisibility(View.VISIBLE);
        refreshSettings();
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
        setFullscreen(value, true);
    }

    private void setFullscreen(boolean value, boolean restorePlayingChannel) {
        fullscreen = value;
        sidebar.setVisibility(value ? View.GONE : View.VISIBLE);
        topbar.setVisibility(value ? View.GONE : View.VISIBLE);
        hint.setVisibility(value ? View.GONE : View.VISIBLE);
        if (!value && restorePlayingChannel) focusSelected();
        hideSystemBars();
    }

    @Override public boolean onKeyDown(int keyCode, KeyEvent event) {
        if (keyCode == KeyEvent.KEYCODE_MENU || keyCode == KeyEvent.KEYCODE_SETTINGS
                || (fullscreen && keyCode == KeyEvent.KEYCODE_DPAD_RIGHT)) {
            if (event.getRepeatCount() == 0) openSettings();
            return true;
        }
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

    @Override public boolean dispatchKeyEvent(KeyEvent event) {
        int key = event.getKeyCode();
        if (!fullscreen && channelRows.hasFocus()
                && (key == KeyEvent.KEYCODE_DPAD_LEFT || key == KeyEvent.KEYCODE_DPAD_RIGHT)) {
            if (event.getAction() == KeyEvent.ACTION_DOWN && event.getRepeatCount() == 0) {
                if (key == KeyEvent.KEYCODE_DPAD_RIGHT) {
                    if ("yangshi".equals(group)) setGroup("weishi");
                    else if ("weishi".equals(group)) openSettings();
                } else if ("weishi".equals(group)) setGroup("yangshi");
            }
            return true;
        }
        if (fullscreen && (key == KeyEvent.KEYCODE_DPAD_UP || key == KeyEvent.KEYCODE_DPAD_DOWN
                || key == KeyEvent.KEYCODE_DPAD_LEFT || key == KeyEvent.KEYCODE_DPAD_RIGHT
                || key == KeyEvent.KEYCODE_DPAD_CENTER)) {
            if (event.getAction() == KeyEvent.ACTION_DOWN) onKeyDown(key, event);
            return true;
        }
        return super.dispatchKeyEvent(event);
    }

    private void handleBack() {
        if (settingsDialog != null && settingsDialog.isShowing()) settingsDialog.dismiss();
        else if (!fullscreen) setFullscreen(true); else finish();
    }
    @SuppressLint("GestureBackNavigation") // API 33+ uses the native dispatcher above.
    @SuppressWarnings("deprecation")
    @Override public void onBackPressed() { handleBack(); }

    @Override protected void onResume() {
        super.onResume(); resumed = true;
        if (session != 0) try { NativeBridge.resume(session); }
        catch (RuntimeException | LinkageError error) { showError(getString(R.string.native_failure)); }
        handler.post(poll); hideSystemBars();
        if (!automaticCheckStarted) handler.postDelayed(automaticCheck, 10000);
    }
    @Override protected void onPause() {
        resumed = false; handler.removeCallbacks(poll);
        handler.removeCallbacks(automaticCheck);
        if (session != 0) try { NativeBridge.pause(session); }
        catch (RuntimeException | LinkageError error) { showError(getString(R.string.native_failure)); }
        super.onPause();
    }
    @Override protected void onSaveInstanceState(Bundle state) {
        if (selected != null) state.putString("pid", selected.pid);
        super.onSaveInstanceState(state);
    }
    @Override protected void onDestroy() {
        handler.removeCallbacks(automaticCheck);
        if (qualityDialog != null) qualityDialog.dismiss();
        if (messageDialog != null) messageDialog.dismiss();
        if (settingsDialog != null) settingsDialog.dismiss();
        long oldUpdater = updater; updater = 0;
        if (oldUpdater != 0) try { NativeBridge.updateDestroy(oldUpdater); } catch (RuntimeException | LinkageError ignored) { }
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
        } else getWindow().getDecorView().setSystemUiVisibility(View.SYSTEM_UI_FLAG_LAYOUT_STABLE
                | View.SYSTEM_UI_FLAG_LAYOUT_FULLSCREEN | View.SYSTEM_UI_FLAG_LAYOUT_HIDE_NAVIGATION | View.SYSTEM_UI_FLAG_FULLSCREEN
                | View.SYSTEM_UI_FLAG_HIDE_NAVIGATION | View.SYSTEM_UI_FLAG_IMMERSIVE_STICKY);
    }
    private int dp(int value) { return Math.round(value * getResources().getDisplayMetrics().density); }
    private TextView text(String value, int size) {
        TextView view = new TextView(this); view.setText(value); view.setTextSize(size); view.setTextColor(Color.WHITE); return view;
    }
    private Button button(String value) {
        Button view = new Button(this);
        view.setText(value);
        view.setTextColor(new android.content.res.ColorStateList(new int[][] {
                new int[]{-android.R.attr.state_enabled}, new int[]{}
        }, new int[]{Color.GRAY, Color.WHITE}));
        StateListDrawable background = new StateListDrawable();
        background.addState(new int[]{android.R.attr.state_pressed}, new ColorDrawable(Color.argb(230, 42, 111, 161)));
        background.addState(new int[]{android.R.attr.state_focused}, new ColorDrawable(Color.argb(220, 42, 111, 161)));
        background.addState(new int[]{}, new ColorDrawable(Color.argb(45, 255, 255, 255)));
        view.setBackground(background);
        view.setPadding(dp(12), dp(4), dp(12), dp(4));
        return view;
    }

    private void styleOverlayWindow(Dialog dialog, boolean customPanel) {
        android.view.Window window = dialog.getWindow();
        if (window == null) return;
        window.setBackgroundDrawable(new ColorDrawable(customPanel ? Color.TRANSPARENT : Color.argb(225, 20, 22, 28)));
        window.clearFlags(android.view.WindowManager.LayoutParams.FLAG_DIM_BEHIND);
        window.setGravity(Gravity.RIGHT | Gravity.CENTER_VERTICAL);
        android.view.WindowManager.LayoutParams attributes = window.getAttributes();
        attributes.x = dp(16);
        window.setAttributes(attributes);
        android.util.DisplayMetrics display = getResources().getDisplayMetrics();
        window.setLayout(Math.min(display.widthPixels - dp(32), dp(420)),
                customPanel ? Math.min(display.heightPixels - dp(32), dp(520)) : -2);
        window.getDecorView().setSystemUiVisibility(getWindow().getDecorView().getSystemUiVisibility());
    }

    private void openSettings() {
        if (settingsDialog != null && settingsDialog.isShowing()) return;
        final View previousFocus = getCurrentFocus();
        final boolean restoreChannels = !fullscreen;
        final boolean[] returnToSatellites = {false};
        setFullscreen(true);
        settingsDialog = new Dialog(this);
        settingsDialog.requestWindowFeature(android.view.Window.FEATURE_NO_TITLE);
        LinearLayout content = new LinearLayout(this);
        content.setOrientation(LinearLayout.VERTICAL);
        content.setPadding(dp(24), dp(12), dp(24), dp(12));
        content.setBackgroundColor(Color.argb(200, 20, 22, 28));
        content.addView(text("设置", 24));
        qualitySummary = text("正在读取直播信息…", 17);
        content.addView(qualitySummary);
        qualityButton = button("选择清晰度");
        qualityButton.setOnClickListener(view -> chooseQuality());
        content.addView(qualityButton, new LinearLayout.LayoutParams(-1, dp(48)));
        content.addView(text("软件更新", 21));
        content.addView(text("当前版本：" + BuildConfig.VERSION_NAME, 16));
        content.addView(text(getString(R.string.update_hint), 14));
        updateSummary = text("尚未检查", 16);
        content.addView(updateSummary);
        updateCheck = button("检查更新");
        updateCheck.setOnClickListener(view -> updateCommand(0));
        content.addView(updateCheck, new LinearLayout.LayoutParams(-1, dp(48)));
        updateDownload = button("下载更新");
        updateDownload.setOnClickListener(view -> updateCommand(1));
        content.addView(updateDownload, new LinearLayout.LayoutParams(-1, dp(48)));
        updateInstall = button("安装更新");
        updateInstall.setOnClickListener(view -> installUpdate());
        content.addView(updateInstall, new LinearLayout.LayoutParams(-1, dp(48)));
        updateCancel = button("取消更新任务");
        updateCancel.setOnClickListener(view -> updateCommand(2));
        content.addView(updateCancel, new LinearLayout.LayoutParams(-1, dp(48)));
        Button close = button("返回直播");
        close.setOnClickListener(view -> settingsDialog.dismiss());
        content.addView(close, new LinearLayout.LayoutParams(-1, dp(48)));
        ScrollView scroll = new ScrollView(this);
        scroll.setFillViewport(true);
        scroll.addView(content);
        settingsDialog.setContentView(scroll);
        settingsDialog.setOnKeyListener((dialog, key, event) -> {
            if (key == KeyEvent.KEYCODE_DPAD_LEFT) {
                if (event.getAction() == KeyEvent.ACTION_DOWN && event.getRepeatCount() == 0) {
                    returnToSatellites[0] = true;
                    dialog.dismiss();
                }
                return true;
            }
            if ((key == KeyEvent.KEYCODE_MENU || key == KeyEvent.KEYCODE_SETTINGS) && event.getAction() == KeyEvent.ACTION_DOWN) {
                dialog.dismiss(); return true;
            }
            return false;
        });
        settingsDialog.setOnDismissListener(dialog -> {
            settingsDialog = null;
            qualitySummary = updateSummary = null;
            qualityButton = updateCheck = updateDownload = updateInstall = updateCancel = null;
            if (!isFinishing() && !isDestroyed()) {
                if (restoreChannels || returnToSatellites[0]) setFullscreen(false, false);
                if (returnToSatellites[0] && !"weishi".equals(group)) setGroup("weishi");
                else if (previousFocus != null && previousFocus.isShown()
                        && (!returnToSatellites[0] || previousFocus.getParent() == channelRows)) previousFocus.requestFocus();
                else if (returnToSatellites[0]) focusGroup();
            }
            hideSystemBars();
        });
        settingsDialog.show();
        styleOverlayWindow(settingsDialog, true);
        refreshSettings();
        qualityButton.requestFocus();
    }

    private String qualityLabel(String id) {
        switch (id) {
            case "sd": return "标清";
            case "hd": return "高清";
            case "shd": return "超清";
            case "fhd": return "蓝光";
            default: return "自动（优先流畅）";
        }
    }
    private void refreshSettingsVisibility() {
        if (settingsDialog == null) return;
        View content = settingsDialog.findViewById(android.R.id.content);
        if (content != null) content.setVisibility(
                (qualityDialog != null && qualityDialog.isShowing()) || (messageDialog != null && messageDialog.isShowing())
                ? View.INVISIBLE : View.VISIBLE);
    }
    private void chooseQuality() {
        List<String> ids = new ArrayList<>(); ids.add("auto");
        JSONArray available = lastPlayback.optJSONArray("availableQualities");
        if (available != null) for (int i = 0; i < available.length(); i++) {
            String id = available.optString(i);
            if (java.util.Arrays.asList("sd", "hd", "shd", "fhd").contains(id) && !ids.contains(id)) ids.add(id);
        }
        String[] labels = new String[ids.size()];
        for (int i = 0; i < ids.size(); i++) labels[i] = qualityLabel(ids.get(i));
        qualityDialog = new AlertDialog.Builder(this).setTitle("频道可用清晰度")
                .setSingleChoiceItems(labels, Math.max(0, ids.indexOf(preferredQuality)), (choice, index) -> {
                    String quality = ids.get(index);
                    if (session != 0 && !NativeBridge.quality(session, quality)) {
                        Toast.makeText(this, "清晰度切换失败，请重试", Toast.LENGTH_SHORT).show(); return;
                    }
                    preferredQuality = quality;
                    getPreferences(MODE_PRIVATE).edit().putString("quality", quality).apply();
                    choice.dismiss(); refreshSettings();
                }).setNegativeButton("返回", null).create();
        qualityDialog.setOnDismissListener(dialog -> {
            qualityDialog = null; refreshSettingsVisibility();
            if (qualityButton != null) qualityButton.requestFocus();
        });
        qualityDialog.show();
        styleOverlayWindow(qualityDialog, false);
        refreshSettingsVisibility();
    }

    private void refreshSettings() {
        String phase = lastUpdate.optString("state", "idle");
        boolean available = "available".equals(phase), ready = "ready".equals(phase);
        if (settingsButton != null) setTextChanged(settingsButton, available || ready ? "设置 · 有更新" : getString(R.string.settings));
        if (settingsDialog == null || qualitySummary == null) return;
        String actual = lastPlayback.optString("effectiveQuality", "");
        int width = lastPlayback.optInt("videoWidth"), height = lastPlayback.optInt("videoHeight");
        String details = "偏好：" + qualityLabel(preferredQuality) + "\n当前："
                + (actual.isEmpty() ? "正在获取直播档位" : qualityLabel(actual));
        if (width > 0 && height > 0) details += " · " + width + " × " + height;
        if (!playbackError.isEmpty()) details += "\n" + playbackError;
        setTextChanged(qualitySummary, details);
        boolean busy = "checking".equals(phase) || "downloading".equals(phase) || "verifying".equals(phase) || "cancelling".equals(phase);
        String message;
        String version = lastUpdate.optString("versionName");
        switch (phase) {
            case "checking": message = "检查中…"; break;
            case "available": message = "新版本：" + version; break;
            case "ready": message = version + " · 可安装"; break;
            case "downloading":
                long total = lastUpdate.optLong("totalBytes");
                message = total > 0 ? String.format(java.util.Locale.ROOT, "下载中 · %.0f%%",
                        Math.min(100.0, lastUpdate.optLong("downloadedBytes") * 100.0 / total)) : "下载中…";
                break;
            case "verifying": message = "校验中…"; break;
            case "cancelling": message = "取消中…"; break;
            case "current": message = "已是最新版本"; break;
            case "idle": message = "尚未检查"; break;
            default: message = lastUpdate.optString("message", "尚未检查"); break;
        }
        if (updater == 0) message = "更新不可用";
        setTextChanged(updateSummary, message);
        updateCheck.setEnabled(updater != 0 && !busy && !installing);
        updateDownload.setVisibility(available ? View.VISIBLE : View.GONE);
        updateDownload.setEnabled(!installing);
        updateInstall.setVisibility(ready ? View.VISIBLE : View.GONE);
        updateInstall.setEnabled(!installing);
        updateCancel.setVisibility(busy ? View.VISIBLE : View.GONE);
        updateCancel.setEnabled(!"cancelling".equals(phase));
    }
    private void updateCommand(int command) {
        if (updater == 0) return;
        NativeBridge.updateCommand(updater, command);
        try { lastUpdate = new JSONObject(NativeBridge.updateStatus(updater)); } catch (org.json.JSONException ignored) { }
        refreshSettings();
    }
    private void installUpdate() {
        if (installing || updater == 0 || !"ready".equals(lastUpdate.optString("state"))) return;
        installing = true; refreshSettings();
        final long handle = updater;
        final String relative = lastUpdate.optString("readyPath"), certificate = lastUpdate.optString("certificateSha256");
        final int version = lastUpdate.optInt("versionCode");
        new Thread(() -> {
            try {
                if (!NativeBridge.updateVerify(handle)) throw new UpdateInstaller.Failure("安装包校验失败，请重新下载");
                File file = UpdateInstaller.inspect(this, updateDirectory, relative, version, certificate);
                handler.post(() -> {
                    installing = false;
                    if (isFinishing() || isDestroyed()) return;
                    try {
                        boolean permission = Build.VERSION.SDK_INT >= 26 && !getPackageManager().canRequestPackageInstalls();
                        UpdateInstaller.launch(this, file, relative);
                        if (permission) Toast.makeText(this, "请允许安装更新，返回后再点击安装", Toast.LENGTH_LONG).show();
                    } catch (RuntimeException error) { updateMessage("无法打开系统安装器或授权页面"); }
                    refreshSettings();
                });
            } catch (Exception error) {
                String message = error instanceof UpdateInstaller.Failure ? error.getMessage() : "无法验证安装包，请重新下载";
                handler.post(() -> {
                    installing = false;
                    if (!isFinishing() && !isDestroyed()) updateMessage(message);
                    refreshSettings();
                });
            }
        }, "atv-install").start();
    }
    private void updateMessage(String message) {
        if (messageDialog != null) messageDialog.dismiss();
        messageDialog = new AlertDialog.Builder(this).setMessage(message).setPositiveButton("返回", null).create();
        messageDialog.setOnDismissListener(dialog -> {
            messageDialog = null; refreshSettingsVisibility();
            if (updateInstall != null && updateInstall.isShown()) updateInstall.requestFocus();
        });
        messageDialog.show();
        styleOverlayWindow(messageDialog, false);
        refreshSettingsVisibility();
    }
    private void setTextChanged(TextView view, String value) {
        if (!android.text.TextUtils.equals(view.getText(), value)) view.setText(value);
    }

    private static final class Channel {
        final String pid, streamId, name, group;
        Channel(String pid, String streamId, String name, String group) {
            this.pid = pid; this.streamId = streamId; this.name = name; this.group = group;
        }
    }
}
