# ToDo

DO NOT DELETE ANY OF THE TODOs, JUST MARK THEM DONE.

### QoL or UX features
- [ ] Add a copy button to the red "error" modals and popups. So issues, bugs, errors can be easily reported and communicated.
- [x] add mobile (ios and android) artifacts to nightly builds too.
- [ ] The ability to move/reposition timelines on the timeline tray. For example if one has three different timelines they can drag and drop the first between, after, or before the other timelines thereby reordering the order.
- feature: add langauge support locales and fast i18n support. Locales live in editable json files.
- [ ] in Library>Media panel: the audio waveform pattern has padding/margin on the x-axis which should be removed. we need the waveform pattern to start at the very begining of the audio preview card.
- [ ] The timeline tools tray needs a revamp, a lot of people wont be able to find tools like Text to Speech if they are placed in tabs in File (macOS). We need to show more tools (or dropdowns) on the Timeline tools tray.
- [ ] We need to add texts presets. In Library>Text, create a new tab/dropdown under Text in the sidebar called Presets where pre-designed Texts profiles/configs are shown and can be used immediately. Text profiles use typography settings like font, size, etc. For font we create a consistent/idempotent copy where if a Text preset is distributed openly among the users/community the "Installer" checks if the new text preset-profile font is already cached/installed/imported for used in Concat, else it installs the font bundled with the Text preset/profile. Each profile has a unique name/identifier.
- [ ] In Library>Transitions>Basic: use the same light/thinner font-weight as Transitions>Motion.
- [ ] Remove the "Concat" watermark from the bottom-left of the Preview.
- [ ] Add basic CapCut-like keybinds to the app. Like for delete, CTRL+B to toggle split tool, etc.
- [ ] Recheck and fix Export system/modal. Things like 9:16 export output is broken and does not work.
- [ ] in Settings>Appearance: remove the "Dark is the edit's own room" text-paragraph.
- [ ] Change the Timeline>Timestamps-line tray background color from dark to the same as Tracks section background.
- [ ] Inside Library>Media>Images(for images): replace the "still" label with the format/mimetype of the Image (eg png, jpg, etc).
- [ ] For "Generate Auto-Captions" and "Text-to-Speech" feature, instead of directly generating the output, show a modal to let the user decide what configs they want. Design examples: wc-ui-rnd/web/ -> Waiting & Text to speech sections.
- [ ] In Inspector: for panels with lots of Subsections, give the "header" of each subsection a lighter background color than the rest of the background (kind of like in wc-ui-rnd/web/ -> Editor panels>Text panel expandable accordion-like subsections).
- [ ] For things like filter: use the other rectangular green-fill input-range slider like in wc-ui-rnd/web/->Inspectors>Text panel.
- [ ] ux: For the right-click context menu on timelines, increase the padding and increase the border-radius.
- [ ] fix: on Inspector>Details>Modify Button: Create an actual modal for the Modify button and wire it in so users can actually modify project settings.
- [ ] fix: The details panel does not render correct data, fix it.
- [ ] tweak: in Timelines the Text object should have Small height and instead of rendering the Text in a top header and then the body we can just still to header. This way we save more space and don't render the text preview twice on its timeline card.
- [ ] ux qol: For Library>Media>{Videos,Images}: grabbing (mouse button down) any visual media should immediately show its "floating preview" and not wait for the first-drag, it makes the app look snappier and faster.
- [ ] ux: remove the "Concat" label from the right-click context menu that shows on timelines.
- [ ] fix: a lot of inputs have a bug where when they are focused and you click somewhere else to de-focus, it does not work. fix that.
- [ ] feature: right now visual-media (images, videos) can only be scaled (resized on both dimensions propotionally). We need to add a feature to allow single-axis scale/stretch as well using the 2d bounding box in preview.
- [ ] feature: add a "resize/scale" snapping too along with translate snapping.
- [ ] we need to add a "Remove BG" toggle feature in Inspector for Images and Videos, enabling this removes the media's background like how CapCut does (CapCut has different options like Auto Removal, Custom Removal, Chroma key, etc.). Custom removal gives the user Smart Brush, Brush, Smart Eraser, Eraser to outline the target.