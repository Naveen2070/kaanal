// The theme switch: one button that cycles system, light, dark. The icon shows the current choice
// and the label says what a click does.

import { Monitor, Moon, Sun } from "lucide-react";
import { useState } from "react";
import { getTheme, setTheme, type Theme } from "../theme/theme";

const NEXT: Record<Theme, Theme> = { system: "light", light: "dark", dark: "system" };
const ICON = { system: Monitor, light: Sun, dark: Moon };

export function ThemeToggle() {
  const [theme, set] = useState<Theme>(getTheme);
  const Icon = ICON[theme];
  const label = `Theme: ${theme}. Switch to ${NEXT[theme]}`;
  return (
    <button
      type="button"
      data-command="theme"
      aria-label={label}
      title={label}
      onClick={() => {
        setTheme(NEXT[theme]);
        set(NEXT[theme]);
      }}
      className="flex h-8 w-8 items-center justify-center rounded-lg text-muted hover:bg-raised focus-visible:outline-2 focus-visible:outline-accent"
    >
      <Icon size={16} aria-hidden />
    </button>
  );
}
