# Keyboard shortcuts

Shortcuts work when the editor window has focus. On macOS, `Ctrl` below means `⌘`.

These are the keys Concat ships with. Every one in the tables below except the window's own (see [Fixed keys](#fixed-keys)) can be changed in **Settings > Shortcuts**: click a row, press the new keys, or press `Escape` to cancel. A key another action already has moves to the new one, and the old action is left with no key until its row's Reset gives it back; **Reset all** puts every key back as shipped. The menus and the timeline's tooltips show the keys as they are now bound. The Shortcuts page is a desktop one; phones have no keyboard to bind.

Single-key shortcuts don't fire while typing in a text field.

On the start screen only `Ctrl` + `O`, `Ctrl` + `Q` (Windows/Linux) and `Ctrl` + `W` (macOS) work. `Escape` closes the sheet or dialog on top.

## Playback and navigation

| Shortcut | Action |
| --- | --- |
| `Space` | Play / pause |
| `Home` / `End` | Go to start / end of timeline |
| `←` / `→` | Step one frame back / forward |
| `Shift` + `←` / `→` | Step ten frames back / forward (follows whatever key steps one) |

## Editing

| Shortcut | Action |
| --- | --- |
| `Ctrl` + `B` | Split the selected clips at the playhead (every clip under it when nothing is selected) |
| `B` | Toggle the split (razor) tool |
| `Q` | Delete left: cut the clip from its start up to the playhead |
| `W` | Delete right: cut the clip from the playhead to its end |
| `F` | Freeze frame at the playhead |
| `Delete` / `Backspace` | Delete selection |
| `Shift` + `Delete` / `Backspace` | Ripple delete (close the gap) |
| `Ctrl` + `Z` | Undo |
| `Ctrl` + `Shift` + `Z`, or `Ctrl` + `Y` on Windows/Linux | Redo |
| `Ctrl` + `A` | Select all |
| `Ctrl` + `C` / `Ctrl` + `V` | Copy / paste |
| `Ctrl` + `D` | Duplicate the selected clip |
| `H` | Toggle the pan (hand) tool |
| `N` | Toggle snapping |
| `M` | Mute / unmute the selected clip |
| `S` | Toggle preview axis overlay |

`Q` and `W` are CapCut's keys for the same thing. They cut the selected clips the playhead is over, as one undo step; with nothing selected they cut the one clip under the playhead, and ask you to select one when there are several. On a magnetic timeline the gap closes up, and after `Q` the playhead lands on the frame it was showing. Both are also in a clip's right-click menu, which is the **More** menu on a phone.

## Project and app

| Shortcut | Action |
| --- | --- |
| `Ctrl` + `S` | Save project |
| `Ctrl` + `E` | Export |
| `Ctrl` + `I` | Import media |
| `Ctrl` + `,` | Settings |
| `Ctrl` + `=` / `Ctrl` + `+` | Zoom timeline in |
| `Ctrl` + `-` | Zoom timeline out |

## Fixed keys

These belong to the window and can't be rebound; Settings > Shortcuts refuses them.

| Shortcut | Action |
| --- | --- |
| `Ctrl` + `O` | Open project |
| `Ctrl` + `W` | Close project (close window on macOS) |
| `Ctrl` + `Q` | Quit (Windows/Linux) |
| `F10`, `Alt` + `F` / `E` / `V` | Open app menus (Windows/Linux) |
| `Escape` | Close the sheet on top; cancel a rebind in Settings > Shortcuts |
