// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

import {
  EFFECTS,
  findEffect,
  findTransition,
  resolveEffectParams,
  type AppliedEffect,
  type ClipTransition,
} from "../lib/effects";
import type { Clip } from "../lib/editor";
import { useLocale } from "../lib/i18n";
import { HelpTip, Slider } from "./controls";
import { Icon, IconButton } from "./Icon";
import { Empty } from "./Panel";

/**
 * The Effects tab: the video sibling of the Filters tab.
 *
 * Effects are a list on the clip, applied in order, each with its own
 * parameters, bypass and remove - the exact shape FiltersPanel gives audio,
 * because the two are the same idea pointed at different senses. The browsing
 * library lives in the bin's Effects page; this panel manages what is already
 * on the clip, plus the transition on its cut.
 */
export function EffectsPanel({
  clip,
  hasPreceding,
  onChangeEffects,
  onChangeTransition,
  onCommit,
}: {
  clip: Clip | null;
  /** Whether a clip ends where this one starts - what a transition needs. */
  hasPreceding: boolean;
  onChangeEffects: (effects: AppliedEffect[]) => void;
  onChangeTransition: (transition: ClipTransition | undefined) => void;
  /** Ends the gesture: the accumulated change becomes one engine command. */
  onCommit: () => void;
}) {
  const { t } = useLocale();

  if (!clip || (clip.kind !== "video" && clip.kind !== "image")) {
    return (
      <Empty icon={<Icon name="sparkles" size={26} strokeWidth={1.5} />}>
        {t("effects.panel.empty")}
      </Empty>
    );
  }

  const applied = clip.videoEffects;
  const transition = clip.transitionIn;
  const transitionDefinition = transition ? findTransition(transition.id) : null;

  const commitEffects = (effects: AppliedEffect[]) => {
    onChangeEffects(effects);
    onCommit();
  };
  const add = (id: string) => commitEffects([...applied, { id, params: {} }]);
  const remove = (index: number) => commitEffects(applied.filter((_, at) => at !== index));
  const patch = (index: number, change: Partial<AppliedEffect>) =>
    commitEffects(applied.map((effect, at) => (at === index ? { ...effect, ...change } : effect)));
  // Live: parameters echo while the slider moves and commit on release.
  const setParam = (index: number, key: string, value: number) =>
    onChangeEffects(
      applied.map((effect, at) =>
        at === index ? { ...effect, params: { ...effect.params, [key]: value } } : effect,
      ),
    );
  const move = (index: number, by: number) => {
    const target = index + by;
    if (target < 0 || target >= applied.length) return;
    const next = [...applied];
    [next[index], next[target]] = [next[target], next[index]];
    commitEffects(next);
  };

  return (
    <div className="px-3 py-3">
      {transition && transitionDefinition && (
        <section className="mb-5">
          <h3 className="mb-2 flex items-center gap-1.5 text-[11px] font-semibold uppercase tracking-wider text-tertiary">
            {t("effects.panel.transition")}
            <HelpTip text={t("effects.panel.transitionHelp")} />
          </h3>
          <div className="rounded-lg bg-sunken p-2.5">
            <div className="mb-2 flex items-center gap-1">
              <span className="min-w-0 flex-1 truncate text-xs text-primary">
                {transitionDefinition.label}
              </span>
              <IconButton
                icon="close"
                label={t("effects.panel.remove", { name: transitionDefinition.label })}
                size={7}
                tone="danger"
                onClick={() => {
                  onChangeTransition(undefined);
                  onCommit();
                }}
              />
            </div>
            {!hasPreceding && (
              <p className="mb-2 text-[11px] leading-snug text-danger">
                {t("effects.panel.noPreceding")}
              </p>
            )}
            <Slider
              label={t("effects.panel.duration")}
              value={transition.duration}
              min={0.2}
              max={3}
              step={0.1}
              format={(value) => `${value.toFixed(1)}s`}
              onReset={() => {
                onChangeTransition({ ...transition, duration: transitionDefinition.defaultDuration });
                onCommit();
              }}
              onChange={(value) => onChangeTransition({ ...transition, duration: value })}
              onCommit={onCommit}
            />
          </div>
        </section>
      )}

      {applied.length > 0 && (
        <section className="mb-5">
          <h3 className="mb-2 flex items-center gap-1.5 text-[11px] font-semibold uppercase tracking-wider text-tertiary">
            {t("effects.panel.applied")}
            <HelpTip text={t("effects.panel.appliedHelp")} />
          </h3>

          <ul className="flex flex-col gap-2">
            {applied.map((entry, index) => {
              const definition = findEffect(entry.id);
              if (!definition) return null;
              const values = resolveEffectParams(definition, entry.params);
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
                          ? t("effects.panel.bypass", { name: definition.label })
                          : t("effects.panel.enable", { name: definition.label })
                      }
                      size={7}
                      onClick={() => patch(index, { enabled: !active })}
                    />
                    <IconButton
                      icon="chevronUp"
                      label={t("effects.panel.moveEarlier")}
                      size={7}
                      disabled={index === 0}
                      onClick={() => move(index, -1)}
                    />
                    <IconButton
                      icon="chevronDown"
                      label={t("effects.panel.moveLater")}
                      size={7}
                      disabled={index === applied.length - 1}
                      onClick={() => move(index, 1)}
                    />
                    <IconButton
                      icon="close"
                      label={t("effects.panel.remove", { name: definition.label })}
                      size={7}
                      tone="danger"
                      onClick={() => remove(index)}
                    />
                  </div>

                  {/* Dimmed but still editable while bypassed, so a setting
                      can be dialled in before the effect is switched on. */}
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
                    {definition.params.length === 0 && (
                      <p className="text-[11px] text-tertiary">{t("effects.panel.nothingToAdjust")}</p>
                    )}
                  </div>
                </li>
              );
            })}
          </ul>
        </section>
      )}

      <h3 className="mb-2 text-[11px] font-semibold uppercase tracking-wider text-tertiary">
        {t("effects.panel.addEffect")}
      </h3>
      <ul className="flex flex-col gap-0.5">
        {EFFECTS.map((effect) => (
          <li key={effect.id}>
            <button
              type="button"
              onClick={() => add(effect.id)}
              title={effect.blurb}
              className="flex w-full cursor-pointer items-center gap-2 rounded-lg px-2 py-1.5
                         text-left transition-colors hover:bg-hover"
            >
              <Icon name="plus" size={12} className="shrink-0 text-tertiary" />
              <span className="min-w-0 flex-1 truncate text-xs text-primary">{effect.label}</span>
            </button>
          </li>
        ))}
      </ul>
    </div>
  );
}
