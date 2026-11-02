// The theme switch: three icon buttons in a group (system, light, dark), the chosen one pressed, so
// what a click does is plain. The command palette's "Switch theme" presses the next one in turn.

import { Monitor, Moon, Sun } from "lucide-react";
import { useState } from "react";
import { getTheme, setTheme, type Theme } from "../../look/theme/theme";

const NEXT: Record<Theme, Theme> = { system: "light", light: "dark", dark: "system" };
const CHOICES: { theme: Theme; icon: typeof Sun; label: string }[] = [
  { theme: "system", icon: Monitor, label: "Follow the system theme" },
  { theme: "light", icon: Sun, label: "Light theme" },
  { theme: "dark", icon: Moon, label: "Dark theme" },
];

export function ThemeToggle() {
  const [theme, set] = useState<Theme>(getTheme);
  return (
    <fieldset
      aria-label="Theme"
      className="m-0 min-w-0 border-0 flex items-center gap-0.5 rounded-lg bg-well p-0.5"
    >
      {CHOICES.map(({ theme: t, icon: Icon, label }) => (
        <button
          key={t}
          type="button"
          data-command={NEXT[theme] === t ? "theme" : undefined}
          aria-label={label}
          aria-pressed={theme === t}
          title={label}
          onClick={() => {
            setTheme(t);
            set(t);
          }}
          className={`flex h-7 w-7 items-center justify-center rounded-md focus-visible:outline-2 focus-visible:outline-accent ${
            theme === t ? "bg-sheet text-text shadow-sm" : "text-muted hover:text-text"
          }`}
        >
          <Icon size={15} aria-hidden />
        </button>
      ))}
    </fieldset>
  );
}
