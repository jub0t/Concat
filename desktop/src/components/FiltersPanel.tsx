// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

import { useState } from "react";

import {
  CATEGORIES,
  FILTERS,
  findFilter,
  resolveParams,
  type ClipFilter,
  type FilterCategory,
  type FilterDefinition,
} from "../lib/filters";
import type { Clip } from "../lib/editor";
import { useLocale } from "../lib/i18n";
import { HelpTip, Slider } from "./controls";
import { Icon, IconButton } from "./Icon";
import { Empty } from "./Panel";

/**
 * The Filters tab.
 *
 * Filters are a *list* on the clip, not a set of toggles, because they apply
 * in order and order is audible. Adding the same filter twice is allowed for
 * the same reason - two gentle EQ lifts are a real thing to want.
 *
 * Everything here writes plain numbers onto the clip. The FFmpeg chain is
 * built from those in one place (`lib/filters.ts`) and used by both the
 * preview and the exporter, so what you hear is what gets rendered - and a
 * bypassed filter drops out of both through the same single check.
 */
export function FiltersPanel({
  clip,
  onChange,
  onCommit,
}: {
  clip: Clip | null;
  onChange: (filters: ClipFilter[]) => void;
  /** Ends the gesture: the accumulated change becomes one engine command. */
  onCommit: () => void;
}) {
  const [category, setCategory] = useState<FilterCategory>("voice");
  const [query, setQuery] = useState("");
  const { t } = useLocale();

  if (!clip) {
    return (
      <Empty icon={<Icon name="settings" size={26} strokeWidth={1.5} />}>
        {t("filters.panel.empty")}
      </Empty>
    );
  }

  if (clip.kind === "image") {
    return (
      <Empty icon={<Icon name="image" size={26} strokeWidth={1.5} />}>
        {t("filters.panel.emptyImage")}
      </Empty>
    );
  }

  const applied = clip.filters;

  const commitChange = (filters: ClipFilter[]) => {
    onChange(filters);
    onCommit();
  };
  const add = (id: string) => commitChange([...applied, { id, params: {} }]);
  const remove = (index: number) => commitChange(applied.filter((_, at) => at !== index));
  const patch = (index: number, change: Partial<ClipFilter>) =>
    commitChange(applied.map((filter, at) => (at === index ? { ...filter, ...change } : filter)));
  // Live: parameters echo while the slider moves and commit on release.
  const setParam = (index: number, key: string, value: number) =>
    onChange(
      applied.map((filter, at) =>
        at === index ? { ...filter, params: { ...filter.params, [key]: value } } : filter,
      ),
    );
  const move = (index: number, by: number) => {
    const target = index + by;
    if (target < 0 || target >= applied.length) return;
    const next = [...applied];
    [next[index], next[target]] = [next[target], next[index]];
    commitChange(next);
  };

  // A search cuts across the categories: when you know the name you should
  // not have to remember which drawer it lives in.
  const needle = query.trim().toLowerCase();
  const matches = (filter: FilterDefinition) =>
    needle === ""
      ? filter.category === category
      : `${filter.label} ${filter.blurb}`.toLowerCase().includes(needle);
  const browsable = FILTERS.filter(matches);

  return (
    <div className="px-3 py-3">
      {applied.length > 0 && (
        <section className="mb-5">
          <h3 className="mb-2 flex items-center gap-1.5 text-[11px] font-semibold uppercase tracking-wider text-tertiary">
            {t("filters.panel.applied")}
            <HelpTip text={t("filters.panel.appliedHelp")} />
          </h3>

          <ul className="flex flex-col gap-2">
            {applied.map((entry, index) => {
              const definition = findFilter(entry.id);
              if (!definition) return null;
              const values = resolveParams(definition, entry.params);
              const active = entry.enabled !== false;

              return (
                <li key={`${entry.id}-${index}`} className="rounded-lg bg-sunken p-2.5">
                  <div className="mb-2 flex items-center gap-1">
                    <span
                      className={`min-w-0 flex-1 truncate text-xs ${
                        active ? "text-primary" : "text-tertiary line-through"
                      }`}
                    >
                      {definition.label}
                    </span>
                    <IconButton
                      icon={active ? "eye" : "eyeOff"}
                      label={
                        active
                          ? t("filters.panel.bypass", { name: definition.label })
                          : t("filters.panel.enable", { name: definition.label })
                      }
                      size={7}
                      onClick={() => patch(index, { enabled: !active })}
                    />
                    <IconButton
                      icon="chevronUp"
                      label={t("filters.panel.moveEarlier")}
                      size={7}
                      disabled={index === 0}
                      onClick={() => move(index, -1)}
                    />
                    <IconButton
                      icon="chevronDown"
                      label={t("filters.panel.moveLater")}
                      size={7}
                      disabled={index === applied.length - 1}
                      onClick={() => move(index, 1)}
                    />
                    <IconButton
                      icon="close"
                      label={t("filters.panel.remove", { name: definition.label })}
                      size={7}
                      tone="danger"
                      onClick={() => remove(index)}
                    />
                  </div>

                  {/* Dimmed but still editable while bypassed, so a setting
                      can be dialled in before the filter is switched on. */}
                  <div className={active ? "" : "opacity-50"}>
                    {definition.params.map((param) => (
                      <Slider
                        key={param.key}
                        label={param.label}
                        value={values[param.key]}
                        min={param.min}
                        max={param.max}
                        step={param.step}
                        format={param.format}
                        onReset={() => {
                          setParam(index, param.key, param.default);
                          onCommit();
                        }}
                        onChange={(value) => setParam(index, param.key, value)}
                        onCommit={onCommit}
                      />
                    ))}
                  </div>
                </li>
              );
            })}
          </ul>
        </section>
      )}

      <div className="mb-2 flex items-center gap-1.5 rounded-lg bg-sunken px-2">
        <Icon name="search" size={12} className="shrink-0 text-tertiary" />
        <input
          value={query}
          onChange={(event) => setQuery(event.target.value)}
          placeholder={t("filters.panel.searchPlaceholder")}
          className="h-7 w-full bg-transparent text-xs text-primary outline-none
                     placeholder:text-tertiary"
        />
        {query !== "" && (
          <button
            type="button"
            aria-label={t("filters.panel.clearSearch")}
            onClick={() => setQuery("")}
            className="cursor-pointer text-tertiary transition-colors hover:text-primary"
          >
            <Icon name="close" size={10} />
          </button>
        )}
      </div>

      {needle === "" && (
        <div className="mb-3 flex rounded-lg bg-sunken p-0.5">
          {CATEGORIES.map((entry) => (
            <button
              key={entry.id}
              type="button"
              aria-pressed={category === entry.id}
              onClick={() => setCategory(entry.id)}
              className={`flex-1 cursor-pointer rounded-md px-2 py-1 text-[12px] transition-colors ${
                category === entry.id
                  ? "bg-panel text-primary shadow-[0_1px_2px_rgba(0,0,0,0.14)]"
                  : "text-secondary hover:text-primary"
              }`}
            >
              {entry.label}
            </button>
          ))}
        </div>
      )}

      {browsable.length === 0 ? (
        <p className="px-2 py-3 text-center text-[11px] text-tertiary">
          {t("filters.panel.noMatches", { query: query.trim() })}
        </p>
      ) : (
        <ul className="flex flex-col gap-0.5">
          {browsable.map((filter) => (
            <li key={filter.id}>
              <button
                type="button"
                onClick={() => add(filter.id)}
                title={filter.blurb}
                className="flex w-full cursor-pointer items-center gap-2 rounded-lg px-2 py-1.5
                           text-left transition-colors hover:bg-hover"
              >
                <Icon name="plus" size={12} className="shrink-0 text-tertiary" />
                <span className="min-w-0 flex-1 truncate text-xs text-primary">{filter.label}</span>
                {needle !== "" && (
                  <span className="shrink-0 text-[10px] uppercase tracking-wider text-tertiary">
                    {CATEGORIES.find((entry) => entry.id === filter.category)?.label}
                  </span>
                )}
              </button>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
