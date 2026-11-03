// The theme switch: one icon button that shows the current choice (system, light, dark) and moves to
// the next on a click, so the top right needs one icon, not three. The command palette's "Switch
// theme" presses the same button.

import { Monitor, Moon, Sun } from "lucide-react";
import { useState } from "react";
import { getTheme, setTheme, type Theme } from "../../look/theme/theme";

const NEXT: Record<Theme, Theme> = { system: "light", light: "dark", dark: "system" };
const NAME: Record<Theme, string> = { system: "the system theme", light: "light", dark: "dark" };
const ICON: Record<Theme, typeof Sun> = { system: Monitor, light: Sun, dark: Moon };

export function ThemeToggle() {
  const [theme, set] = useState<Theme>(getTheme);
  const Icon = ICON[theme];
  const label = `Theme: ${NAME[theme]}. Switch to ${NAME[NEXT[theme]]}`;
  return (
    <button
      type="button"
      data-command="theme"
      aria-label={label}
      title={label}
      onClick={() => {
        const next = NEXT[theme];
        setTheme(next);
        set(next);
      }}
      className="flex size-8 items-center justify-center rounded-lg text-muted hover:bg-raised hover:text-text focus-visible:outline-2 focus-visible:outline-accent"
    >
      <Icon size={16} aria-hidden />
    </button>
  );
}
