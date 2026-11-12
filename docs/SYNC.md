# Edit a project on Windows and Android

Concat stores a project's manifest and its media together in the project folder. Syncthing can
copy that folder between a Windows computer and an Android device, so either device can open the
same project without relinking its videos and audio.

## Set up the shared folder

1. Install Syncthing on the Windows computer and Android device, connect the devices, and share a
   folder between them. Choose a folder the Android device can access, such as `Sync/Concat`.
2. In Syncthing on each device, confirm the folder is connected and fully synchronized before
   opening a project there.
3. Create or move each Concat project into its own folder inside the shared folder. Keep the
   project's `concat.json` and its `media/` folder together.
4. On Windows, open the project from that folder. Media and custom fonts you import are copied
   into the project's `media/` folder. For an older project, choose **Collect media into project**
   once and save it before opening it on Android.
5. On Android, choose **Open**. If Android opens all-files access settings, allow access, return to
   Concat, and choose **Open** again. Pick the shared folder and the project folder inside it.
   Concat imports phone media into the open project's `media/` folder.

## Ignore generated files

In Syncthing, add these ignore patterns to the shared folder's `.stignore` file at its root:

```text
**/concat.lock
**/concat.json.saving.*
**/cache/art
**/cache/audio
**/cache/proxy
**/cache/masks
**/cache/stabilize
**/cache/voice
**/cache/*.peaks
**/cache/poster.jpg
**/cache/missing_media.log
```

Do not ignore `cache/reverse-*` or `cache/enhance-*`. Those files are media used by the project,
not disposable cache.

## Edit on one device at a time

Do not edit the same project on both devices at once. Save and close it on the first device, wait
for Syncthing to finish syncing, and only then open it on the other device. The project lock is
local to each device and does not coordinate edits across devices.

If a file named `concat.sync-conflict-*.json` appears, both devices changed the project before
their edits synchronized. Stop editing, keep a copy of both versions, and recover the intended
changes before continuing; Syncthing cannot merge project edits for Concat.
