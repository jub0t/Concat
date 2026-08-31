import { memo, useEffect, useState } from "react";
import type { ReactNode } from "react";
import { isTauri } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";

import logo from "../assets/concat-logo.png";
import { useLocale } from "../lib/i18n";
import type { Theme } from "../lib/theme";
import { Icon } from "./Icon";
import { Menu, type MenuOption } from "./Menu";

/**
 * The custom title bar.
 *
 * On Windows the window is undecorated (`decorations: false`), so this strip
 * *is* the title bar: it carries the app menu, the project name and the window
 * controls, and everything not otherwise interactive is a drag region.
 *
 * On macOS the native traffic lights are overlaid instead
 * (`tauri.macos.conf.json` sets `titleBarStyle: "Overlay"`), because
 * Windows-style controls on a Mac read as "ported, carelessly". The strip
 * keeps everything else but drops its own window buttons and leaves room on
 * the left for the traffic lights.
 *
 * `data-tauri-drag-region` is what makes an area draggable, and Tauri also
 * gives it double-click-to-maximise for free. It has to be on the actual
 * element under the pointer - a parent with the attribute does not delegate -
 * which is why it appears on the spacers rather than only on the root.
 *
 * In a plain browser (`npm run dev` without the host) the window APIs are
 * absent, so the controls hide themselves rather than throwing.
 *
 * Memoised: the editor re-renders at animation rate during playback and
 * nothing up here depends on the playhead. App.tsx keeps `menus`, `actions`
 * and the callbacks referentially stable so this actually holds.
 */
export const TitleBar = memo(function TitleBar({
  projectName,
  menus,
  status,
  actions,
  theme,
  onToggleTheme,
  onOpenSettings,
}: {
  projectName: string;
  menus: { label: string; groups: MenuOption[][] }[];
  status?: string;
  /** Sits to the left of the window controls, CapCut-style. */
  actions?: ReactNode;
  theme: Theme;
  onToggleTheme: () => void;
  /** Shows the settings gear beside the theme toggle when provided - the
   * visible way in; the File menu entry remains for menu-first users. */
  onOpenSettings?: () => void;
}) {
  const [maximized, setMaximized] = useState(false);
  const { t } = useLocale();
  const native = isTauri();
  const macOS = navigator.userAgent.includes("Mac");

  useEffect(() => {
    if (!native) return;
    const appWindow = getCurrentWindow();

    void appWindow.isMaximized().then(setMaximized);
    // The window can also be maximised by dragging it to the top edge or by
    // the double-click Tauri handles itself, so poll the real state on resize
    // rather than tracking our own button presses.
    const unlisten = appWindow.onResized(() => {
      void appWindow.isMaximized().then(setMaximized);
    });

    return () => {
      void unlisten.then((stop) => stop());
    };
  }, [native]);

  const control = async (action: "minimize" | "toggle" | "close") => {
    if (!native) return;
    const appWindow = getCurrentWindow();
    if (action === "minimize") await appWindow.minimize();
    if (action === "toggle") await appWindow.toggleMaximize();
    if (action === "close") await appWindow.close();
  };

  return (
    <header
      data-tauri-drag-region
      className={`flex h-8 shrink-0 items-center gap-1 border-b border-hairline bg-panel ${
        macOS ? "pl-20 pr-1" : "pl-2"
      }`}
    >
      <span className="flex items-center gap-1.5 pr-1" data-tauri-drag-region>
        <img src={logo} alt="" className="pointer-events-none h-4 w-4" draggable={false} />
        <span className="font-display text-[13px] font-bold tracking-tight text-primary">WolfCut</span>
      </span>

      <nav className="flex items-center">
        {menus.map((menu) => (
          <Menu
            key={menu.label}
            groups={menu.groups}
            trigger={(open) => (
              <span
                className={`rounded-md px-2 py-1 text-xs transition-colors ${
                  open ? "bg-active text-primary" : "text-secondary hover:bg-hover"
                }`}
              >
                {menu.label}
              </span>
            )}
          />
        ))}
      </nav>

      {/* Centre: the project name, absolutely placed so it stays centred in the
          window rather than centred in whatever space the menus leave. */}
      <div
        data-tauri-drag-region
        className="pointer-events-none absolute left-1/2 flex -translate-x-1/2 items-center gap-2"
      >
        <span className="text-xs text-secondary">{projectName}</span>
        {status && <span className="font-technical text-[10px] text-tertiary">{status}</span>}
      </div>

      <span className="h-full flex-1" data-tauri-drag-region />

      {onOpenSettings && (
        <button
          type="button"
          title={t("settings.title")}
          aria-label={t("titleBar.openSettings")}
          onClick={onOpenSettings}
          className="flex h-6 w-6 cursor-pointer items-center justify-center rounded-md
                     text-secondary transition-colors hover:bg-hover hover:text-primary"
        >
          <Icon name="settings" size={14} />
        </button>
      )}

      <button
        type="button"
        title={theme === "dark" ? t("titleBar.switchLight") : t("titleBar.switchDark")}
        aria-label={
          theme === "dark" ? t("titleBar.switchLightAria") : t("titleBar.switchDarkAria")
        }
        onClick={onToggleTheme}
        className="mr-1 flex h-6 w-6 cursor-pointer items-center justify-center rounded-md
                   text-secondary transition-colors hover:bg-hover hover:text-primary"
      >
        <Icon name={theme === "dark" ? "sun" : "moon"} size={14} />
      </button>

      {actions && <div className="flex items-center gap-1 pr-2">{actions}</div>}

      {native && !macOS && (
        <div className="flex h-full items-stretch">
          <WindowButton label={t("titleBar.minimise")} onClick={() => void control("minimize")}>
            <Icon name="winMinimize" size={14} strokeWidth={1.5} />
          </WindowButton>
          <WindowButton
            label={maximized ? t("titleBar.restore") : t("titleBar.maximise")}
            onClick={() => void control("toggle")}
          >
            <Icon name={maximized ? "winRestore" : "winMaximize"} size={13} strokeWidth={1.5} />
          </WindowButton>
          <WindowButton label={t("common.close")} onClick={() => void control("close")} danger>
            <Icon name="close" size={15} strokeWidth={1.5} />
          </WindowButton>
        </div>
      )}
    </header>
  );
});

/**
 * Window controls are wider than they are tall and reach the very top edge, so
 * that throwing the pointer at the corner hits Close. They are not `IconButton`
 * for that reason - the rounded 36px square would break the corner target.
 */
function WindowButton({
  label,
  onClick,
  danger = false,
  children,
}: {
  label: string;
  onClick: () => void;
  danger?: boolean;
  children: ReactNode;
}) {
  return (
    <button
      type="button"
      title={label}
      aria-label={label}
      onClick={onClick}
      className={`flex w-11 cursor-pointer items-center justify-center text-secondary transition-colors
                  ${danger ? "hover:bg-danger hover:text-on-accent" : "hover:bg-hover hover:text-primary"}`}
    >
      {children}
    </button>
  );
}
