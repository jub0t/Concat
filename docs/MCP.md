# Concat for AI agents (MCP)

Concat ships an [MCP](https://modelcontextprotocol.io) server inside the app. An agent - Claude Code, Claude Desktop, or any other MCP client - starts it as `concat --mcp` and can then create projects, import media, cut, add titles and captions, keyframe, apply effects, look at frames and export, with the same edit commands the window uses. Nothing extra to install.

## Set it up

Point your client at the Concat binary with the one argument `--mcp`.

| System | Command |
| --- | --- |
| macOS | `/Applications/Concat.app/Contents/MacOS/concat` |
| Windows (installed for you) | `%LOCALAPPDATA%\Programs\Concat\concat.exe` |
| Windows (installed for everyone, or the .msi) | `C:\Program Files\Concat\concat.exe` |
| Linux (.deb, .rpm, Arch) | `/usr/bin/concat` |
| Linux (AppImage) | the `.AppImage` file itself |

**Claude Code:**

```sh
claude mcp add concat -- /Applications/Concat.app/Contents/MacOS/concat --mcp
```

**Claude Desktop** (Settings > Developer > Edit Config, `claude_desktop_config.json`), and most other clients, which take the same shape:

```json
{
  "mcpServers": {
    "concat": {
      "command": "/Applications/Concat.app/Contents/MacOS/concat",
      "args": ["--mcp"]
    }
  }
}
```

On Windows, write the path with doubled backslashes in JSON: `"C:\\Program Files\\Concat\\concat.exe"`.

## How it reaches Concat

`concat --mcp` works one of two ways, and the `concat_version` tool says which (`mcp.mode`):

- **Through the window** (`window`). When Concat is open with **Settings > Automation > Scripts and apps** on, the window writes where its server is, and its token, to `automation.json` in Concat's config folder (readable only by you). The MCP server finds it and goes through the window's server. This is the safer way while you are editing: the agent cannot save over the project you have on screen behind your back, and exports take turns with yours.
- **On its own** (`headless`). Otherwise the MCP server runs Concat's engine itself, with no window. Nothing has to be open. Every Concat process locks the project folders it has open (`concat.lock`, let go on close or when the process dies), so a project open in the window is refused here with a note to turn on Automation and use `--mcp window`, and the window refuses a project an agent has open.

Choose explicitly with `--mcp window` (fail rather than run on its own) or `--mcp headless`; the default is `--mcp auto`. To reach a server elsewhere, set `CONCAT_API_ADDRESS` (e.g. `127.0.0.1:7420`) and `CONCAT_API_TOKEN` in the client's `env` for the server.

Either way the agent writes files - projects, exports, frames - only under your home folder. Set `CONCAT_MCP_ROOTS` (a path list, `:`-separated, `;` on Windows) to allow other folders in headless mode.

### The project on screen

Through the window, an agent can edit the project you have open. When it opens that project (`project_open`), the window saves it, closes it and shows **Remote edit**: nothing in the window responds until the agent is done. When the agent closes the project (`project_close`, with `save: true` to keep its edits), the window opens it again from its folder, with those edits in it.

**Take back** on that sheet ends it from your side: the agent's edits so far are saved, its project is closed and the window opens it again. Use it when an agent stops without closing the project.

What it needs:

- **Settings > Automation > Scripts and apps** on in the window.
- The agent's server in `auto` or `window` mode. A client entry that sets `CONCAT_MCP_MODE=headless` never reaches the window; remove it.

Through the window, `CONCAT_MCP_ROOTS` does not apply: an open project is edited and saved wherever its folder is, while a new project, an export or a saved frame must land under your home folder.

What to know:

- The window saves before it lends. If that save fails, the project stays open in the window and the agent is refused with the reason.
- The window opens no other project while one is lent.
- An agent's edits live in memory until it saves. Edits it has not saved are lost if Concat quits or the Automation server restarts while the project is lent; the window then opens the project as last saved. **Take back** saves them first.
- If the agent's own save fails on **Take back**, the project stays lent and the window says why.

On its own (`headless`) the agent still cannot open the project on screen; it is refused as above. Watching the agent's edits arrive one by one while you keep working ("live mode") is planned in [#337](https://github.com/jub0t/Concat/issues/337).

## What the agent can do

Tools are named for what they do, and their argument schemas are generated from the Concat API's own types, so they carry the same limits and descriptions the engine does.

| Area | Tools |
| --- | --- |
| Projects | `project_list`, `project_create`, `project_open`, `project_save`, `project_close`, `project_set_video` |
| Reading | `timeline_read` (compact: lanes, clips in time order, media, ids), `project_get` (everything), `concat_version` |
| Media | `media_probe`, `media_import` |
| Editing | `clip_add`, `clips_add_sequence`, `clip_trim`, `clips_split`, `clips_move`, `clips_move_to_new_lane`, `clips_remove`, `clip_update`, `clip_transform`, `clip_speed`, `clip_frame`, `track_add`, `timeline_linkage` |
| Picture | `clip_masks`, `mask_key_set`, `clip_stabilize`, `timeline_background` |
| Titles and captions | `title_add`, `shapes_list` (the Stickers cards and the style each places), `title_style_words` (some words in another colour, weight, size...), `title_clear_word_styles`, `captions_add` (on a caption lane of their own; restyle them all with `edit_apply` and `setCaptionLook`) |
| Keyframes and effects | `keyframe_set`, `keyframe_clear`, `effects_list`, `effect_layer_add`, `effect_key_set` |
| Anything else | `edit_apply` (any edit command, or a batch of them as one undo step), `edit_undo`, `edit_redo` |
| Templates | `template_list`, `template_instantiate` |
| Export | `export_start`, `export_status`, `export_wait` (with progress), `export_cancel` |
| Looking | `preview_frame` (the composited frame as an image), `project_cover` (the project's cover as a PNG), `clips_bounds` (where each title and shape is drawn) |

Edits stay in memory until `project_save`, as in the window. Times are seconds; a keyframe's position is a fraction of its clip (0 to 1); sizes and offsets are fractions of the frame.

## Protocol

MCP over stdio, newline-delimited JSON-RPC 2.0. Both protocol eras are served: the `initialize` handshake (2025-11-25, 2025-06-18, 2025-03-26, 2024-11-05) and the per-request `_meta` of 2026-07-28 with `server/discover`. `ping`, `tools/list`, `tools/call`, progress notifications for `export_wait`, and `notifications/cancelled` are supported. Standard output carries only protocol; the server logs to standard error.

For scripts that do not speak MCP, the same API is served as JSON-RPC lines on a socket from Settings > Automation; see [the developer docs](https://concat.standalones.org/docs).
