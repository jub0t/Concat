// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

import { memo, useEffect, useRef, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import type { PointerEvent as ReactPointerEvent, ReactNode } from "react";

import type { MediaAssets } from "../lib/assets";
import { subscribeAssets } from "../lib/assets";
import {
  EFFECT_CATEGORIES,
  EFFECTS,
  TRANSITION_CATEGORIES,
  TRANSITIONS,
  type EffectCategory,
  type TransitionCategory,
} from "../lib/effects";
import type { MediaItem } from "../lib/editor";
import { t, useLocale, type MsgKey } from "../lib/i18n";
import { templateDelete, templateList, type TemplateInfo } from "../lib/engine";
import { themeColor } from "../lib/theme";
import { shortDuration } from "../lib/time";
import { ErrorNotice } from "./ErrorNotice";
import { Icon } from "./Icon";
import { Empty, Panel } from "./Panel";
import { TemplateThumb } from "./TemplateThumb";

/** Which kinds of media the bin is showing. */
export interface BinFilter {
  video: boolean;
  audio: boolean;
  image: boolean;
}

export const ALL_MEDIA: BinFilter = { video: true, audio: true, image: true };

/** The pages of the library strip, in display order. */
type LibraryTab = "media" | "text" | "transitions" | "effects" | "templates";

const TABS: { id: LibraryTab; labelKey: MsgKey }[] = [
  { id: "media", labelKey: "mediaBin.tabMedia" },
  { id: "text", labelKey: "mediaBin.tabText" },
  { id: "transitions", labelKey: "mediaBin.tabTransitions" },
  { id: "effects", labelKey: "mediaBin.tabEffects" },
  { id: "templates", labelKey: "mediaBin.tabTemplates" },
];

/**
 * The library panel.
 *
 * A strip of pages across the top - Media, Text, Transitions, Effects - then a
 * full-width divider, then the page itself split in two: a narrow category
 * sidebar on the left, and the browsing area on the right where media, effect
 * previews and so on are shown.
 *
 * Media is fully live. Text offers the one title style that exists.
 * Transitions and Effects browse the catalogues in `lib/effects.ts` and apply
 * to the selected clip; only the motion transitions (wipe, push, zoom) still
 * say "Soon", waiting on animated transforms in the engine. Templates lists
 * saved bundles and hands off to the launch screen's fill flow.
 *
 * Media items are drag sources: dragging one onto the timeline places a clip
 * at the drop position. That is the primary way material gets into an edit, so
 * it needs to be the obvious one - hence the whole card being the drag handle
 * rather than some grip affordance.
 *
 * Memoised: the editor re-renders per animation frame during playback and
 * nothing in the library depends on the playhead. App.tsx keeps every
 * callback here referentially stable (useCallback) so the memo actually
 * skips those renders.
 */
export const MediaBin = memo(function MediaBin({
  items,
  filter,
  selectedIds,
  busy,
  error,
  onFilter,
  onAddText,
  onSelect,
  onImport,
  onRemove,
  onRemoveSelected,
  onDismissError,
  onBeginDrag,
  onAddToTimeline,
  onApplyEffect,
  onApplyTransition,
  onToggleSlot,
  onUseTemplate,
  onSaveTemplate,
  assets,
  dropping,
}: {
  items: MediaItem[];
  filter: BinFilter;
  /** A file is being dragged over the window from outside. */
  dropping: boolean;
  /** Shared with the timeline, so a waveform is only ever computed once. */
  assets: MediaAssets;
  selectedIds: readonly string[];
  busy: boolean;
  error: string | null;
  onFilter: (filter: BinFilter) => void;
  /** Puts a new title on the timeline at the playhead. */
  onAddText: () => void;
  /** Replaces the bin selection; cards and the marquee both call it. */
  onSelect: (ids: string[]) => void;
  onImport: (paths: string[]) => void;
  onRemove: (id: string) => void;
  /** Deletes everything in the current bin selection, as one undo step. */
  onRemoveSelected: () => void;
  onDismissError: () => void;
  /** Called once the pointer has moved far enough to count as a drag. */
  onBeginDrag: (item: MediaItem, clientX: number, clientY: number) => void;
  onAddToTimeline: (mediaId: string) => void;
  /** Applies an effect to the selected clip; the app validates and explains. */
  onApplyEffect: (effectId: string) => void;
  /** Puts a transition on the selected clip's cut; the app validates. */
  onApplyTransition: (transitionId: string) => void;
  /** Marks or unmarks a media item as a template slot. */
  onToggleSlot: (mediaId: string, placeholder: boolean) => void;
  /** Takes the user to the fill flow for one template. */
  onUseTemplate: (template: TemplateInfo) => void;
  /** Opens the save-as-template sheet for the current project. */
  onSaveTemplate: () => void;
}) {
  const [tab, setTab] = useState<LibraryTab>("media");
  const [effectCategory, setEffectCategory] = useState<EffectCategory>("basic");
  const [transitionCategory, setTransitionCategory] = useState<TransitionCategory>("basic");
  const { t } = useLocale();

  return (
    <Panel
      // Dropped files land in the bin, so the bin is what lights up. A
      // full-screen overlay would obscure the editor to say something that
      // only concerns one panel.
      className={dropping ? "ring-2 ring-accent ring-inset" : ""}
    >
      <div className="flex h-full min-h-0 flex-col">
        {/* The page strip. Its bottom border is the full-width divider. */}
        <div className="shrink-0 border-b border-hairline px-2 pb-2 pt-2">
          <div className="flex rounded-lg bg-sunken p-0.5">
            {TABS.map((entry) => (
              <button
                key={entry.id}
                type="button"
                aria-pressed={tab === entry.id}
                onClick={() => setTab(entry.id)}
                // flex-auto, not flex-1: extra space is shared, but each tab's
                // base is its own label, so "Transitions" is never crammed to
                // make room for padding around "Text".
                className={`min-w-0 flex-auto cursor-pointer truncate rounded-md px-1.5 py-1
                            text-[11px] transition-colors ${
                              tab === entry.id
                                ? "bg-panel text-primary shadow-[0_1px_2px_rgba(0,0,0,0.14)]"
                                : "text-secondary hover:text-primary"
                            }`}
              >
                {t(entry.labelKey)}
              </button>
            ))}
          </div>
        </div>

        {/* The page: category sidebar on the left, browsing area on the right. */}
        <div className="flex min-h-0 flex-1">
          <aside className="thin-scroll w-26 shrink-0 overflow-y-auto border-r border-hairline py-1">
            {tab === "media" && <MediaCategories items={items} filter={filter} onFilter={onFilter} />}
            {tab === "text" && (
              <CategoryGroup title={t("mediaBin.tabText")}>
                <CategoryRow label={t("mediaBin.titles")} selected onSelect={() => {}} />
              </CategoryGroup>
            )}
            {tab === "transitions" && (
              <CategoryGroup title={t("mediaBin.tabTransitions")}>
                {TRANSITION_CATEGORIES.map((category) => (
                  <CategoryRow
                    key={category.id}
                    label={category.label}
                    selected={transitionCategory === category.id}
                    onSelect={() => setTransitionCategory(category.id)}
                  />
                ))}
              </CategoryGroup>
            )}
            {tab === "effects" && (
              <CategoryGroup title={t("mediaBin.videoEffects")}>
                {EFFECT_CATEGORIES.map((category) => (
                  <CategoryRow
                    key={category.id}
                    label={category.label}
                    selected={effectCategory === category.id}
                    onSelect={() => setEffectCategory(category.id)}
                  />
                ))}
              </CategoryGroup>
            )}
            {tab === "templates" && (
              <CategoryGroup title={t("mediaBin.tabTemplates")}>
                <CategoryRow label={t("mediaBin.myTemplates")} selected onSelect={() => {}} />
              </CategoryGroup>
            )}
          </aside>

          <div className="thin-scroll min-w-0 flex-1 overflow-y-auto">
            {tab === "media" && (
              <MediaPage
                items={items}
                filter={filter}
                selectedIds={selectedIds}
                busy={busy}
                error={error}
                dropping={dropping}
                assets={assets}
                onSelect={onSelect}
                onImport={onImport}
                onRemove={onRemove}
                onRemoveSelected={onRemoveSelected}
                onDismissError={onDismissError}
                onBeginDrag={onBeginDrag}
                onAddToTimeline={onAddToTimeline}
                onToggleSlot={onToggleSlot}
              />
            )}
            {tab === "text" && <TextPage onAddText={onAddText} />}
            {tab === "transitions" && (
              <TransitionsPage category={transitionCategory} onApply={onApplyTransition} />
            )}
            {tab === "effects" && <EffectsPage category={effectCategory} onApply={onApplyEffect} />}
            {tab === "templates" && (
              <TemplatesPage onUse={onUseTemplate} onSaveTemplate={onSaveTemplate} />
            )}
          </div>
        </div>
      </div>
    </Panel>
  );
});

// ── the category sidebar ─────────────────────────────────────────────────────

/**
 * A collapsible dropdown of categories - "Video Effects" with its list under
 * it, and the equivalents on the other pages.
 */
function CategoryGroup({ title, children }: { title: string; children: ReactNode }) {
  const [open, setOpen] = useState(true);
  return (
    <div className="px-1">
      <button
        type="button"
        aria-expanded={open}
        onClick={() => setOpen((current) => !current)}
        className="flex w-full cursor-pointer items-center gap-1 rounded-md px-1.5 py-1.5
                   text-[11px] font-medium text-secondary transition-colors hover:bg-hover
                   hover:text-primary"
      >
        <Icon name={open ? "chevronDown" : "chevronRight"} size={10} className="shrink-0" />
        <span className="min-w-0 flex-1 truncate text-left">{title}</span>
      </button>
      {open && <div className="flex flex-col gap-px pb-1">{children}</div>}
    </div>
  );
}

function CategoryRow({
  label,
  count,
  selected,
  onSelect,
}: {
  label: string;
  count?: number;
  selected: boolean;
  onSelect: () => void;
}) {
  return (
    <button
      type="button"
      aria-pressed={selected}
      onClick={onSelect}
      className={`flex w-full cursor-pointer items-center gap-1 rounded-md py-1 pl-4.5 pr-1.5
                  text-[11px] transition-colors ${
                    selected ? "bg-active text-primary" : "text-secondary hover:bg-hover"
                  }`}
    >
      <span className="min-w-0 flex-1 truncate text-left">{label}</span>
      {count !== undefined && count > 0 && (
        <span className="font-technical text-[10px] text-tertiary">{count}</span>
      )}
    </button>
  );
}

/** One media kind on, the rest off - what a sidebar category means. */
function onlyKind(kind: keyof BinFilter): BinFilter {
  return { video: false, audio: false, image: false, [kind]: true };
}

const KIND_LABEL_KEYS: Record<keyof BinFilter, MsgKey> = {
  video: "mediaBin.kindVideo",
  audio: "mediaBin.kindAudio",
  image: "mediaBin.kindImages",
};

function MediaCategories({
  items,
  filter,
  onFilter,
}: {
  items: MediaItem[];
  filter: BinFilter;
  onFilter: (filter: BinFilter) => void;
}) {
  const kinds = ["video", "audio", "image"] as const;
  const enabled = kinds.filter((kind) => filter[kind]);
  // The old filter menu could leave two of three kinds on; the sidebar has no
  // row for that state, so nothing highlights until a category is clicked.
  const selected = enabled.length === 3 ? "all" : enabled.length === 1 ? enabled[0] : null;

  const { t } = useLocale();

  return (
    <CategoryGroup title={t("mediaBin.tabMedia")}>
      <CategoryRow
        label={t("mediaBin.allMedia")}
        count={items.length}
        selected={selected === "all"}
        onSelect={() => onFilter(ALL_MEDIA)}
      />
      {kinds.map((kind) => (
        <CategoryRow
          key={kind}
          label={t(KIND_LABEL_KEYS[kind])}
          count={items.filter((item) => item.kind === kind).length}
          selected={selected === kind}
          onSelect={() => onFilter(onlyKind(kind))}
        />
      ))}
    </CategoryGroup>
  );
}

// ── the media page ───────────────────────────────────────────────────────────

/** Cards sized for the browsing area, however narrow the panel is dragged. */
const CARD_GRID = "grid grid-cols-[repeat(auto-fill,minmax(88px,1fr))] gap-2 px-2 pb-2";

function MediaPage({
  items,
  filter,
  selectedIds,
  busy,
  error,
  dropping,
  assets,
  onSelect,
  onImport,
  onRemove,
  onRemoveSelected,
  onDismissError,
  onBeginDrag,
  onAddToTimeline,
  onToggleSlot,
}: {
  items: MediaItem[];
  filter: BinFilter;
  selectedIds: readonly string[];
  busy: boolean;
  error: string | null;
  dropping: boolean;
  assets: MediaAssets;
  onSelect: (ids: string[]) => void;
  onImport: (paths: string[]) => void;
  onRemove: (id: string) => void;
  /** Deletes everything in the current bin selection, as one undo step. */
  onRemoveSelected: () => void;
  onDismissError: () => void;
  onBeginDrag: (item: MediaItem, clientX: number, clientY: number) => void;
  onAddToTimeline: (mediaId: string) => void;
  onToggleSlot: (mediaId: string, placeholder: boolean) => void;
}) {
  const { t } = useLocale();

  const browse = async () => {
    if (busy) return;
    const chosen = await open({
      multiple: true,
      title: t("mediaBin.importMedia"),
      filters: [
        {
          name: t("mediaBin.mediaFilter"),
          extensions: [
            "mp4", "mov", "mkv", "webm", "avi", "m4v",
            "mp3", "wav", "flac", "aac", "m4a", "ogg", "opus",
            "png", "jpg", "jpeg", "webp", "bmp", "tif", "tiff", "avif", "gif",
          ],
        },
        { name: t("mediaBin.allFilesFilter"), extensions: ["*"] },
      ],
    });

    // The picker hands back a string for one file and an array for many.
    if (Array.isArray(chosen)) onImport(chosen);
    else if (typeof chosen === "string") onImport([chosen]);
  };

  /**
   * Marquee selection over the card grid.
   *
   * A press on the grid's background (never on a card - cards are drag
   * sources) starts a band in client coordinates; every move re-derives the
   * selection from the cards' live rects, so scrolled positions and wrapped
   * rows need no bookkeeping. Shift adds to the selection that existed when
   * the band started, like the timeline's marquee.
   */
  const grid = useRef<HTMLDivElement>(null);
  const [band, setBand] = useState<{ x0: number; y0: number; x1: number; y1: number } | null>(
    null,
  );
  const beginMarquee = (event: ReactPointerEvent<HTMLDivElement>) => {
    if (event.button !== 0) return;
    if ((event.target as Element).closest("[data-media-card]")) return;
    const basis = event.shiftKey ? [...selectedIds] : [];
    if (!event.shiftKey) onSelect([]);
    const origin = { x: event.clientX, y: event.clientY };
    setBand({ x0: origin.x, y0: origin.y, x1: origin.x, y1: origin.y });

    const caught = (x: number, y: number) => {
      const left = Math.min(origin.x, x);
      const right = Math.max(origin.x, x);
      const top = Math.min(origin.y, y);
      const bottom = Math.max(origin.y, y);
      const ids: string[] = [];
      grid.current
        ?.querySelectorAll<HTMLElement>("[data-media-card]")
        .forEach((card) => {
          const rect = card.getBoundingClientRect();
          const hit =
            rect.left < right && rect.right > left && rect.top < bottom && rect.bottom > top;
          const id = card.dataset.mediaCard;
          if (hit && id) ids.push(id);
        });
      return ids;
    };

    const onMove = (move: PointerEvent) => {
      setBand({ x0: origin.x, y0: origin.y, x1: move.clientX, y1: move.clientY });
      onSelect([...new Set([...basis, ...caught(move.clientX, move.clientY)])]);
    };
    const onUp = () => {
      window.removeEventListener("pointermove", onMove);
      window.removeEventListener("pointerup", onUp);
      setBand(null);
    };
    window.addEventListener("pointermove", onMove);
    window.addEventListener("pointerup", onUp);
  };

  /**
   * Starts a drag using raw pointer events rather than HTML5 drag-and-drop.
   *
   * HTML5 dragging cannot be used here at all: Tauri's `dragDropEnabled`
   * native handler intercepts every drag over the webview, including ones that
   * begin inside it, so an internal drag registers as an incoming OS file drop
   * and raises the import overlay. Pointer events never involve the OLE drag
   * machinery, so the two stop fighting.
   *
   * The 4px threshold keeps a sloppy click from starting a drag.
   */
  const beginDrag = (event: ReactPointerEvent<HTMLLIElement>, item: MediaItem) => {
    if (event.button !== 0) return;
    const originX = event.clientX;
    const originY = event.clientY;

    const onMove = (moveEvent: PointerEvent) => {
      if (Math.hypot(moveEvent.clientX - originX, moveEvent.clientY - originY) <= 4) return;
      cleanup();
      onBeginDrag(item, moveEvent.clientX, moveEvent.clientY);
    };
    const cleanup = () => {
      window.removeEventListener("pointermove", onMove);
      window.removeEventListener("pointerup", cleanup);
    };

    window.addEventListener("pointermove", onMove);
    window.addEventListener("pointerup", cleanup);
  };

  const visible = items.filter((item) => filter[item.kind]);

  return (
    // The grid area is the marquee surface; pressing empty space drags a
    // selection band, pressing a card starts a media drag.
    <div ref={grid} onPointerDown={beginMarquee} className="flex min-h-full flex-col">
      <div className="px-2 pb-2 pt-2">
        <button
          type="button"
          onClick={() => void browse()}
          disabled={busy}
          className="flex w-full cursor-pointer items-center justify-center gap-2 rounded-lg
                     border border-dashed border-hairline-strong px-3 py-2.5 text-xs text-secondary
                     transition-colors hover:border-accent hover:bg-hover hover:text-primary
                     disabled:cursor-not-allowed disabled:opacity-40"
        >
          <Icon name="import" size={14} />
          {busy ? t("mediaBin.importing") : t("mediaBin.importMedia")}
        </button>
      </div>

      {selectedIds.length > 1 && (
        <div className="mx-2 mb-2 flex items-center justify-between rounded-lg bg-sunken
                        px-2.5 py-1.5">
          <span className="text-[11px] text-secondary">
            {t("mediaBin.selectedCount", { count: selectedIds.length })}
          </span>
          <button
            type="button"
            onClick={onRemoveSelected}
            className="cursor-pointer rounded px-2 py-0.5 text-[11px] font-medium text-danger
                       transition-colors hover:bg-danger-soft"
          >
            {t("mediaBin.delete")}
          </button>
        </div>
      )}

      {error && <ErrorNotice message={error} onDismiss={onDismissError} className="mx-2 mb-2" />}

      {visible.length === 0 ? (
        <Empty
          icon={
            <Icon
              name="import"
              size={28}
              strokeWidth={1.5}
              className={dropping ? "text-accent" : ""}
            />
          }
        >
          {dropping ? t("mediaBin.releaseToImport") : t("mediaBin.dropHint")}
        </Empty>
      ) : (
        <ul className={CARD_GRID}>
          {visible.map((item) => (
            <li
              key={item.id}
              data-media-card={item.id}
              onPointerDown={(event) => beginDrag(event, item)}
              onClick={(event) => {
                const additive = event.shiftKey || event.metaKey || event.ctrlKey;
                onSelect(
                  additive
                    ? selectedIds.includes(item.id)
                      ? selectedIds.filter((id) => id !== item.id)
                      : [...selectedIds, item.id]
                    : [item.id],
                );
              }}
              onDoubleClick={() => onAddToTimeline(item.id)}
              // The details that used to sit on a second line live here now.
              // They are worth having, but not worth a permanent row each.
              title={`${item.name}\n${describe(item)}\n${t("mediaBin.cardHint")}`}
              className="group cursor-grab select-none active:cursor-grabbing"
            >
              <div
                className={`relative overflow-hidden rounded-lg transition-shadow ${
                  selectedIds.includes(item.id)
                    ? "ring-2 ring-accent"
                    : "ring-1 ring-hairline group-hover:ring-hairline-strong"
                }`}
              >
                <MediaThumb item={item} assets={assets} />

                <span
                  className="pointer-events-none absolute left-1 top-1 flex h-5 w-5
                             items-center justify-center rounded text-white"
                  // The kind's own hue, matching the clip it becomes on a track.
                  style={{ background: KIND_TINTS[item.kind].mark }}
                >
                  <Icon
                    name={item.kind === "video" ? "film" : item.kind === "image" ? "image" : "music"}
                    size={11}
                  />
                </span>

                <span className="pointer-events-none absolute bottom-1 right-1 rounded bg-black/55
                                 px-1 py-px font-technical text-[10px] text-white">
                  {item.kind === "image" ? t("mediaBin.still") : shortDuration(item.duration)}
                </span>

                {/* The slot state stays visible; only the controls are hover-only.
                    A template's whole point is which items are slots. */}
                {item.placeholder && (
                  <span className="pointer-events-none absolute bottom-1 left-1 flex items-center
                                   gap-1 rounded bg-accent px-1 py-px font-technical text-[10px]
                                   text-on-accent">
                    <Icon name="slot" size={9} />
                    {t("mediaBin.slotBadge")}
                  </span>
                )}

                <button
                  type="button"
                  aria-pressed={item.placeholder ?? false}
                  aria-label={
                    item.placeholder
                      ? t("mediaBin.unmarkSlot", { name: item.name })
                      : t("mediaBin.markSlot", { name: item.name })
                  }
                  title={
                    item.placeholder ? t("mediaBin.unmarkSlotHint") : t("mediaBin.markSlotHint")
                  }
                  onClick={(event) => {
                    event.stopPropagation();
                    onToggleSlot(item.id, !item.placeholder);
                  }}
                  className={`absolute right-7 top-1 cursor-pointer rounded p-1 text-white
                              transition-colors hover:bg-accent ${
                                item.placeholder
                                  ? "bg-accent"
                                  : "invisible bg-black/55 group-hover:visible"
                              }`}
                >
                  <Icon name="slot" size={10} />
                </button>

                <button
                  type="button"
                  aria-label={t("mediaBin.removeItem", { name: item.name })}
                  title={t("mediaBin.removeFromBin")}
                  onClick={(event) => {
                    event.stopPropagation();
                    onRemove(item.id);
                  }}
                  className="invisible absolute right-1 top-1 cursor-pointer rounded bg-black/55 p-1
                             text-white transition-colors hover:bg-danger group-hover:visible"
                >
                  <Icon name="close" size={10} />
                </button>
              </div>

              <span className="mt-1 block truncate text-[11px] leading-tight text-secondary">
                {item.name}
              </span>
            </li>
          ))}
        </ul>
      )}

      {band && (
        <div
          aria-hidden
          className="pointer-events-none fixed z-40 border"
          style={{
            left: Math.min(band.x0, band.x1),
            top: Math.min(band.y0, band.y1),
            width: Math.abs(band.x1 - band.x0),
            height: Math.abs(band.y1 - band.y0),
            background: "var(--color-accent-soft)",
            borderColor: "var(--color-accent)",
          }}
        />
      )}
    </div>
  );
}

/** The second line that used to be shown for every item, now on hover. */
function describe(item: MediaItem): string {
  if (item.kind === "audio") return item.audioCodec ?? t("mediaBin.audioFallback");
  const size = `${item.width}x${item.height}`;
  return item.kind === "image"
    ? t("mediaBin.sizeStill", { size })
    : `${size} · ${item.frameRate?.toFixed(2)} fps`;
}

// ── the text page ────────────────────────────────────────────────────────────

function TextPage({ onAddText }: { onAddText: () => void }) {
  const { t } = useLocale();
  return (
    <ul className={`${CARD_GRID} pt-2`}>
      <li
        onClick={onAddText}
        onDoubleClick={onAddText}
        title={t("mediaBin.addTitleHint")}
        className="group cursor-pointer select-none"
      >
        <div
          className="relative flex aspect-video items-center justify-center overflow-hidden
                     rounded-lg ring-1 ring-hairline transition-shadow
                     group-hover:ring-hairline-strong"
          // Titles wear the rose the timeline gives them.
          style={{ background: "var(--color-clip-text-body)" }}
        >
          <span className="text-xl font-semibold" style={{ color: "var(--color-clip-text)" }}>
            Aa
          </span>
        </div>
        <span className="mt-1 block truncate text-[11px] leading-tight text-secondary">
          {t("mediaBin.defaultTitle")}
        </span>
      </li>
    </ul>
  );
}

// ── the transitions and effects pages ────────────────────────────────────────

/**
 * A catalogue card. Clickable when the thing it offers actually works;
 * otherwise the "Soon" badge stays - the honest version of a disabled drag,
 * kept for the motion transitions the engine cannot animate yet.
 */
function CatalogueCard({
  label,
  blurb,
  onApply,
  children,
}: {
  label: string;
  blurb: string;
  /** Absent means not implemented yet - the card browses but does nothing. */
  onApply?: () => void;
  children: ReactNode;
}) {
  const { t } = useLocale();
  return (
    <li
      title={`${label}\n${blurb}\n${
        onApply ? t("mediaBin.applyHint") : t("mediaBin.notAvailable")
      }`}
      onClick={onApply}
      className={`group select-none ${onApply ? "cursor-pointer" : ""}`}
    >
      <div
        className={`relative flex aspect-video items-center justify-center overflow-hidden
                    rounded-lg bg-sunken ring-1 ring-hairline ${
                      onApply ? "transition-shadow group-hover:ring-hairline-strong" : ""
                    }`}
      >
        {children}
        {onApply ? (
          <span
            className="invisible absolute bottom-1 right-1 flex h-5 w-5 items-center
                       justify-center rounded bg-black/55 text-white group-hover:visible"
          >
            <Icon name="plus" size={11} />
          </span>
        ) : (
          <span
            className="absolute bottom-1 right-1 rounded bg-black/55 px-1 py-px font-technical
                       text-[10px] text-white"
          >
            {t("mediaBin.soon")}
          </span>
        )}
      </div>
      <span className="mt-1 block truncate text-[11px] leading-tight text-secondary">{label}</span>
    </li>
  );
}

function TransitionsPage({
  category,
  onApply,
}: {
  category: TransitionCategory;
  onApply: (transitionId: string) => void;
}) {
  const visible = TRANSITIONS.filter((transition) => transition.category === category);
  return (
    <ul className={`${CARD_GRID} pt-2`}>
      {visible.map((transition) => (
        <CatalogueCard
          key={transition.id}
          label={transition.label}
          blurb={transition.blurb}
          onApply={transition.implemented ? () => onApply(transition.id) : undefined}
        >
          <Icon name="transition" size={22} strokeWidth={1.5} className="text-tertiary" />
        </CatalogueCard>
      ))}
    </ul>
  );
}

/**
 * Rendered effect previews, one image per effect id, produced by
 * `scripts/generate-effect-previews.mjs` running each effect's real FFmpeg
 * chain over the chosen source still. Globbed so a missing render simply
 * falls back to the effect's gradient swatch - the catalogue never breaks
 * because a preview was not generated yet.
 */
const EFFECT_PREVIEWS: Record<string, string> = Object.fromEntries(
  Object.entries(
    import.meta.glob("../assets/effect-previews/*.{webp,jpg,png}", {
      eager: true,
      query: "?url",
      import: "default",
    }),
  ).map(([file, url]) => [file.replace(/^.*\/|\.\w+$/g, ""), url as string]),
);

function EffectsPage({
  category,
  onApply,
}: {
  category: EffectCategory;
  onApply: (effectId: string) => void;
}) {
  const visible = EFFECTS.filter((effect) => effect.category === category);
  return (
    <ul className={`${CARD_GRID} pt-2`}>
      {visible.map((effect) => {
        const preview = EFFECT_PREVIEWS[effect.id];
        return (
          <CatalogueCard
            key={effect.id}
            label={effect.label}
            blurb={effect.blurb}
            onApply={() => onApply(effect.id)}
          >
            {preview ? (
              <img
                src={preview}
                alt=""
                aria-hidden
                draggable={false}
                className="absolute inset-0 h-full w-full object-cover"
              />
            ) : (
              <>
                <span
                  className="absolute inset-0"
                  style={{ background: effect.swatch }}
                  aria-hidden
                />
                <Icon
                  name="sparkles"
                  size={20}
                  strokeWidth={1.5}
                  className="relative text-white/85"
                />
              </>
            )}
          </CatalogueCard>
        );
      })}
    </ul>
  );
}

// ── the templates page ───────────────────────────────────────────────────────

/**
 * The template gallery, in the library where CapCut keeps its own.
 *
 * Using a template means leaving this project for the fill flow on the launch
 * screen - a template makes a *new* project - so the card's verb hands off
 * rather than pretending to drag. Loaded fresh each time the tab opens, which
 * is what makes a just-saved template appear without any plumbing.
 */
function TemplatesPage({
  onUse,
  onSaveTemplate,
}: {
  onUse: (template: TemplateInfo) => void;
  onSaveTemplate: () => void;
}) {
  const [templates, setTemplates] = useState<TemplateInfo[] | null>(null);
  const { t, tp } = useLocale();

  useEffect(() => {
    let cancelled = false;
    void templateList()
      .then((list) => !cancelled && setTemplates(list))
      .catch(() => !cancelled && setTemplates([]));
    return () => {
      cancelled = true;
    };
  }, []);

  const remove = async (doomed: TemplateInfo) => {
    setTemplates((current) =>
      current ? current.filter((entry) => entry.path !== doomed.path) : current,
    );
    await templateDelete(doomed.path).catch(() => undefined);
  };

  return (
    <>
      <div className="px-2 pb-2 pt-2">
        <button
          type="button"
          onClick={onSaveTemplate}
          className="flex w-full cursor-pointer items-center justify-center gap-2 rounded-lg
                     border border-dashed border-hairline-strong px-3 py-2.5 text-xs text-secondary
                     transition-colors hover:border-accent hover:bg-hover hover:text-primary"
        >
          <Icon name="slot" size={14} />
          {t("mediaBin.saveAsTemplate")}
        </button>
      </div>

      {templates !== null && templates.length === 0 ? (
        <Empty icon={<Icon name="slot" size={28} strokeWidth={1.5} />}>
          {t("mediaBin.noTemplates")}
        </Empty>
      ) : (
        <ul className={CARD_GRID}>
          {(templates ?? []).map((template) => (
            <li key={template.path} className="group relative select-none">
              <button
                type="button"
                title={`${template.name}\n${tp("mediaBin.slotCount", template.slots.length)} · ${
                  template.width
                } x ${template.height}\n${t("mediaBin.useTemplateHint")}`}
                onClick={() => onUse(template)}
                className="w-full cursor-pointer text-left"
              >
                <TemplateThumb template={template} />
                <span className="mt-1 block truncate text-[11px] leading-tight text-secondary">
                  {template.name}
                </span>
                <span className="block font-technical text-[10px] text-tertiary">
                  {tp("mediaBin.slotCount", template.slots.length)}
                </span>
              </button>
              <button
                type="button"
                aria-label={t("mediaBin.deleteTemplate", { name: template.name })}
                title={t("mediaBin.deleteTemplateHint")}
                onClick={(event) => {
                  event.stopPropagation();
                  void remove(template);
                }}
                className="invisible absolute right-1 top-1 cursor-pointer rounded bg-black/55 p-1
                           text-white transition-colors hover:bg-danger group-hover:visible"
              >
                <Icon name="trash" size={10} />
              </button>
            </li>
          ))}
        </ul>
      )}
    </>
  );
}

// ── thumbnails ───────────────────────────────────────────────────────────────

/**
 * Each media kind's hue pair - the vivid mark and the darker same-hue ground
 * it sits on - matching the timeline's clip palette, so a card in the bin and
 * the clip it becomes read as the same thing.
 */
const KIND_TINTS: Record<MediaItem["kind"], { mark: string; body: string }> = {
  video: { mark: "var(--color-clip-video)", body: "var(--color-clip-video-body)" },
  audio: { mark: "var(--color-clip-audio)", body: "var(--color-clip-audio-body)" },
  image: { mark: "var(--color-clip-image)", body: "var(--color-clip-image-body)" },
};

/**
 * The bin preview for one item.
 *
 * Reuses exactly the artwork the timeline uses - the first frame of the cached
 * filmstrip for video, the cached peaks for audio - so nothing is decoded
 * twice and a clip looks the same in the bin as it does on a track.
 *
 * Artwork arrives asynchronously and deliberately does not cause a React
 * render, so this subscribes and repaints itself instead.
 */
function MediaThumb({ item, assets }: { item: MediaItem; assets: MediaAssets }) {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const [hasArt, setHasArt] = useState(false);

  useEffect(() => {
    const paint = () => {
      const canvas = canvasRef.current;
      const context = canvas?.getContext("2d");
      if (!canvas || !context) return;

      const ratio = window.devicePixelRatio || 1;
      const width = canvas.clientWidth;
      const height = canvas.clientHeight;
      if (width === 0 || height === 0) return;

      canvas.width = Math.round(width * ratio);
      canvas.height = Math.round(height * ratio);
      context.setTransform(ratio, 0, 0, ratio, 0, 0);
      context.clearRect(0, 0, width, height);

      // Stills and footage both draw from the strip cache; only audio differs.
      if (item.kind !== "audio") {
        const strip = assets.strips.get(item.id);
        const frames = assets.stripFrames.get(item.id);
        if (!strip || !frames) {
          setHasArt(false);
          return;
        }
        // Cover-fit the first frame: letterboxing a small thumbnail wastes
        // most of it, and a crop reads better than bars at this size.
        const tile = strip.width / frames;
        const scale = Math.max(width / tile, height / strip.height);
        context.imageSmoothingQuality = "high";
        const drawWidth = tile * scale;
        const drawHeight = strip.height * scale;
        context.drawImage(
          strip,
          0,
          0,
          tile,
          strip.height,
          (width - drawWidth) / 2,
          (height - drawHeight) / 2,
          drawWidth,
          drawHeight,
        );
        setHasArt(true);
        return;
      }

      const peaks = assets.peaks.get(item.id);
      if (!peaks) {
        setHasArt(false);
        return;
      }

      // The wave sits on its own darker orange rather than on the panel's
      // near-black - the same two-intensity pair the timeline clip uses.
      context.fillStyle = themeColor("clip-audio-body", "#33220e");
      context.fillRect(0, 0, width, height);

      // The whole file squeezed into the thumbnail, extremes per column so a
      // transient does not vanish at this scale.
      const centre = height / 2;
      const half = height / 2 - 1;
      const buckets = peaks.max.length;

      context.fillStyle = themeColor("clip-audio", "#f09e2f");
      context.beginPath();
      for (let column = 0; column < width; column += 1) {
        const from = Math.floor((column / width) * buckets);
        const to = Math.max(from + 1, Math.floor(((column + 1) / width) * buckets));

        let low = 0;
        let high = 0;
        for (let bucket = from; bucket < to && bucket < buckets; bucket += 1) {
          if (peaks.min[bucket] < low) low = peaks.min[bucket];
          if (peaks.max[bucket] > high) high = peaks.max[bucket];
        }

        const top = centre - high * half;
        context.rect(column, top, 1, Math.max(1, centre - low * half - top));
      }
      context.fill();
      setHasArt(true);
    };

    paint();
    return subscribeAssets(assets, paint);
  }, [assets, item]);

  return (
    <span
      className="relative block aspect-video w-full overflow-hidden"
      // The kind's darker hue as the ground, so a card without artwork yet is
      // already colour-coded rather than a black hole.
      style={{ background: KIND_TINTS[item.kind].body }}
    >
      <canvas ref={canvasRef} className="h-full w-full" />
      {!hasArt && (
        <span
          className="absolute inset-0 flex items-center justify-center"
          style={{ color: KIND_TINTS[item.kind].mark }}
        >
          <Icon name={item.kind === "video" ? "film" : item.kind === "image" ? "image" : "music"} size={14} />
        </span>
      )}
    </span>
  );
}
