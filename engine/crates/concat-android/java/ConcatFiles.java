// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

package app.concat.editor;

import android.app.Activity;
import android.app.Fragment;
import android.content.ClipData;
import android.content.Intent;
import android.database.Cursor;
import android.net.Uri;
import android.provider.OpenableColumns;

import java.io.File;
import java.io.FileOutputStream;
import java.io.IOException;
import java.io.InputStream;
import java.io.OutputStream;
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
 * Compiled by build.rs with the SDK's javac and d8, and loaded at run time
 * from the dex inside the binary; see src/lib.rs.
 */
public class ConcatFiles extends Fragment {
    private static final int REQUEST_PICK = 0xC0C4;
    private static final String TAG = "concat-files";

    /** Registered from Rust: the picked files' paths, or none. */
    public static native void filesPicked(String[] paths);

    /** Shows the picker. From any thread; the fragment is added on the UI thread. */
    public static void pick(final Activity activity) {
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
                intent.putExtra(Intent.EXTRA_MIME_TYPES, new String[] {"video/*", "audio/*", "image/*"});
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
        } catch (IOException e) {
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
