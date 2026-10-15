package io.github.klanthier.vanillish;

import android.app.Activity;
import android.content.ActivityNotFoundException;
import android.content.ClipData;
import android.content.ContentResolver;
import android.content.Context;
import android.content.Intent;
import android.content.SharedPreferences;
import android.database.Cursor;
import android.net.Uri;
import android.os.Handler;
import android.os.Looper;
import android.os.ParcelFileDescriptor;
import android.provider.DocumentsContract;
import android.util.Log;

import androidx.activity.result.ActivityResultLauncher;
import androidx.activity.result.contract.ActivityResultContracts;
import androidx.core.content.FileProvider;

import org.json.JSONArray;
import org.json.JSONException;
import org.json.JSONObject;

import java.io.File;
import java.util.ArrayDeque;
import java.util.Arrays;
import java.util.HashSet;
import java.util.Locale;
import java.util.Set;
import java.util.concurrent.Executor;
import java.util.concurrent.Executors;

/**
 * The game data from a folder the player picks (the Storage Access Framework), and files handed
 * to other apps (the share sheet). Rust drives the copy (inilla-core's {@code import}, on the
 * import's own thread); this side only does what needs Android:
 *
 * <ul>
 *   <li>opens the folder picker ({@code ACTION_OPEN_DOCUMENT_TREE}, starting in Download) and
 *       takes a persistable read grant on the answer, so a copy cut off by Pause, a crash or the
 *       process being killed in the background resumes after a restart ({@link #pendingFolder});
 *   <li>lists the folder's archives as JSON — one child query per folder, much faster than
 *       {@code DocumentFile};
 *   <li>opens one archive as a detached file descriptor, which Rust then owns and closes.
 * </ul>
 *
 * <p>A picked folder is never a path the app can open (there is no storage permission for .MPQ
 * files): the import is a copy, and the original stays.
 */
final class FolderImport {
    private static final String TAG = "vanillish";

    private FolderImport() {}

    static native void nativeFolderPicked(String tree, String label);

    static native void nativeFolderPickCancelled();

    static native void nativeFolderPickFailed(String message);

    /** Where the picker opens: Download, where a browser or Quick Share leaves things. */
    private static final Uri INITIAL = DocumentsContract.buildDocumentUri(
            "com.android.externalstorage.documents", "primary:Download");

    /** Folders never entered (lower case): a client's big trees with no archive in them. */
    private static final Set<String> SKIP = new HashSet<>(Arrays.asList(
            "interface", "wtf", "cache", "screenshots", "logs", "errors"));

    /** The most folders one listing visits before it says "truncated". */
    private static final int MAX_FOLDERS = 400;

    private static final String PREFS = "folder-import";
    private static final String PENDING = "pendingFolder";

    private static final Handler ui = new Handler(Looper.getMainLooper());

    private static volatile ActivityResultLauncher<Uri> launcher;
    private static volatile String lastError;

    /** From the Activity's onCreate: a result launcher must be registered before it starts. */
    static void register(VanillishActivity a) {
        launcher = a.registerForActivityResult(
                new ActivityResultContracts.OpenDocumentTree(), FolderImport::onResult);
    }

    // ---- The picker ---------------------------------------------------------------------------

    /** Show the picker (posted to the UI thread). False when there is nothing to show it from. */
    static boolean pick() {
        ActivityResultLauncher<Uri> l = launcher;
        if (l == null || Bridge.activity == null) return false;
        ui.post(() -> {
            try {
                l.launch(INITIAL);
            } catch (ActivityNotFoundException | IllegalStateException e) {
                Log.w(TAG, "folder picker: " + e);
                nativeFolderPickFailed("this phone has no folder picker (" + e.getMessage() + ")");
            }
        });
        return true;
    }

    /**
     * The answer's follow-up, in order, off the UI thread: the folder's name is a query into its
     * provider, which may be a network share or a drive spinning up (seconds, and the UI thread
     * would stall into an ANR meanwhile).
     */
    private static final Executor io =
            Executors.newSingleThreadExecutor(r -> new Thread(r, "vanillish-folder"));

    /** The picker's answer, on the UI thread: null when the player backed out. */
    static void onResult(Uri tree) {
        if (tree == null) {
            nativeFolderPickCancelled();
            return;
        }
        Context c = Bridge.app;
        if (c == null) {
            nativeFolderPickFailed("the app was not ready for the folder");
            return;
        }
        ContentResolver r = c.getContentResolver();
        try {
            r.takePersistableUriPermission(tree, Intent.FLAG_GRANT_READ_URI_PERMISSION);
        } catch (SecurityException e) {
            // Still readable for as long as this process lives; only a resume after a restart
            // would need the grant again.
            Log.w(TAG, "folder grant not persisted: " + e);
        }
        // The pending record is written before Rust hears of the folder, so the import (which
        // forgets it once done) always finds it.
        io.execute(() -> {
            String label = displayName(r, tree);
            // A folder picked earlier and never finished: this one replaces it.
            String old = pendingTree(c);
            if (old != null && !old.equals(tree.toString())) forget(old);
            try {
                String json = new JSONObject().put("tree", tree.toString()).put("label", label)
                        .toString();
                prefs(c).edit().putString(PENDING, json).apply();
            } catch (JSONException e) {
                Log.w(TAG, "folder: " + e);
            }
            nativeFolderPicked(tree.toString(), label);
        });
    }

    private static SharedPreferences prefs(Context c) {
        return c.getSharedPreferences(PREFS, Context.MODE_PRIVATE);
    }

    private static String pendingTree(Context c) {
        String json = prefs(c).getString(PENDING, null);
        if (json == null) return null;
        try {
            return new JSONObject(json).optString("tree", null);
        } catch (JSONException e) {
            return null;
        }
    }

    /** The picked folder's own name ("World of Warcraft"). */
    private static String displayName(ContentResolver r, Uri tree) {
        String id = DocumentsContract.getTreeDocumentId(tree);
        Uri doc = DocumentsContract.buildDocumentUriUsingTree(tree, id);
        try (Cursor cur = r.query(doc,
                new String[] {DocumentsContract.Document.COLUMN_DISPLAY_NAME}, null, null, null)) {
            if (cur != null && cur.moveToFirst() && !cur.isNull(0)) return cur.getString(0);
        } catch (RuntimeException e) {
            Log.w(TAG, "folder name: " + e);
        }
        // "primary:Download/WoW" → "WoW".
        String name = id.substring(id.lastIndexOf(':') + 1);
        return name.substring(name.lastIndexOf('/') + 1);
    }

    /** The picked folder whose copy has not finished: {"tree": …, "label": …}, or null. */
    static String pendingFolder() {
        Context c = Bridge.app;
        return c != null ? prefs(c).getString(PENDING, null) : null;
    }

    /** Give the folder's grant back and stop offering to resume it. */
    static void forget(String tree) {
        Context c = Bridge.app;
        if (c == null || tree == null) return;
        try {
            c.getContentResolver().releasePersistableUriPermission(
                    Uri.parse(tree), Intent.FLAG_GRANT_READ_URI_PERMISSION);
        } catch (SecurityException e) {
            // Never persisted (or already given back).
        }
        if (tree.equals(pendingTree(c))) prefs(c).edit().remove(PENDING).apply();
    }

    static String lastError() {
        return lastError;
    }

    // ---- Reading the folder (Rust's import thread) ---------------------------------------------

    private static final String[] COLUMNS = {
        DocumentsContract.Document.COLUMN_DOCUMENT_ID,
        DocumentsContract.Document.COLUMN_DISPLAY_NAME,
        DocumentsContract.Document.COLUMN_MIME_TYPE,
        DocumentsContract.Document.COLUMN_SIZE,
        DocumentsContract.Document.COLUMN_LAST_MODIFIED,
    };

    private static final class Dir {
        final String id;
        final String path;
        final int depth;

        Dir(String id, String path, int depth) {
            this.id = id;
            this.path = path;
            this.depth = depth;
        }
    }

    /**
     * The folder's archives, breadth first down to {@code maxDepth} folders: names starting with
     * "." skipped, {@link #SKIP} not entered, at most {@link #MAX_FOLDERS} folders. JSON:
     * {"label", "truncated", "files": [{"path": "Data/dbc.MPQ", "size", "modified", "doc"}]},
     * paths relative to the picked folder. Null on failure ({@link #lastError}).
     */
    static String list(String tree, int maxDepth) {
        Context c = Bridge.app;
        if (c == null || tree == null) {
            lastError = "the app is not ready";
            return null;
        }
        try {
            Uri treeUri = Uri.parse(tree);
            ContentResolver r = c.getContentResolver();
            JSONArray files = new JSONArray();
            ArrayDeque<Dir> queue = new ArrayDeque<>();
            queue.add(new Dir(DocumentsContract.getTreeDocumentId(treeUri), "", 0));
            int folders = 0;
            boolean truncated = false;
            while (!queue.isEmpty()) {
                Dir dir = queue.poll();
                if (++folders > MAX_FOLDERS) {
                    truncated = true;
                    break;
                }
                Uri children = DocumentsContract.buildChildDocumentsUriUsingTree(treeUri, dir.id);
                try (Cursor cur = r.query(children, COLUMNS, null, null, null)) {
                    if (cur == null) continue;
                    while (cur.moveToNext()) {
                        String id = cur.getString(0);
                        String name = cur.getString(1);
                        String mime = cur.getString(2);
                        if (id == null || name == null || name.startsWith(".")) continue;
                        String lower = name.toLowerCase(Locale.ROOT);
                        String path = dir.path.isEmpty() ? name : dir.path + "/" + name;
                        if (DocumentsContract.Document.MIME_TYPE_DIR.equals(mime)) {
                            if (dir.depth < maxDepth && !SKIP.contains(lower)) {
                                queue.add(new Dir(id, path, dir.depth + 1));
                            }
                        } else if (lower.endsWith(".mpq")) {
                            files.put(new JSONObject()
                                    .put("path", path)
                                    .put("size", cur.isNull(3) ? 0 : cur.getLong(3))
                                    .put("modified", cur.isNull(4) ? 0 : cur.getLong(4))
                                    .put("doc", id));
                        }
                    }
                }
            }
            return new JSONObject()
                    .put("label", displayName(r, treeUri))
                    .put("truncated", truncated)
                    .put("files", files)
                    .toString();
        } catch (Exception e) {
            Log.w(TAG, "folder listing: " + e);
            lastError = e.toString();
            return null;
        }
    }

    /** One document of the folder, as a detached descriptor Rust owns; -1 on failure. */
    static int open(String tree, String docId) {
        Context c = Bridge.app;
        if (c == null || tree == null || docId == null) {
            lastError = "the app is not ready";
            return -1;
        }
        try {
            Uri doc = DocumentsContract.buildDocumentUriUsingTree(Uri.parse(tree), docId);
            ParcelFileDescriptor pfd = c.getContentResolver().openFileDescriptor(doc, "r");
            if (pfd == null) {
                lastError = "the provider gave no file";
                return -1;
            }
            return pfd.detachFd();
        } catch (Exception e) {
            Log.w(TAG, "folder open: " + e);
            lastError = e.toString();
            return -1;
        }
    }

    // ---- Sharing a file -------------------------------------------------------------------------

    /**
     * Offer a file of the app's (a session log) to another app through the share sheet, as a
     * read-only content URI from the FileProvider (res/xml/file_paths.xml) — the one way to get a
     * log off the phone without a computer: no files app can open Android/data.
     */
    static boolean share(String path, String mime) {
        Context c = Bridge.app;
        if (c == null || path == null) return false;
        File f = new File(path);
        if (!f.isFile()) return false;
        Uri uri;
        try {
            uri = FileProvider.getUriForFile(c, c.getPackageName() + ".files", f);
        } catch (IllegalArgumentException e) {
            Log.w(TAG, "share: " + e);
            lastError = e.toString();
            return false;
        }
        Intent send = new Intent(Intent.ACTION_SEND)
                .setType(mime != null ? mime : "application/octet-stream")
                .putExtra(Intent.EXTRA_STREAM, uri)
                .putExtra(Intent.EXTRA_SUBJECT, f.getName())
                .addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION);
        send.setClipData(ClipData.newRawUri(f.getName(), uri));
        Intent chooser = Intent.createChooser(send, null)
                .addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION);
        ui.post(() -> {
            Activity a = Bridge.activity;
            try {
                if (a != null) {
                    a.startActivity(chooser);
                } else {
                    c.startActivity(chooser.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK));
                }
            } catch (ActivityNotFoundException e) {
                Log.w(TAG, "share: " + e);
            }
        });
        return true;
    }
}
