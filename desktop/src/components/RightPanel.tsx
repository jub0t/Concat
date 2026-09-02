// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

import { memo } from "react";

import type { AppliedEffect, ClipTransition } from "../lib/effects";
import type { ClipFilter } from "../lib/filters";
import {
  precedingClip,
  type Clip,
  type CustomFont,
  type EditorProject,
  type MediaItem,
} from "../lib/editor";
import { useLocale, type MsgKey } from "../lib/i18n";
import { AdjustPanel } from "./AdjustPanel";
import { EffectsPanel } from "./EffectsPanel";
import { FiltersPanel } from "./FiltersPanel";
import { Inspector } from "./Inspector";
import inspector from "./inspector/inspector.module.css";
import { Panel } from "./Panel";
import { TextPanel } from "./TextPanel";

export type RightTab = "details" | "adjust" | "filters" | "effects" | "text";

/**
 * The tab strip follows the selection.
 *
 * Nothing selected: only Details, showing the project itself - there is
 * nothing to edit, so no editing tabs. A media clip selected: Adjust and
 * Filters, the things that can actually be changed. A title: just Text,
 * because a text clip has no volume, no speed and no audio filters, and
 * offering those tabs would be offering panels that can only say "nothing
 * here".
 */
const DETAILS_TABS: { id: RightTab; labelKey: MsgKey }[] = [
  { id: "details", labelKey: "rightPanel.details" },
];

const CLIP_TABS: { id: RightTab; labelKey: MsgKey }[] = [
  { id: "adjust", labelKey: "rightPanel.adjust" },
  { id: "filters", labelKey: "rightPanel.filters" },
  { id: "effects", labelKey: "rightPanel.effects" },
];

// A still has picture but no sound, so it gets Effects but not Filters.
const IMAGE_TABS: { id: RightTab; labelKey: MsgKey }[] = [
  { id: "adjust", labelKey: "rightPanel.adjust" },
  { id: "effects", labelKey: "rightPanel.effects" },
];

const TEXT_TABS: { id: RightTab; labelKey: MsgKey }[] = [{ id: "text", labelKey: "rightPanel.text" }];

/**
 * The right-hand panel.
 *
 * Details is what a thing *is*, Adjust is what you can change about it, and
 * Filters is what you can add to it. Keeping those apart matters because the
 * first is read-only and the other two are not, and mixing them makes it
 * unclear which numbers you are allowed to touch.
 *
 * Memoised: playback re-renders the editor per animation frame, and nothing
 * here reads the playhead - the panel shows the selection, not the clock.
 * App.tsx keeps every callback and derived prop referentially stable
 * (useCallback / useMemo / objects straight off the memoised project).
 */
export const RightPanel = memo(function RightPanel({
  tab,
  onTab,
  clip,
  media,
  project,
  fonts,
  projectName,
  projectPath,
  frame,
  duration,
  frameRate,
  onChangeClip,
  onCommitClip,
  onSpeedChange,
  onAddFont,
  onRemoveFont,
  onModifyProject,
}: {
  tab: RightTab;
  onTab: (tab: RightTab) => void;
  clip: Clip | null;
  media: MediaItem | null;
  project: EditorProject;
  /** Custom fonts with the UI's missing-file marks applied. */
  fonts: CustomFont[];
  projectName: string;
  projectPath: string;
  frame: { width: number; height: number };
  duration: number;
  frameRate: number;
  onChangeClip: (patch: Partial<Clip>) => void;
  /** Ends a control gesture: the accumulated change becomes one command. */
  onCommitClip: () => void;
  onSpeedChange: (speed: number) => void;
  onAddFont: () => void;
  onRemoveFont: (family: string) => void;
  /** Opens the project-details editor (name, output frame). */
  onModifyProject: () => void;
}) {
  const { t } = useLocale();
  const isText = clip?.kind === "text";
  const tabs = clip
    ? isText
      ? TEXT_TABS
      : clip.kind === "image"
        ? IMAGE_TABS
        : CLIP_TABS
    : DETAILS_TABS;

  // A selection change can leave the strip showing a tab that is no longer
  // offered, so the active tab falls back to the first one that exists.
  const active = tabs.some((entry) => entry.id === tab) ? tab : tabs[0].id;
  // A segmented control with one segment is a label wearing a costume; a
  // lone tab renders as the panel's plain heading instead.
  const single = tabs.length === 1;

  return (
    // The Text tab is the first panel on the new inspector chrome, which brings
    // its own palette rather than borrowing this app's. `inspector.shell` hands
    // that palette to the heading strip above it too, so the panel is one
    // surface instead of a light header sitting on a near-black body.
    <Panel
      title={single ? t(tabs[0].labelKey) : undefined}
      className={active === "text" ? inspector.shell : ""}
    >
      {!single && (
        <div className="sticky top-0 z-10 border-b border-hairline bg-panel px-2 pb-2 pt-2">
          <div className="flex rounded-lg bg-sunken p-0.5">
            {tabs.map((entry) => (
              <button
                key={entry.id}
                type="button"
                aria-pressed={active === entry.id}
                onClick={() => onTab(entry.id)}
                className={`flex-1 cursor-pointer rounded-[6px] px-2 py-1 text-[12px] transition-colors ${
                  active === entry.id
                    ? "bg-panel text-primary shadow-[0_1px_2px_rgba(0,0,0,0.14)]"
                    : "text-secondary hover:text-primary"
                }`}
              >
                {t(entry.labelKey)}
              </button>
            ))}
          </div>
        </div>
      )}

      {active === "details" && (
        <Inspector
          clip={clip}
          media={media}
          frameRate={frameRate}
          project={project}
          projectName={projectName}
          projectPath={projectPath}
          frame={frame}
          duration={duration}
          onModify={onModifyProject}
        />
      )}
      {active === "text" && (
        <TextPanel
          clip={clip}
          fonts={fonts}
          onChange={onChangeClip}
          onCommit={onCommitClip}
          onAddFont={onAddFont}
          onRemoveFont={onRemoveFont}
        />
      )}
      {active === "adjust" && (
        <AdjustPanel
          clip={clip}
          onChange={onChangeClip}
          onCommit={onCommitClip}
          onSpeedChange={onSpeedChange}
        />
      )}
      {active === "filters" && (
        <FiltersPanel
          clip={clip}
          onChange={(filters: ClipFilter[]) => onChangeClip({ filters })}
          onCommit={onCommitClip}
        />
      )}
      {active === "effects" && (
        <EffectsPanel
          clip={clip}
          hasPreceding={clip ? precedingClip(project, clip.id) !== null : false}
          onChangeEffects={(videoEffects: AppliedEffect[]) => onChangeClip({ videoEffects })}
          onChangeTransition={(transitionIn: ClipTransition | undefined) =>
            onChangeClip({ transitionIn })
          }
          onCommit={onCommitClip}
        />
      )}
    </Panel>
  );
});
