// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

import { useEffect, useState } from "react";

import { templatePoster, type TemplateInfo } from "../lib/engine";
import { Icon } from "./Icon";

/**
 * A template's poster, saved into the bundle when the template was. Shared by
 * the launch screen's gallery and the bin's Templates tab, so a template looks
 * like itself everywhere. The same quiet fallback as project thumbnails: a
 * slot icon, never an error.
 */
export function TemplateThumb({ template }: { template: TemplateInfo }) {
  const [poster, setPoster] = useState<string | null>(null);

  useEffect(() => {
    if (!template.hasPoster) return;
    let url: string | null = null;
    let cancelled = false;
    void templatePoster(template.path)
      .then((bytes) => {
        if (cancelled) return;
        url = URL.createObjectURL(new Blob([bytes], { type: "image/jpeg" }));
        setPoster(url);
      })
      .catch(() => undefined);
    return () => {
      cancelled = true;
      if (url) URL.revokeObjectURL(url);
    };
  }, [template.path, template.hasPoster]);

  return (
    <span
      className="relative block aspect-video w-full overflow-hidden rounded-lg bg-sunken
                 ring-1 ring-hairline transition-shadow group-hover:ring-hairline-strong"
    >
      {poster ? (
        <img src={poster} alt="" draggable={false} className="h-full w-full object-cover" />
      ) : (
        <span className="absolute inset-0 flex items-center justify-center text-tertiary">
          <Icon name="slot" size={18} strokeWidth={1.5} />
        </span>
      )}
    </span>
  );
}
