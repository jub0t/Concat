// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

package app.concat.editor;

import android.Manifest;
import android.app.Activity;
import android.app.ActivityManager;
import android.app.ApplicationExitInfo;
import android.app.Fragment;
import android.content.ClipData;
import android.content.ContentResolver;
import android.content.ContentUris;
import android.content.ContentValues;
import android.content.Context;
import android.content.Intent;
import android.content.pm.PackageManager;
import android.database.Cursor;
import android.net.Uri;
import android.os.Build;
import android.os.Environment;
import android.provider.MediaStore;
import android.provider.OpenableColumns;
import android.util.Log;

import java.io.File;
import java.io.FileInputStream;
import java.io.FileOutputStream;
import java.io.IOException;
import java.io.InputStream;
import java.io.OutputStream;
import java.nio.charset.StandardCharsets;
import java.util.ArrayList;
import java.util.List;

/**
 * The system's document picker, for an activity that has no Java of its
 * own to receive its answer.
 *
 * The window is a NativeActivity: the framework's own class, with nothing
 * of ours to override, and a picker's result comes back only through an
 * activity's or a fragment's onActivityResult. So this is a fragment - a
 * headless one, added to the activity for the length of one pick - and
 * the result comes back here.
 *
 * What is picked is a content URI, which the engine's decoder cannot open:
 * it reads files by path. Each is copied into the app's own external
 * files, under Imported/, keeping its display name, and the paths are
 * what the engine is handed - through the native method below, which the
 * Rust side registers before it asks for a pick.
 *
 * The way out is here as well: a finished export is written into the
 * app's own folder, which on a phone nothing but the app can open, so
 * {@link #publishVideo} moves it into the phone's Movies through the
 * media store, where the gallery and every file manager show it.
 *
 * And the microphone, for a voiceover: a permission granted while the
 * app runs, asked for through the activity by {@link #askMicrophone}.
 *
 * Compiled by build.rs with the SDK's javac and d8, and loaded at run time
 * from the dex inside the binary; see src/lib.rs.
 */
public class ConcatFiles extends Fragment {
    private static final int REQUEST_PICK = 0xC0C4;
    private static final int REQUEST_MICROPHONE = 0xC0C5;
    private static final String TAG = "concat-files";

    /**
     * Where a finished export goes, under the phone's storage: Movies/Concat.
     * Null before Android 10, which has no media store an app can write
     * through without a permission; there the file stays in the app's folder.
     */
    public static String publishedFolder() {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.Q) {
            return null;
        }
        return Environment.DIRECTORY_MOVIES + "/Concat";
    }

    /**
     * Moves the video at {@code path} into {@link #publishedFolder()} through
     * the media store and deletes the original. Two strings come back: the
     * file's path under the phone's storage and null, or null and why it
     * could not be moved - in which case the original is still where it was.
     * Blocking: a video is big and the copy takes a while, so this is called
     * off the window's thread.
     */
    public static String[] publishVideo(Activity activity, String path) {
        String folder = publishedFolder();
        if (folder == null) {
            return new String[] {null, "this phone has no media store to put it in"};
        }
        return publish(activity, path, "video/mp4", folder,
                MediaStore.Video.Media.getContentUri(MediaStore.VOLUME_EXTERNAL_PRIMARY));
    }

    /**
     * {@link #publishVideo} for a small file that is not a video - a
     * subtitle file - of MIME type {@code mime}: moved into the phone's
     * Download/Concat, which every file manager shows, under its own name
     * or "name (1)" where that is taken. The same two strings come back.
     */
    public static String[] publishDocument(Activity activity, String path, String mime) {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.Q) {
            return new String[] {null, "this phone has no media store to put it in"};
        }
        return publish(activity, path, mime, Environment.DIRECTORY_DOWNLOADS + "/Concat",
                MediaStore.Downloads.getContentUri(MediaStore.VOLUME_EXTERNAL_PRIMARY));
    }

    /** Moves the file at {@code path} into {@code collection}, under {@code folder}. */
    private static String[] publish(
            Activity activity, String path, String mime, String folder, Uri collection) {
        File file = new File(path);
        ContentResolver resolver = activity.getContentResolver();
        ContentValues values = new ContentValues();
        values.put(MediaStore.MediaColumns.DISPLAY_NAME, file.getName());
        values.put(MediaStore.MediaColumns.MIME_TYPE, mime);
        values.put(MediaStore.MediaColumns.RELATIVE_PATH, folder);
        values.put(MediaStore.MediaColumns.IS_PENDING, 1);
        Uri uri;
        try {
            uri = resolver.insert(collection, values);
        } catch (Exception e) {
            return new String[] {null, "the media store refused the file: " + e.getMessage()};
        }
        if (uri == null) {
            return new String[] {null, "the media store refused the file"};
        }
        InputStream in = null;
        OutputStream out = null;
        try {
            in = new FileInputStream(file);
            out = resolver.openOutputStream(uri);
            if (out == null) {
                throw new IOException("the media store would not open the file for writing");
            }
            byte[] buffer = new byte[1 << 16];
            int read;
            while ((read = in.read(buffer)) > 0) {
                out.write(buffer, 0, read);
            }
            out.close();
            out = null;
            values.clear();
            values.put(MediaStore.MediaColumns.IS_PENDING, 0);
            resolver.update(uri, values, null, null);
        } catch (Exception e) {
            try {
                resolver.delete(uri, null, null);
            } catch (Exception ignored) {
            }
            return new String[] {null, e.getMessage() == null ? e.toString() : e.getMessage()};
        } finally {
            try {
                if (in != null) in.close();
            } catch (IOException ignored) {
            }
            try {
                if (out != null) out.close();
            } catch (IOException ignored) {
            }
        }
        file.delete();
        // The store may have renamed it: a second "name.mp4" is "name (1).mp4".
        return new String[] {folder + "/" + displayName(activity, uri), null};
    }

    /**
     * Whether the app may record from the microphone. When it may not yet,
     * the system's question is put up and false comes back at once; the
     * answer is read the next time this is asked, which is the voiceover
     * panel's Try again. From any thread.
     */
    public static boolean askMicrophone(final Activity activity) {
        if (Build.VERSION.SDK_INT < 23) {
            return true;
        }
        if (activity.checkSelfPermission(Manifest.permission.RECORD_AUDIO)
                == PackageManager.PERMISSION_GRANTED) {
            return true;
        }
        activity.runOnUiThread(new Runnable() {
            @Override
            public void run() {
                activity.requestPermissions(
                        new String[] {Manifest.permission.RECORD_AUDIO}, REQUEST_MICROPHONE);
            }
        });
        return false;
    }

    /**
     * Opens a web address in the browser, or in whichever app claims it -
     * the launch screen's guides, the repository, the Discord invite. Null
     * when it went, or why it could not. Asked straight rather than through
     * resolveActivity, which from Android 11 answers no for any app the
     * manifest does not name in its queries. From any thread.
     */
    public static String openLink(Activity activity, String url) {
        try {
            Intent intent = new Intent(Intent.ACTION_VIEW, Uri.parse(url));
            intent.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK);
            activity.startActivity(intent);
            return null;
        } catch (android.content.ActivityNotFoundException e) {
            return "no app on this phone opens " + url;
        } catch (RuntimeException e) {
            Log.w(TAG, "could not open " + url, e);
            return String.valueOf(e.getMessage());
        }
    }

    /**
     * Hands the log files at {@code paths} - one per line - to the system's
     * share sheet, so a report can carry them to mail, a chat, a drive.
     * Nothing else on the phone can read the app's own folder, so from
     * Android 10 each is copied into Download/Concat as a text file, the
     * same run's copy written over, and shared by the media store's URI with
     * a read grant: no permission asked, no file manager needed. Before
     * Android 10 an app cannot write there without a permission, and the
     * end of the logs goes as text instead. Null when the sheet went up, or
     * why it could not. From any thread.
     */
    public static String shareLogs(Activity activity, String paths) {
        try {
            String[] files = paths.split("\n");
            ArrayList<Uri> uris = new ArrayList<>();
            Intent send = new Intent(files.length > 1 && Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q
                    ? Intent.ACTION_SEND_MULTIPLE : Intent.ACTION_SEND);
            send.setType("text/plain");
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
                for (String path : files) {
                    uris.add(logCopy(activity, new File(path)));
                }
                ClipData clip = ClipData.newRawUri(null, uris.get(0));
                for (int i = 1; i < uris.size(); i++) {
                    clip.addItem(new ClipData.Item(uris.get(i)));
                }
                send.setClipData(clip);
                send.addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION);
                if (uris.size() > 1) {
                    send.putParcelableArrayListExtra(Intent.EXTRA_STREAM, uris);
                } else {
                    send.putExtra(Intent.EXTRA_STREAM, uris.get(0));
                }
            } else {
                // A share's extras cross to the other app in one binder
                // call, which holds about a megabyte: each file's end.
                StringBuilder text = new StringBuilder();
                for (String path : files) {
                    File file = new File(path);
                    text.append("==== ").append(file.getName()).append(" ====\n");
                    text.append(tail(file, 120 * 1024)).append('\n');
                }
                send.putExtra(Intent.EXTRA_TEXT, text.toString());
            }
            activity.startActivity(Intent.createChooser(send, null));
            return null;
        } catch (Exception e) {
            Log.w(TAG, "could not share the log", e);
            return e.getMessage() == null ? e.toString() : e.getMessage();
        }
    }

    /** The log at {@code file}, copied into Download/Concat as text. */
    private static Uri logCopy(Activity activity, File file) throws IOException {
        ContentResolver resolver = activity.getContentResolver();
        Uri collection = MediaStore.Downloads.getContentUri(MediaStore.VOLUME_EXTERNAL_PRIMARY);
        String folder = Environment.DIRECTORY_DOWNLOADS + "/Concat/";
        String name = file.getName().replaceFirst("\\.log$", "") + ".txt";
        Uri uri = null;
        // The copy an earlier share of this run made, which this one
        // replaces rather than adding "name (1)" beside it.
        Cursor cursor = resolver.query(collection, new String[] {MediaStore.MediaColumns._ID},
                MediaStore.MediaColumns.DISPLAY_NAME + "=? AND "
                        + MediaStore.MediaColumns.RELATIVE_PATH + "=?",
                new String[] {name, folder}, null);
        if (cursor != null) {
            if (cursor.moveToFirst()) {
                uri = ContentUris.withAppendedId(collection, cursor.getLong(0));
            }
            cursor.close();
        }
        if (uri == null) {
            ContentValues values = new ContentValues();
            values.put(MediaStore.MediaColumns.DISPLAY_NAME, name);
            values.put(MediaStore.MediaColumns.MIME_TYPE, "text/plain");
            values.put(MediaStore.MediaColumns.RELATIVE_PATH, folder);
            uri = resolver.insert(collection, values);
            if (uri == null) {
                throw new IOException("the media store refused " + name);
            }
        }
        InputStream in = new FileInputStream(file);
        try {
            OutputStream out = resolver.openOutputStream(uri, "wt");
            if (out == null) {
                throw new IOException("the media store would not open " + name + " for writing");
            }
            try {
                byte[] buffer = new byte[1 << 16];
                int read;
                while ((read = in.read(buffer)) > 0) {
                    out.write(buffer, 0, read);
                }
            } finally {
                out.close();
            }
        } finally {
            in.close();
        }
        return uri;
    }

    /** The last {@code most} bytes of {@code file}, as text. */
    private static String tail(File file, int most) throws IOException {
        java.io.RandomAccessFile in = new java.io.RandomAccessFile(file, "r");
        try {
            long from = Math.max(0, in.length() - most);
            byte[] bytes = new byte[(int) (in.length() - from)];
            in.seek(from);
            in.readFully(bytes);
            return new String(bytes, StandardCharsets.UTF_8);
        } finally {
            in.close();
        }
    }

    /**
     * How the system says the app's last process ended, read once as the
     * app starts; Android 11 and on keep a record, and null comes back on
     * an older phone or when there is none. Three strings: the log file
     * that process was writing (as {@code log} named it for this one), or
     * empty; "died" when it ended in a way nobody asked for - a crash, a
     * hang, killed while on screen - or "ended"; and what the system says,
     * for the log. A native crash brings its signal, thread, abort message
     * and top frames, read from the crash report the system kept.
     *
     * This run is then marked with {@code log}, so the next run knows which
     * file this one is.
     */
    public static String[] lastExit(Activity activity, String log) {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.R) {
            return null;
        }
        ActivityManager manager =
                (ActivityManager) activity.getSystemService(Context.ACTIVITY_SERVICE);
        String[] answer = null;
        try {
            List<ApplicationExitInfo> exits =
                    manager.getHistoricalProcessExitReasons(null, 0, 1);
            if (!exits.isEmpty()) {
                answer = describe(exits.get(0));
            }
        } catch (RuntimeException e) {
            Log.w(TAG, "could not read how the last run ended", e);
        }
        try {
            manager.setProcessStateSummary(log.getBytes(StandardCharsets.UTF_8));
        } catch (RuntimeException e) {
            Log.w(TAG, "could not mark this run", e);
        }
        return answer;
    }

    private static String[] describe(ApplicationExitInfo exit) {
        int status = exit.getStatus();
        boolean seen = exit.getImportance()
                <= ActivityManager.RunningAppProcessInfo.IMPORTANCE_VISIBLE;
        String reason;
        boolean died;
        switch (exit.getReason()) {
            case ApplicationExitInfo.REASON_CRASH_NATIVE:
                reason = "a native crash, signal " + status;
                died = true;
                break;
            case ApplicationExitInfo.REASON_CRASH:
                reason = "an uncaught Java exception";
                died = true;
                break;
            case ApplicationExitInfo.REASON_ANR:
                reason = "not responding (ANR)";
                died = true;
                break;
            case ApplicationExitInfo.REASON_INITIALIZATION_FAILURE:
                reason = "a failure to start";
                died = true;
                break;
            case ApplicationExitInfo.REASON_EXCESSIVE_RESOURCE_USAGE:
                reason = "killed for excessive resource use";
                died = true;
                break;
            case ApplicationExitInfo.REASON_LOW_MEMORY:
                reason = "killed for memory";
                died = seen;
                break;
            case ApplicationExitInfo.REASON_SIGNALED:
                reason = "killed by signal " + status;
                died = seen;
                break;
            case ApplicationExitInfo.REASON_EXIT_SELF:
                reason = "its own exit, status " + status;
                died = status != 0;
                break;
            case ApplicationExitInfo.REASON_USER_REQUESTED:
                reason = "the person (force stop, or swiped away)";
                died = false;
                break;
            default:
                reason = "reason " + exit.getReason() + ", status " + status;
                died = false;
        }
        StringBuilder detail = new StringBuilder(reason);
        detail.append(seen ? " · on screen" : " · in the background");
        if (exit.getPss() > 0) {
            detail.append(" · ").append(exit.getPss() / 1024).append(" MB in use");
        }
        if (exit.getDescription() != null) {
            detail.append(" · \"").append(exit.getDescription()).append('"');
        }
        if (exit.getReason() == ApplicationExitInfo.REASON_CRASH_NATIVE
                && Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
            try {
                InputStream trace = exit.getTraceInputStream();
                if (trace != null) {
                    detail.append(" · ").append(Tombstone.summary(readAll(trace)));
                }
            } catch (IOException | RuntimeException e) {
                detail.append(" · its crash report could not be read: ").append(e);
            }
        }
        byte[] mark = exit.getProcessStateSummary();
        String log = mark == null ? "" : new String(mark, StandardCharsets.UTF_8);
        return new String[] {log, died ? "died" : "ended", detail.toString()};
    }

    private static byte[] readAll(InputStream in) throws IOException {
        try {
            java.io.ByteArrayOutputStream out = new java.io.ByteArrayOutputStream();
            byte[] buffer = new byte[1 << 16];
            int read;
            while ((read = in.read(buffer)) > 0) {
                out.write(buffer, 0, read);
            }
            return out.toByteArray();
        } finally {
            in.close();
        }
    }

    /**
     * The few fields of a native crash report worth a log line, read from
     * the system's protobuf (debuggerd's tombstone.proto) by hand: the
     * signal, the thread it hit, the abort message and the top frames.
     */
    static final class Tombstone {
        private final byte[] data;
        private int at;
        private final int end;

        private Tombstone(byte[] data, int from, int end) {
            this.data = data;
            this.at = from;
            this.end = end;
        }

        static String summary(byte[] data) {
            Tombstone top = new Tombstone(data, 0, data.length);
            long tid = -1;
            String signal = "", abort = "", cause = "";
            java.util.Map<Long, int[]> threads = new java.util.HashMap<>();
            while (top.more()) {
                long key = top.varint();
                int field = (int) (key >>> 3);
                if (field == 6 && (key & 7) == 0) {
                    tid = top.varint();
                } else if (field == 10 && (key & 7) == 2) {
                    Tombstone info = top.message();
                    String name = "", code = "";
                    while (info.more()) {
                        long k = info.varint();
                        if (k >>> 3 == 2 && (k & 7) == 2) name = info.string();
                        else if (k >>> 3 == 4 && (k & 7) == 2) code = info.string();
                        else info.skip(k);
                    }
                    signal = name + (code.isEmpty() ? "" : " (" + code + ")");
                } else if (field == 14 && (key & 7) == 2) {
                    abort = top.string();
                } else if (field == 15 && (key & 7) == 2 && cause.isEmpty()) {
                    Tombstone c = top.message();
                    while (c.more()) {
                        long k = c.varint();
                        if (k >>> 3 == 1 && (k & 7) == 2) cause = c.string();
                        else c.skip(k);
                    }
                } else if (field == 16 && (key & 7) == 2) {
                    // A map entry: the thread's id, and where the thread is.
                    Tombstone entry = top.message();
                    long id = -1;
                    int[] span = null;
                    while (entry.more()) {
                        long k = entry.varint();
                        if (k >>> 3 == 1 && (k & 7) == 0) {
                            id = entry.varint();
                        } else if (k >>> 3 == 2 && (k & 7) == 2) {
                            int length = (int) entry.varint();
                            span = new int[] {entry.at, entry.at + length};
                            entry.at += length;
                        } else {
                            entry.skip(k);
                        }
                    }
                    if (span != null) threads.put(id, span);
                } else {
                    top.skip(key);
                }
            }
            StringBuilder out = new StringBuilder("crash report: ").append(signal);
            int[] span = threads.get(tid);
            if (span != null) {
                Tombstone thread = new Tombstone(data, span[0], span[1]);
                String name = "";
                StringBuilder frames = new StringBuilder();
                int count = 0;
                while (thread.more()) {
                    long k = thread.varint();
                    if (k >>> 3 == 2 && (k & 7) == 2) {
                        name = thread.string();
                    } else if (k >>> 3 == 4 && (k & 7) == 2 && count < 12) {
                        frames.append(" | ").append(frame(thread.message()));
                        count++;
                    } else {
                        thread.skip(k);
                    }
                }
                out.append(" in thread ").append(tid).append(" (").append(name).append(')');
                if (!cause.isEmpty()) out.append(" · cause: ").append(cause);
                if (!abort.isEmpty()) out.append(" · abort message: \"").append(abort).append('"');
                out.append(" · frames").append(frames);
            } else {
                if (!cause.isEmpty()) out.append(" · cause: ").append(cause);
                if (!abort.isEmpty()) out.append(" · abort message: \"").append(abort).append('"');
            }
            return out.toString();
        }

        /** One frame: `libconcat_android.so+0x1a2b3c (function+12)`. */
        private static String frame(Tombstone frame) {
            long pc = 0, offset = 0;
            String file = "", function = "";
            while (frame.more()) {
                long k = frame.varint();
                int field = (int) (k >>> 3);
                if (field == 1 && (k & 7) == 0) pc = frame.varint();
                else if (field == 4 && (k & 7) == 2) function = frame.string();
                else if (field == 5 && (k & 7) == 0) offset = frame.varint();
                else if (field == 6 && (k & 7) == 2) file = frame.string();
                else frame.skip(k);
            }
            String module = file.substring(file.lastIndexOf('/') + 1);
            String where = module + "+0x" + Long.toHexString(pc);
            return function.isEmpty() ? where : where + " (" + function + "+" + offset + ")";
        }

        private boolean more() {
            return at < end;
        }

        private long varint() {
            long value = 0;
            for (int shift = 0; shift < 64 && at < end; shift += 7) {
                byte b = data[at++];
                value |= (long) (b & 0x7f) << shift;
                if ((b & 0x80) == 0) {
                    break;
                }
            }
            return value;
        }

        private Tombstone message() {
            int length = (int) varint();
            Tombstone inner = new Tombstone(data, at, Math.min(end, at + length));
            at = Math.min(end, at + length);
            return inner;
        }

        private String string() {
            Tombstone inner = message();
            return new String(data, inner.at, inner.end - inner.at, StandardCharsets.UTF_8);
        }

        /** Steps over a field this does not read. */
        private void skip(long key) {
            switch ((int) (key & 7)) {
                case 0: varint(); break;
                case 1: at += 8; break;
                case 2: message(); break;
                case 5: at += 4; break;
                default: at = end;
            }
        }
    }

    /** Registered from Rust: the picked files' paths, or none. */
    public static native void filesPicked(String[] paths);

    /**
     * Shows the picker, offering the MIME types in {@code types} - comma
     * separated, "video/*,audio/*" - or every file when it is empty. From
     * any thread; the fragment is added on the UI thread.
     */
    public static void pick(final Activity activity, final String types) {
        activity.runOnUiThread(new Runnable() {
            @Override
            public void run() {
                ConcatFiles fragment = new ConcatFiles();
                activity.getFragmentManager()
                        .beginTransaction()
                        .add(fragment, TAG)
                        .commitAllowingStateLoss();
                activity.getFragmentManager().executePendingTransactions();

                Intent intent = new Intent(Intent.ACTION_OPEN_DOCUMENT);
                intent.addCategory(Intent.CATEGORY_OPENABLE);
                intent.setType("*/*");
                if (types != null && !types.isEmpty()) {
                    intent.putExtra(Intent.EXTRA_MIME_TYPES, types.split(","));
                }
                intent.putExtra(Intent.EXTRA_ALLOW_MULTIPLE, true);
                try {
                    fragment.startActivityForResult(intent, REQUEST_PICK);
                } catch (Exception e) {
                    fragment.done(new ArrayList<Uri>());
                }
            }
        });
    }

    @Override
    public void onActivityResult(int request, int result, Intent data) {
        if (request != REQUEST_PICK) {
            return;
        }
        List<Uri> uris = new ArrayList<Uri>();
        if (result == Activity.RESULT_OK && data != null) {
            ClipData clip = data.getClipData();
            if (clip != null) {
                for (int i = 0; i < clip.getItemCount(); i++) {
                    Uri uri = clip.getItemAt(i).getUri();
                    if (uri != null) {
                        uris.add(uri);
                    }
                }
            } else if (data.getData() != null) {
                uris.add(data.getData());
            }
        }
        done(uris);
    }

    /** Copies what was picked and reports it; then the fragment goes. */
    private void done(final List<Uri> uris) {
        final Activity activity = getActivity();
        try {
            getFragmentManager().beginTransaction().remove(this).commitAllowingStateLoss();
        } catch (Exception ignored) {
        }
        if (activity == null || uris.isEmpty()) {
            filesPicked(new String[0]);
            return;
        }
        // Off the UI thread: a video is big and the copy takes a while.
        new Thread(new Runnable() {
            @Override
            public void run() {
                List<String> paths = new ArrayList<String>();
                File dir = new File(activity.getExternalFilesDir(null), "Imported");
                dir.mkdirs();
                for (Uri uri : uris) {
                    File out = unique(dir, displayName(activity, uri));
                    if (copy(activity, uri, out)) {
                        paths.add(out.getAbsolutePath());
                    }
                }
                filesPicked(paths.toArray(new String[0]));
            }
        }, TAG).start();
    }

    /** The name the picker showed for the file, or one made from the URI. */
    private static String displayName(Activity activity, Uri uri) {
        String name = null;
        Cursor cursor = null;
        try {
            cursor = activity.getContentResolver().query(uri, null, null, null, null);
            if (cursor != null && cursor.moveToFirst()) {
                int column = cursor.getColumnIndex(OpenableColumns.DISPLAY_NAME);
                if (column >= 0) {
                    name = cursor.getString(column);
                }
            }
        } catch (Exception ignored) {
        } finally {
            if (cursor != null) {
                cursor.close();
            }
        }
        if (name == null || name.isEmpty()) {
            name = uri.getLastPathSegment();
        }
        if (name == null || name.isEmpty()) {
            name = "import";
        }
        // Whatever the provider called it, it is one file name.
        return name.replace('/', '_').replace('\\', '_');
    }

    /** `name`, or `name (2)`, `name (3)`... until nothing is overwritten. */
    private static File unique(File dir, String name) {
        File out = new File(dir, name);
        if (!out.exists()) {
            return out;
        }
        int dot = name.lastIndexOf('.');
        String stem = dot > 0 ? name.substring(0, dot) : name;
        String ext = dot > 0 ? name.substring(dot) : "";
        for (int n = 2; ; n++) {
            out = new File(dir, stem + " (" + n + ")" + ext);
            if (!out.exists()) {
                return out;
            }
        }
    }

    private static boolean copy(Activity activity, Uri uri, File out) {
        InputStream in = null;
        OutputStream os = null;
        try {
            in = activity.getContentResolver().openInputStream(uri);
            if (in == null) {
                return false;
            }
            os = new FileOutputStream(out);
            byte[] buffer = new byte[1 << 16];
            int read;
            while ((read = in.read(buffer)) > 0) {
                os.write(buffer, 0, read);
            }
            return true;
        } catch (IOException | RuntimeException e) {
            // A provider that revokes its grant mid-copy throws a
            // SecurityException, not an IOException; uncaught on this
            // thread it closed the app and lost the rest of the pick.
            Log.w(TAG, "could not copy " + uri + ": " + e);
            out.delete();
            return false;
        } finally {
            try {
                if (in != null) in.close();
            } catch (IOException ignored) {
            }
            try {
                if (os != null) os.close();
            } catch (IOException ignored) {
            }
        }
    }
}
